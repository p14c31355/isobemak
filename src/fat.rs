// isobemak/src/fat32.rs
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    path::Path,
};

/// Copies a file from the host filesystem into a FAT directory.
fn copy_to_fat<T: Read + Write + Seek>(
    dir: &fatfs::Dir<T>,
    src_path: &Path,
    dest: &str,
) -> io::Result<()> {
    let mut src_file = File::open(src_path)?;
    let mut f = dir.create_file(dest)?;
    io::copy(&mut src_file, &mut f)?;
    f.flush()?;
    println!("Copied {} to {} in FAT image.", src_path.display(), dest);
    Ok(())
}

/// Creates a FAT image file and populates it with the necessary files for UEFI boot.
/// The image size and format (FAT16 or FAT32) are dynamically calculated based on the size of the bootloader and kernel.
pub fn create_fat_image(
    writer: &mut File,
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
    const MIN_FAT_SIZE: u64 = 65535 * 512; // Max sectors for El Torito Nsect (u16::MAX) * 512 bytes
    const FAT_OVERHEAD: u64 = 2 * 1024 * 1024;
    let mut total_size = (content_size + FAT_OVERHEAD).max(MIN_FAT_SIZE);

    // Round up to the nearest sector size
    const SECTOR_SIZE: u64 = 512;
    total_size = total_size.div_ceil(SECTOR_SIZE) * SECTOR_SIZE;

    writer.set_len(total_size)?;
    println!(
        "create_fat_image: FAT image size set to {} bytes.",
        total_size
    );

    // Determine FAT type based on total size
    let fat_type = if total_size < 32 * 1024 * 1024 { // Use FAT16 for volumes smaller than 32MB
        println!("create_fat_image: Formatting volume as FAT16 due to size.");
        FatType::Fat16
    } else {
        println!("create_fat_image: Formatting volume as FAT32 due to size.");
        FatType::Fat32
    };

    // Format the file as a FAT volume
    fatfs::format_volume(&mut *writer, FormatVolumeOptions::new().fat_type(fat_type))?;

    // Open filesystem and create directories
    let fs = FileSystem::new(&mut *writer, FsOptions::new())?;
    let root_dir = fs.root_dir();
    let efi_dir = root_dir.create_dir("EFI")?;
    let boot_dir = efi_dir.create_dir("BOOT")?;

    // Copy the bootloader and kernel into the FAT filesystem
    println!("create_fat_image: Copying bootloader and kernel.");
    copy_to_fat(&boot_dir, loader_path, "BOOTX64.EFI")?;
    copy_to_fat(&boot_dir, kernel_path, "KERNEL.EFI")?;

    println!("create_fat_image: FAT image creation complete.");
    u32::try_from(total_size)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Image size exceeds 4GB limit"))
}
