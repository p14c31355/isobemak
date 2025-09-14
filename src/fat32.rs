// isobemak/src/fat32.rs
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

const FAT32_IMAGE_SIZE: u64 = 32 * 1024 * 1024; // 32 MiB

fn copy_to_fat<T: Read + Write + Seek>(
    dir: &fatfs::Dir<T>,
    src_file: &mut File,
    dest: &str,
) -> io::Result<()> {
    let mut f = dir.create_file(dest)?;
    src_file.seek(SeekFrom::Start(0))?;
    io::copy(src_file, &mut f)?;
    Ok(())
}

pub fn create_fat32_image(path: &Path, bellows: &mut File, kernel: &mut File) -> io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)?;
    file.set_len(FAT32_IMAGE_SIZE)?; // 32 MiB
    {
        fatfs::format_volume(
            &mut file,
            FormatVolumeOptions::new().fat_type(FatType::Fat32),
        )?;
        let fs = FileSystem::new(&mut file, FsOptions::new())?;
        let root = fs.root_dir();
        root.create_dir("EFI/BOOT")?;
        copy_to_fat(&root, bellows, "EFI/BOOT/BOOTX64.EFI")?;
        copy_to_fat(&root, kernel, "EFI/BOOT/KERNEL.EFI")?;
    }
    file.sync_all()?;
    println!("FAT32 image successfully created at {}", path.display());
    Ok(())
}
