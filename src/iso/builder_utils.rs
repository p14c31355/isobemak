use std::io::{self};
use std::path::Path;

use crate::io_error;
use crate::iso::boot_catalog::{BOOT_CATALOG_EFI_PLATFORM_ID, BootCatalogEntry};
use crate::iso::fs_node::{IsoDirectory, IsoFsNode};
use crate::utils::ISO_SECTOR_SIZE;

/// Calculates the Logical Block Addresses (LBAs) for all files and directories.
pub fn calculate_lbas(current_lba: &mut u32, dir: &mut IsoDirectory) -> io::Result<()> {
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
                calculate_lbas(current_lba, subdir)?;
            }
        }
    }
    Ok(())
}

/// Helper to find the LBA for a given path in the ISO filesystem.
pub fn get_lba_for_path(root: &IsoDirectory, path: &str) -> io::Result<u32> {
    match get_node_for_path(root, path)? {
        IsoFsNode::File(file) => Ok(file.lba),
        IsoFsNode::Directory(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Path is a directory, not a file: {}", path),
        )),
    }
}

/// Helper to find the size for a given path in the ISO filesystem.
pub fn get_file_size_in_iso(root: &IsoDirectory, path: &str) -> io::Result<u64> {
    match get_node_for_path(root, path)? {
        IsoFsNode::File(file) => Ok(file.size),
        IsoFsNode::Directory(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Path is a directory, not a file: {}", path),
        )),
    }
}

/// Helper to find the IsoFsNode for a given path in the ISO filesystem.
fn get_node_for_path<'a>(root: &'a IsoDirectory, path: &str) -> io::Result<&'a IsoFsNode> {
    let mut current_node = root;
    let components: Vec<_> = Path::new(path).components().collect();

    for (i, component) in components.iter().enumerate() {
        let component_name = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| io_error!(io::ErrorKind::InvalidInput, "Invalid path component"))?;

        if i == components.len() - 1 {
            // Last component, this is the target node
            return current_node
                .children
                .get(component_name)
                .ok_or_else(|| io_error!(io::ErrorKind::NotFound, "Path not found: {}", path));
        } else {
            // Intermediate component, must be a directory
            match current_node.children.get(component_name) {
                Some(IsoFsNode::Directory(dir)) => current_node = dir,
                _ => {
                    return Err(io_error!(
                        io::ErrorKind::NotFound,
                        "Directory not found in path: {}",
                        path
                    ));
                }
            }
        }
    }
    // This part should be unreachable if components is not empty
    Err(io_error!(
        io::ErrorKind::NotFound,
        "Path not found: {}",
        path
    ))
}

/// Calculate the number of sectors needed for a given file size
pub fn calculate_sectors_from_size(file_size: u64) -> u32 {
    file_size.div_ceil(ISO_SECTOR_SIZE as u64) as u32
}

/// Validate that a boot image size is suitable for the boot catalog
pub fn validate_boot_image_size(size: u64, max_size: u64, image_type: &str) -> io::Result<()> {
    if size > max_size {
        return Err(io_error!(
            io::ErrorKind::InvalidInput,
            "{} image is too large for the boot catalog ({} > {})",
            image_type,
            size,
            max_size
        ));
    }
    Ok(())
}

/// Create a boot catalog entry for boot images (BIOS or UEFI)
fn create_boot_entry(
    root: &IsoDirectory,
    destination_path: &str,
    platform_id: u8,
    image_type: &str,
) -> io::Result<BootCatalogEntry> {
    let lba = get_lba_for_path(root, destination_path)?;
    let size = get_file_size_in_iso(root, destination_path)?;
    const EL_TORITO_SECTOR_SIZE: u64 = 512;
    let sectors = size.div_ceil(EL_TORITO_SECTOR_SIZE).max(1);

    validate_boot_image_size(sectors, u16::MAX as u64, image_type)?;

    Ok(BootCatalogEntry {
        platform_id,
        boot_image_lba: lba,
        boot_image_sectors: sectors as u16,
        bootable: true,
    })
}

/// Create a boot catalog entry for BIOS boot
pub fn create_bios_boot_entry(
    root: &IsoDirectory,
    destination_path: &str,
) -> io::Result<BootCatalogEntry> {
    create_boot_entry(root, destination_path, 0x00, "BIOS boot")
}

/// Create a boot catalog entry for UEFI boot
pub fn create_uefi_boot_entry(
    root: &IsoDirectory,
    destination_path: &str,
) -> io::Result<BootCatalogEntry> {
    create_boot_entry(root, destination_path, BOOT_CATALOG_EFI_PLATFORM_ID, "UEFI boot")
}

/// Create a boot catalog entry for UEFI ESP partition
pub fn create_uefi_esp_boot_entry(
    esp_lba: u32,
    esp_size_sectors: u32,
) -> io::Result<BootCatalogEntry> {
    let boot_image_512_sectors = esp_size_sectors.checked_mul(4).ok_or_else(|| {
        io_error!(
            io::ErrorKind::InvalidInput,
            "UEFI ESP boot image size calculation overflowed"
        )
    })?;

    validate_boot_image_size(boot_image_512_sectors as u64, u16::MAX as u64, "UEFI ESP")?;

    Ok(BootCatalogEntry {
        platform_id: BOOT_CATALOG_EFI_PLATFORM_ID,
        boot_image_lba: esp_lba,
        boot_image_sectors: boot_image_512_sectors as u16,
        bootable: true,
    })
}

/// Get file metadata with consistent error handling
pub fn get_file_metadata(path: &Path) -> io::Result<std::fs::Metadata> {
    std::fs::metadata(path).map_err(|e| {
        io_error!(
            io::ErrorKind::NotFound,
            "Failed to get file metadata for {}: {}",
            path.display(),
            e
        )
    })
}

/// Navigate to a directory by path, creating intermediate directories if they don't exist.
pub fn ensure_directory_path<'a>(
    root: &'a mut IsoDirectory,
    path: &str,
) -> io::Result<&'a mut IsoDirectory> {
    let components: Vec<_> = Path::new(path).components().collect();
    let mut current_dir = root;

    for component in components.iter().take(components.len().saturating_sub(1)) {
        let component_name = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| io_error!(io::ErrorKind::InvalidInput, "Invalid path component"))?;
        current_dir = match current_dir
            .children
            .entry(component_name.to_string())
            .or_insert_with(|| IsoFsNode::Directory(IsoDirectory::new()))
        {
            IsoFsNode::Directory(dir) => dir,
            IsoFsNode::File(_) => {
                return Err(io_error!(
                    io::ErrorKind::AlreadyExists,
                    "Path component '{}' is a file, expected directory",
                    component_name
                ));
            }
        };
    }

    Ok(current_dir)
}

/// Navigate to a directory by path, finding existing directories.
fn navigate_directory_path<'a>(
    root: &'a IsoDirectory,
    components: &[std::path::Component],
) -> io::Result<&'a IsoDirectory> {
    let mut current_dir = root;

    for (i, component) in components.iter().enumerate().take(components.len().saturating_sub(1)) {
        let component_name = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| io_error!(io::ErrorKind::InvalidInput, "Invalid path component"))?;
        match current_dir.children.get(component_name) {
            Some(IsoFsNode::Directory(dir)) => current_dir = dir,
            _ => {
                return Err(io_error!(
                    io::ErrorKind::NotFound,
                    "Directory not found in path: {}",
                    components
                        .iter()
                        .take(i + 1)
                        .map(|c| c.as_os_str().to_str().unwrap_or("??"))
                        .collect::<Vec<_>>()
                        .join("/")
                ));
            }
        };
    }

    Ok(current_dir)
}

/// Validate path components with consistent error handling
pub fn validate_path_components(path: &Path) -> io::Result<()> {
    for component in path.components() {
        component.as_os_str().to_str().ok_or_else(|| {
            io_error!(
                io::ErrorKind::InvalidInput,
                "Invalid path component: {:?}",
                component
            )
        })?;
    }
    Ok(())
}
