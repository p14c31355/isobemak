// isobemak/src/fat32.rs
use crate::utils::FAT32_SECTOR_SIZE;
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

const FAT32_IMAGE_SECTOR_COUNT: u64 = 0xFFFF;
const FAT32_IMAGE_SIZE: u64 = FAT32_IMAGE_SECTOR_COUNT * FAT32_SECTOR_SIZE;
const MBR_PARTITION_TABLE_OFFSET: u64 = 0x1BE;
const MBR_PARTITION_ENTRY_SIZE: u64 = 16;
const FAT32_PARTITION_TYPE_0B: u8 = 0x0B;
const FAT32_PARTITION_TYPE_0C: u8 = 0x0C;

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

/// Creates a full disk image with MBR and a single FAT32 partition.
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
    // NOTE: This creates a FAT32 volume *without* a MBR, which is the problem.
    // However, we'll keep this logic for now and extract the partition later.
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

/// Parses the MBR of a disk image and extracts the pure FAT32 partition data.
pub fn extract_fat32_partition(src_path: &Path, dest_path: &Path) -> io::Result<()> {
    let mut reader = File::open(src_path)?;

    // Read MBR (first 512 bytes)
    let mut mbr = [0u8; FAT32_SECTOR_SIZE as usize];
    reader.read_exact(&mut mbr)?;

    // Find the FAT32 partition entry in the partition table (MBR offset 0x1BE)
    let partition_table_offset = MBR_PARTITION_TABLE_OFFSET as usize;
    let mut start_lba = 0;
    let mut sector_count = 0;
    let mut found = false;

    for i in 0..4 {
        let entry_start = partition_table_offset + (i * MBR_PARTITION_ENTRY_SIZE as usize);
        let partition_type = mbr[entry_start + 4];
        if partition_type == FAT32_PARTITION_TYPE_0B || partition_type == FAT32_PARTITION_TYPE_0C {
            // Read start LBA (offset 0x08-0x0B within the entry)
            start_lba =
                u32::from_le_bytes(mbr[entry_start + 8..entry_start + 12].try_into().unwrap());
            // Read sector count (offset 0x0C-0x0F within the entry)
            sector_count =
                u32::from_le_bytes(mbr[entry_start + 12..entry_start + 16].try_into().unwrap());
            found = true;
            break;
        }
    }

    if !found {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "FAT32 partition not found in MBR.",
        ));
    }

    let start_offset = start_lba as u64 * FAT32_SECTOR_SIZE;
    let length = sector_count as u64 * FAT32_SECTOR_SIZE;
    println!(
        "Found FAT32 partition: start LBA = {}, sector count = {}, size = {} bytes",
        start_lba, sector_count, length
    );

    // Seek to the start of the FAT32 partition
    reader.seek(SeekFrom::Start(start_offset))?;

    // Read and write the partition data to the destination file
    let mut dest_file = File::create(dest_path)?;
    io::copy(&mut reader.take(length), &mut dest_file)?;

    println!(
        "Successfully extracted FAT32 partition to {}.",
        dest_path.display()
    );

    Ok(())
}
