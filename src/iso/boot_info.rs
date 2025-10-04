use std::path::PathBuf;

/// High-level boot information for the ISO.
#[derive(Clone, Debug)]
pub struct BootInfo {
    pub bios_boot: Option<BiosBootInfo>,
    pub uefi_boot: Option<UefiBootInfo>,
}

/// Configuration for BIOS boot (El Torito).
#[derive(Clone, Debug)]
pub struct BiosBootInfo {
    pub boot_catalog: PathBuf,
    pub boot_image: PathBuf,
    pub destination_in_iso: String,
}

/// Configuration for UEFI boot.
#[derive(Clone, Debug)]
pub struct UefiBootInfo {
    pub boot_image: PathBuf,
    pub kernel_image: PathBuf,
    pub destination_in_iso: String,
}
