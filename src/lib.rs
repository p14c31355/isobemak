// isobemak/src/lib.rs
use std::{io, path::Path};

pub use crate::fat32::create_fat32_image;
pub use crate::iso::create_iso_from_img;

mod fat32;
mod iso;
mod utils;

/// High-level function to create the FAT32 image and then the final ISO.
pub fn create_disk_and_iso(iso_path: &Path, bellows_path: &Path, kernel_path: &Path) -> io::Result<()> {
    let fat32_img_path = Path::new("./fat32.img");

    // 1. Create a pure FAT32 filesystem image.
    println!("create_disk_and_iso: Starting process...");
    create_fat32_image(fat32_img_path, bellows_path, kernel_path)?;

    // 2. Embed the pure FAT32 image into the ISO as the El Torito boot image.
    create_iso_from_img(iso_path, fat32_img_path)?;
    
    // Clean up the temporary FAT32 image
    println!("create_disk_and_iso: Cleaning up temporary FAT32 image.");
    std::fs::remove_file(fat32_img_path)?;
    
    println!("create_disk_and_iso: Process complete. ISO created successfully.");
    Ok(())
}