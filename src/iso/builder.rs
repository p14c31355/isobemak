// src/iso/builder.rs

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::io::{self, Write, Seek, copy, Read};
use std::fs::File;

use crate::iso::boot_catalog::{LBA_BOOT_CATALOG, write_boot_catalog, BootCatalogEntry, BOOT_CATALOG_EFI_PLATFORM_ID};
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::volume_descriptor::{write_volume_descriptors, update_total_sectors_in_pvd};
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba};
use crate::builder::{BootInfo, BiosBootInfo, UefiBootInfo};

pub enum IsoFsNode {
    File(IsoFile),
    Directory(IsoDirectory),
}

pub struct IsoFile {
    pub path: PathBuf,
    pub size: u64,
    pub lba: u32,
}

pub struct IsoDirectory {
    pub children: HashMap<String, IsoFsNode>,
    pub lba: u32,
    pub size: u32,
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

pub struct IsoBuilder {
    root: IsoDirectory,
    iso_file: Option<File>,
    boot_info: Option<BootInfo>,
    current_lba: u32,
    total_sectors: u32,
}

impl IsoBuilder {
    pub fn new() -> Self {
        Self {
            root: IsoDirectory::new(),
            iso_file: None,
            boot_info: None,
            current_lba: 0,
            total_sectors: 0,
        }
    }

    pub fn add_file(&mut self, path_in_iso: &str, real_path: PathBuf) -> io::Result<()> {
        let path = Path::new(path_in_iso);
        let mut current_dir = &mut self.root;

        let components: Vec<_> = path.components().collect();
        if components.len() > 1 {
            for component in components.iter().take(components.len() - 1) {
                let component_name = component.as_os_str().to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid path component"))?.to_string();
                current_dir = match current_dir.children.entry(component_name).or_insert_with(|| {
                    IsoFsNode::Directory(IsoDirectory::new())
                }) {
                    IsoFsNode::Directory(dir) => dir,
                    _ => return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} is not a directory", path_in_iso))),
                };
            }
        }

        let file_name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name"))?.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name"))?.to_string();
        let file_size = std::fs::metadata(&real_path)?.len();

        let file = IsoFile {
            path: real_path,
            size: file_size,
            lba: 0, // LBA will be calculated later
        };

        current_dir.children.insert(file_name, IsoFsNode::File(file));

        Ok(())
    }

    pub fn set_boot_info(&mut self, boot_info: BootInfo) {
        self.boot_info = Some(boot_info);
    }

    pub fn build(&mut self, iso_path: &Path) -> io::Result<()> {
        self.iso_file = Some(File::create(iso_path)?);
        let mut iso_file = self.iso_file.take().unwrap(); // Take ownership of the file
        self.current_lba = 16; // Start after primary and supplementary descriptors

        // 1. Calculate LBAs
        Self::calculate_lbas(&mut self.root, &mut self.current_lba)?;

        // 2. Write Volume Descriptors
        self.write_descriptors(&mut iso_file)?;

        // 3. Write Boot Catalog
        self.write_boot_catalog(&mut iso_file)?;

        // 4. Write Directory Records
        self.write_directories(&mut iso_file, &self.root, self.root.lba)?;

        // 5. Copy Files
        self.copy_files(&mut iso_file, &self.root)?;

        // 6. Finalize
        self.finalize(&mut iso_file)?;

        Ok(())
    }

    fn calculate_lbas(dir: &mut IsoDirectory, current_lba: &mut u32) -> io::Result<()> {
        // Assign LBA to the directory itself
        dir.lba = *current_lba;
        *current_lba += 1; // Directory takes one sector

        // Sort children for consistent LBA assignment
        let mut sorted_children: Vec<_> = dir.children.iter_mut().collect();
        sorted_children.sort_by_key(|(name, _)| *name);

        for (_, node) in sorted_children {
            match node {
                IsoFsNode::File(file) => {
                    file.lba = *current_lba;
                    let sectors = (file.size as f64 / ISO_SECTOR_SIZE as f64).ceil() as u32;
                    *current_lba += sectors;
                }
                IsoFsNode::Directory(subdir) => {
                    Self::calculate_lbas(subdir, current_lba)?;
                }
            }
        }
        Ok(())
    }

    fn write_descriptors(&self, iso_file: &mut File) -> io::Result<()> {
        let root_entry = IsoDirEntry {
            lba: self.root.lba,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        };
        write_volume_descriptors(iso_file, 0, LBA_BOOT_CATALOG, &root_entry)
    }

    fn write_boot_catalog(&self, iso_file: &mut File) -> io::Result<()> {
        let mut boot_entries = Vec::new();

        if let Some(boot_info) = &self.boot_info {
            if let Some(bios_boot) = &boot_info.bios_boot {
                let boot_image_size = std::fs::metadata(&bios_boot.boot_image)?.len();
                let boot_image_sectors = (boot_image_size as f64 / 512.0).ceil() as u16;
                boot_entries.push(BootCatalogEntry {
                    platform_id: 0x00, // 0x00 for x86 BIOS
                    boot_image_lba: self.get_lba_for_path(&bios_boot.destination_in_iso)?,
                    boot_image_sectors,
                    bootable: true,
                });
            }
            if let Some(uefi_boot) = &boot_info.uefi_boot {
                // The UEFI boot image is the FAT image we added as EFI/BOOT/EFI.img
                let uefi_fat_img_lba = self.get_lba_for_path(&uefi_boot.destination_in_iso)?;
                let uefi_fat_img_size = self.get_file_size_in_iso(&uefi_boot.destination_in_iso)?;
                let uefi_fat_img_sectors = (uefi_fat_img_size as f64 / 512.0).ceil() as u16;
                boot_entries.push(BootCatalogEntry {
                    platform_id: BOOT_CATALOG_EFI_PLATFORM_ID,
                    boot_image_lba: uefi_fat_img_lba,
                    boot_image_sectors: uefi_fat_img_sectors,
                    bootable: true,
                });
            }
        }
        write_boot_catalog(iso_file, boot_entries)
    }

    fn write_directories(&self, iso_file: &mut File, dir: &IsoDirectory, parent_lba: u32) -> io::Result<()> {
        pad_to_lba(iso_file, dir.lba)?;

        let mut dir_content = Vec::new();

        // Self entry
        dir_content.extend_from_slice(&IsoDirEntry {
            lba: dir.lba,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        }.to_bytes());

        // Parent entry
        dir_content.extend_from_slice(&IsoDirEntry {
            lba: parent_lba,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "..",
        }.to_bytes());

        let mut sorted_children: Vec<_> = dir.children.iter().collect();
        sorted_children.sort_by_key(|(name, _)| *name);

        for (name, node) in sorted_children {
            match node {
                IsoFsNode::File(file) => {
                    dir_content.extend_from_slice(&IsoDirEntry {
                        lba: file.lba,
                        size: file.size as u32,
                        flags: 0x00,
                        name: name,
                    }.to_bytes());
                }
                IsoFsNode::Directory(subdir) => {
                    dir_content.extend_from_slice(&IsoDirEntry {
                        lba: subdir.lba,
                        size: ISO_SECTOR_SIZE as u32,
                        flags: 0x02,
                        name: name,
                    }.to_bytes());
                    self.write_directories(iso_file, subdir, dir.lba)?;
                }
            }
        }

        dir_content.resize(ISO_SECTOR_SIZE, 0);
        iso_file.write_all(&dir_content)?;

        Ok(())
    }

    fn copy_files(&self, iso_file: &mut File, dir: &IsoDirectory) -> io::Result<()> {
        let mut sorted_children: Vec<_> = dir.children.iter().collect();
        sorted_children.sort_by_key(|(name, _)| *name);

        for (_, node) in sorted_children {
            match node {
                IsoFsNode::File(file) => {
                    pad_to_lba(iso_file, file.lba)?;
                    let mut src_file = File::open(&file.path)?;
                    copy(&mut src_file, iso_file)?;
                }
                IsoFsNode::Directory(subdir) => {
                    self.copy_files(iso_file, subdir)?;
                }
            }
        }
        Ok(())
    }

    fn finalize(&mut self, iso_file: &mut File) -> io::Result<()> {
        // Final padding to ISO sector
        let current_pos = iso_file.stream_position()?;
        let remainder = current_pos % ISO_SECTOR_SIZE as u64;
        if remainder != 0 {
            io::copy(
                &mut io::repeat(0).take(ISO_SECTOR_SIZE as u64 - remainder),
                iso_file,
            )?;
        }

        // Update PVD total sectors
        let final_pos = iso_file.stream_position()?;
        self.total_sectors = (final_pos as f64 / ISO_SECTOR_SIZE as f64).ceil() as u32;
        update_total_sectors_in_pvd(iso_file, self.total_sectors)?;

        println!(
            "ISO created with {} sectors",
            self.total_sectors
        );
        Ok(())
    }

    fn get_lba_for_path(&self, path: &str) -> io::Result<u32> {
        let mut current_dir = &self.root;
        let components: Vec<_> = Path::new(path).components().collect();

        for (i, component) in components.iter().enumerate() {
            let component_name = component.as_os_str().to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid path component"))?;
            if i == components.len() - 1 { // Last component, could be file or directory
                match current_dir.children.get(component_name) {
                    Some(IsoFsNode::File(file)) => return Ok(file.lba),
                    Some(IsoFsNode::Directory(dir)) => return Ok(dir.lba),
                    _ => return Err(io::Error::new(io::ErrorKind::NotFound, format!("Path not found: {}", path))),
                }
            } else { // Intermediate component, must be a directory
                match current_dir.children.get(component_name) {
                    Some(IsoFsNode::Directory(dir)) => current_dir = dir,
                    _ => return Err(io::Error::new(io::ErrorKind::NotFound, format!("Path not found: {}", path))),
                }
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, format!("Path not found: {}", path)))
    }

    fn get_file_size_in_iso(&self, path: &str) -> io::Result<u64> {
        let mut current_dir = &self.root;
        let components: Vec<_> = Path::new(path).components().collect();

        for (i, component) in components.iter().enumerate() {
            let component_name = component.as_os_str().to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid path component"))?;
            if i == components.len() - 1 { // Last component, must be a file
                match current_dir.children.get(component_name) {
                    Some(IsoFsNode::File(file)) => return Ok(file.size),
                    _ => return Err(io::Error::new(io::ErrorKind::NotFound, format!("File not found: {}", path))),
                }
            } else { // Intermediate component, must be a directory
                match current_dir.children.get(component_name) {
                    Some(IsoFsNode::Directory(dir)) => current_dir = dir,
                    _ => return Err(io::Error::new(io::ErrorKind::NotFound, format!("File not found: {}", path))),
                }
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, format!("File not found: {}", path)))
    }
}