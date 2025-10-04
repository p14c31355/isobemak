use crate::iso::boot_info::BootInfo;
use std::path::PathBuf; // Import BootInfo

/// Configuration for a file to be added to the ISO.
#[derive(Clone, Debug)]
pub struct IsoImageFile {
    pub source: PathBuf,
    pub destination: String,
}

/// Configuration for the entire ISO image to be built.
#[derive(Clone, Debug)]
pub struct IsoImage {
    pub files: Vec<IsoImageFile>,
    pub boot_info: BootInfo,
}
