// lib.rs
//! A library for creating bootable ISO 9660 images with UEFI support.

// Public modules for interacting with the library's core functionalities.
pub mod fat;
pub mod iso;
pub mod utils;
// The builder module contains high-level orchestration logic
// for creating a complete disk and ISO image.
pub mod builder;

#[cfg(test)]
mod tests {
    use super::builder::{create_custom_iso, BiosBootInfo, BootInfo, IsoImage, IsoImageFile, UefiBootInfo};
    use std::path::{Path, PathBuf};
    use std::io;
    use tempfile::tempdir;

    // Helper function to create dummy files and IsoImage
    fn setup_iso_creation(temp_dir: &Path) -> io::Result<(IsoImage, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf)> {
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

        let iso_image = IsoImage {
            files: vec![
                IsoImageFile {
                    source: isolinux_bin_path.clone(),
                    destination: "isolinux/isolinux.bin".to_string(),
                },
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
                    boot_catalog: PathBuf::from("BOOT.CAT"), // This will be generated
                    boot_image: isolinux_bin_path.clone(),
                    destination_in_iso: "isolinux/isolinux.bin".to_string(),
                }),
                uefi_boot: Some(UefiBootInfo {
                    boot_image: bootx64_efi_path.clone(),
                    destination_in_iso: "EFI/BOOT/EFI.img".to_string(),
                }),
            },
        };

        Ok((iso_image, isolinux_bin_path, isolinux_cfg_path, bootx64_efi_path, kernel_path, initrd_img_path))
    }

    #[test]
    fn test_create_custom_iso_example() -> io::Result<()> {
        let temp_dir = tempdir()?;
        let iso_output_path = temp_dir.path().join("custom_boot.iso");

        let (iso_image, _isolinux_bin_path, _isolinux_cfg_path, _bootx64_efi_path, _kernel_path, _initrd_img_path) = setup_iso_creation(temp_dir.path())?;

        create_custom_iso(&iso_output_path, &iso_image)?;

        assert!(iso_output_path.exists());
        println!("Custom ISO created at: {:?}", iso_output_path);

        Ok(())
    }
}