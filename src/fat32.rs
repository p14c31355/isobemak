// isobemak/src/fat32.rs
use crate::utils::FAT32_SECTOR_SIZE;
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    path::Path,
};

// To ensure the volume is formatted as FAT32, we must use a sufficiently large size.
// A cluster count greater than 65524 is required for FAT32.
// 131072 sectors (64MB) will result in a cluster count well over this limit.
const FAT32_IMAGE_SECTOR_COUNT: u64 = 131072;
const FAT32_IMAGE_SIZE: u64 = FAT32_IMAGE_SECTOR_COUNT * FAT32_SECTOR_SIZE;

/// Copies a file from the host filesystem into a FAT32 directory.
fn copy_to_fat<T: Read + Write + Seek>(
    dir: &fatfs::Dir<T>,
    src_path: &Path,
    dest: &str,
) -> io::Result<()> {
    let mut src_file = File::open(src_path)?;
    let mut f = dir.create_file(dest)?;
    io::copy(&mut src_file, &mut f)?;
    f.flush()?;
    println!("Copied {} to {} in FAT32 image.", src_path.display(), dest);
    Ok(())
}

/// Creates a FAT32 image file and populates it with the necessary files for UEFI boot.
/// This function uses the `fatfs` crate for high-level filesystem operations.
pub fn create_fat32_image(
    writer: &mut File,
    bellows_path: &Path,
    kernel_path: &Path,
) -> io::Result<u32> {
    println!("create_fat32_image: Starting creation of FAT32 image.");

    // 1. Set the size of the image file
    writer.set_len(FAT32_IMAGE_SIZE)?;

    // 2. Format the file as a FAT32 volume
    println!("create_fat32_image: Formatting volume as FAT32.");
    fatfs::format_volume(
        &mut *writer,
        FormatVolumeOptions::new().fat_type(FatType::Fat32),
    )?;

    // 3. Get the root directory and create the necessary directory structure
    let fs = FileSystem::new(&mut *writer, FsOptions::new())?;
    let root_dir = fs.root_dir();
    let efi_dir = root_dir.create_dir("EFI")?;
    let boot_dir = efi_dir.create_dir("BOOT")?;

    // 4. Copy the bootloader and kernel into the FAT32 filesystem
    println!("create_fat32_image: Copying bootloader and kernel.");

    if !bellows_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("bellows.efi not found at {:?}", bellows_path),
        ));
    }
    copy_to_fat(&boot_dir, bellows_path, "BOOTX64.EFI")?;

    if !kernel_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("kernel.bin not found at {:?}", kernel_path),
        ));
    }
    copy_to_fat(&boot_dir, kernel_path, "KERNEL.EFI")?;

    println!("create_fat32_image: FAT32 image creation complete.");
    Ok(FAT32_IMAGE_SIZE as u32)
}
