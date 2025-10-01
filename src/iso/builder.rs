use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::fat;
use crate::iso::boot_catalog::{
    BOOT_CATALOG_EFI_PLATFORM_ID, BootCatalogEntry, write_boot_catalog,
};
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::volume_descriptor::{update_total_sectors_in_pvd, write_volume_descriptors};
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba};

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
    pub kernel_image: PathBuf,
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
    uefi_catalog_path: Option<String>,
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
            uefi_catalog_path: None,
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
        let reserved_sectors = if self.is_isohybrid { 34u32 } else { 16u32 };
        let data_start_lba = reserved_sectors;

        // Set current_lba to the start of filesystem data after VDs and catalog
        // The volume descriptors and boot catalog will occupy sectors starting from data_start_lba.
        // The actual file data will start after these.
        // The calculate_lbas function will determine the LBA for the root directory and subsequent files/directories.
        // We need to ensure that the total size calculation in finalize accounts for all written data.

        // Seek to the start of the ISO9660 data area.
        // LBA 16-18 for VDs, 19 for boot catalog. Data starts after.
        let boot_catalog_lba = 19;
        self.current_lba = boot_catalog_lba + 1;
        iso_file.seek(SeekFrom::Start(
            (data_start_lba as u64) * ISO_SECTOR_SIZE as u64,
        ))?;

        // Calculate LBAs for all files and directories. This also updates self.current_lba to the end of the filesystem data.
        IsoBuilder::calculate_lbas(&mut self.current_lba, &mut self.root)?;

        // Write volume descriptors (PVD, BRVD, Terminator). These will be written starting at data_start_lba.
        // Pass the calculated end of filesystem data as a preliminary total_sectors.
        // This will be correctly updated by finalize later. The VDs are at fixed locations.
        self.write_descriptors(&mut iso_file, self.current_lba)?;
        self.write_boot_catalog(&mut iso_file, boot_catalog_lba)?;

        // Write directory records and copy file contents.
        self.write_directories(&mut iso_file, &self.root, self.root.lba)?;
        self.copy_files(&mut iso_file, &self.root)?;

        // Finalize the ISO by padding and updating the total sector count in the PVD
        self.finalize(&mut iso_file)?;

        // If not isohybrid, clear the initial reserved sectors (MBR area).
        if !self.is_isohybrid {
            let reserved_sectors = 16u32;
            iso_file.seek(SeekFrom::Start(0))?;
            let reserved_bytes = reserved_sectors as u64 * ISO_SECTOR_SIZE as u64;
            io::copy(&mut io::repeat(0).take(reserved_bytes), &mut iso_file)?;
        }

        // Now that total_sectors is known, write MBR and GPT structures if hybrid
        let total_lbas = self.total_sectors as u64;

        if self.is_isohybrid {
            // GPT structures require at least 69 LBAs (1 MBR + 1 GPT Header + 32 Partition Entries + 1 Backup GPT Header + 32 Backup Partition Entries + 2 for safety)
            if total_lbas < 69 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "ISO image is too small for isohybrid with GPT ({} sectors, requires at least 69)",
                        total_lbas
                    ),
                ));
            }

            // Write MBR
            iso_file.seek(SeekFrom::Start(0))?;
            let mbr =
                crate::iso::mbr::create_mbr_for_gpt_hybrid(self.total_sectors, self.is_isohybrid)?;
            mbr.write_to(&mut iso_file)?;

            // Write GPT structures if esp_size_sectors > 0
            if esp_size_sectors > 0 {
                let esp_partition_start_lba = 34; // After MBR (1) + GPT Header (1) + GPT Partition Array (32)
                let esp_partition_end_lba = esp_partition_start_lba + esp_size_sectors - 1;

                let esp_guid_str = crate::iso::gpt::EFI_SYSTEM_PARTITION_GUID;
                let esp_unique_guid_str = Uuid::new_v4().to_string(); // Generate a new unique GUID
                let partitions = vec![crate::iso::gpt::GptPartitionEntry::new(
                    esp_guid_str,
                    &esp_unique_guid_str,
                    esp_partition_start_lba as u64,
                    esp_partition_end_lba as u64,
                    "EFI System Partition",
                )];
                crate::iso::gpt::write_gpt_structures(&mut iso_file, total_lbas, &partitions)?;
            }
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
    fn write_descriptors(&self, iso_file: &mut File, total_sectors: u32) -> io::Result<()> {
        let root_entry = IsoDirEntry {
            lba: self.root.lba,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        };
        // Pass total_sectors to write_volume_descriptors. This will be updated in finalize if needed.
        write_volume_descriptors(iso_file, total_sectors, &root_entry)
    }

    /// Writes the El Torito boot catalog.
    fn write_boot_catalog(&self, iso_file: &mut File, boot_catalog_lba: u32) -> io::Result<()> {
        let mut boot_entries = Vec::new();

        if let Some(boot_info) = &self.boot_info {
            // Add BIOS boot entry
            if let Some(bios_boot) = &boot_info.bios_boot {
                let boot_image_size = self.get_file_size_in_iso(&bios_boot.destination_in_iso)?;
                // El Torito specification requires sector count in 512-byte sectors.
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
        }

        // Add UEFI boot entry
        if let Some(path) = &self.uefi_catalog_path {
            let uefi_boot_lba = self.get_lba_for_path(path)?;
            let uefi_boot_size = self.get_file_size_in_iso(path)?;
            let uefi_boot_sectors_u64 = uefi_boot_size.div_ceil(512).max(1);
            if uefi_boot_sectors_u64 > u16::MAX as u64 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "UEFI boot image is too large for the boot catalog",
                ));
            }
            let uefi_boot_sectors = uefi_boot_sectors_u64 as u16;
            boot_entries.push(BootCatalogEntry {
                platform_id: BOOT_CATALOG_EFI_PLATFORM_ID,
                boot_image_lba: uefi_boot_lba,
                boot_image_sectors: uefi_boot_sectors,
                bootable: true,
            });
        }

        if !boot_entries.is_empty() {
            // Seek to the correct LBA before writing the boot catalog
            iso_file.seek(SeekFrom::Start(
                (boot_catalog_lba as u64) * ISO_SECTOR_SIZE as u64,
            ))?;
            write_boot_catalog(iso_file, boot_entries)?;
        }

        Ok(())
    }

    /// Writes the directory records for the ISO filesystem.
    fn write_directories(
        &self,
        iso_file: &mut File,
        dir: &IsoDirectory,
        parent_lba: u32,
    ) -> io::Result<()> {
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
            lba: parent_lba,
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
                self.write_directories(iso_file, subdir, dir.lba)?;
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
    let mut _temp_fat_file_holder: Option<NamedTempFile> = None;
    let mut esp_size_sectors = 0; // Initialize esp_size_sectors
    let mut fat_img_path: Option<PathBuf> = None;

    if let Some(uefi_boot) = &image.boot_info.uefi_boot {
        // Add BOOTX64.EFI for El Torito catalog
        // The file is expected to be added via the `image.files` list.
        // We just record its path for the boot catalog.
        iso_builder.uefi_catalog_path = Some(uefi_boot.destination_in_iso.clone());

        // For hybrid, create and add FAT image for ESP
        if is_isohybrid {
            let temp_fat_file = NamedTempFile::new()?;
            let path = temp_fat_file.path().to_path_buf();
            _temp_fat_file_holder = Some(temp_fat_file);

            fat::create_fat_image(&path, &uefi_boot.boot_image, &uefi_boot.kernel_image)?;

            // Calculate ESP size
            let fat_img_metadata = std::fs::metadata(&path)?;
            esp_size_sectors =
                (fat_img_metadata.len() as u32).div_ceil(crate::utils::ISO_SECTOR_SIZE as u32);

            // Do not add efi.img to ISO filesystem for hybrid, it will be overlaid
            fat_img_path = Some(path);
        }
    }

    // Add all regular files to the ISO builder
    for file in &image.files {
        iso_builder.add_file(&file.destination, file.source.clone())?;
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

    if let Some(path) = fat_img_path {
        let mut iso_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(iso_path)?;
        iso_file.seek(SeekFrom::Start(
            34u64 * crate::utils::ISO_SECTOR_SIZE as u64,
        ))?;
        let mut temp_fat = std::fs::File::open(path)?;
        io::copy(&mut temp_fat, &mut iso_file)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_add_file() -> io::Result<()> {
        let mut builder = IsoBuilder::new();
        let temp_file = NamedTempFile::new()?;
        let temp_path = temp_file.path().to_path_buf();

        // Add a root file
        builder.add_file("root.txt", temp_path.clone())?;
        assert!(builder.root.children.contains_key("root.txt"));

        // Add a nested file
        builder.add_file("dir1/nested.txt", temp_path.clone())?;
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
        let file1 = IsoFile { path: PathBuf::new(), size: 1000, lba: 0 }; // Less than 1 sector
        let file2 = IsoFile { path: PathBuf::new(), size: 3000, lba: 0 }; // 2 sectors
        subdir.children.insert("file2.txt".to_string(), IsoFsNode::File(file2));
        root.children.insert("file1.txt".to_string(), IsoFsNode::File(file1));
        root.children.insert("subdir".to_string(), IsoFsNode::Directory(subdir));

        IsoBuilder::calculate_lbas(&mut current_lba, &mut root)?;

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

        builder.add_file("A/B/C.txt", temp_path)?;
        builder.current_lba = 20;
        IsoBuilder::calculate_lbas(&mut builder.current_lba, &mut builder.root)?;

        let lba = builder.get_lba_for_path("A/B/C.txt")?;
        let size = builder.get_file_size_in_iso("A/B/C.txt")?;

        // root dir: 20, A: 21, B: 22, C.txt: 23
        assert_eq!(lba, 23);
        assert_eq!(size, 9);

        // Test not found
        assert!(builder.get_lba_for_path("A/D.txt").is_err());

        Ok(())
    }
}
