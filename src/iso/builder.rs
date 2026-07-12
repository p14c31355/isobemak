use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

use crate::fat;
use crate::iso::boot_catalog::BootCatalogEntry;
use crate::iso::boot_catalog::LBA_BOOT_CATALOG;
use crate::iso::boot_info::BootInfo;
use crate::iso::builder_utils::{
    calculate_lbas, create_bios_boot_entry, create_uefi_boot_entry, create_uefi_esp_boot_entry,
    ensure_directory_path, get_file_metadata, get_file_size_in_iso, get_lba_for_path,
};
use crate::iso::constants::{BACKUP_GPT_RESERVED_512, ISO_SECTOR_SIZE};
use crate::iso::disk_layout::DiskLayout;
use crate::iso::fs_node::{IsoDirectory, IsoFile, IsoFsNode};
use crate::iso::gpt::main_gpt_functions::write_gpt_structures;
use crate::iso::gpt::partition_entry::{EFI_SYSTEM_PARTITION_GUID, GptPartitionEntry};
use crate::iso::iso_image::IsoImage;
use crate::iso::iso_writer::{
    copy_files, finalize_iso, write_boot_catalog_to_iso, write_boot_info_table, write_descriptors,
    write_directories,
};
use crate::iso::layout_profile::{HiddenSectorMode, IsoLayoutProfile};
use crate::iso::mbr::create_mbr_for_gpt_hybrid;
use crate::iso::volume_descriptor::update_total_sectors_in_pvd;

pub struct IsoBuilder {
    volume_id: Option<String>,
    root: IsoDirectory,
    boot_info: Option<BootInfo>,
    iso_data_lba: u32,
    total_sectors: u32,
    is_isohybrid: bool,
    uefi_catalog_path: Option<String>,
    pub esp_lba: Option<u32>,
    pub esp_size_sectors: Option<u32>,
    profile: IsoLayoutProfile,
    disk_layout: Option<DiskLayout>,
    efi_boot_image_iso_path: Option<String>,
}

impl Default for IsoBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl IsoBuilder {
    pub fn new() -> Self {
        Self {
            volume_id: None,
            root: IsoDirectory::new(),
            boot_info: None,
            iso_data_lba: 0,
            total_sectors: 0,
            is_isohybrid: false,
            uefi_catalog_path: None,
            esp_lba: None,
            esp_size_sectors: None,
            profile: IsoLayoutProfile::default(),
            disk_layout: None,
            efi_boot_image_iso_path: None,
        }
    }

    pub fn set_volume_id(&mut self, v: Option<String>) {
        self.volume_id = v;
    }

    pub fn add_file(&mut self, path_in_iso: &str, real_path: &Path) -> io::Result<()> {
        let file_name = Path::new(path_in_iso)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name"))?
            .to_string();
        let current_dir = ensure_directory_path(&mut self.root, path_in_iso)?;
        let sz = get_file_metadata(real_path)?.len();
        current_dir.children.insert(
            file_name,
            IsoFsNode::File(IsoFile {
                path: real_path.to_path_buf(),
                size: sz,
                lba: 0,
            }),
        );
        Ok(())
    }

    pub fn set_boot_info(&mut self, bi: BootInfo) {
        self.boot_info = Some(bi);
    }
    pub fn set_profile(&mut self, p: IsoLayoutProfile) {
        self.profile = p;
    }
    pub fn set_isohybrid(&mut self, v: bool) {
        self.is_isohybrid = v;
    }
    pub fn set_disk_layout(&mut self, l: DiskLayout) {
        self.disk_layout = Some(l);
    }

    fn prepare_boot_entries(
        &self,
        esp_lba: Option<u32>,
        esp_size_sectors: Option<u32>,
    ) -> io::Result<Vec<BootCatalogEntry>> {
        use crate::iso::boot_catalog::{BOOT_CATALOG_EFI_PLATFORM_ID, BootCatalogEntryType};
        let mut entries = Vec::new();
        let bi = self.boot_info.as_ref();

        let bios_boot_info = bi.and_then(|b| b.bios_boot.as_ref());
        let uefi_boot_info = bi.and_then(|b| b.uefi_boot.as_ref());

        // Validate ESP parameters (always, not only when UEFI boot is requested)
        match (esp_lba, esp_size_sectors) {
            (Some(_), None) | (None, Some(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid ESP configuration: esp_lba and esp_size_sectors must both be Some or both be None",
                ));
            }
            (Some(_), Some(0)) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid ESP configuration: esp_size_sectors cannot be zero when esp_lba is provided",
                ));
            }
            _ => {}
        }

        // Determine effective UEFI LBA/size
        let (has_uefi, uefi_lba, uefi_size_sectors) =
            if let (Some(lba), Some(size)) = (esp_lba, esp_size_sectors) {
                if size > 0 {
                    (true, lba, size)
                } else {
                    (false, 0, 0)
                }
            } else {
                (false, 0, 0)
            };

        // --- BIOS as Initial/Default Entry (if present) ---
        // SeaBIOS only checks the Initial/Default Entry; if its platform_id
        // is 0xEF (UEFI), SeaBIOS skips BIOS boot entirely.  Placing BIOS
        // here ensures it can boot on legacy firmware while UEFI firmware
        // discovers the EFI entries via the Section Header with
        // platform_id=0xEF.
        if let Some(bios) = bios_boot_info {
            entries.push(create_bios_boot_entry(
                &self.root,
                &bios.destination_in_iso,
            )?);

            // UEFI entries follow under a dedicated Section Header
            if has_uefi {
                entries.push(BootCatalogEntry {
                    platform_id: BOOT_CATALOG_EFI_PLATFORM_ID,
                    boot_image_lba: 0,
                    boot_image_sectors: 0,
                    entry_type: BootCatalogEntryType::SectionHeader { more_follow: false },
                });
                entries.push(create_uefi_esp_boot_entry(uefi_lba, uefi_size_sectors)?);
            } else if let Some(u) = uefi_boot_info {
                // BIOS + non-isohybrid UEFI: UEFI entry under a Section Header
                entries.push(BootCatalogEntry {
                    platform_id: BOOT_CATALOG_EFI_PLATFORM_ID,
                    boot_image_lba: 0,
                    boot_image_sectors: 0,
                    entry_type: BootCatalogEntryType::SectionHeader { more_follow: false },
                });
                entries.push(create_uefi_boot_entry(&self.root, &u.destination_in_iso)?);
            }
        } else {
            // UEFI-only boot: UEFI BootEntry is the Initial/Default Entry.
            // El Torito spec requires offset 32 to be a BootEntry, NOT a
            // SectionHeader.  A Section Header follows for firmware that
            // requires platform_id=0xEF to discover the entry.
            if has_uefi {
                // Initial / Default entry: sector_count MUST be 0 for
                // no-emulation boot according to El Torito spec § 6.4.
                entries.push(BootCatalogEntry {
                    platform_id: BOOT_CATALOG_EFI_PLATFORM_ID,
                    boot_image_lba: uefi_lba,
                    boot_image_sectors: 0,
                    entry_type: BootCatalogEntryType::BootEntry { bootable: true },
                });
                entries.push(BootCatalogEntry {
                    platform_id: BOOT_CATALOG_EFI_PLATFORM_ID,
                    boot_image_lba: 0,
                    boot_image_sectors: 0,
                    entry_type: BootCatalogEntryType::SectionHeader { more_follow: false },
                });
                entries.push(create_uefi_esp_boot_entry(uefi_lba, uefi_size_sectors)?);
            } else if let Some(u) = uefi_boot_info {
                entries.push(create_uefi_boot_entry(&self.root, &u.destination_in_iso)?);
            }
        }
        Ok(entries)
    }

    fn write_hybrid_structures(
        &self,
        iso_file: &mut File,
        total_lbas: u64,
        esp_size_sectors: Option<u32>,
    ) -> io::Result<()> {
        let raw_512 = total_lbas
            .checked_mul(4)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "ISO too large"))?;
        let total_512 = ((raw_512 + BACKUP_GPT_RESERVED_512) + 3) & !3u64;
        let total_for_mbr = u32::try_from(total_512)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "ISO too large for MBR"))?;

        let (esp_start_512, esp_size_512) =
            if let (Some(l), Some(s)) = (self.esp_lba, self.esp_size_sectors) {
                (
                    u32::try_from(l as u64 * 4).ok(),
                    u32::try_from(s as u64 * 4).ok(),
                )
            } else if let Some(ref layout) = self.disk_layout {
                layout.esp_partition().map_or((None, None), |esp| {
                    (
                        Some(esp.start_lba_512 as u32),
                        Some(esp.size_lba_512 as u32),
                    )
                })
            } else if let Some(sz) = esp_size_sectors {
                (Some(self.profile.esp_alignment_lba_512), Some(sz * 4))
            } else {
                (None, None)
            };

        iso_file.seek(SeekFrom::Start(0))?;
        if self.profile.use_gpt {
            create_mbr_for_gpt_hybrid(
                total_for_mbr,
                self.is_isohybrid,
                esp_start_512,
                esp_size_512,
            )?
            .write_to(iso_file)?;

            let mut parts = Vec::new();
            let start: u64 = 34;
            let end: u64 = total_512.saturating_sub(34);
            if end > start {
                parts.push(GptPartitionEntry::new(
                    "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7",
                    &uuid::Uuid::new_v4().to_string(),
                    start,
                    end,
                    "ISO9660",
                    0,
                ));
            }
            if let (Some(s), Some(sz)) = (esp_start_512, esp_size_512) {
                let e = s.saturating_add(sz).saturating_sub(1);
                if e > s {
                    parts.push(GptPartitionEntry::new(
                        EFI_SYSTEM_PARTITION_GUID,
                        &uuid::Uuid::new_v4().to_string(),
                        s as u64,
                        e as u64,
                        "EFI System Partition",
                        1,
                    ));
                }
            }
            if !parts.is_empty() {
                write_gpt_structures(iso_file, total_512, &parts)?;
            }
        }
        iso_file.sync_data()?;
        Ok(())
    }

    pub fn build(
        &mut self,
        iso_file: &mut File,
        _iso_path: &Path,
        esp_lba: Option<u32>,
        esp_size_sectors: Option<u32>,
    ) -> io::Result<()> {
        self.esp_lba = esp_lba;
        self.esp_size_sectors = esp_size_sectors;

        self.iso_data_lba = self
            .disk_layout
            .as_ref()
            .map_or(LBA_BOOT_CATALOG + 1, |l| l.iso_region.data_start_lba);
        iso_file.seek(SeekFrom::Start(self.iso_data_lba as u64 * ISO_SECTOR_SIZE))?;
        calculate_lbas(&mut self.iso_data_lba, &mut self.root)?;

        let (resolved_lba, resolved_size) = if let Some(ref ip) = self.efi_boot_image_iso_path {
            (
                Some(get_lba_for_path(&self.root, ip)?),
                Some(get_file_size_in_iso(&self.root, ip)?.div_ceil(ISO_SECTOR_SIZE) as u32),
            )
        } else {
            (esp_lba, esp_size_sectors)
        };
        self.esp_lba = resolved_lba;
        self.esp_size_sectors = resolved_size;

        write_descriptors(
            iso_file,
            self.volume_id.as_deref(),
            self.root.lba,
            self.iso_data_lba,
        )?;
        write_boot_catalog_to_iso(
            iso_file,
            LBA_BOOT_CATALOG,
            self.prepare_boot_entries(resolved_lba, resolved_size)?,
        )?;
        write_directories(iso_file, &self.root, self.root.lba)?;
        copy_files(iso_file, &self.root)?;

        // Capture the exact end of the newly written ISO data *before*
        // patching the boot information table (which seeks back into the
        // data stream).  Using this saved position in the seek below is
        // more robust than SeekFrom::End(0) because it does not depend on
        // whether the underlying file was truncated before being passed in.
        let end_of_data = iso_file.stream_position()?;

        if let Some(bi) = &self.boot_info
            && let Some(bios) = &bi.bios_boot
        {
            let lba = get_lba_for_path(&self.root, &bios.destination_in_iso)?;
            let size = get_file_size_in_iso(&self.root, &bios.destination_in_iso)?;
            write_boot_info_table(iso_file, lba, size)?;
        }

        // Seek back to the saved end-of-data position so finalize_iso can
        // compute the correct total sector count.
        iso_file.seek(SeekFrom::Start(end_of_data))?;

        finalize_iso(iso_file, &mut self.total_sectors)?;

        if self.is_isohybrid {
            self.write_hybrid_structures(iso_file, self.total_sectors as u64, esp_size_sectors)?;
            let pos = iso_file.seek(SeekFrom::End(0))?;
            let rem = pos % ISO_SECTOR_SIZE;
            if rem != 0 {
                io::copy(&mut io::repeat(0).take(ISO_SECTOR_SIZE - rem), iso_file)?;
            }
            let total = u32::try_from(iso_file.seek(SeekFrom::End(0))?.div_ceil(ISO_SECTOR_SIZE))
                .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "ISO too large after GPT backup",
                )
            })?;
            update_total_sectors_in_pvd(iso_file, total)?;
            self.total_sectors = total;
        }
        Ok(())
    }
}

pub fn build_iso(
    iso_path: &Path,
    image: &IsoImage,
    is_isohybrid: bool,
) -> io::Result<(PathBuf, Option<NamedTempFile>, File, Option<u32>)> {
    let mut b = IsoBuilder::new();
    b.set_profile(image.layout_profile.clone());
    b.set_volume_id(image.volume_id.clone());
    b.set_isohybrid(is_isohybrid);

    let mut fat_holder: Option<NamedTempFile> = None;
    let mut _grub_holder: Option<NamedTempFile> = None;
    let mut fat_size_512: Option<u32> = None;
    let mut iso_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(iso_path)?;

    if let Some(uefi) = &image.boot_info.uefi_boot {
        b.uefi_catalog_path = Some(uefi.destination_in_iso.clone());
        if is_isohybrid {
            let tf = NamedTempFile::new()?;
            let p = tf.path().to_path_buf();
            fat_holder = Some(tf);

            let mut ff: Vec<(&str, &Path)> = vec![
                ("BOOTX64.EFI", uefi.boot_image.as_path()),
                ("KERNEL.EFI", uefi.kernel_image.as_path()),
            ];
            for (dn, sp) in &uefi.additional_efi_boot_files {
                ff.push((dn, sp));
            }
            let _grub_path: Option<PathBuf>;
            if let Some(cfg) = &uefi.grub_cfg_content {
                let mut t = NamedTempFile::new()?;
                write!(t, "{}", cfg)?;
                _grub_path = Some(t.path().to_path_buf());
                _grub_holder = Some(t);
                ff.push(("grub.cfg", _grub_path.as_ref().unwrap()));
            }
            let hidden = match b.profile.hidden_sectors_mode {
                HiddenSectorMode::Zero => 0,
                HiddenSectorMode::PartitionOffset => b.profile.esp_alignment_lba_512,
            };
            fat_size_512 = Some(fat::create_fat_image(&p, &ff, hidden)?);
            b.efi_boot_image_iso_path = Some("boot/efiboot.img".into());
            b.add_file("boot/efiboot.img", &p)?;
        }
    }

    for f in &image.files {
        b.add_file(&f.destination, &f.source)?;
    }
    if let Some(bios) = &image.boot_info.bios_boot {
        b.add_file(&bios.destination_in_iso, &bios.boot_image)?;
    }
    b.set_boot_info(image.boot_info.clone());
    b.build(&mut iso_file, iso_path, b.esp_lba, b.esp_size_sectors)?;
    Ok((iso_path.to_path_buf(), fat_holder, iso_file, fat_size_512))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_add_file() -> io::Result<()> {
        let mut builder = IsoBuilder::new();
        let tp = NamedTempFile::new()?.into_temp_path();
        builder.add_file("root.txt", &tp)?;
        assert!(builder.root.children.contains_key("root.txt"));
        builder.add_file("dir1/nested.txt", &tp)?;
        match builder.root.children.get("dir1") {
            Some(IsoFsNode::Directory(d)) => assert!(d.children.contains_key("nested.txt")),
            _ => panic!(),
        };
        Ok(())
    }

    #[test]
    fn test_calculate_lbas() -> io::Result<()> {
        let mut root = IsoDirectory::new();
        let mut lba = 20;
        let mut subdir = IsoDirectory::new();
        subdir.children.insert(
            "file2.txt".into(),
            IsoFsNode::File(IsoFile {
                path: PathBuf::new(),
                size: 3000,
                lba: 0,
            }),
        );
        root.children.insert(
            "file1.txt".into(),
            IsoFsNode::File(IsoFile {
                path: PathBuf::new(),
                size: 1000,
                lba: 0,
            }),
        );
        root.children
            .insert("subdir".into(), IsoFsNode::Directory(subdir));
        calculate_lbas(&mut lba, &mut root)?;
        assert_eq!(root.lba, 20);
        assert_eq!(
            root.children
                .get("file1.txt")
                .and_then(|n| if let IsoFsNode::File(f) = n {
                    Some(f.lba)
                } else {
                    None
                }),
            Some(21)
        );
        let (sl, fl) = match root.children.get("subdir") {
            Some(IsoFsNode::Directory(d)) => (
                d.lba,
                d.children.get("file2.txt").and_then(|n| {
                    if let IsoFsNode::File(f) = n {
                        Some(f.lba)
                    } else {
                        None
                    }
                }),
            ),
            _ => panic!(),
        };
        assert_eq!(sl, 22);
        assert_eq!(fl, Some(23));
        assert_eq!(lba, 25);
        Ok(())
    }

    #[test]
    fn test_get_path_helpers() -> io::Result<()> {
        let mut builder = IsoBuilder::new();
        let mut tf = NamedTempFile::new()?;
        tf.write_all(b"some data")?;
        let tp = tf.into_temp_path();
        builder.add_file("A/B/C.txt", &tp)?;
        builder.iso_data_lba = 20;
        calculate_lbas(&mut builder.iso_data_lba, &mut builder.root)?;
        assert_eq!(get_lba_for_path(&builder.root, "A/B/C.txt")?, 23);
        assert_eq!(get_file_size_in_iso(&builder.root, "A/B/C.txt")?, 9);
        assert!(get_lba_for_path(&builder.root, "A/D.txt").is_err());
        Ok(())
    }
}
