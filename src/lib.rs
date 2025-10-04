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

    use crate::create_dummy_files;
    /// Helper function to create dummy files and IsoImage for testing.
    fn setup_iso_creation(temp_dir: &Path) -> io::Result<IsoImage> {
        let files = create_dummy_files!(
            temp_dir,
            "isolinux.bin" => 64,
            "isolinux.cfg" => 1,
            "BOOTX64.EFI" => 64,
            "kernel" => 16,
            "initrd.img" => 16
        );

        let isolinux_bin_path = files.get("isolinux.bin").unwrap().clone();
        let isolinux_cfg_path = files.get("isolinux.cfg").unwrap().clone();
        let bootx64_efi_path = files.get("BOOTX64.EFI").unwrap().clone();
        let kernel_path = files.get("kernel").unwrap().clone();
        let initrd_img_path = files.get("initrd.img").unwrap().clone();

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
                IsoImageFile {
                    source: bootx64_efi_path.clone(),
                    destination: "EFI/BOOT/BOOTX64.EFI".to_string(),
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
                    kernel_image: kernel_path.clone(),
                    destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
                }),
            },
        };

        Ok(iso_image)
    }

    #[test]
    fn test_create_custom_iso_example() -> io::Result<()> {
        let temp_dir = tempdir()?;
        let iso_output_path = temp_dir.path().join("custom_boot.iso");

        let iso_image = setup_iso_creation(temp_dir.path())?;

        // Create the ISO
        build_iso(&iso_output_path, &iso_image, true)?;

        // Assert that the ISO file was created and is not empty
        assert!(iso_output_path.exists());
        assert!(iso_output_path.metadata()?.len() > 0);

        Ok(())
    }
}
