// tests/integration.rs
use std::{
    fs::File,
    io::{self, Write},
};

use isobemak::iso_builder::create_disk_and_iso;
use tempfile::tempdir;

#[test]
fn test_create_disk_and_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let iso_path = temp_dir.path().join("test.iso");
    let fat_img_path = temp_dir.path().join("test.img");

    let bellows_path = temp_dir.path().join("bellows.efi");
    let kernel_path = temp_dir.path().join("kernel.efi");

    // Create mock files and ensure they are flushed by closing the handles
    File::create(&bellows_path)?.write_all(b"this is a mock bellows file")?;
    File::create(&kernel_path)?.write_all(b"this is a mock kernel file")?;

    // Call the main function
    create_disk_and_iso(&iso_path, &bellows_path, &kernel_path, &fat_img_path)?;

    // Assert that the files were created
    assert!(iso_path.exists());

    Ok(())
}
