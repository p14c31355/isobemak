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
    let bootx64_path = temp_dir.join("bootx64.efi");
    std::fs::write(&bootx64_path, b"dummy bootx64.efi")?;

    let kernel_path = temp_dir.join("kernel.elf");
    std::fs::write(&kernel_path, b"dummy kernel")?;

    let iso_path = temp_dir.join("test.iso");

    Ok((bootx64_path, kernel_path, iso_path))
}

#[test]
fn test_create_disk_and_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;

    // Setup files and paths
    let (bootx64_path, kernel_path, iso_path) = setup_integration_test_files(temp_dir.path())?;

    let iso_image = isobemak::iso::builder::IsoImage {
        files: vec![
            // Add any other files if needed for the test
        ],
        boot_info: isobemak::iso::builder::BootInfo {
            bios_boot: None, // Not testing BIOS boot in this specific test
            uefi_boot: Some(isobemak::iso::builder::UefiBootInfo {
                boot_image: bootx64_path.clone(),
                kernel_image: kernel_path.clone(),
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
            }),
        },
    };

        // Call the main function with correct arguments
    build_iso(&iso_path, &iso_image, false)?;
    // Assert that the ISO file was created
    assert!(iso_path.exists());

    Ok(())
}
