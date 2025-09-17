// isobemak/src/lib.rs
use std::{io, path::Path};
use crate::iso::create_iso_from_img;
use crate::fat32::create_fat32_image;
use tempfile::NamedTempFile;

mod fat32;
mod iso;
mod utils;

/// High-level function to create the FAT32 image and then the final ISO.
pub fn create_disk_and_iso(
    iso_path: &Path,
    bellows_path: &Path,
    kernel_path: &Path,
) -> io::Result<()> {
    println!("create_disk_and_iso: Starting process...");

    // 1. Create a temporary FAT32 image file.
    let mut fat32_img_file = NamedTempFile::new()?;
    let fat32_img_path = fat32_img_file.path().to_owned();

    // 2. Create and populate the FAT32 filesystem.
    create_fat32_image(
        fat32_img_file.as_file_mut(),
        bellows_path,
        kernel_path,
    )?;

    // 3. Create the ISO, embedding the temporary FAT32 filesystem.
    create_iso_from_img(iso_path, &fat32_img_path)?;

    // 4. The temporary file will be automatically deleted when it goes out of scope.
    println!("create_disk_and_iso: Process complete. ISO created successfully.");
    Ok(())
}
