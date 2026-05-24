// isobemak/src/fat.rs
use fatfs::{Dir, FileSystem, FsOptions};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

// ── FAT32 constants ──
const SECTOR_SIZE: u64 = 512;
const SEC_PER_CLUS: u64 = 1;
const RESERVED_SECTORS: u64 = 32;
const MIN_FAT_SIZE: u64 = 256 * 1024 * 1024; // 256 MiB ─ enough for FAT32
const FAT_OVERHEAD: u64 = 2 * 1024 * 1024;   // 2 MiB for structures

/// Copies a file into a directory within the FAT filesystem.
fn copy_to_fat(fat_dir: &Dir<fs::File>, source_path: &Path, dest_name: &str) -> io::Result<()> {
    let mut dest_file = fat_dir.create_file(dest_name)?;
    let mut source_file = fs::File::open(source_path)?;
    io::copy(&mut source_file, &mut dest_file)?;
    Ok(())
}

/// Iteratively solve for FAT32 layout parameters.
///
/// Returns `(fat_sectors, data_sectors)` where `fat_sectors` is the number of
/// 512‑byte sectors needed for **one** FAT table, and `data_sectors` is the
/// number of sectors available for clusters (including the two reserved FAT
/// entries 0 and 1).
fn calculate_fat32_layout(
    total_sectors: u64,
    reserved_sectors: u64,
    sec_per_clus: u64,
) -> (u64, u64) {
    let mut data_sectors = total_sectors - reserved_sectors;
    loop {
        // +2 for FAT entries 0 and 1 (media / EOC reservations)
        let fat_entries = data_sectors.div_ceil(sec_per_clus) + 2;
        let fat_bytes = fat_entries * 4; // FAT32 ─ 4 bytes per entry
        let fat_sectors = fat_bytes.div_ceil(SECTOR_SIZE);
        let new_data = if 2 * fat_sectors + reserved_sectors < total_sectors {
            total_sectors - reserved_sectors - 2 * fat_sectors
        } else {
            // Not enough room for two FAT copies — clamp to minimum
            1
        };
        // Converged (or cannot shrink further)
        if new_data >= data_sectors {
            break;
        }
        data_sectors = new_data;
    }
    let fat_entries = data_sectors.div_ceil(sec_per_clus) + 2;
    let fat_sectors = (fat_entries * 4).div_ceil(SECTOR_SIZE);
    (fat_sectors, data_sectors)
}

/// Write a complete FAT32 BPB (BIOS Parameter Block) into the first 90 bytes
/// of the file (seek position 0).
fn write_fat32_bpb(
    file: &mut fs::File,
    total_sectors: u32,
    fat_sectors: u32,
    hidden_sectors: u32,
) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut bpb = [0u8; 90];

    // ── Common BPB ──
    bpb[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]); // jmp short + nop
    bpb[3..11].copy_from_slice(b"isobemak");         // OEM name (8 bytes)
    bpb[11..13].copy_from_slice(&512u16.to_le_bytes()); // bytes per sector

    bpb[13] = SEC_PER_CLUS as u8;                     // sectors per cluster

    bpb[14..16].copy_from_slice(&(RESERVED_SECTORS as u16).to_le_bytes()); // reserved sectors
    bpb[16] = 2; // number of FATs
    // RootEntCnt ─ 0 for FAT32 (offset 0x11)
    // TotSec16  ─ 0 for FAT32 (offset 0x13)
    bpb[21] = 0xF8; // media descriptor (fixed disk)
    // FATSz16   ─ 0 for FAT32 (offset 0x16)

    bpb[24..26].copy_from_slice(&32u16.to_le_bytes()); // sectors per track
    bpb[26..28].copy_from_slice(&64u16.to_le_bytes()); // heads
    bpb[28..32].copy_from_slice(&hidden_sectors.to_le_bytes()); // hidden sectors

    bpb[32..36].copy_from_slice(&total_sectors.to_le_bytes()); // TotSec32

    // ── FAT32 extended BPB ──
    bpb[36..40].copy_from_slice(&fat_sectors.to_le_bytes()); // FATSz32
    // ExtFlags  ─ 0 (offset 0x28)
    // FSVer     ─ 0 (offset 0x2A)
    bpb[44..48].copy_from_slice(&2u32.to_le_bytes()); // RootClus = 2
    bpb[48..50].copy_from_slice(&1u16.to_le_bytes()); // FSInfo sector = 1
    bpb[50..52].copy_from_slice(&6u16.to_le_bytes()); // backup boot sector = 6

    // ── Extended boot signature ──
    bpb[64] = 0x80;   // drive number
    bpb[66] = 0x29;   // extended boot signature
    // Volume serial number (bytes 67-70): use a deterministic but unique-ish value
    let serial = 0x12345678u32;
    bpb[67..71].copy_from_slice(&serial.to_le_bytes());
    bpb[71..82].copy_from_slice(b"NO NAME    "); // volume label (11 bytes, space padded)
    bpb[82..90].copy_from_slice(b"FAT32   ");    // file system type

    // Boot sector signature at offset 510
    file.seek(SeekFrom::Start(510))?;
    file.write_all(&0xAA55u16.to_le_bytes())?;

    file.seek(SeekFrom::Start(0))?;
    file.write_all(&bpb)?;
    Ok(())
}

/// Write the FAT32 FSInfo sector at sector 1.
fn write_fat32_fsinfo(file: &mut fs::File) -> io::Result<()> {
    file.seek(SeekFrom::Start(1 * SECTOR_SIZE))?;
    let mut sector = [0u8; 512];
    sector[0..4].copy_from_slice(&0x41615252u32.to_le_bytes()); // lead sig
    sector[484..488].copy_from_slice(&0x61417272u32.to_le_bytes()); // struct sig
    // free count ─ 0xFFFFFFFF (unknown)
    sector[488..492].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    // next free ─ 0xFFFFFFFF (unknown)
    sector[492..496].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    sector[508..512].copy_from_slice(&0xAA550000u32.to_le_bytes()); // trail sig
    file.write_all(&sector)?;
    Ok(())
}

/// Write backup BPB at sector 6.
fn write_backup_bpb(
    file: &mut fs::File,
    total_sectors: u32,
    fat_sectors: u32,
    hidden_sectors: u32,
) -> io::Result<()> {
    file.seek(SeekFrom::Start(6 * SECTOR_SIZE))?;
    write_fat32_bpb(file, total_sectors, fat_sectors, hidden_sectors)?;
    Ok(())
}

/// Initialize both FAT tables.
///
/// FAT entry 0: media byte = 0xF8 → 0x0FFFFFF8
/// FAT entry 1: EOC (reserved) → 0x0FFFFFFF
/// FAT entry 2 (root dir): EOC (end of chain) → 0x0FFFFFFF
/// Remaining entries: 0x00000000 (free)
fn init_fat_tables(
    file: &mut fs::File,
    fat_sectors: u64,
) -> io::Result<()> {
    let fat_start_sector = RESERVED_SECTORS;
    // Write first FAT
    file.seek(SeekFrom::Start(fat_start_sector * SECTOR_SIZE))?;
    // Entry 0
    file.write_all(&0x0FFFFFF8u32.to_le_bytes())?;
    // Entry 1
    file.write_all(&0x0FFFFFFFu32.to_le_bytes())?;
    // Entry 2 (root dir)
    file.write_all(&0x0FFFFFFFu32.to_le_bytes())?;
    // All remaining entries are already zero — the file was zero-filled

    // Copy first FAT to second FAT
    file.seek(SeekFrom::Start(fat_start_sector * SECTOR_SIZE))?;
    let mut fat0 = vec![0u8; fat_sectors as usize * 512];
    file.read_exact(&mut fat0)?;

    let fat1_start = fat_start_sector + fat_sectors;
    file.seek(SeekFrom::Start(fat1_start * SECTOR_SIZE))?;
    file.write_all(&fat0)?;
    Ok(())
}

/// Creates a FAT image file for UEFI boot and populates it with files.
///
/// `files` is a list of (destination_filename, source_path) pairs copied to `EFI/BOOT/`.
/// `hidden_sectors` sets the BPB hidden sectors field (LBA of the partition start in 512B sectors).
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

    // Calculate target size
    let logical_size = (content_size + FAT_OVERHEAD).div_ceil(SECTOR_SIZE) * SECTOR_SIZE;
    let total_size = std::cmp::max(logical_size, MIN_FAT_SIZE);
    let total_sectors = (total_size / SECTOR_SIZE) as u32;

    // Determine FAT32 geometry
    let (fat_sectors, _data_sectors) =
        calculate_fat32_layout(total_sectors as u64, RESERVED_SECTORS, SEC_PER_CLUS);
    let fat_sectors = fat_sectors as u32;
    // Adjust total_size to match exact sector count
    let actual_total_sectors =
        RESERVED_SECTORS as u64 + 2 * fat_sectors as u64 + _data_sectors;
    let total_size = actual_total_sectors * SECTOR_SIZE;
    let total_sectors = actual_total_sectors as u32;

    // Create, size, and zero-fill the file
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(fat_img_path)?;
    file.set_len(total_size)?;
    let zero_buf = vec![0u8; 65536];
    let mut remaining = total_size;
    file.seek(SeekFrom::Start(0))?;
    while remaining > 0 {
        let chunk = remaining.min(zero_buf.len() as u64) as usize;
        file.write_all(&zero_buf[..chunk])?;
        remaining -= chunk as u64;
    }

    // ── Self-written FAT32 structures ──
    write_fat32_bpb(&mut file, total_sectors, fat_sectors, hidden_sectors)?;
    write_fat32_fsinfo(&mut file)?;
    write_backup_bpb(&mut file, total_sectors, fat_sectors, hidden_sectors)?;
    init_fat_tables(&mut file, fat_sectors as u64)?;

    // ── Use fatfs for directory / file population ──
    file.seek(SeekFrom::Start(0))?;
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
    fn test_calculate_fat32_layout() {
        let total = 524288u64; // 256 MiB
        let (fat, data) = calculate_fat32_layout(total, 32, 1);
        assert_eq!(fat, 4032);
        assert_eq!(data, 516064);
    }

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
        let hidden = u32::from_le_bytes(fat_bytes[0x1C..0x20].try_into().unwrap());
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
        let mut found = root_dir
            .open_file("EFI/BOOT/BOOTX64.EFI")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let mut content = Vec::new();
        found
            .read_to_end(&mut content)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        assert_eq!(content, b"BOOT");

        Ok(())
    }
}