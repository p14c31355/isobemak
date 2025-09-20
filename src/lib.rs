use crate::fat::create_fat_image;
use crate::iso::create_iso_from_img;
use std::{io, io::Write, path::Path};
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

    let mut fat_img_file = NamedTempFile::new()?;
    let fat_img_path = fat_img_file.path().to_owned();

    let _fat_image_size = create_fat_image(fat_img_file.as_file_mut(), loader_path, kernel_path)?;

    fat_img_file.as_file_mut().flush()?;

    let boot_img_size = std::fs::metadata(loader_path)?.len();
    let boot_img_sectors = boot_img_size.div_ceil(512);

    create_iso_from_img(iso_path, loader_path, kernel_path, boot_img_sectors as u32 * 512)?;

    println!("create_disk_and_iso: Process complete. ISO created successfully.");
    Ok(())
}
