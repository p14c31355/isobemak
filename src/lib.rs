// isobemak/src/lib.rs
pub use crate::fat32::create_fat32_image;
pub use crate::iso::create_iso;
use std::{
    fs::File,
    io::{self},
    path::Path,
};

mod fat32;
mod iso;
mod utils;

pub fn create_disk_and_iso(
    fat32_img: &Path,
    iso: &Path,
    bellows_path: &Path,
    kernel_path: &Path,
) -> io::Result<()> {
    create_fat32_image(fat32_img, bellows_path, kernel_path)?;
    println!("FAT32 image successfully created.");

    let mut bellows_file = File::open(bellows_path)?;
    let mut kernel_file = File::open(kernel_path)?;

    create_iso(iso, &mut bellows_file, &mut kernel_file)?;
    println!("ISO successfully created.");
    Ok(())
}
