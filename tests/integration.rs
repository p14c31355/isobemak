// tests/integration.rs
use std::{
    io::{self},
    path::{Path, PathBuf},
};

use isobemak::iso::builder::create_disk_and_iso;
use tempfile::tempdir;

// Helper function to create dummy files and IsoImage for testing.
// This function is defined in src/lib.rs, so we need to import it.
// The original test in tests/integration.rs was not using it.
// We will use it here to correctly construct the IsoImage.
// Note: The setup_iso_creation function in src/lib.rs is for its own internal tests.
// We need to replicate similar logic here for tests/integration.rs.
fn setup_integration_test_files(
    temp_dir: &Path,
) -> io::Result<(PathBuf, PathBuf, PathBuf, PathBuf)> {
    // Create dummy files needed for the ISO image
    let bellows_path = temp_dir.join("bellows.efi");
    std::fs::write(&bellows_path, b"dummy bellows.efi")?;

    let kernel_path = temp_dir.join("kernel.efi");
    std::fs::write(&kernel_path, b"dummy kernel.efi")?;

    let fat_img_path = temp_dir.join("fat.img"); // This is for the UEFI FAT image
    std::fs::write(&fat_img_path, b"dummy fat.img")?;

    let iso_path = temp_dir.join("test.iso");

    Ok((bellows_path, kernel_path, fat_img_path, iso_path))
}

#[test]
fn test_create_disk_and_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;

    // Setup files and paths
    let (bellows_path, _kernel_path, _fat_img_path, iso_path) =
        setup_integration_test_files(temp_dir.path())?;

    // Construct the IsoImage configuration
    let iso_image = isobemak::iso::builder::IsoImage {
        files: vec![
            // Add any other files if needed for the test
        ],
        boot_info: isobemak::iso::builder::BootInfo {
            bios_boot: None, // Not testing BIOS boot in this specific test
            uefi_boot: Some(isobemak::iso::builder::UefiBootInfo {
                boot_image: bellows_path.clone(),
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(), // Standard UEFI path
            }),
        },
    };

    // Call the main function with correct arguments
    create_disk_and_iso(&iso_path, &iso_image)?;

    // Assert that the ISO file was created
    assert!(iso_path.exists());

    Ok(())
}
