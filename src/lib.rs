// lib.rs
use crate::iso::create_iso_from_img;
use std::{io, path::Path};

mod iso;
mod utils;

/// High-level function to create a UEFI ISO.
/// This version no longer creates a BIOS boot FAT image, and instead
/// embeds the EFI loader directly for UEFI boot.
pub fn create_disk_and_iso(
    iso_path: &Path,
    loader_path: &Path,
    kernel_path: &Path,
) -> io::Result<()> {
    println!("create_disk_and_iso: Starting UEFI ISO creation...");

    // Create the ISO from the EFI loader and kernel directly
    create_iso_from_img(iso_path, loader_path, kernel_path)?;
    println!("create_disk_and_iso: ISO created successfully at {:?}", iso_path);

    Ok(())
}
