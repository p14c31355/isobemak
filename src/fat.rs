// isobemak/src/fat.rs
use crate::utils;
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::{
    fs::OpenOptions,
    io::{self, Seek, Write},
    path::Path,
};

/// Creates a FAT image file and populates it with the necessary files for UEFI boot.
/// The image size and format (FAT16 or FAT32) are dynamically calculated based on the size of the bootloader and kernel.
pub fn create_fat_image(
    fat_img_path: &Path,
    loader_path: &Path,
    kernel_path: &Path,
) -> io::Result<u32> {
    println!("create_fat_image: Starting creation of FAT image.");

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

    // Calculate the minimum image size based on both files
    let loader_size = loader_path.metadata()?.len();
    let kernel_size = kernel_path.metadata()?.len();
    let content_size = loader_size + kernel_size;

    // Add overhead and enforce a minimum size.
    const MIN_FAT_SIZE: u64 = 16 * 1024 * 1024; // 16MB. Ensure FAT16 formatting for El Torito compatibility.
    const FAT_OVERHEAD: u64 = 2 * 1024 * 1024;
    let mut total_size = (content_size + FAT_OVERHEAD).max(MIN_FAT_SIZE);

    // Round up to the nearest sector size
    const SECTOR_SIZE: u64 = 512;
    total_size = total_size.div_ceil(SECTOR_SIZE) * SECTOR_SIZE;

    // Determine FAT type based on total size
    let fat_type = if total_size < 32 * 1024 * 1024 {
        println!("create_fat_image: Formatting volume as FAT16 due to size.");
        FatType::Fat16
    } else {
        println!("create_fat_image: Formatting volume as FAT32 due to size.");
        FatType::Fat32
    };

    // Create the file and set its length
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true) // ファイルが存在する場合は切り詰める
        .open(fat_img_path)?;
    file.set_len(total_size)?;
    println!(
        "create_fat_image: FAT image size set to {} bytes.",
        total_size
    );

    // Format the FAT image
    fatfs::format_volume(&mut file, FormatVolumeOptions::new().fat_type(fat_type))?;
    file.flush()?; // Ensure all data is written to disk
    file.seek(io::SeekFrom::Start(0))?; // ファイルポインタを先頭に戻す

    // Open filesystem and create directories
    let fs = FileSystem::new(&mut file, FsOptions::new())?;
    let root_dir = fs.root_dir();
    let efi_dir = root_dir.create_dir("EFI")?;
    let boot_dir = efi_dir.create_dir("BOOT")?;

    // Copy the bootloader and kernel into the FAT filesystem
    println!("create_fat_image: Copying bootloader and kernel.");
    utils::copy_to_fat(&boot_dir, loader_path, "BOOTX64.EFI")?;
    utils::copy_to_fat(&boot_dir, kernel_path, "KERNEL.EFI")?;

    println!("create_fat_image: FAT image creation complete.");
    u32::try_from(total_size)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Image size exceeds 4GB limit"))
}
