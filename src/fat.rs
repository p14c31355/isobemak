// isobemak/src/fat.rs
use fatfs::{Dir, FileSystem, FsOptions};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

// ── FAT32 constants ──
const SECTOR_SIZE: u64 = 512;
const SEC_PER_CLUS: u64 = 8; // 4 KiB cluster — Ventoy/UEFI firmware rejects 512 B clusters
const RESERVED_SECTORS: u64 = 32;
const MIN_FAT_SIZE: u64 = 260 * 1024 * 1024; // 260 MiB ─ fatfs needs ≥65525 clusters for FAT32
const FAT_OVERHEAD: u64 = 2 * 1024 * 1024;   // 2 MiB for structures

/// Copies a file into a directory within the FAT filesystem.
fn copy_to_fat(fat_dir: &Dir<fs::File>, source_path: &Path, dest_name: &str) -> io::Result<()> {
    let mut dest_file = fat_dir.create_file(dest_name)?;
    let mut source_file = fs::File::open(source_path)?;
    io::copy(&mut source_file, &mut dest_file)?;
    Ok(())
}

/// Iteratively solve for FAT32 layout parameters.
fn calculate_fat32_layout(
    total_sectors: u64,
    reserved_sectors: u64,
    sec_per_clus: u64,
) -> (u64, u64) {
    let mut data_sectors = total_sectors - reserved_sectors;
    loop {
        let fat_entries = data_sectors.div_ceil(sec_per_clus) + 2;
        let fat_bytes = fat_entries * 4;
        let fat_sectors = fat_bytes.div_ceil(SECTOR_SIZE);
        let new_data = if 2 * fat_sectors + reserved_sectors < total_sectors {
            total_sectors - reserved_sectors - 2 * fat_sectors
        } else {
            1
        };
        if new_data >= data_sectors {
            break;
        }
        data_sectors = new_data;
    }
    let fat_entries = data_sectors.div_ceil(sec_per_clus) + 2;
    let fat_sectors = (fat_entries * 4).div_ceil(SECTOR_SIZE);
    (fat_sectors, data_sectors)
}

/// Write a complete FAT32 BPB at the **current seek position**.
/// Caller must seek to the desired sector before calling.
fn write_fat32_bpb(
    file: &mut fs::File,
    total_sectors: u32,
    fat_sectors: u32,
    hidden_sectors: u32,
) -> io::Result<()> {
    let mut bpb = [0u8; 90];

    bpb[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    bpb[3..11].copy_from_slice(b"MSWIN4.1");
    bpb[11..13].copy_from_slice(&512u16.to_le_bytes());

    bpb[13] = SEC_PER_CLUS as u8;

    bpb[14..16].copy_from_slice(&(RESERVED_SECTORS as u16).to_le_bytes());
    bpb[16] = 2;
    bpb[21] = 0xF8;

    bpb[24..26].copy_from_slice(&32u16.to_le_bytes());
    bpb[26..28].copy_from_slice(&64u16.to_le_bytes());
    bpb[28..32].copy_from_slice(&hidden_sectors.to_le_bytes());
    bpb[32..36].copy_from_slice(&total_sectors.to_le_bytes());

    // FAT32 extended BPB
    bpb[36..40].copy_from_slice(&fat_sectors.to_le_bytes()); // FATSz32
    bpb[44..48].copy_from_slice(&2u32.to_le_bytes());        // RootClus = 2
    bpb[48..50].copy_from_slice(&1u16.to_le_bytes());        // FSInfo = 1
    bpb[50..52].copy_from_slice(&6u16.to_le_bytes());        // backup = 6

    bpb[64] = 0x80;   // drive number
    bpb[66] = 0x29;   // extended boot signature
    let serial: u32 = rand::random();
    bpb[67..71].copy_from_slice(&serial.to_le_bytes());
    bpb[71..82].copy_from_slice(b"EFI        ");
    bpb[82..90].copy_from_slice(b"FAT32   ");

    // Write BPB fields (bytes 0-89) at current position
    let pos = file.stream_position()?;
    file.write_all(&bpb)?;

    // Boot sector signature at offset 510 from sector start
    file.seek(SeekFrom::Start(pos + 510))?;
    file.write_all(&0xAA55u16.to_le_bytes())?;

    Ok(())
}

/// Write the FAT32 FSInfo sector at its designated offset (sector 1).
fn write_fat32_fsinfo(file: &mut fs::File, total_clusters: u64) -> io::Result<()> {
    file.seek(SeekFrom::Start(1 * SECTOR_SIZE))?;
    let mut sector = [0u8; 512];
    sector[0..4].copy_from_slice(&0x41615252u32.to_le_bytes());   // lead sig
    sector[484..488].copy_from_slice(&0x61417272u32.to_le_bytes()); // struct sig
    let free_count = total_clusters - 3; // entries 0,1,2 are used
    sector[488..492].copy_from_slice(&(free_count as u32).to_le_bytes());
    sector[492..496].copy_from_slice(&3u32.to_le_bytes()); // next free = 3
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
    write_fat32_bpb(file, total_sectors, fat_sectors, hidden_sectors)
}

/// Initialize both FAT tables.
fn init_fat_tables(file: &mut fs::File, fat_sectors: u64) -> io::Result<()> {
    let fat_start = RESERVED_SECTORS;
    file.seek(SeekFrom::Start(fat_start * SECTOR_SIZE))?;
    file.write_all(&0x0FFFFFF8u32.to_le_bytes())?;  // entry 0
    file.write_all(&0x0FFFFFFFu32.to_le_bytes())?;  // entry 1
    file.write_all(&0x0FFFFFFFu32.to_le_bytes())?;  // entry 2 (root dir EOC)

    // Copy FAT0 → FAT1
    file.seek(SeekFrom::Start(fat_start * SECTOR_SIZE))?;
    let mut fat0 = vec![0u8; fat_sectors as usize * 512];
    file.read_exact(&mut fat0)?;
    file.seek(SeekFrom::Start((fat_start + fat_sectors) * SECTOR_SIZE))?;
    file.write_all(&fat0)?;
    Ok(())
}

/// After fatfs has written all files, scan the actual FAT to determine the
/// real free-cluster count and update both FSInfo sector 1 and backup FSInfo
/// sector 7.  UEFI firmware (including Ventoy) often validates FSInfo
/// rigorously; a stale free-count from before file creation causes rejection.
fn update_fsinfo_after_write(fat_img_path: &Path, fat_sectors: u64) -> io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(fat_img_path)?;
    let mut fat = vec![0u8; fat_sectors as usize * 512];
    file.seek(SeekFrom::Start(RESERVED_SECTORS * SECTOR_SIZE))?;
    file.read_exact(&mut fat)?;

    let total_entries = fat.len() / 4;
    let mut used = 0u64;
    for i in 2..total_entries {
        let v = u32::from_le_bytes([fat[i * 4], fat[i * 4 + 1], fat[i * 4 + 2], fat[i * 4 + 3]]);
        if v != 0x0000_0000 {
            used += 1;
        }
    }
    let total_clusters = (total_entries - 2) as u64;
    let free_count = total_clusters.saturating_sub(used);

    // Find next free cluster
    let mut next_free = 2u32;
    for i in 2..total_entries {
        let v = u32::from_le_bytes([fat[i * 4], fat[i * 4 + 1], fat[i * 4 + 2], fat[i * 4 + 3]]);
        if v == 0x0000_0000 {
            next_free = i as u32;
            break;
        }
    }

    // Write FSInfo sector 1
    let mut sector = [0u8; 512];
    sector[0..4].copy_from_slice(&0x41615252u32.to_le_bytes());
    sector[484..488].copy_from_slice(&0x61417272u32.to_le_bytes());
    sector[488..492].copy_from_slice(&(free_count as u32).to_le_bytes());
    sector[492..496].copy_from_slice(&next_free.to_le_bytes());
    sector[508..512].copy_from_slice(&0xAA550000u32.to_le_bytes());
    file.seek(SeekFrom::Start(1 * SECTOR_SIZE))?;
    file.write_all(&sector)?;

    // Write backup FSInfo sector 7
    file.seek(SeekFrom::Start(7 * SECTOR_SIZE))?;
    file.write_all(&sector)?;

    Ok(())
}

/// Creates a FAT image file for UEFI boot and populates it with files.
pub fn create_fat_image(
    fat_img_path: &Path,
    files: &[(&str, &Path)],
    hidden_sectors: u32,
) -> io::Result<u32> {
    let mut content_size = 0u64;
    for (_dest_name, source_path) in files {
        if !source_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found at {:?}", source_path),
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

    let logical_size = (content_size + FAT_OVERHEAD).div_ceil(SECTOR_SIZE) * SECTOR_SIZE;
    let total_size = std::cmp::max(logical_size, MIN_FAT_SIZE);
    let total_sectors = (total_size / SECTOR_SIZE) as u32;

    let (fat_sectors, _data_sectors) =
        calculate_fat32_layout(total_sectors as u64, RESERVED_SECTORS, SEC_PER_CLUS);
    // Align data_sectors to sec_per_clus boundary so that fatfs sees
    // an exact integer cluster count >= 65525.  Without this, fatfs
    // misidentifies the volume as FAT16.
    let data_sectors = (_data_sectors / SEC_PER_CLUS) * SEC_PER_CLUS;
    let fat_sectors = fat_sectors as u32;
    let actual_total_sectors = RESERVED_SECTORS as u64 + 2 * fat_sectors as u64 + data_sectors;
    let total_size = actual_total_sectors * SECTOR_SIZE;
    let total_sectors = actual_total_sectors as u32;

    // Create, size, zero-fill
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
    // Main BPB + boot sig at sector 0
    file.seek(SeekFrom::Start(0))?;
    write_fat32_bpb(&mut file, total_sectors, fat_sectors, hidden_sectors)?;
    // FSInfo at sector 1
    write_fat32_fsinfo(&mut file, data_sectors / SEC_PER_CLUS)?;
    // Backup BPB at sector 6
    write_backup_bpb(&mut file, total_sectors, fat_sectors, hidden_sectors)?;
    // FAT tables
    init_fat_tables(&mut file, fat_sectors as u64)?;

    // ── Use fatfs for directory / file population ──
    {
        file.seek(SeekFrom::Start(0))?;
        let fs = FileSystem::new(file, FsOptions::new())?;
        let root_dir = fs.root_dir();
        let efi_dir = root_dir.create_dir("EFI")?;
        let boot_dir = efi_dir.create_dir("BOOT")?;
        for (dest_name, source_path) in files {
            copy_to_fat(&boot_dir, source_path, dest_name)?;
        }
        // fs and all borrowed handles go out of scope here, commits + closes
    }

    // fatfs writes are now committed; the file handle was moved into
    // FileSystem and dropped at the end of the scope above.  Re-open and
    // update FSInfo with the *actual* free-cluster count (the earlier
    // write_fat32_fsinfo call happened before any files existed, so its
    // free-count was stale — UEFI firmware/Ventoy may reject the volume).
    update_fsinfo_after_write(fat_img_path, fat_sectors as u64)?;

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
        // Test with the actual MIN_FAT_SIZE sector count (260 MiB = 532480 sectors)
        let total = 532480u64;
        let (fat, data) = calculate_fat32_layout(total, 32, 8);
        assert!(data + 2 * fat + 32 <= total, "layout overflow");
        assert!(fat > 0 && fat < 4096, "unexpected FAT sectors: {}", fat);
        assert!(data / 8 >= 65525, "too few clusters for FAT32: {}", data / 8);
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

        create_fat_image(&fat_img_path, &files, 2048)?;

        let mut fat_bytes = Vec::new();
        fs::File::open(&fat_img_path)?.read_to_end(&mut fat_bytes)?;
        let hidden = u32::from_le_bytes(fat_bytes[0x1C..0x20].try_into().unwrap());
        assert_eq!(hidden, 2048, "BPB hidden_sectors must be 2048, got {}", hidden);

        let fat_file = fs::File::open(&fat_img_path)?;
        let fs = FileSystem::new(fat_file, FsOptions::new())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let root_dir = fs.root_dir();
        let mut found = root_dir
            .open_file("EFI/BOOT/BOOTX64.EFI")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let mut content = Vec::new();
        found.read_to_end(&mut content)?;
        assert_eq!(content, b"BOOT");

        Ok(())
    }
}