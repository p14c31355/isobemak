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
    pub boot_image: PathBuf,
    pub destination_in_iso: String,
}

/// Configuration for UEFI boot.
#[derive(Clone, Debug)]
pub struct UefiBootInfo {
    pub boot_image: PathBuf,
    pub kernel_image: PathBuf,
    pub destination_in_iso: String,
    /// Additional EFI boot files to include in the ESP FAT image (for isohybrid).
    /// Each entry is (destination_filename, source_path) copied to `EFI/BOOT/` in the ESP.
    /// For example, `("GRUBX64.EFI", path_to_grub)`.
    pub additional_efi_boot_files: Vec<(String, PathBuf)>,
    /// Optional content for an auto-generated `grub.cfg` placed in `EFI/BOOT/grub.cfg`
    /// in the ESP FAT image. If `None`, no grub.cfg is created.
    /// Example: `Some("set default=0\nset timeout=5\nmenuentry \"Boot\" {\n  chainloader /EFI/BOOT/BOOTX64.EFI\n}")`
    pub grub_cfg_content: Option<String>,
}
