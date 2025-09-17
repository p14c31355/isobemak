// isobemak/src/lib.rs
use std::{io, path::Path};

pub use crate::fat32::{create_fat32_image, extract_fat32_partition};
pub use crate::iso::create_iso_from_img;

mod fat32;
mod iso;
mod utils;

/// High-level function to create the FAT32 image and then the final ISO.
pub fn create_disk_and_iso(
    iso_path: &Path,
    bellows_path: &Path,
    kernel_path: &Path,
) -> io::Result<()> {
    let fat32_img_path = Path::new("./fat32.img");
    let esp_img_path = Path::new("./esp.img");

    // 1. Create a full disk image with MBR and a FAT32 partition.
    println!("Creating full FAT32 disk image with MBR...");
    create_fat32_image(fat32_img_path, bellows_path, kernel_path)?;

    // 2. Extract the pure FAT32 filesystem partition from the MBR image.
    println!("Extracting pure FAT32 partition...");
    extract_fat32_partition(fat32_img_path, esp_img_path)?;

    // 3. Embed the pure FAT32 image into the ISO as the El Torito boot image.
    println!("Creating bootable ISO from the pure FAT32 partition...");
    create_iso_from_img(iso_path, esp_img_path)?;

    // Clean up temporary files
    std::fs::remove_file(fat32_img_path)?;
    std::fs::remove_file(esp_img_path)?;

    Ok(())
}
