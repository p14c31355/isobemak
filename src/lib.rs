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

    create_fat32_image(fat32_img_path, bellows_path, kernel_path)?;

    create_iso_from_img(iso_path, fat32_img_path)?;
    Ok(())
}