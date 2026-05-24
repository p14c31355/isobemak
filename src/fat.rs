// isobemak/src/fat.rs — Hand-written FAT32 image creator (in-memory)
//
// We write every FAT structure ourselves because the fatfs crate:
//  1. Prepends LFN entries to "." and ".." (FAT spec violation, rejected by
//     fsck.fat and some UEFI firmware including Ventoy).
//  2. Never writes a volume-label entry in the root directory.
//
// API change: create_fat_image now returns a Vec<u8> (the complete FAT image)
// instead of writing to a file.  This avoids kernel page-cache visibility
// issues where data written + fsync'd was readable in-process but appeared
// as zeroes to external tools (fsck.fat / hexdump).

use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

// ── FAT32 constants ──
const SECTOR_SIZE: u64 = 512;
const CLUSTER_SIZE: u64 = SECTOR_SIZE * 8; // 4 KiB
const SEC_PER_CLUS: u64 = 8;
const RESERVED_SECTORS: u64 = 32;
// Minimum 65525 clusters (~256 MiB) required by FAT32 specification.
// Every FAT32 image produced by isobemak is at least this size.
const MIN_CLUSTERS: usize = 65525;
const FAT_OVERHEAD: u64 = 2 * 1024 * 1024;

// ── 8.3 name helpers ──

fn pack_83(name: &[u8], ext: &[u8]) -> [u8; 11] {
    let mut out = [b' '; 11];
    let n = name.len().min(8);
    out[..n].copy_from_slice(&name[..n]);
    let e = ext.len().min(3);
    out[8..8 + e].copy_from_slice(&ext[..e]);
    out
}

fn lfn_checksum(short: &[u8; 11]) -> u8 {
    let mut sum: u8 = 0;
    for &b in short.iter() {
        sum = sum.rotate_right(1).wrapping_add(b);
    }
    sum
}

fn make_directory_entries(
    long_name: &str,
    short: &[u8; 11],
    attr: u8,
    first_cluster: u32,
    file_size: u32,
) -> Option<(Vec<u8>, Vec<u8>)> {
    let plain = long_name.len() <= 12
        && !long_name.contains('.')
        && long_name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if plain {
        return None;
    }

    let chk = lfn_checksum(short);
    let utf16: Vec<u16> = long_name.encode_utf16().collect();
    let padded_len = ((utf16.len() + 12) / 13) * 13;
    let mut chars = utf16;
    chars.resize(padded_len, 0xFFFF);
    let num_lfn = chars.len() / 13;

    let slot_start = (num_lfn - 1) * 13;
    let tail_len = (chars.len() - slot_start).min(13);
    chars[slot_start + tail_len - 1] = 0x0000;
    for j in (slot_start + tail_len)..slot_start + 13 {
        if j < chars.len() {
            chars[j] = 0xFFFF;
        }
    }

    let mut lfn_bytes = Vec::with_capacity(num_lfn * 32);
    for i in (0..num_lfn).rev() {
        let seq = if i == num_lfn - 1 {
            (i + 1) as u8 | 0x40
        } else {
            (i + 1) as u8
        };
        let segment = &chars[i * 13..(i + 1) * 13];
        let mut entry = [0u8; 32];
        entry[0] = seq;
        entry[1..3].copy_from_slice(&segment[0].to_le_bytes());
        entry[3..5].copy_from_slice(&segment[1].to_le_bytes());
        entry[5..7].copy_from_slice(&segment[2].to_le_bytes());
        entry[7..9].copy_from_slice(&segment[3].to_le_bytes());
        entry[9..11].copy_from_slice(&segment[4].to_le_bytes());
        entry[11] = 0x0F;
        entry[13] = chk;
        entry[14..16].copy_from_slice(&segment[5].to_le_bytes());
        entry[16..18].copy_from_slice(&segment[6].to_le_bytes());
        entry[18..20].copy_from_slice(&segment[7].to_le_bytes());
        entry[20..22].copy_from_slice(&segment[8].to_le_bytes());
        entry[22..24].copy_from_slice(&segment[9].to_le_bytes());
        entry[24..26].copy_from_slice(&segment[10].to_le_bytes());
        // bytes 26..28 = 0 (first cluster, always 0 for LFN)
        entry[28..30].copy_from_slice(&segment[11].to_le_bytes());
        entry[30..32].copy_from_slice(&segment[12].to_le_bytes());
        lfn_bytes.extend_from_slice(&entry);
    }

    let mut sfn = [0u8; 32];
    sfn[..11].copy_from_slice(short);
    sfn[11] = attr;
    // Set valid creation time (00:00:00) and date (2000-01-01 = 0x21 in FAT encoding).
    // This matches entry_83 and avoids fsck.fat warnings about zero timestamps.
    sfn[16..18].copy_from_slice(&0x0000u16.to_le_bytes()); // creation time 00:00:00
    sfn[18..20].copy_from_slice(&0x21u16.to_le_bytes());   // creation date 2000-01-01
    sfn[20..22].copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
    sfn[26..28].copy_from_slice(&(first_cluster as u16).to_le_bytes());
    sfn[28..32].copy_from_slice(&file_size.to_le_bytes());

    Some((lfn_bytes, sfn.to_vec()))
}

// ── Cluster allocator ──

struct Alloc {
    fat: Vec<u32>,
    clusters: usize,
    data_start: u64,
}

impl Alloc {
    fn new(total_sectors: u64, sectors_per_fat: u64) -> Self {
        let data_start = RESERVED_SECTORS + 2 * sectors_per_fat;
        let data_secs = total_sectors - data_start;
        let clusters = (data_secs / SEC_PER_CLUS) as usize;
        let mut fat = vec![0u32; clusters + 2];
        fat[0] = 0x0FFFFFF8;
        fat[1] = 0x0FFFFFFF;
        Self {
            fat,
            clusters,
            data_start,
        }
    }

    fn alloc(&mut self, count: u32) -> Option<u32> {
        let mut first: Option<u32> = None;
        let mut prev: Option<u32> = None;
        let mut n = 0u32;
        for i in 2..self.fat.len() {
            if self.fat[i] == 0x0000_0000 {
                // set or extend the chain
                match (first, prev) {
                    (None, _) => first = Some(i as u32),
                    (_, Some(p)) => self.fat[p as usize] = i as u32,
                    _ => {}
                }
                prev = Some(i as u32);
                self.fat[i] = 0x0FFFFFFF; // end-of-chain marker
                n += 1;
                if n >= count {
                    return first;
                }
            }
        }
        None
    }

    fn sector_of(&self, cluster: u32) -> u64 {
        self.data_start + (cluster as u64 - 2) * SEC_PER_CLUS
    }
}

// ── Directory-entry builders ──

fn entry_83(short: &[u8; 11], attr: u8, first_cluster: u32, file_size: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(short);
    e[11] = attr;
    // Set valid creation time (00:00:00) and date (2000-01-01 = 0x21 in FAT encoding)
    // This avoids fsck.fat warnings about zero timestamps.
    e[16..18].copy_from_slice(&0x0000u16.to_le_bytes()); // creation time 00:00:00
    e[18..20].copy_from_slice(&0x21u16.to_le_bytes());   // creation date 2000-01-01
    e[20..22].copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
    e[22..24].copy_from_slice(&0x0000u16.to_le_bytes()); // last access date
    e[26..28].copy_from_slice(&(first_cluster as u16).to_le_bytes());
    e[28..32].copy_from_slice(&file_size.to_le_bytes());
    e
}

fn dot_entry(parent_cluster: u32) -> [u8; 32] {
    let mut name = [b' '; 11];
    name[0] = b'.';
    entry_83(&name, 0x10, parent_cluster, 0)
}

fn dotdot_entry(parent_cluster: u32) -> [u8; 32] {
    let mut name = [b' '; 11];
    name[0] = b'.';
    name[1] = b'.';
    entry_83(&name, 0x10, parent_cluster, 0)
}

fn vol_entry(label: &[u8; 11]) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(label);
    e[11] = 0x08;
    e[16..18].copy_from_slice(&0x21u16.to_le_bytes());
    e[18..20].copy_from_slice(&0x21u16.to_le_bytes());
    e
}

// ── In-memory image builder (slice-based) ──

/// Write a byte slice into the image at the given byte offset.
fn write_at(img: &mut [u8], offset: u64, data: &[u8]) {
    let off = offset as usize;
    img[off..off + data.len()].copy_from_slice(data);
}

fn write_u16_le(img: &mut [u8], offset: u64, val: u16) {
    img[offset as usize..offset as usize + 2].copy_from_slice(&val.to_le_bytes());
}

/// Write the complete FAT32 image into a Vec<u8>.
fn build_image(
    files: &[(&str, &Path)],
    hidden: u32,
) -> io::Result<(Vec<u8>, u32)> {
    let mut content_size = 0u64;
    for (_, p) in files {
        content_size += p.metadata()?.len();
    }

    let logical = (content_size + FAT_OVERHEAD).div_ceil(SECTOR_SIZE) * SECTOR_SIZE;
    let min_size = MIN_CLUSTERS as u64 * CLUSTER_SIZE + RESERVED_SECTORS * SECTOR_SIZE + 2 * 1024 * SECTOR_SIZE;
    let total_size = logical.max(min_size);
    let (fat_sectors_raw, _data) = calculate_fat32_layout(
        total_size / SECTOR_SIZE,
        RESERVED_SECTORS,
        SEC_PER_CLUS,
    );
    let data_sectors = (_data / SEC_PER_CLUS) * SEC_PER_CLUS;
    let fat_sectors = fat_sectors_raw as u32;
    let total_sectors = (RESERVED_SECTORS + 2 * fat_sectors as u64 + data_sectors) as u32;
    let total_size = total_sectors as u64 * SECTOR_SIZE;
    let total_bytes = total_size as usize;

    let serial: u32 = rand::random();
    let volume_label = pack_83(b"EFI", b"");

    // Allocate image buffer (zero-initialized)
    let mut img = vec![0u8; total_bytes];

    // 1. Main BPB at sector 0
    write_bpb_to_slice(&mut img, 0, total_sectors, fat_sectors, hidden, serial);

    // 2. Allocate clusters
    let mut alloc = Alloc::new(total_sectors as u64, fat_sectors as u64);
    let root = alloc.alloc(1).ok_or_else(|| {
        io::Error::new(io::ErrorKind::Other, "FAT32: out of free clusters for root directory")
    })?;
    let efi = alloc.alloc(1).ok_or_else(|| {
        io::Error::new(io::ErrorKind::Other, "FAT32: out of free clusters for EFI directory")
    })?;
    let boot = alloc.alloc(1).ok_or_else(|| {
        io::Error::new(io::ErrorKind::Other, "FAT32: out of free clusters for BOOT directory")
    })?;
    let mut file_starts = Vec::with_capacity(files.len());
    let mut file_sizes = Vec::with_capacity(files.len());
    for (_name, p) in files {
        let sz = p.metadata()?.len();
        let n = (sz.div_ceil(CLUSTER_SIZE)).max(1) as u32;
        let start = alloc.alloc(n).ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, format!("FAT32: out of free clusters for file (need {} clusters)", n))
        })?;
        file_starts.push(start);
        file_sizes.push(sz);
    }

    // 3. Write directory tree + file data into the image buffer
    write_tree_to_slice(
        &mut img,
        &alloc,
        files,
        &volume_label,
        root,
        efi,
        boot,
        &file_starts,
        &file_sizes,
    )?;

    // 4. Write FAT tables
    write_fat_to_slice(&mut img, &alloc, fat_sectors as u64);

    // 5. FSInfo
    let total_clusters = alloc.clusters as u32;
    let used = alloc.fat.iter().filter(|&&v| v != 0x0000_0000).count() as u32 - 2;
    let free = total_clusters - used;
    let next = alloc.fat.iter().position(|&v| v == 0x0000_0000).unwrap_or(2) as u32;
    write_fsinfo_to_slice(&mut img, 1, free, next);
    write_fsinfo_to_slice(&mut img, 7, free, next);

    // 6. Backup BPB at sector 6 (last)
    write_bpb_to_slice(&mut img, 6 * SECTOR_SIZE, total_sectors, fat_sectors, hidden, serial);

    Ok((img, total_sectors))
}

fn write_bpb_to_slice(
    img: &mut [u8],
    sector_start: u64,
    total_sectors: u32,
    fat_sectors: u32,
    hidden: u32,
    serial: u32,
) {
    let off = sector_start as usize;
    let mut b = [0u8; 90];
    b[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    b[3..11].copy_from_slice(b"MSWIN4.1");
    b[11..13].copy_from_slice(&512u16.to_le_bytes());
    b[13] = SEC_PER_CLUS as u8;
    b[14..16].copy_from_slice(&(RESERVED_SECTORS as u16).to_le_bytes());
    b[16] = 2;
    b[21] = 0xF8;
    b[24..26].copy_from_slice(&32u16.to_le_bytes());
    b[26..28].copy_from_slice(&64u16.to_le_bytes());
    b[28..32].copy_from_slice(&hidden.to_le_bytes());
    b[32..36].copy_from_slice(&total_sectors.to_le_bytes());
    b[36..40].copy_from_slice(&fat_sectors.to_le_bytes());
    b[44..48].copy_from_slice(&2u32.to_le_bytes());
    b[48..50].copy_from_slice(&1u16.to_le_bytes());
    b[50..52].copy_from_slice(&6u16.to_le_bytes());
    b[64] = 0x80;
    b[66] = 0x29;
    b[67..71].copy_from_slice(&serial.to_le_bytes());
    b[71..82].copy_from_slice(b"EFI        ");
    b[82..90].copy_from_slice(b"FAT32   ");
    img[off..off + 90].copy_from_slice(&b);
    // Boot signature at offset 510
    write_u16_le(img, sector_start + 510, 0xAA55);
}

fn write_fsinfo_to_slice(img: &mut [u8], sector_num: u64, free_count: u32, next_free: u32) {
    let off = (sector_num * SECTOR_SIZE) as usize;
    let mut buf = [0u8; 512];
    buf[0..4].copy_from_slice(&0x41615252u32.to_le_bytes());
    buf[484..488].copy_from_slice(&0x61417272u32.to_le_bytes());
    buf[488..492].copy_from_slice(&free_count.to_le_bytes());
    buf[492..496].copy_from_slice(&next_free.to_le_bytes());
    buf[508..512].copy_from_slice(&0xAA550000u32.to_le_bytes());
    img[off..off + 512].copy_from_slice(&buf);
}

fn write_fat_to_slice(img: &mut [u8], alloc: &Alloc, sectors_per_fat: u64) {
    let fat_bytes: Vec<u8> = alloc.fat.iter().flat_map(|v| v.to_le_bytes()).collect();
    let off0 = (RESERVED_SECTORS * SECTOR_SIZE) as usize;
    let off1 = ((RESERVED_SECTORS + sectors_per_fat) * SECTOR_SIZE) as usize;
    img[off0..off0 + fat_bytes.len()].copy_from_slice(&fat_bytes);
    img[off1..off1 + fat_bytes.len()].copy_from_slice(&fat_bytes);
}

fn write_tree_to_slice(
    img: &mut [u8],
    alloc: &Alloc,
    files: &[(&str, &Path)],
    volume_label: &[u8; 11],
    root: u32,
    efi: u32,
    boot: u32,
    file_starts: &[u32],
    file_sizes: &[u64],
) -> io::Result<()> {
    // ── Root directory ──
    // FAT spec order: volume label (if present), ".", "..", then other entries.
    let mut root_ents = Vec::<u8>::new();
    root_ents.extend_from_slice(&vol_entry(volume_label));
    // "." entry: points to root's own cluster
    root_ents.extend_from_slice(&dot_entry(root));
    // ".." entry: for root dir, parent cluster is 0 (El Torito / no parent)
    root_ents.extend_from_slice(&dotdot_entry(0));
    // EFI subdirectory
    root_ents.extend_from_slice(&entry_83(&pack_83(b"EFI", b""), 0x10, efi, 0));
    root_ents.resize(CLUSTER_SIZE as usize, 0);
    write_at(img, alloc.sector_of(root) * SECTOR_SIZE, &root_ents);

    // ── EFI directory ──
    let mut efi_ents = Vec::<u8>::new();
    // "." entry: points to efi's own cluster
    efi_ents.extend_from_slice(&dot_entry(efi));
    // ".." entry: points to parent (root)
    efi_ents.extend_from_slice(&dotdot_entry(root));
    // BOOT subdirectory
    efi_ents.extend_from_slice(&entry_83(&pack_83(b"BOOT", b""), 0x10, boot, 0));
    efi_ents.resize(CLUSTER_SIZE as usize, 0);
    write_at(img, alloc.sector_of(efi) * SECTOR_SIZE, &efi_ents);

    // ── BOOT directory + file data ──
    let mut boot_ents = Vec::<u8>::new();
    // "." entry: points to boot's own cluster
    boot_ents.extend_from_slice(&dot_entry(boot));
    // ".." entry: points to parent (efi)
    boot_ents.extend_from_slice(&dotdot_entry(efi));

    for (idx, (dest_name, source_path)) in files.iter().enumerate() {
        let file_size = file_sizes[idx] as u32;
        let first_clus = file_starts[idx];

        let upper = dest_name.to_uppercase();
        let parts: Vec<&str> = upper.rsplitn(2, '.').collect();
        let (stem, ext) = if parts.len() == 2 {
            (parts[1].as_bytes(), parts[0].as_bytes())
        } else {
            (parts[0].as_bytes(), b"".as_ref())
        };
        let short = pack_83(stem, ext);

        if let Some((lfn, sfn)) =
            make_directory_entries(dest_name, &short, 0x20, first_clus, file_size)
        {
            boot_ents.extend_from_slice(&lfn);
            boot_ents.extend_from_slice(&sfn);
        } else {
            boot_ents.extend_from_slice(&entry_83(&short, 0x20, first_clus, file_size));
        }

        // Copy file data directly from source into image
        let mut src = File::open(source_path)?;
        src.seek(SeekFrom::Start(0))?;
        let mut cur = first_clus;
        let mut remaining = file_size as u64;
        while remaining > 0 {
            let chunk = remaining.min(CLUSTER_SIZE) as usize;
            let off = (alloc.sector_of(cur) * SECTOR_SIZE) as usize;
            // read_exact is used because we have pre-calculated the exact chunk size
            // and the img buffer is zero-initialized, so any short read would
            // leave zeroes anyway.
            src.read_exact(&mut img[off..off + chunk])?;
            remaining = remaining.saturating_sub(chunk as u64);
            let next = alloc.fat[cur as usize];
            if next == 0x0FFFFFFF || remaining == 0 {
                break;
            }
            cur = next;
        }
    }

    // BOOT directory is allocated a single cluster (4 KiB = ~128 dir entries).
    // Reject images that would exceed this capacity to avoid silent truncation.
    if boot_ents.len() > CLUSTER_SIZE as usize {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "BOOT directory entries ({} bytes) exceed single cluster limit ({} bytes): \
                 too many EFI boot files for this FAT image",
                boot_ents.len(),
                CLUSTER_SIZE,
            ),
        ));
    }
    boot_ents.resize(CLUSTER_SIZE as usize, 0);
    write_at(img, alloc.sector_of(boot) * SECTOR_SIZE, &boot_ents);

    Ok(())
}

// ── FAT32 layout solver ──

fn calculate_fat32_layout(total_sectors: u64, reserved: u64, spc: u64) -> (u64, u64) {
    let mut data = total_sectors - reserved;
    loop {
        let entries = data.div_ceil(spc) + 2;
        let fat_sectors = (entries * 4).div_ceil(SECTOR_SIZE);
        let new = if 2 * fat_sectors + reserved < total_sectors {
            total_sectors - reserved - 2 * fat_sectors
        } else {
            1
        };
        if new >= data {
            break;
        }
        data = new;
    }
    let entries = data.div_ceil(spc) + 2;
    let fat_sectors = (entries * 4).div_ceil(SECTOR_SIZE);
    (fat_sectors, data)
}

// ── Public API ──

/// Build a FAT32 UEFI boot image in memory and write it to disk.
/// Returns the size of the image in 512-byte sectors.
pub fn create_fat_image(
    fat_img_path: &Path,
    files: &[(&str, &Path)],
    hidden: u32,
) -> io::Result<u32> {
    // Check files exist
    for (_, p) in files {
        if !p.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{:?}", p),
            ));
        }
    }
    if files.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one file",
        ));
    }

    let (img, total_sectors) = build_image(files, hidden)?;

    // Write to file and fsync
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(fat_img_path)?;
    file.write_all(&img)?;
    file.sync_all()?;
    drop(file);

    Ok(total_sectors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::tempdir;

    #[test]
    fn test_layout() {
        let (fat, data) = calculate_fat32_layout(532480, 32, 8);
        assert!(data + 2 * fat + 32 <= 532480);
        assert!(fat > 0 && fat < 4096);
        assert!(data / 8 >= 65525);
    }

    #[test]
    fn test_create_inmem() -> io::Result<()> {
        let dir = tempdir()?;
        let l = dir.path().join("l.efi");
        let k = dir.path().join("k.elf");
        std::fs::write(&l, b"UEFI loader")?;
        std::fs::write(&k, b"ELF kernel")?;
        let img = dir.path().join("f.img");
        create_fat_image(&img, &[("BOOTX64.EFI", l.as_path()), ("KERNEL.EFI", k.as_path())], 0)?;
        assert!(img.exists());
        // fatfs read-back
        let r = File::open(&img)?;
        let fs = fatfs::FileSystem::new(r, fatfs::FsOptions::new())?;
        let root = fs.root_dir();
        let mut v = Vec::new();
        root.open_file("EFI/BOOT/BOOTX64.EFI")?.read_to_end(&mut v)?;
        assert_eq!(v, b"UEFI loader");
        v.clear();
        root.open_file("EFI/BOOT/KERNEL.EFI")?.read_to_end(&mut v)?;
        assert_eq!(v, b"ELF kernel");
        Ok(())
    }

    #[test]
    fn test_hidden() -> io::Result<()> {
        let dir = tempdir()?;
        let l = dir.path().join("b.efi");
        std::fs::write(&l, b"BOOT")?;
        let img = dir.path().join("fh.img");
        create_fat_image(&img, &[("BOOTX64.EFI", l.as_path())], 2048)?;
        let mut bytes = Vec::new();
        File::open(&img)?.read_to_end(&mut bytes)?;
        let hidden = u32::from_le_bytes(bytes[0x1C..0x20].try_into().unwrap());
        assert_eq!(hidden, 2048);
        let fs = fatfs::FileSystem::new(File::open(&img)?, fatfs::FsOptions::new())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let mut v = Vec::new();
        fs.root_dir()
            .open_file("EFI/BOOT/BOOTX64.EFI")?
            .read_to_end(&mut v)?;
        assert_eq!(v, b"BOOT");
        Ok(())
    }

    #[test]
    fn test_checksum() {
        assert_eq!(
            lfn_checksum(&pack_83(b"BOOTX64", b"EFI")),
            0x1D
        );
    }

    #[test]
    fn test_no_lfn() {
        assert!(make_directory_entries("EFI", &pack_83(b"EFI", b""), 0x10, 3, 0).is_none());
    }

    #[test]
    fn test_lfn() {
        let r = make_directory_entries(
            "BOOTX64.EFI",
            &pack_83(b"BOOTX64", b"EFI"),
            0x20,
            5,
            1024,
        )
        .unwrap();
        assert_eq!(r.0.len(), 32);
        assert_eq!(r.1.len(), 32);
    }

    #[test]
    fn test_lfn2() {
        let r = make_directory_entries(
            "KERNEL.EFI",
            &pack_83(b"KERNEL", b"EFI"),
            0x20,
            0x15,
            0x4000,
        )
        .unwrap();
        assert_eq!(r.0.len(), 32);
    }
}