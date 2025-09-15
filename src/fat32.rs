// isobemak/src/fat32.rs
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, Write},
    path::Path,
};

const FAT32_IMAGE_SIZE: u64 = 0xFFFF * 512; // 33553920 bytes (65535 sectors of 512 bytes)

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
    let root = fs.root_dir();
    let efi_dir = root.create_dir("EFI")?;
    let boot_dir = efi_dir.create_dir("BOOT")?;

    copy_to_fat(&boot_dir, bellows_path, "BOOTX64.EFI")?;
    copy_to_fat(&boot_dir, kernel_path, "KERNEL.EFI")?;

    Ok(())
}
