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
///
/// This version ensures the FAT image contains both BOOTX64.EFI (loader)
/// and KERNEL.EFI, and embeds it into an El Torito compatible ISO.
pub fn create_disk_and_iso(
    iso_path: &Path,
    loader_path: &Path,
    kernel_path: &Path,
    fat_img_path: &Path,
) -> io::Result<PathBuf> {
    println!("create_disk_and_iso: Starting process...");

    // --- 1. Create the FAT image ---
    // The FAT image will contain BOOTX64.EFI and KERNEL.EFI automatically.
    create_fat_image(fat_img_path, loader_path, kernel_path)?;
    println!(
        "create_disk_and_iso: FAT image created at {:?}",
        fat_img_path
    );

    // --- 2. Create the ISO from the FAT image ---
    // The ISO will be El Torito compliant and include the FAT boot image.
    create_iso_from_img(iso_path, fat_img_path, kernel_path)?;
    println!(
        "create_disk_and_iso: ISO created successfully at {:?}",
        iso_path
    );

    Ok(fat_img_path.to_owned())
}
