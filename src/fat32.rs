// isobemak/src/fat32.rs
use crate::utils::FAT32_SECTOR_SIZE;
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, Write},
    path::Path,
};

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

    // Determine the size of the files to be included
    let bellows_metadata = fs::metadata(bellows_path)?;
    let kernel_metadata = fs::metadata(kernel_path)?;
    let total_file_size = bellows_metadata.len() + kernel_metadata.len();

    // Add a buffer for filesystem overhead (boot sector, FAT, root dir).
    // A 1 MiB buffer should be more than sufficient.
    const BUF_SIZE: u64 = 1024 * 1024;
    let image_size = total_file_size + BUF_SIZE;

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)?;
    file.set_len(image_size)?;

    fatfs::format_volume(
        &mut file,
        FormatVolumeOptions::new().fat_type(FatType::Fat32),
    )?;

    {
        let fs = FileSystem::new(&mut file, FsOptions::new())?;
        let root = fs.root_dir();
        let efi_dir = root.create_dir("EFI")?;
        let boot_dir = efi_dir.create_dir("BOOT")?;

        copy_to_fat(&boot_dir, bellows_path, "BOOTX64.EFI")?;
        copy_to_fat(&boot_dir, kernel_path, "KERNEL.EFI")?;
    }

    file.sync_all()?;
    Ok(())
}