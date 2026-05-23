use crate::iso::boot_info::BootInfo;
use crate::iso::layout_profile::IsoLayoutProfile;
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
    /// Defaults to ISOBEMAKI. Maximum length is 32 bytes.
    pub volume_id: Option<String>,
    pub files: Vec<IsoImageFile>,
    pub boot_info: BootInfo,
    /// ISO layout profile for firmware compatibility.
    /// Default: [IsoLayoutProfile::hardware] (GPT disabled, 2 MiB ESP alignment).
    /// For QEMU/OVMF, use [IsoLayoutProfile::emulator] (GPT enabled).
    pub layout_profile: IsoLayoutProfile,
}
