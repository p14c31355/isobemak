// tests/integration.rs
use std::{
    fs::{self, File},
    io::{self, Write},
};

use isobemak::create_disk_and_iso;
use tempfile::tempdir;

#[test]
fn test_create_disk_and_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let fat32_path = temp_dir.path().join("test.img");
    let iso_path = temp_dir.path().join("test.iso");

    let bellows_path = temp_dir.path().join("bellows.efi");
    let kernel_path = temp_dir.path().join("kernel.efi");

    // Create mock files and ensure they are flushed by closing the handles
    File::create(&bellows_path)?.write_all(b"this is a mock bellows file")?;
    File::create(&kernel_path)?.write_all(b"this is a mock kernel file")?;

    // Call the main function
    create_disk_and_iso(&fat32_path, &iso_path, &bellows_path, &kernel_path)?;

    // Assert that the files were created
    assert!(fat32_path.exists());
    assert!(iso_path.exists());

    Ok(())
}
