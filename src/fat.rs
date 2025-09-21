// isobemak/src/fat.rs
use crate::utils;
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::{
    fs::OpenOptions,
    io::{self, Seek, Write},
    path::Path,
};

/// Creates a FAT image file for UEFI boot and populates it with a loader and kernel.
/// The image size and format (FAT16 or FAT32) are dynamically calculated.
pub fn create_fat_image(
    fat_img_path: &Path,
    loader_path: &Path,
    kernel_path: &Path,
) -> io::Result<u32> {
    // Ensure both files exist
    if !loader_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Loader file not found at {:?}", loader_path),
        ));
    }

    if !kernel_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Kernel file not found at {:?}", kernel_path),
        ));
    }

    // Calculate the minimum image size
    let loader_size = loader_path.metadata()?.len();
    let kernel_size = kernel_path.metadata()?.len();
    let content_size = loader_size + kernel_size;

    // Add overhead and enforce a minimum size.
    const MIN_FAT_SIZE: u64 = 16 * 1024 * 1024; // 16MB. Ensures FAT16 formatting.
    const FAT_OVERHEAD: u64 = 2 * 1024 * 1024; // 2MB. Overhead for filesystem structures.
    let mut total_size = std::cmp::max(content_size + FAT_OVERHEAD, MIN_FAT_SIZE);

    // Round up to the nearest sector size
    const SECTOR_SIZE: u64 = 512;
    total_size = total_size.div_ceil(SECTOR_SIZE) * SECTOR_SIZE;

    // Determine FAT type based on size
    let fat_type = if total_size <= 268_435_456 {
        FatType::Fat16
    } else {
        FatType::Fat32
    };

    // Create the file and set its length
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(fat_img_path)?;
    file.set_len(total_size)?;
    file.flush()?;
    file.seek(io::SeekFrom::Start(0))?;

    // Format the FAT image
    fatfs::format_volume(&mut file, FormatVolumeOptions::new().fat_type(fat_type))?;

    // Open filesystem and create directories
    let fs = FileSystem::new(&mut file, FsOptions::new())?;
    let root_dir = fs.root_dir();
    let efi_dir = root_dir.create_dir("EFI")?;
    let boot_dir = efi_dir.create_dir("BOOT")?;

    // Copy the bootloader and kernel into the FAT filesystem
    utils::copy_to_fat(&boot_dir, loader_path, "BOOTX64.EFI")?;
    utils::copy_to_fat(&boot_dir, kernel_path, "KERNEL.EFI")?;

    Ok(total_size as u32 / utils::ISO_SECTOR_SIZE as u32)
}
