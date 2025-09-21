// src/builder.rs
use crate::fat;
use crate::iso::builder::IsoBuilder as NewIsoBuilder;
use crate::iso::iso::create_iso_from_img;
use std::{
    io,
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

pub struct IsoImageFile {
    pub source: PathBuf,
    pub destination: String,
}

pub struct IsoImage {
    pub files: Vec<IsoImageFile>,
    pub boot_info: BootInfo,
}

#[derive(Clone)]
pub struct BootInfo {
    pub bios_boot: Option<BiosBootInfo>,
    pub uefi_boot: Option<UefiBootInfo>,
}

#[derive(Clone)]
pub struct BiosBootInfo {
    pub boot_catalog: PathBuf,
    pub boot_image: PathBuf,
    pub destination_in_iso: String,
}

#[derive(Clone)]
pub struct UefiBootInfo {
    pub boot_image: PathBuf,
    pub destination_in_iso: String,
}

pub fn create_custom_iso(iso_path: &Path, image: &IsoImage) -> io::Result<()> {
    let mut iso_builder = NewIsoBuilder::new();

    // Add all regular files to the ISO builder
    for file in &image.files {
        iso_builder.add_file(&file.destination, file.source.clone())?;
    }

    // Handle UEFI boot image
    let mut uefi_fat_img_path = None;
    let mut dummy_kernel_file_for_fat_creation = None;
    let mut temp_fat_file_holder: Option<NamedTempFile> = None; // Hold the NamedTempFile

    if let Some(uefi_boot_info) = &image.boot_info.uefi_boot {
        let temp_fat_file = NamedTempFile::new()?;
        let fat_img_path = temp_fat_file.path().to_path_buf();

        // Create a dummy kernel path in the same directory as the ISO output
        // This ensures it lives long enough and is cleaned up with the temp_dir of the test.
        let dummy_kernel_path = iso_path.parent().unwrap().join("dummy_kernel_for_fat");
        std::fs::write(&dummy_kernel_path, b"")?;
        dummy_kernel_file_for_fat_creation = Some(dummy_kernel_path.clone());

        fat::create_fat_image(
            &fat_img_path,
            &uefi_boot_info.boot_image,
            &dummy_kernel_path,
        )?;
        iso_builder.add_file(&uefi_boot_info.destination_in_iso, fat_img_path.clone())?;
        uefi_fat_img_path = Some(fat_img_path);
        temp_fat_file_holder = Some(temp_fat_file); // Keep the NamedTempFile alive
    }

    // Set boot information for the ISO builder
    iso_builder.set_boot_info(image.boot_info.clone());

    // Build the ISO
    iso_builder.build(iso_path)?;

    // Clean up temporary FAT image if created
    if let Some(path) = uefi_fat_img_path {
        std::fs::remove_file(path)?;
    }
    // Clean up dummy kernel file if created
    if let Some(path) = dummy_kernel_file_for_fat_creation {
        std::fs::remove_file(path)?;
    }
    // temp_fat_file_holder will be dropped here, cleaning up the actual temp file

    Ok(())
}

/// High-level function to create the FAT image and then the final ISO.
///
/// This version ensures the FAT image contains both BOOTX64.EFI (loader)
/// and KERNEL.EFI, and embeds it into an El Torito compatible ISO.
#[deprecated(note = "Use `create_custom_iso` instead")]
pub fn create_disk_and_iso(
    iso_path: &Path,
    loader_path: &Path,
    kernel_path: &Path,
    fat_img_path: &Path,
) -> io::Result<PathBuf> {
    println!("create_disk_and_iso: Starting process...");

    // 1. Create the FAT image.
    fat::create_fat_image(fat_img_path, loader_path, kernel_path)?;
    println!(
        "create_disk_and_and_iso: FAT image created at {:?}",
        fat_img_path
    );

    // 2. Create the ISO from the FAT image.
    create_iso_from_img(iso_path, fat_img_path, kernel_path)?;
    println!(
        "create_disk_and_iso: ISO created successfully at {:?}",
        iso_path
    );

    Ok(fat_img_path.to_owned())
}
