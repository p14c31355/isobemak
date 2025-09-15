// isobemak/src/lib.rs
use std::{io, path::Path};

pub use crate::fat32::create_fat32_image;
pub use crate::iso::create_iso_from_img;

mod fat32;
mod iso;
mod utils;

pub fn create_disk_and_iso(
    fat32_img_path: &Path,
    iso_path: &Path,
    bellows_path: &Path,
    kernel_path: &Path,
) -> io::Result<()> {
    create_fat32_image(fat32_img_path, bellows_path, kernel_path)?;
    create_iso_from_img(iso_path, fat32_img_path)?;
    Ok(())
}
