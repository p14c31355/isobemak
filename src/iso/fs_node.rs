use crate::utils::ISO_SECTOR_SIZE;
use std::collections::HashMap;
use std::path::PathBuf;

/// Represents a file within the ISO filesystem.
#[derive(Clone, Debug)]
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

impl IsoFsNode {
    /// Returns the LBA of the node.
    pub fn lba(&self) -> u32 {
        match self {
            IsoFsNode::File(file) => file.lba,
            IsoFsNode::Directory(dir) => dir.lba,
        }
    }

    /// Returns the size of the node.
    pub fn size(&self) -> u64 {
        match self {
            IsoFsNode::File(file) => file.size,
            IsoFsNode::Directory(dir) => dir.size as u64,
        }
    }
}
