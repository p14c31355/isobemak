use crate::fat::create_fat_image;
use crate::iso::create_iso_from_img;
use std::{io, path::{Path, PathBuf}}; // io::Write を削除

mod fat;
mod iso;
mod utils;

/// High-level function to create the FAT32 image and then the final ISO.
pub fn create_disk_and_iso(
    iso_path: &Path,
    loader_path: &Path,
    kernel_path: &Path,
    fat_img_path: &Path,
) -> io::Result<PathBuf> {
    println!("create_disk_and_iso: Starting process...");

    let _fat_image_size = create_fat_image(fat_img_path, loader_path, kernel_path)?;

    let boot_img_size = std::fs::metadata(loader_path)?.len();
    let _boot_img_sectors = boot_img_size.div_ceil(512);

    let fat_img_metadata = std::fs::metadata(&fat_img_path)?;
    let fat_img_padded_size = (fat_img_metadata.len()).div_ceil(512) * 512;
    create_iso_from_img(
        iso_path,
        &fat_img_path,
        kernel_path,
        fat_img_padded_size as u32,
    )?;

    println!("create_disk_and_iso: Process complete. ISO created successfully.");
    Ok(fat_img_path.to_owned())
}
