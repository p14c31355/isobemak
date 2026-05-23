use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::fat;
use crate::iso::constants::{BACKUP_GPT_RESERVED_512, ESP_START_LBA_ISO, ESP_START_LBA_512};
use crate::utils::ISO_SECTOR_SIZE;

// Import definitions from new modules
use crate::iso::boot_catalog::BootCatalogEntry;
use crate::iso::boot_info::BootInfo;
use crate::iso::builder_utils::{
    calculate_lbas, create_bios_boot_entry, create_uefi_boot_entry, create_uefi_esp_boot_entry,
    ensure_directory_path, get_file_metadata,
};
use crate::iso::fs_node::{IsoDirectory, IsoFile, IsoFsNode};
use crate::iso::gpt::main_gpt_functions::write_gpt_structures;
use crate::iso::gpt::partition_entry::{EFI_SYSTEM_PARTITION_GUID, GptPartitionEntry};
use crate::iso::iso_image::IsoImage;
use crate::iso::iso_writer::{
    copy_files, finalize_iso, write_boot_catalog_to_iso, write_descriptors, write_directories,
};
use crate::iso::mbr::create_mbr_for_gpt_hybrid; // Import specific function

/// The main builder for creating an ISO 9660 image.
pub struct IsoBuilder {
    volume_id: Option<String>,
    root: IsoDirectory,
    boot_info: Option<BootInfo>,
    iso_data_lba: u32,     // LBA where ISO9660 filesystem data starts (QEMU/El Torito path)
    total_sectors: u32,
    is_isohybrid: bool,
    uefi_catalog_path: Option<String>,
    esp_lba: Option<u32>,          // LBA of the EFI System Partition image (in ISO 2048-byte sectors)
    esp_size_sectors: Option<u32>, // Size of the EFI System Partition image in ISO sectors
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
            is_isohybrid: false, // Initialize new field
            uefi_catalog_path: None,
            esp_lba: None,
            esp_size_sectors: None,
        }
    }

    pub fn set_volume_id(&mut self, volume_id: Option<String>) {
        self.volume_id = volume_id;
    }

    /// Adds a file to the ISO filesystem tree.
    pub fn add_file(&mut self, path_in_iso: &str, real_path: &Path) -> io::Result<()> {
        let file_name = Path::new(path_in_iso)
            .file_name()
            .ok_or_else(|| io_error!(io::ErrorKind::InvalidInput, "Invalid file name"))?
            .to_str()
            .ok_or_else(|| io_error!(io::ErrorKind::InvalidInput, "Invalid file name"))?
            .to_string();

        let current_dir = ensure_directory_path(&mut self.root, path_in_iso)?;

        let file_metadata = get_file_metadata(real_path)?;
        let file_size = file_metadata.len();

        let file = IsoFile {
            path: real_path.to_path_buf(),
            size: file_size,
            lba: 0,
        };

        current_dir
            .children
            .insert(file_name, IsoFsNode::File(file));

        Ok(())
    }

    /// Sets the boot information for the ISO.
    pub fn set_boot_info(&mut self, boot_info: BootInfo) {
        self.boot_info = Some(boot_info);
    }

    /// Sets whether the ISO should be built as an isohybrid image.
    pub fn set_isohybrid(&mut self, is_isohybrid: bool) {
        self.is_isohybrid = is_isohybrid;
    }

    /// Prepares the list of boot catalog entries based on boot configuration.
    fn prepare_boot_entries(
        &self,
        esp_lba: Option<u32>,
        esp_size_sectors: Option<u32>,
    ) -> io::Result<Vec<BootCatalogEntry>> {
        let mut entries = Vec::new();
        let bi = self.boot_info.as_ref();

        // UEFI ESP boot entry (FAT image as a whole)
        // OVMF and many real UEFI firmwares use El Torito to locate the
        // boot image on CD-ROM.  The ESP FAT image must be listed as a
        // no‑emulation boot entry so the firmware can mount it as a FAT
        // filesystem and find BOOTX64.EFI inside.
        if let (Some(lba), Some(size)) = (esp_lba, esp_size_sectors)
            && size > 0
        {
            entries.push(create_uefi_esp_boot_entry(lba, size)?);
        }

        // UEFI direct-file entry (points to BOOTX64.EFI inside ISO9660)
        // Some firmwares prefer the FAT image entry above, others may
        // fall back to a direct file entry.  Keep it as a secondary
        // non‑bootable entry so the catalog remains valid.
        if let Some(u) = bi.and_then(|b| b.uefi_boot.as_ref()) {
            let mut entry = create_uefi_boot_entry(&self.root, &u.destination_in_iso)?;
            entry.bootable = false; // ESP FAT image entry is the primary boot target
            entries.push(entry);
        }

        // BIOS boot entry
        if let Some(b) = bi.and_then(|b| b.bios_boot.as_ref()) {
            entries.push(create_bios_boot_entry(&self.root, &b.destination_in_iso)?);
        }

        Ok(entries)
    }

    /// Writes MBR and GPT structures for hybrid ISOs.
    fn write_hybrid_structures(
        &self,
        iso_file: &mut File,
        total_lbas: u64,
        esp_size_sectors: Option<u32>,
    ) -> io::Result<()> {
        // total_lbas is in 2048-byte ISO sectors → convert to 512-byte sectors
        let total_512_sectors = total_lbas
            .checked_mul(4)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "ISO image too large: total_lbas * 4 overflows u64 ({} * 4)",
                        total_lbas
                    ),
                )
            })?;

        if total_512_sectors < 69 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "ISO image is too small for isohybrid with GPT ({} sectors, requires at least 69)",
                    total_512_sectors
                ),
            ));
        }

        // Check that ESP does not overlap with backup GPT region.
        // backup GPT = 33 sectors at the end (1 header + 32 partition entries).
        if let Some(esp_size) = esp_size_sectors
            && esp_size > 0
        {
            let esp_start_512 = ESP_START_LBA_512 as u64;
            let esp_size_512 = (esp_size as u64) * 4;
            let esp_partition_end_lba = esp_start_512 + esp_size_512 - 1;
            let backup_gpt_start_lba = total_512_sectors.saturating_sub(BACKUP_GPT_RESERVED_512);

            if esp_partition_end_lba >= backup_gpt_start_lba {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "ESP (LBA {}-{}) overlaps with backup GPT region (starts at LBA {}); \
                         increase total image size or reduce ESP size",
                        esp_start_512, esp_partition_end_lba, backup_gpt_start_lba
                    ),
                ));
            }
        }

        // Write MBR (MBR uses 512-byte sector LBA values).
        // total_512_sectors is already validated; for MBR cap at u32::MAX
        // but fail explicitly if information loss would occur.
        let total_for_mbr = if total_512_sectors <= u32::MAX as u64 {
            total_512_sectors as u32
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "ISO image too large for MBR ({} 512-byte sectors > u32::MAX)",
                    total_512_sectors
                ),
            ));
        };

        iso_file.seek(SeekFrom::Start(0))?;
        let (esp_start_512, esp_size_512) = if let Some(esp_size) = esp_size_sectors {
            (Some(ESP_START_LBA_512), Some(esp_size * 4))
        } else {
            (None, None)
        };
        let mbr = create_mbr_for_gpt_hybrid(
            total_for_mbr,
            self.is_isohybrid,
            esp_start_512,
            esp_size_512,
        )?;
        mbr.write_to(iso_file)?;

        // Write GPT structures if esp_size_sectors > 0
        if let Some(esp_size_sectors_val) = esp_size_sectors
            && esp_size_sectors_val > 0
        {
            // ESP position in 512-byte sector units (direct from ESP_START_LBA_512)
            let esp_start_512 = ESP_START_LBA_512 as u64;
            let esp_size_512 = (esp_size_sectors_val as u64) * 4;
            let esp_partition_end_lba = esp_start_512 + esp_size_512 - 1;

            let esp_guid_str = EFI_SYSTEM_PARTITION_GUID;
            let esp_unique_guid_str = Uuid::new_v4().to_string();
            let partitions = vec![GptPartitionEntry::new(
                esp_guid_str,
                &esp_unique_guid_str,
                esp_start_512,
                esp_partition_end_lba,
                "EFI System Partition",
                0x0000000000000002, // EFI_PART_SYSTEM_PARTITION_ATTR_PLATFORM_REQUIRED
            )];
            write_gpt_structures(iso_file, total_512_sectors, &partitions)?;
            iso_file.sync_data()?;
        }

        Ok(())
    }

    /// Builds the ISO file based on the configured files and boot information.
    pub fn build(
        &mut self,
        iso_file: &mut File,
        _iso_path: &Path,
        esp_lba: Option<u32>,
        esp_size_sectors: Option<u32>,
    ) -> io::Result<()> {
        self.esp_lba = esp_lba;
        self.esp_size_sectors = esp_size_sectors;
        // iso_file is now passed directly

        // Physical layout (ISO 2048-byte sectors) for isohybrid:
        //
        //   LBA 0-15:  system area (MBR/GPT written later by write_hybrid_structures)
        //   LBA 16:    Primary Volume Descriptor (fixed by ISO 9660 spec)
        //   LBA 17:    Boot Record Volume Descriptor (El Torito)
        //   LBA 18:    Volume Descriptor Set Terminator
        //   LBA 19:    El Torito Boot Catalog
        //   LBA 20-511: padding / alignment gap (ensures ESP starts at 1 MiB = LBA 512)
        //   LBA 512..512+esp_size-1: EFI System Partition (FAT image)
        //   LBA 512+esp_size..:       ISO9660 directory records and file data
        //
        // Volume descriptors and boot catalog are written at fixed LBAs
        // (16-19), independent of iso_data_lba.
        // File data starts at iso_data_lba = ESP_START_LBA + esp_size,
        // ensuring ISO9660 metadata never overlaps with the ESP region.

        let boot_catalog_lba = 19;

        // iso_data_lba: start of ISO9660 directory records and file contents.
        // For isohybrid, this begins after the ESP partition.
        // For non-hybrid, this begins right after VDs+boot catalog.
        self.iso_data_lba = if self.is_isohybrid {
            ESP_START_LBA_ISO + esp_size_sectors.unwrap_or(0)
        } else {
            boot_catalog_lba + 1 // LBA 20
        };
        iso_file.seek(SeekFrom::Start(
            (self.iso_data_lba as u64) * ISO_SECTOR_SIZE as u64,
        ))?;

        // Calculate LBAs for all files and directories. This also updates self.iso_data_lba to the end of the filesystem data.
        calculate_lbas(&mut self.iso_data_lba, &mut self.root)?;

        // Write volume descriptors at fixed ISO 9660 positions
        // (LBA 16=PVD, 17=BRVD, 18=Terminator).
        write_descriptors(
            iso_file,
            self.volume_id.as_deref(),
            self.root.lba,
            self.iso_data_lba,
        )?;

        let boot_entries = self.prepare_boot_entries(esp_lba, esp_size_sectors)?;
        write_boot_catalog_to_iso(iso_file, boot_catalog_lba, boot_entries)?;

        // Write directory records and copy file contents.
        write_directories(iso_file, &self.root, self.root.lba)?;
        copy_files(iso_file, &self.root)?;

        // Finalize the ISO by padding and updating the total sector count in the PVD
        finalize_iso(iso_file, &mut self.total_sectors)?;

        // If not isohybrid, clear the initial reserved sectors (MBR area).

        // Now that total_sectors is known, write MBR and GPT structures if hybrid
        if self.is_isohybrid {
            self.write_hybrid_structures(iso_file, self.total_sectors as u64, esp_size_sectors)?;
        }

        Ok(())
    }
}

/// High-level function to create an ISO 9660 image from a structured `IsoImage`.
/// Returns the path to the generated ISO, the temporary FAT image holder (if created),
/// and the `File` handle to the ISO itself.
pub fn build_iso(
    iso_path: &Path,
    image: &IsoImage,
    is_isohybrid: bool,
) -> io::Result<(PathBuf, Option<NamedTempFile>, File, Option<u32>)> {
    // Added Option<u32> for logical_fat_size_512_sectors
    let mut iso_builder = IsoBuilder::new();
    iso_builder.set_volume_id(image.volume_id.clone());
    iso_builder.set_isohybrid(is_isohybrid);

    let mut temp_fat_file_holder: Option<NamedTempFile> = None;
    let mut _temp_grub_cfg_file_holder: Option<NamedTempFile> = None; // Hold grub.cfg temp file
    let mut logical_fat_size_512_sectors: Option<u32> = None; // Declare here

    // Create the ISO file
    let mut iso_file = File::create(iso_path)?;

    if let Some(uefi_boot) = &image.boot_info.uefi_boot {
        iso_builder.uefi_catalog_path = Some(uefi_boot.destination_in_iso.clone());

        if is_isohybrid {
            let temp_fat_file = NamedTempFile::new()?;
            let path = temp_fat_file.path().to_path_buf();
            temp_fat_file_holder = Some(temp_fat_file);

            // Build the list of files for the FAT image
            let mut fat_files: Vec<(&str, &Path)> = Vec::new();
            fat_files.push(("BOOTX64.EFI", &uefi_boot.boot_image));
            fat_files.push(("KERNEL.EFI", &uefi_boot.kernel_image));
            // Add any additional EFI boot files (e.g. GRUBX64.EFI)
            for (dest_name, source_path) in &uefi_boot.additional_efi_boot_files {
                fat_files.push((dest_name.as_str(), source_path.as_path()));
            }
            // If grub.cfg content is specified, create a temporary file and add it
            let grub_cfg_path_buf: Option<PathBuf> =
                if let Some(grub_cfg) = &uefi_boot.grub_cfg_content {
                    let mut grub_temp = NamedTempFile::new()?;
                    write!(grub_temp, "{}", grub_cfg)?;
                    let path = grub_temp.path().to_path_buf();
                    _temp_grub_cfg_file_holder = Some(grub_temp);
                    Some(path)
                } else {
                    None
                };
            if let Some(ref grub_path) = grub_cfg_path_buf {
                fat_files.push(("grub.cfg", grub_path));
            }
            // ESP hidden sectors: offset from disk start to ESP in 512-byte units.
            let esp_hidden_sectors = ESP_START_LBA_512;
            let size_512_sectors = fat::create_fat_image(&path, &fat_files, esp_hidden_sectors)?;
            logical_fat_size_512_sectors = Some(size_512_sectors);

            // Convert logical FAT size from 512-byte sectors to ISO 2048-byte sectors
            let calculated_esp_size_iso_sectors = size_512_sectors.div_ceil(4); // 1 ISO sector = 4 * 512-byte sectors

            // Store ESP LBA and size for the boot catalog
            iso_builder.esp_lba = Some(ESP_START_LBA_ISO);
            iso_builder.esp_size_sectors = Some(calculated_esp_size_iso_sectors);

            // Copy the FAT image to the ISO file at the ESP LBA (1 MiB aligned)
            iso_file.seek(SeekFrom::Start(
                ESP_START_LBA_ISO as u64 * crate::utils::ISO_SECTOR_SIZE as u64,
            ))?;
            let mut temp_fat = std::fs::File::open(&path)?;
            io::copy(&mut temp_fat, &mut iso_file)?;
        }
    }

    // Add all regular files to the ISO builder
    for file in &image.files {
        iso_builder.add_file(&file.destination, &file.source)?;
    }

    // Handle BIOS boot image (add to ISO tree)
    if let Some(bios_boot_info) = &image.boot_info.bios_boot {
        iso_builder.add_file(
            &bios_boot_info.destination_in_iso,
            &bios_boot_info.boot_image,
        )?;
    }

    // Set boot information for the ISO builder
    iso_builder.set_boot_info(image.boot_info.clone());

    // Build the ISO using the mutable iso_file
    iso_builder.build(
        &mut iso_file,
        iso_path,
        iso_builder.esp_lba,
        iso_builder.esp_size_sectors,
    )?;

    // The iso_file is already the final_iso_file
    let final_iso_file = iso_file;

    Ok((
        iso_path.to_path_buf(),
        temp_fat_file_holder,
        final_iso_file,
        logical_fat_size_512_sectors, // Return this value
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_add_file() -> io::Result<()> {
        let mut builder = IsoBuilder::new();
        let temp_file = NamedTempFile::new()?;
        let temp_path = temp_file.path().to_path_buf();

        // Add a root file
        builder.add_file("root.txt", &temp_path)?;
        assert!(builder.root.children.contains_key("root.txt"));

        // Add a nested file
        builder.add_file("dir1/nested.txt", &temp_path)?;
        let dir1 = match builder.root.children.get("dir1") {
            Some(IsoFsNode::Directory(dir)) => dir,
            _ => panic!("dir1 was not created as a directory"),
        };
        assert!(dir1.children.contains_key("nested.txt"));

        Ok(())
    }

    #[test]
    fn test_calculate_lbas() -> io::Result<()> {
        let mut root = IsoDirectory::new();
        let mut current_lba = 20; // Start at a known LBA

        // Add a directory and a file
        let mut subdir = IsoDirectory::new();
        let file1 = IsoFile {
            path: PathBuf::new(),
            size: 1000,
            lba: 0,
        }; // Less than 1 sector
        let file2 = IsoFile {
            path: PathBuf::new(),
            size: 3000,
            lba: 0,
        }; // 2 sectors
        subdir
            .children
            .insert("file2.txt".to_string(), IsoFsNode::File(file2));
        root.children
            .insert("file1.txt".to_string(), IsoFsNode::File(file1));
        root.children
            .insert("subdir".to_string(), IsoFsNode::Directory(subdir));

        calculate_lbas(&mut current_lba, &mut root)?;

        // Expected LBA assignments:
        // root: 20
        // file1.txt: 21 (1 sector)
        // subdir: 22
        // file2.txt: 23 (2 sectors)
        // final lba: 25

        assert_eq!(root.lba, 20);
        match root.children.get("file1.txt") {
            Some(IsoFsNode::File(f)) => assert_eq!(f.lba, 21),
            _ => panic!("file1.txt not found"),
        }
        let (subdir_lba, file2_lba) = match root.children.get("subdir") {
            Some(IsoFsNode::Directory(d)) => {
                let file2_lba = match d.children.get("file2.txt") {
                    Some(IsoFsNode::File(f)) => f.lba,
                    _ => panic!("file2.txt not found"),
                };
                (d.lba, file2_lba)
            }
            _ => panic!("subdir not found"),
        };
        assert_eq!(subdir_lba, 22);
        assert_eq!(file2_lba, 23);
        assert_eq!(current_lba, 25);

        Ok(())
    }

    #[test]
    fn test_get_path_helpers() -> io::Result<()> {
        let mut builder = IsoBuilder::new();
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(b"some data")?;
        let temp_path = temp_file.path().to_path_buf();

        builder.add_file("A/B/C.txt", &temp_path)?;
        builder.iso_data_lba = 20;
        calculate_lbas(&mut builder.iso_data_lba, &mut builder.root)?;

        let lba = crate::iso::builder_utils::get_lba_for_path(&builder.root, "A/B/C.txt")?;
        let size = crate::iso::builder_utils::get_file_size_in_iso(&builder.root, "A/B/C.txt")?;

        // root dir: 20, A: 21, B: 22, C.txt: 23
        assert_eq!(lba, 23);
        assert_eq!(size, 9);

        // Test not found
        assert!(crate::iso::builder_utils::get_lba_for_path(&builder.root, "A/D.txt").is_err());

        Ok(())
    }
}
