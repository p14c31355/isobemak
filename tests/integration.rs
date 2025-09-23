// tests/integration.rs
use std::{
    io::{self},
    path::{Path, PathBuf},
};

use isobemak::iso::builder::build_iso;
use tempfile::tempdir;

// Helper function to create dummy files for this integration test.
// This is a simplified version of `setup_iso_creation` from the library's internal tests.
fn setup_integration_test_files(temp_dir: &Path) -> io::Result<(PathBuf, PathBuf, PathBuf)> {
    // Create dummy files needed for the ISO image
    let bellows_path = temp_dir.join("bellows.efi");
    std::fs::write(&bellows_path, b"dummy bellows.efi")?;

    let kernel_path = temp_dir.join("kernel.elf");
    std::fs::write(&kernel_path, b"dummy kernel")?;

    let iso_path = temp_dir.join("test.iso");

    Ok((bellows_path, kernel_path, iso_path))
}

#[test]
fn test_create_disk_and_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;

    // Setup files and paths
    let (bellows_path, kernel_path, iso_path) = setup_integration_test_files(temp_dir.path())?;

    // Construct the IsoImage configuration
    let iso_image = isobemak::iso::builder::IsoImage {
        files: vec![
            // Add any other files if needed for the test
        ],
        boot_info: isobemak::iso::builder::BootInfo {
            bios_boot: None, // Not testing BIOS boot in this specific test
            uefi_boot: Some(isobemak::iso::builder::UefiBootInfo {
                boot_image: bellows_path.clone(),
                kernel_image: kernel_path.clone(),
                destination_in_iso: "EFI/BOOT/efi.img".to_string(),
            }),
        },
    };

    let persistent_iso = PathBuf::from("../test.iso");
    // Call the main function with correct arguments
    build_iso(&persistent_iso, &iso_image, false)?;

    // Assert that the ISO file was created
    assert!(persistent_iso.exists());

    Ok(())
}
