// isobemak/src/fat.rs
use fatfs::{Dir, FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::{
    fs::{self, OpenOptions},
    io::{self, Seek, Write},
    path::Path,
};

/// Copies a file into a directory within the FAT filesystem.
fn copy_to_fat(fat_dir: &Dir<fs::File>, source_path: &Path, dest_name: &str) -> io::Result<()> {
    let mut dest_file = fat_dir.create_file(dest_name)?;
    let mut source_file = fs::File::open(source_path)?;
    io::copy(&mut source_file, &mut dest_file)?;
    Ok(())
}

/// Creates a FAT image file for UEFI boot and populates it with files.
/// The image size and format (FAT16 or FAT32) are dynamically calculated.
///
/// `files` is a list of (destination_filename, source_path) pairs copied to `EFI/BOOT/`.
/// `hidden_sectors` sets the BPB hidden sectors field (LBA of the partition start in 512B sectors).
/// For USB-HDD boot on real hardware (NEC/Insyde/AMI), this MUST be the partition's
/// starting LBA in 512-byte sectors, NOT zero.  Setting it to zero tells firmware the
/// FAT volume starts at LBA 0 of the disk, which is only true for raw floppy/superfloppy
/// images without an MBR.
pub fn create_fat_image(
    fat_img_path: &Path,
    files: &[(&str, &Path)],
    hidden_sectors: u32,
) -> io::Result<u32> {
    // Ensure all input files exist
    let mut content_size = 0u64;
    for (dest_name, source_path) in files {
        if !source_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found at {:?} (dest: {})", source_path, dest_name),
            ));
        }
        content_size += source_path.metadata()?.len();
    }
    if files.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "At least one file is required to create a FAT image",
        ));
    }

    // Add overhead and enforce a minimum size.
    // Keep well under 32768 512-byte sectors to avoid Nsect exceeding i16::MAX
    // (the boot catalog stores sector count as u16, but some UEFI firmware treats it as signed).
    const MIN_FAT_SIZE: u64 = 8 * 1024 * 1024; // 8MB = 16384 sectors. Enough for UEFI boot files.
    const FAT_OVERHEAD: u64 = 2 * 1024 * 1024; // 2MB. Overhead for filesystem structures.
    const SECTOR_SIZE: u64 = 512;

    // Calculate the logical size based on content + overhead, rounded up to sector size.
    let mut logical_size = (content_size + FAT_OVERHEAD).div_ceil(SECTOR_SIZE) * SECTOR_SIZE;
    // Ensure logical_size is at least one sector
    if logical_size == 0 {
        logical_size = SECTOR_SIZE;
    }

    // The actual total size of the FAT image file, ensuring it meets minimum requirements for FAT16.
    let total_size = std::cmp::max(logical_size, MIN_FAT_SIZE);

    // Determine FAT type based on total_size
    let fat_type = if total_size <= 268_435_456 {
        FatType::Fat16
    } else {
        FatType::Fat32
    };

    // Create the file, set length, and zero-fill.
    // `set_len` on some filesystems creates a sparse file with stale disk data,
    // which corrupts FAT directory entries. Zero-fill ensures clean FAT structures.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(fat_img_path)?;
    file.set_len(total_size)?;
    file.seek(io::SeekFrom::Start(0))?;
    let zero_buf = vec![0u8; 65536];
    let mut remaining = total_size;
    while remaining > 0 {
        let chunk = remaining.min(zero_buf.len() as u64) as usize;
        file.write_all(&zero_buf[..chunk])?;
        remaining -= chunk as u64;
    }
    file.seek(io::SeekFrom::Start(0))?;

    // Format the FAT image
    fatfs::format_volume(
        &mut file,
        FormatVolumeOptions::new()
            .fat_type(fat_type)
            .bytes_per_sector(512), // UEFI typically expects 512-byte sectors for FAT
    )?;

    // Patch BPB fields for compatibility with old UEFI firmware
    // (NEC/Insyde/AMI).  fatfs sets these fields to its own defaults,
    // but real hardware often requires specific geometry values.
    //
    // Offsets in the FAT boot sector (FAT12/16 BPB):
    //   0x18 (24): sectors_per_track (u16 LE)
    //   0x1A (26): heads (u16 LE)
    //   0x1C (28): hidden_sectors (u32 LE)
    file.seek(io::SeekFrom::Start(0x18))?;
    file.write_all(&32u16.to_le_bytes())?;  // sectors_per_track = 32
    file.write_all(&64u16.to_le_bytes())?;  // heads = 64
    file.write_all(&hidden_sectors.to_le_bytes())?;   // hidden_sectors = partition start LBA
    file.seek(io::SeekFrom::Start(0))?;

    // Open filesystem and create directories and copy files
    let fs = FileSystem::new(file, FsOptions::new())?;
    let root_dir = fs.root_dir();
    let efi_dir = root_dir.create_dir("EFI")?;
    let boot_dir = efi_dir.create_dir("BOOT")?;
    for (dest_name, source_path) in files {
        copy_to_fat(&boot_dir, source_path, dest_name)?;
    }

    Ok((total_size / SECTOR_SIZE) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Read;
    use tempfile::tempdir;

    #[test]
    fn test_create_fat_image() -> io::Result<()> {
        let dir = tempdir()?;
        let loader_path = dir.path().join("loader.efi");
        let kernel_path = dir.path().join("kernel.elf");
        let fat_img_path = dir.path().join("fat.img");

        let loader_content = b"UEFI loader";
        let kernel_content = b"ELF kernel";
        fs::write(&loader_path, loader_content)?;
        fs::write(&kernel_path, kernel_content)?;

        let files: [(&str, &Path); 2] =
            [("BOOTX64.EFI", &loader_path), ("KERNEL.EFI", &kernel_path)];
        create_fat_image(&fat_img_path, &files, 0)?;

        assert!(fat_img_path.exists());
        let fat_img_size = fat_img_path.metadata()?.len();
        assert!(fat_img_size > 0);

        // Verify the contents of the FAT image
        let fat_file = fs::File::open(&fat_img_path)?;
        let fs = FileSystem::new(fat_file, FsOptions::new())?;
        let root_dir = fs.root_dir();

        let mut loader_in_fat = root_dir.open_file("EFI/BOOT/BOOTX64.EFI")?;
        let mut loader_in_fat_content = Vec::new();
        loader_in_fat.read_to_end(&mut loader_in_fat_content)?;
        assert_eq!(loader_content, loader_in_fat_content.as_slice());

        let mut kernel_in_fat = root_dir.open_file("EFI/BOOT/KERNEL.EFI")?;
        let mut kernel_in_fat_content = Vec::new();
        kernel_in_fat.read_to_end(&mut kernel_in_fat_content)?;
        assert_eq!(kernel_content, kernel_in_fat_content.as_slice());

        Ok(())
    }

    #[test]
    fn test_create_fat_image_with_hidden_sectors() -> io::Result<()> {
        let dir = tempdir()?;
        let loader_path = dir.path().join("boot.efi");
        let fat_img_path = dir.path().join("fat_hidden.img");

        fs::write(&loader_path, b"BOOT")?;
        let files: [(&str, &Path); 1] = [("BOOTX64.EFI", &loader_path)];

        // hidden_sectors = 2048 (1 MiB partition alignment)
        create_fat_image(&fat_img_path, &files, 2048)?;

        // Read the BPB hidden_sectors field at offset 0x1C
        let mut fat_bytes = Vec::new();
        fs::File::open(&fat_img_path)?.read_to_end(&mut fat_bytes)?;
        let hidden = u32::from_le_bytes(
            fat_bytes[0x1C..0x20].try_into().unwrap(),
        );
        assert_eq!(
            hidden, 2048,
            "BPB hidden_sectors must be 2048 (1 MiB), got {}",
            hidden
        );

        // Verify the filesystem is still mountable after patching
        let fat_file = fs::File::open(&fat_img_path)?;
        let fs = FileSystem::new(fat_file, FsOptions::new())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let root_dir = fs.root_dir();
        let mut found = root_dir.open_file("EFI/BOOT/BOOTX64.EFI")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let mut content = Vec::new();
        found.read_to_end(&mut content)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        assert_eq!(content, b"BOOT");

        Ok(())
    }
}
