// isobemak/src/lib.rs
pub use crate::fat32::create_fat32_image;
pub use crate::iso::create_iso;
use std::{fs::File, io, path::Path};

mod fat32;
mod iso;
mod utils;

pub fn create_disk_and_iso(
    fat32_img: &Path,
    iso: &Path,
    bellows: &mut File,
    kernel: &mut File,
) -> io::Result<()> {
    create_fat32_image(fat32_img, bellows, kernel)?;
    println!("FAT32 image successfully created.");
    create_iso(iso, fat32_img)?;
    println!("ISO successfully created.");
    Ok(())
}
