//! A library for creating bootable ISO 9660 images with UEFI support.

// Public modules for interacting with the library's core functionalities.
pub mod fat;
pub mod iso;
pub mod utils;

// Re-export the main function for external use.
pub use iso::builder::build_iso;

#[cfg(test)]
mod tests {
    use super::iso::builder::{
        BiosBootInfo, BootInfo, IsoImage, IsoImageFile, UefiBootInfo, build_iso,
    };
    use std::io;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    /// Helper function to create dummy files and IsoImage for testing.
    fn setup_iso_creation(
        temp_dir: &Path,
    ) -> io::Result<(IsoImage, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf)> {
        // Create dummy files
        let isolinux_bin_path = temp_dir.join("isolinux.bin");
        std::fs::write(&isolinux_bin_path, b"dummy isolinux.bin")?;

        let isolinux_cfg_path = temp_dir.join("isolinux.cfg");
        std::fs::write(&isolinux_cfg_path, b"dummy isolinux.cfg")?;

        let bootx64_efi_path = temp_dir.join("BOOTX64.EFI");
        std::fs::write(&bootx64_efi_path, b"dummy BOOTX64.EFI")?;

        let kernel_path = temp_dir.join("kernel");
        std::fs::write(&kernel_path, b"dummy kernel")?;

        let initrd_img_path = temp_dir.join("initrd.img");
        std::fs::write(&initrd_img_path, b"dummy initrd.img")?;

        // Create the IsoImage configuration
        let iso_image = IsoImage {
            files: vec![
                IsoImageFile {
                    source: isolinux_cfg_path.clone(),
                    destination: "isolinux/isolinux.cfg".to_string(),
                },
                IsoImageFile {
                    source: kernel_path.clone(),
                    destination: "kernel".to_string(),
                },
                IsoImageFile {
                    source: initrd_img_path.clone(),
                    destination: "initrd.img".to_string(),
                },
            ],
            boot_info: BootInfo {
                bios_boot: Some(BiosBootInfo {
                    boot_catalog: PathBuf::from("BOOT.CAT"),
                    boot_image: isolinux_bin_path.clone(),
                    destination_in_iso: "isolinux/isolinux.bin".to_string(),
                }),
                uefi_boot: Some(UefiBootInfo {
                    boot_image: bootx64_efi_path.clone(),
                    destination_in_iso: "EFI/BOOT/EFI.img".to_string(),
                }),
            },
        };

        Ok((
            iso_image,
            isolinux_bin_path,
            isolinux_cfg_path,
            bootx64_efi_path,
            kernel_path,
            initrd_img_path,
        ))
    }

    #[test]
    fn test_create_custom_iso_example() -> io::Result<()> {
        let temp_dir = tempdir()?;
        let iso_output_path = temp_dir.path().join("custom_boot.iso");

        let (iso_image, ..) = setup_iso_creation(temp_dir.path())?;

        // Create the ISO
        build_iso(&iso_output_path, &iso_image, true)?;

        // Assert that the ISO file was created and is not empty
        assert!(iso_output_path.exists());
        assert!(iso_output_path.metadata()?.len() > 0);

        Ok(())
    }
}
