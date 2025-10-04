use std::io::{self, Seek, SeekFrom};
use std::path::Path;

use crate::utils::ISO_SECTOR_SIZE;
use crate::iso::fs_node::{IsoDirectory, IsoFile, IsoFsNode};

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
        let component_name = component.as_os_str().to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Invalid path component")
        })?;

        if i == components.len() - 1 {
            // Last component, this is the target node
            return current_node.children.get(component_name).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("Path not found: {}", path))
            });
        } else {
            // Intermediate component, must be a directory
            match current_node.children.get(component_name) {
                Some(IsoFsNode::Directory(dir)) => current_node = dir,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("Directory not found in path: {}", path),
                    ));
                }
            }
        }
    }
    // This part should be unreachable if components is not empty
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("Path not found: {}", path),
    ))
}
