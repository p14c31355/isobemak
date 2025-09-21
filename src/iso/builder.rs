use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

use crate::fat;
use crate::iso::boot_catalog::{
    BOOT_CATALOG_EFI_PLATFORM_ID, BootCatalogEntry, write_boot_catalog,
};
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::volume_descriptor::{update_total_sectors_in_pvd, write_volume_descriptors};
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba};
use crate::iso::mbr::Mbr; // Import Mbr struct

/// Represents a file within the ISO filesystem.
pub struct IsoFile {
    pub path: PathBuf,
    pub size: u64,
    pub lba: u32,
}

/// Represents a directory within the ISO filesystem.
pub struct IsoDirectory {
    pub children: HashMap<String, IsoFsNode>,
    pub lba: u32,
    pub size: u32,
}

impl Default for IsoDirectory {
    fn default() -> Self {
        Self::new()
    }
}

impl IsoDirectory {
    pub fn new() -> Self {
        Self {
            children: HashMap::new(),
            lba: 0,
            size: ISO_SECTOR_SIZE as u32,
        }
    }
}

/// A node in the ISO filesystem tree, either a file or a directory.
pub enum IsoFsNode {
    File(IsoFile),
    Directory(IsoDirectory),
}

/// Configuration for a file to be added to the ISO.
pub struct IsoImageFile {
    pub source: PathBuf,
    pub destination: String,
}

/// Configuration for the entire ISO image to be built.
pub struct IsoImage {
    pub files: Vec<IsoImageFile>,
    pub boot_info: BootInfo,
}

/// High-level boot information for the ISO.
#[derive(Clone)]
pub struct BootInfo {
    pub bios_boot: Option<BiosBootInfo>,
    pub uefi_boot: Option<UefiBootInfo>,
}

/// Configuration for BIOS boot (El Torito).
#[derive(Clone)]
pub struct BiosBootInfo {
    pub boot_catalog: PathBuf,
    pub boot_image: PathBuf,
    pub destination_in_iso: String,
}

/// Configuration for UEFI boot.
#[derive(Clone)]
pub struct UefiBootInfo {
    pub boot_image: PathBuf,
    pub destination_in_iso: String,
}

/// The main builder for creating an ISO 9660 image.
pub struct IsoBuilder {
    root: IsoDirectory,
    iso_file: Option<File>,
    boot_info: Option<BootInfo>,
    current_lba: u32,
    total_sectors: u32,
    is_isohybrid: bool, // New field
}

impl Default for IsoBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl IsoBuilder {
    pub fn new() -> Self {
        Self {
            root: IsoDirectory::new(),
            iso_file: None,
            boot_info: None,
            current_lba: 0,
            total_sectors: 0,
            is_isohybrid: false, // Initialize new field
        }
    }

    /// Adds a file to the ISO filesystem tree.
    pub fn add_file(&mut self, path_in_iso: &str, real_path: PathBuf) -> io::Result<()> {
        let path = Path::new(path_in_iso);
        let mut current_dir = &mut self.root;

        let components: Vec<_> = path.components().collect();
        if components.len() > 1 {
            for component in components.iter().take(components.len() - 1) {
                let component_name = component
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "Invalid path component")
                    })?
                    .to_string();
                current_dir = match current_dir
                    .children
                    .entry(component_name)
                    .or_insert_with(|| IsoFsNode::Directory(IsoDirectory::new()))
                {
                    IsoFsNode::Directory(dir) => dir,
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            format!("{} is not a directory", path_in_iso),
                        ));
                    }
                };
            }
        }

        let file_name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name"))?
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name"))?
            .to_string();
        let file_size = std::fs::metadata(&real_path)?.len();

        let file = IsoFile {
            path: real_path,
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

    /// Builds the ISO file based on the configured files and boot information.
pub fn build(&mut self, iso_path: &Path, esp_size_sectors: u32) -> io::Result<()> {
    self.iso_file = Some(File::create(iso_path)?);
    let mut iso_file = self.iso_file.take().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "build() has already been called",
        )
    })?;
    // Placeholder for MBR and GPT structures.
    // We'll write the actual MBR/GPT after the ISO9660 content is written and total_sectors is known.
    let mbr_gpt_reserved_sectors = 34; // MBR (1) + GPT Header (1) + GPT Partition Array (32)
    let iso_data_start_lba = mbr_gpt_reserved_sectors;

    // Set current_lba to the start of ISO9660 data
    self.current_lba = iso_data_start_lba;

    // Seek past the MBR/GPT reserved area to start writing ISO9660 data
    iso_file.seek(SeekFrom::Start((iso_data_start_lba as u64) * ISO_SECTOR_SIZE as u64))?;

    IsoBuilder::calculate_lbas(&mut self.current_lba, &mut self.root)?;
    self.write_descriptors(&mut iso_file)?;
    self.write_boot_catalog(&mut iso_file)?;
    self.write_directories(&mut iso_file, &self.root)?;
    self.copy_files(&mut iso_file, &self.root)?;
    self.finalize(&mut iso_file)?;

    // Now that total_sectors is known, write MBR and GPT structures
    let total_lbas = self.total_sectors as u64;

    // Write MBR
    iso_file.seek(SeekFrom::Start(0))?;
    let mbr = crate::gpt::create_mbr_for_gpt_hybrid(self.total_sectors, self.is_isohybrid)?;
    mbr.write_to(&mut iso_file)?;

    // Write GPT structures if isohybrid
    if self.is_isohybrid {
        let esp_partition_start_lba = 34; // After MBR (1) + GPT Header (1) + GPT Partition Array (32)
        let esp_partition_end_lba = esp_partition_start_lba + esp_size_sectors - 1;

        let esp_guid = uuid::Uuid::parse_str(crate::gpt::EFI_SYSTEM_PARTITION_GUID).unwrap();
        let partitions = vec![
            crate::gpt::GptPartitionEntry::new(
                esp_guid,
                esp_partition_start_lba as u64,
                esp_partition_end_lba as u64,
                "EFI System Partition",
            ),
        ];
        crate::gpt::write_gpt_structures(&mut iso_file, total_lbas, &partitions)?;
    }

    Ok(())
}

    /// Calculates the Logical Block Addresses (LBAs) for all files and directories.
    fn calculate_lbas(current_lba: &mut u32, dir: &mut IsoDirectory) -> io::Result<()> {
        dir.lba = *current_lba;
        *current_lba += 1;

        let mut sorted_children: Vec<_> = dir.children.iter_mut().collect();
        sorted_children.sort_by_key(|(name, _)| *name);

        for (_, node) in sorted_children {
            match node {
                IsoFsNode::File(file) => {
                    file.lba = *current_lba;
                    let sectors = file.size.div_ceil(ISO_SECTOR_SIZE as u64) as u32;
                    *current_lba += sectors;
                }
                IsoFsNode::Directory(subdir) => {
                    IsoBuilder::calculate_lbas(current_lba, subdir)?;
                }
            }
        }
        Ok(())
    }

    /// Writes all ISO volume descriptors.
    fn write_descriptors(&self, iso_file: &mut File) -> io::Result<()> {
        let root_entry = IsoDirEntry {
            lba: self.root.lba,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        };
        // Pass 0 for total_sectors as a placeholder, it will be updated in finalize.
        write_volume_descriptors(iso_file, 0, &root_entry)
    }

    /// Writes the El Torito boot catalog.
    fn write_boot_catalog(&self, iso_file: &mut File) -> io::Result<()> {
        let mut boot_entries = Vec::new();

        if let Some(boot_info) = &self.boot_info {
            // Add BIOS boot entry
            if let Some(bios_boot) = &boot_info.bios_boot {
                let boot_image_size = std::fs::metadata(&bios_boot.boot_image)?.len();
                // El Torito specification requires sector count in 512-byte sectors.
                // The calculation is simplified to sectors.div_ceil(512).max(1).
                let boot_image_sectors_u64 = boot_image_size.div_ceil(512).max(1);

                if boot_image_sectors_u64 > u16::MAX as u64 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "BIOS boot image is too large for the boot catalog",
                    ));
                }
                let boot_image_sectors = boot_image_sectors_u64 as u16;
                boot_entries.push(BootCatalogEntry {
                    platform_id: 0x00,
                    boot_image_lba: self.get_lba_for_path(&bios_boot.destination_in_iso)?,
                    boot_image_sectors,
                    bootable: true,
                });
            }

            // Add UEFI boot entry
            if let Some(uefi_boot) = &boot_info.uefi_boot {
                let uefi_fat_img_lba = self.get_lba_for_path(&uefi_boot.destination_in_iso)?;
                let uefi_fat_img_size = self.get_file_size_in_iso(&uefi_boot.destination_in_iso)?;
                // El Torito specification requires sector count in 512-byte sectors.
                // The calculation is simplified to sectors.div_ceil(512).max(1).
                let uefi_fat_img_sectors_u64 = uefi_fat_img_size.div_ceil(512).max(1);

                if uefi_fat_img_sectors_u64 > u16::MAX as u64 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "UEFI boot image is too large for the boot catalog (exceeds u16::MAX sectors)",
                    ));
                }
                let uefi_fat_img_sectors = uefi_fat_img_sectors_u64 as u16;

                boot_entries.push(BootCatalogEntry {
                    platform_id: BOOT_CATALOG_EFI_PLATFORM_ID,
                    boot_image_lba: uefi_fat_img_lba,
                    boot_image_sectors: uefi_fat_img_sectors,
                    bootable: true,
                });
            }
        }

        if !boot_entries.is_empty() {
            write_boot_catalog(iso_file, boot_entries)?;
        }

        Ok(())
    }

    /// Writes the directory records for the ISO filesystem.
    fn write_directories(&self, iso_file: &mut File, dir: &IsoDirectory) -> io::Result<()> {
        pad_to_lba(iso_file, dir.lba)?;

        let mut sorted_children: Vec<_> = dir.children.iter().collect();
        sorted_children.sort_by_key(|(name, _)| *name);

        let mut dir_entries = Vec::new();
        // Self-reference
        dir_entries.push(IsoDirEntry {
            lba: dir.lba,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        });
        // Parent directory
        dir_entries.push(IsoDirEntry {
            lba: self.root.lba,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "..",
        });

        for (name, node) in &sorted_children {
            let (lba, size, flags) = match node {
                IsoFsNode::File(file) => {
                    let file_size_u32 = u32::try_from(file.size).map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!(
                                "File '{}' is too large for ISO9660 (exceeds u32::MAX bytes)",
                                name
                            ),
                        )
                    })?;
                    (file.lba, file_size_u32, 0x00)
                }
                IsoFsNode::Directory(subdir) => (subdir.lba, ISO_SECTOR_SIZE as u32, 0x02),
            };
            dir_entries.push(IsoDirEntry {
                lba,
                size,
                flags,
                name: name.as_str(),
            });
        }

        let mut dir_sector = [0u8; ISO_SECTOR_SIZE];
        let mut offset = 0;

        for entry in &dir_entries {
            let entry_bytes = entry.to_bytes();
            dir_sector[offset..offset + entry_bytes.len()].copy_from_slice(&entry_bytes);
            offset += entry_bytes.len();
        }

        iso_file.write_all(&dir_sector)?;

        for (_, node) in sorted_children {
            if let IsoFsNode::Directory(subdir) = node {
                self.write_directories(iso_file, subdir)?;
            }
        }

        Ok(())
    }

    /// Copies all file contents to the ISO image.
    fn copy_files(&self, iso_file: &mut File, dir: &IsoDirectory) -> io::Result<()> {
        let mut sorted_children: Vec<_> = dir.children.iter().collect();
        sorted_children.sort_by_key(|(name, _)| *name);

        for (_, node) in sorted_children {
            match node {
                IsoFsNode::File(file) => {
                    pad_to_lba(iso_file, file.lba)?;
                    let mut real_file = File::open(&file.path)?;
                    io::copy(&mut real_file, iso_file)?;
                }
                IsoFsNode::Directory(subdir) => {
                    self.copy_files(iso_file, subdir)?;
                }
            }
        }

        Ok(())
    }

    /// Finalizes the ISO image by padding and updating the total sector count in the PVD.
    fn finalize(&mut self, iso_file: &mut File) -> io::Result<()> {
        let current_pos = iso_file.stream_position()?;
        let remainder = current_pos % ISO_SECTOR_SIZE as u64;
        if remainder != 0 {
            let padding_bytes = ISO_SECTOR_SIZE as u64 - remainder;
            io::copy(&mut io::repeat(0).take(padding_bytes), iso_file)?;
        }

        let final_pos = iso_file.stream_position()?;
        let total_sectors_u64 = final_pos.div_ceil(ISO_SECTOR_SIZE as u64);
        self.total_sectors = u32::try_from(total_sectors_u64)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "ISO image too large"))?;
        update_total_sectors_in_pvd(iso_file, self.total_sectors)?;

        Ok(())
    }

    /// Helper to find the LBA for a given path in the ISO filesystem.
    fn get_lba_for_path(&self, path: &str) -> io::Result<u32> {
        let mut current_dir = &self.root;
        let components: Vec<_> = Path::new(path).components().collect();

        for component in components.iter().take(components.len() - 1) {
            let component_name = component.as_os_str().to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Invalid path component")
            })?;
            match current_dir.children.get(component_name) {
                Some(IsoFsNode::Directory(dir)) => current_dir = dir,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("Directory not found: {}", path),
                    ));
                }
            }
        }

        let file_name = Path::new(path)
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name"))?
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name"))?;

        match current_dir.children.get(file_name) {
            Some(IsoFsNode::File(file)) => Ok(file.lba),
            _ => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found: {}", path),
            )),
        }
    }

    /// Helper to find the size for a given path in the ISO filesystem.
    fn get_file_size_in_iso(&self, path: &str) -> io::Result<u64> {
        let mut current_dir = &self.root;
        let components: Vec<_> = Path::new(path).components().collect();

        for component in components.iter().take(components.len() - 1) {
            let component_name = component.as_os_str().to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Invalid path component")
            })?;
            match current_dir.children.get(component_name) {
                Some(IsoFsNode::Directory(dir)) => current_dir = dir,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("Directory not found: {}", path),
                    ));
                }
            }
        }

        let file_name = Path::new(path)
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name"))?
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name"))?;

        match current_dir.children.get(file_name) {
            Some(IsoFsNode::File(file)) => Ok(file.size),
            _ => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found: {}", path),
            )),
        }
    }
}

/// High-level function to create an ISO 9660 image from a structured `IsoImage`.
pub fn build_iso(iso_path: &Path, image: &IsoImage, is_isohybrid: bool) -> io::Result<()> {
    let mut iso_builder = IsoBuilder::new();
    iso_builder.set_isohybrid(is_isohybrid);

    // Handle UEFI boot image by creating a temporary FAT image.
    let mut uefi_fat_img_path = None;
    let mut _temp_fat_file_holder: Option<NamedTempFile> = None;
    let mut esp_size_sectors = 0; // Initialize esp_size_sectors

    if let Some(uefi_boot_info) = &image.boot_info.uefi_boot {
        let temp_fat_file = NamedTempFile::new()?;
        let fat_img_path = temp_fat_file.path().to_path_buf();
        _temp_fat_file_holder = Some(temp_fat_file);

        // create_fat_image expects a kernel path, so we create a dummy file.
        let dummy_kernel_path = iso_path
            .parent()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "ISO path has no parent directory",
                )
            })?
            .join("dummy_kernel_for_fat");
        std::fs::write(&dummy_kernel_path, b"")?;

        fat::create_fat_image(
            &fat_img_path,
            &uefi_boot_info.boot_image,
            &dummy_kernel_path,
        )?;

        std::fs::remove_file(dummy_kernel_path)?;
        uefi_fat_img_path = Some(fat_img_path.clone()); // Clone here to use it later

        // Calculate ESP size
        let fat_img_metadata = std::fs::metadata(&fat_img_path)?; // Corrected: use fat_img_path
        esp_size_sectors = (fat_img_metadata.len() as u32).div_ceil(crate::utils::ISO_SECTOR_SIZE as u32);
    }

    // Add all regular files to the ISO builder
    for file in &image.files {
        iso_builder.add_file(&file.destination, file.source.clone())?;
    }

    // If a UEFI FAT image was created, add it to the ISO builder.
    if let Some(path) = uefi_fat_img_path {
        let dest = image
            .boot_info
            .uefi_boot
            .as_ref()
            .map(|info| info.destination_in_iso.clone())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Missing UEFI boot destination")
            })?;
        iso_builder.add_file(&dest, path)?;
    }

    // Handle BIOS boot image
    if let Some(bios_boot_info) = &image.boot_info.bios_boot {
        iso_builder.add_file(
            &bios_boot_info.destination_in_iso,
            bios_boot_info.boot_image.clone(),
        )?;
    }

    // Set boot information for the ISO builder
    iso_builder.set_boot_info(image.boot_info.clone());

    // Build the ISO
    iso_builder.build(iso_path, esp_size_sectors)?;

    Ok(())
}
