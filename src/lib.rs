// isobemak/src/lib.rs
use crate::fat::create_fat_image;
use crate::iso::create_iso_from_img;
use std::{io, io::Write, path::Path, fs};
use tempfile::NamedTempFile;

mod fat;
mod iso;
mod utils;

/// High-level function to create the FAT32 image and then the final ISO.
pub fn create_disk_and_iso(
    iso_path: &Path,
    loader_path: &Path,
    kernel_path: &Path,
) -> io::Result<()> {
    println!("create_disk_and_iso: Starting process...");

    // 1. Create a temporary FAT32 image file.
    let mut fat_img_file = NamedTempFile::new()?;
    let fat_img_path = fat_img_file.path().to_owned();

    let _fat_image_actual_size =
        create_fat_image(fat_img_file.as_file_mut(), loader_path, kernel_path)?;


    fat_img_file.as_file_mut().flush()?;

    let loader_metadata = fs::metadata(loader_path)?;
    let loader_size = loader_metadata.len() as u32;
    create_iso_from_img(iso_path, &fat_img_path, loader_size)?;

    // 4. The temporary file will be automatically deleted when it goes out of scope.
    println!("create_disk_and_iso: Process complete. ISO created successfully.");
    Ok(())
}
