use std::io::{self};
use std::path::Path;

use crate::iso::boot_catalog::{
    BOOT_CATALOG_EFI_PLATFORM_ID, BootCatalogEntry, BootCatalogEntryType,
};
use crate::iso::fs_node::{IsoDirectory, IsoFsNode};
use crate::utils::ISO_SECTOR_SIZE;

const EL_TORITO_SECTOR_SIZE: u64 = 512;

pub fn calculate_lbas(current_lba: &mut u32, dir: &mut IsoDirectory) -> io::Result<()> {
    dir.lba = *current_lba;
    *current_lba += 1;
    let mut sorted: Vec<_> = dir.children.iter_mut().collect();
    sorted.sort_by_key(|(name, _)| *name);
    for (_, node) in sorted {
        match node {
            IsoFsNode::File(file) => {
                file.lba = *current_lba;
                *current_lba += file.size.div_ceil(ISO_SECTOR_SIZE as u64) as u32;
            }
            IsoFsNode::Directory(subdir) => calculate_lbas(current_lba, subdir)?,
        }
    }
    Ok(())
}

fn get_node_for_path<'a>(root: &'a IsoDirectory, path: &str) -> io::Result<&'a IsoFsNode> {
    for c in Path::new(path).components() {
        c.as_os_str()
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid path"))?;
    }
    let mut current = root;
    let components: Vec<_> = Path::new(path).components().collect();
    for (i, comp) in components.iter().enumerate() {
        let name = comp.as_os_str().to_str().unwrap();
        if i == components.len() - 1 {
            return current.children.get(name).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("Path not found: {path}"))
            });
        }
        match current.children.get(name) {
            Some(IsoFsNode::Directory(d)) => current = d,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Directory not found: {path}"),
                ));
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("Path not found: {path}"),
    ))
}

pub fn get_lba_for_path(root: &IsoDirectory, path: &str) -> io::Result<u32> {
    match get_node_for_path(root, path)? {
        IsoFsNode::File(f) => Ok(f.lba),
        IsoFsNode::Directory(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Path is a directory: {path}"),
        )),
    }
}

pub fn get_file_size_in_iso(root: &IsoDirectory, path: &str) -> io::Result<u64> {
    match get_node_for_path(root, path)? {
        IsoFsNode::File(f) => Ok(f.size),
        IsoFsNode::Directory(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Path is a directory: {path}"),
        )),
    }
}

pub fn get_file_metadata(path: &Path) -> io::Result<std::fs::Metadata> {
    std::fs::metadata(path).map_err(|e| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("Failed to get metadata for {}: {e}", path.display()),
        )
    })
}

pub fn ensure_directory_path<'a>(
    root: &'a mut IsoDirectory,
    path: &str,
) -> io::Result<&'a mut IsoDirectory> {
    let components: Vec<_> = Path::new(path).components().collect();
    let mut current = root;
    for comp in components.iter().take(components.len().saturating_sub(1)) {
        let name = comp
            .as_os_str()
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid path component"))?;
        current = match current
            .children
            .entry(name.to_string())
            .or_insert_with(|| IsoFsNode::Directory(IsoDirectory::new()))
        {
            IsoFsNode::Directory(d) => d,
            IsoFsNode::File(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("Path component '{name}' is a file"),
                ));
            }
        };
    }
    Ok(current)
}

fn mk_boot_entry(platform_id: u8, lba: u32, sectors: u16) -> BootCatalogEntry {
    BootCatalogEntry {
        platform_id,
        boot_image_lba: lba,
        boot_image_sectors: sectors,
        entry_type: BootCatalogEntryType::BootEntry { bootable: true },
    }
}

pub fn create_bios_boot_entry(root: &IsoDirectory, path: &str) -> io::Result<BootCatalogEntry> {
    let lba = get_lba_for_path(root, path)?;
    let sz = get_file_size_in_iso(root, path)?;
    let sectors = sz.div_ceil(EL_TORITO_SECTOR_SIZE).max(1);
    if sectors > u16::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "BIOS boot image too large",
        ));
    }
    Ok(mk_boot_entry(0x00, lba, sectors as u16))
}

pub fn create_uefi_boot_entry(root: &IsoDirectory, path: &str) -> io::Result<BootCatalogEntry> {
    let lba = get_lba_for_path(root, path)?;
    let sz = get_file_size_in_iso(root, path)?;
    let sectors = sz.div_ceil(EL_TORITO_SECTOR_SIZE).max(1);
    if sectors > u16::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "UEFI boot image too large",
        ));
    }
    Ok(mk_boot_entry(
        BOOT_CATALOG_EFI_PLATFORM_ID,
        lba,
        sectors as u16,
    ))
}

pub fn create_uefi_esp_boot_entry(esp_lba: u32, _esp_size: u32) -> io::Result<BootCatalogEntry> {
    // No-emulation boot entries MUST have sector_count = 0 per El Torito
    // spec § 6.4.  The actual image size is conveyed via the Section Header
    // entry count field.
    Ok(mk_boot_entry(
        BOOT_CATALOG_EFI_PLATFORM_ID,
        esp_lba,
        0,
    ))
}
