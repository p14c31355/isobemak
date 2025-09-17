// isobemak/src/fat32.rs
use crate::utils::FAT32_SECTOR_SIZE;
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, Write},
    path::Path,
};

const FAT32_IMAGE_SECTOR_COUNT: u64 = 2048; 
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

    fatfs::format_volume(
        &mut file,
        FormatVolumeOptions::new().fat_type(FatType::Fat32),
    )?;

    let fs = FileSystem::new(&mut file, FsOptions::new())?;
    let root_dir = fs.root_dir();
    let efi_dir = root_dir.create_dir("EFI")?;
    let boot_dir = efi_dir.create_dir("BOOT")?;

    // Copying `bellows.efi` to `\EFI\BOOT\BOOTX64.EFI`
    assert!(bellows_path.exists(), "bellows.efi not found at {:?}", bellows_path);
    copy_to_fat(&boot_dir, bellows_path, "BOOTX64.EFI")?;

    // Copying `kernel.bin` to `\kernel.bin`
    assert!(kernel_path.exists(), "kernel.bin not found at {:?}", kernel_path);
    copy_to_fat(&root_dir, kernel_path, "kernel.bin")?;

    Ok(())
}