// isobemak/src/fat32.rs
use crate::utils::FAT32_SECTOR_SIZE;
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, Write},
    path::Path,
};

const FAT32_IMAGE_SECTOR_COUNT: u64 = 0xFFFF;
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

/// Creates a pure FAT32 filesystem image (not a full disk image with MBR).
/// This image will be used directly as the El Torito boot image.
pub fn create_fat32_image(path: &Path, bellows_path: &Path, kernel_path: &Path) -> io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)?;
    file.set_len(FAT32_IMAGE_SIZE)?;

    // Format the entire file as a single FAT32 volume. This creates a pure FS image.
    fatfs::format_volume(
        &mut file,
        FormatVolumeOptions::new().fat_type(FatType::Fat32),
    )?;

    {
        // Open the formatted file as a FAT filesystem
        let fs = FileSystem::new(&mut file, FsOptions::new())?;
        let root = fs.root_dir();
        let efi_dir = root.create_dir("EFI")?;
        let boot_dir = efi_dir.create_dir("BOOT")?;

        // Copy EFI executables to the correct location
        copy_to_fat(&boot_dir, bellows_path, "BOOTX64.EFI")?;
        copy_to_fat(&boot_dir, kernel_path, "KERNEL.EFI")?;
    }

    file.sync_all()?;

    Ok(())
}
