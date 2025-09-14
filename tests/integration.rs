// tests/integration.rs
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
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

    // Create mock files with some content
    let mut bellows_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&bellows_path)?;
    bellows_file.write_all(b"this is a mock bellows file")?;
    bellows_file.seek(SeekFrom::Start(0))?;

    let mut kernel_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&kernel_path)?;
    kernel_file.write_all(b"this is a mock kernel file")?;
    kernel_file.seek(SeekFrom::Start(0))?;

    // Call the main function
    create_disk_and_iso(&fat32_path, &iso_path, &mut bellows_file, &mut kernel_file)?;

    // Assert that the files were created
    assert!(fat32_path.exists());
    assert!(iso_path.exists());

    // Clean up
    fs::remove_file(&fat32_path)?;
    fs::remove_file(&iso_path)?;
    fs::remove_file(&bellows_path)?;
    fs::remove_file(&kernel_path)?;
    Ok(())
}
