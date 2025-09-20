use crate::fat::create_fat_image;
use crate::iso::create_iso_from_img;
use std::{
    io,
    path::{Path, PathBuf},
};

mod fat;
mod iso;
mod utils;

/// High-level function to create the FAT image and then the final ISO.
pub fn create_disk_and_iso(
    iso_path: &Path,
    loader_path: &Path,
    kernel_path: &Path,
    fat_img_path: &Path,
) -> io::Result<PathBuf> {
    println!("create_disk_and_iso: Starting process...");

    // Create the FAT image and get its padded size
    let fat_img_padded_size = create_fat_image(fat_img_path, loader_path, kernel_path)?;

    // Use the returned size directly to create the ISO
    create_iso_from_img(iso_path, fat_img_path, kernel_path, fat_img_padded_size)?;

    println!("create_disk_and_iso: Process complete. ISO created successfully.");
    Ok(fat_img_path.to_owned())
}
