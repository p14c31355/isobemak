// FAT32 UEFI boot image creator — hand-written to avoid fatfs LFN bugs on "."/"..".
use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
};

const SECTOR: u64 = 512;
const CLUSTER: u64 = 4096;
const SEC_PER_CLUS: u64 = 8;
const RESERVED: u64 = 32;
const MIN_CLUSTERS: usize = 65525;
const FAT_SPACE: u64 = 2 * 1024 * 1024;

// ── 8.3 names ──

fn pack_83(name: &[u8], ext: &[u8]) -> [u8; 11] {
    let mut out = [b' '; 11];
    let n = name.len().min(8);
    out[..n].copy_from_slice(&name[..n]);
    let e = ext.len().min(3);
    out[8..8 + e].copy_from_slice(&ext[..e]);
    out
}

fn lfn_checksum(short: &[u8; 11]) -> u8 {
    short
        .iter()
        .fold(0u8, |sum, &b| sum.rotate_right(1).wrapping_add(b))
}

fn make_lfn(
    name: &str,
    short: &[u8; 11],
    attr: u8,
    first_cluster: u32,
    file_size: u32,
) -> Option<(Vec<u8>, Vec<u8>)> {
    let plain = name.len() <= 12
        && !name.contains('.')
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if plain {
        return None;
    }

    let chk = lfn_checksum(short);
    let mut chars: Vec<u16> = name.encode_utf16().collect();
    let num_lfn = chars.len().div_ceil(13);
    let padded_len = num_lfn * 13;
    chars.resize(padded_len, 0xFFFF);
    let slot_start = (num_lfn - 1) * 13;
    let tail_len = (chars.len() - slot_start).min(13);
    chars[slot_start + tail_len - 1] = 0x0000;
    for j in (slot_start + tail_len)..slot_start + 13 {
        if j < chars.len() {
            chars[j] = 0xFFFF;
        }
    }

    let mut lfn = Vec::with_capacity(num_lfn * 32);
    for i in (0..num_lfn).rev() {
        let seq = (i + 1) as u8 | if i == num_lfn - 1 { 0x40 } else { 0 };
        let seg = &chars[i * 13..(i + 1) * 13];
        let mut e = [0u8; 32];
        e[0] = seq;
        e[1..3].copy_from_slice(&seg[0].to_le_bytes());
        e[3..5].copy_from_slice(&seg[1].to_le_bytes());
        e[5..7].copy_from_slice(&seg[2].to_le_bytes());
        e[7..9].copy_from_slice(&seg[3].to_le_bytes());
        e[9..11].copy_from_slice(&seg[4].to_le_bytes());
        e[11] = 0x0F;
        e[13] = chk;
        for k in 0..7 {
            e[14 + k * 2..16 + k * 2].copy_from_slice(&seg[5 + k].to_le_bytes());
        }
        e[28..30].copy_from_slice(&seg[11].to_le_bytes());
        e[30..32].copy_from_slice(&seg[12].to_le_bytes());
        lfn.extend_from_slice(&e);
    }

    let mut sfn = [0u8; 32];
    sfn[..11].copy_from_slice(short);
    sfn[11] = attr;
    sfn[16..18].copy_from_slice(&0x0000u16.to_le_bytes());
    sfn[18..20].copy_from_slice(&0x21u16.to_le_bytes());
    sfn[20..22].copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
    sfn[26..28].copy_from_slice(&(first_cluster as u16).to_le_bytes());
    sfn[28..32].copy_from_slice(&file_size.to_le_bytes());
    Some((lfn, sfn.to_vec()))
}

// ── Cluster allocator ──

struct Alloc {
    fat: Vec<u32>,
    clusters: usize,
    data_start: u64,
}

impl Alloc {
    fn new(total_sectors: u64, sectors_per_fat: u64) -> Self {
        let data_start = RESERVED + 2 * sectors_per_fat;
        let clusters = ((total_sectors - data_start) / SEC_PER_CLUS) as usize;
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
        let mut first = None;
        let mut prev = None;
        let mut n = 0;
        for i in 2..self.fat.len() {
            if self.fat[i] == 0 {
                if first.is_none() {
                    first = Some(i as u32);
                }
                if let Some(p) = prev {
                    self.fat[p as usize] = i as u32;
                }
                prev = Some(i as u32);
                self.fat[i] = 0x0FFFFFFF;
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

// ── Directory entry helpers ──

fn entry_83(short: &[u8; 11], attr: u8, first_cluster: u32, file_size: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(short);
    e[11] = attr;
    e[16..18].copy_from_slice(&0x0000u16.to_le_bytes());
    e[18..20].copy_from_slice(&0x21u16.to_le_bytes());
    e[20..22].copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
    e[22..24].copy_from_slice(&0x0000u16.to_le_bytes());
    e[26..28].copy_from_slice(&(first_cluster as u16).to_le_bytes());
    e[28..32].copy_from_slice(&file_size.to_le_bytes());
    e
}

fn dot_entries(curr: u32, parent: u32) -> [u8; 64] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(&entry_83(
        &{
            let mut n = [b' '; 11];
            n[0] = b'.';
            n
        },
        0x10,
        curr,
        0,
    ));
    buf[32..].copy_from_slice(&entry_83(
        &{
            let mut n = [b' '; 11];
            n[0] = b'.';
            n[1] = b'.';
            n
        },
        0x10,
        parent,
        0,
    ));
    buf
}

fn vol_entry(label: &[u8; 11]) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(label);
    e[11] = 0x08;
    e[16..18].copy_from_slice(&0x21u16.to_le_bytes());
    e[18..20].copy_from_slice(&0x21u16.to_le_bytes());
    e
}

// ── BPB / FSInfo writers ──

fn write_bpb(
    img: &mut [u8],
    off: u64,
    total_sectors: u32,
    fat_sectors: u32,
    hidden: u32,
    serial: u32,
) {
    let off = off as usize;
    let mut b = [0u8; 90];
    b[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    b[3..11].copy_from_slice(b"MSWIN4.1");
    b[11..13].copy_from_slice(&512u16.to_le_bytes());
    b[13] = SEC_PER_CLUS as u8;
    b[14..16].copy_from_slice(&(RESERVED as u16).to_le_bytes());
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
    img[off + 510..off + 512].copy_from_slice(&0xAA55u16.to_le_bytes());
}

fn write_fsinfo(img: &mut [u8], sector: u64, free: u32, next: u32) {
    let off = (sector * SECTOR) as usize;
    let mut buf = [0u8; 512];
    buf[0..4].copy_from_slice(&0x41615252u32.to_le_bytes());
    buf[484..488].copy_from_slice(&0x61417272u32.to_le_bytes());
    buf[488..492].copy_from_slice(&free.to_le_bytes());
    buf[492..496].copy_from_slice(&next.to_le_bytes());
    buf[508..512].copy_from_slice(&0xAA550000u32.to_le_bytes());
    img[off..off + 512].copy_from_slice(&buf);
}

// ── FAT32 layout solver ──

fn calc_layout(total_sectors: u64, reserved: u64, spc: u64) -> (u64, u64) {
    let mut data = total_sectors - reserved;
    loop {
        let entries = data.div_ceil(spc) + 2;
        let fat_sectors = (entries * 4).div_ceil(SECTOR);
        let new = total_sectors
            .saturating_sub(reserved + 2 * fat_sectors)
            .max(1);
        if new >= data {
            break;
        }
        data = new;
    }
    let entries = data.div_ceil(spc) + 2;
    let fat_sectors = (entries * 4).div_ceil(SECTOR);
    (fat_sectors, data)
}

// ── Image builder ──

fn build_image(files: &[(&str, &Path)], hidden: u32) -> io::Result<(Vec<u8>, u32)> {
    if files.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one file",
        ));
    }
    let mut content_size = 0u64;
    for (_, p) in files {
        if !p.exists() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("{:?}", p)));
        }
        content_size += p.metadata()?.len();
    }
    let logical = (content_size + FAT_SPACE).div_ceil(SECTOR) * SECTOR;
    let min_size = MIN_CLUSTERS as u64 * CLUSTER + RESERVED * SECTOR + 2 * 1024 * SECTOR;
    let total_sectors_raw = logical.max(min_size) / SECTOR;
    let (fat_sectors_raw, data_raw) = calc_layout(total_sectors_raw, RESERVED, SEC_PER_CLUS);
    let data_aligned = (data_raw / SEC_PER_CLUS) * SEC_PER_CLUS;
    let fat_sectors = fat_sectors_raw as u32;
    let total_sectors = (RESERVED + 2 * fat_sectors as u64 + data_aligned) as u32;

    let serial: u32 = rand::random();
    let vol_label = pack_83(b"EFI", b"");
    let mut img = vec![0u8; total_sectors as usize * SECTOR as usize];

    write_bpb(&mut img, 0, total_sectors, fat_sectors, hidden, serial);

    let mut alloc = Alloc::new(total_sectors as u64, fat_sectors as u64);
    let err = |what| io::Error::other(format!("FAT32: out of free clusters for {what}"));
    let root = alloc.alloc(1).ok_or_else(|| err("root directory"))?;
    let efi = alloc.alloc(1).ok_or_else(|| err("EFI directory"))?;
    let boot = alloc.alloc(1).ok_or_else(|| err("BOOT directory"))?;

    let mut file_starts = Vec::with_capacity(files.len());
    let mut file_sizes = Vec::with_capacity(files.len());
    for (_name, p) in files {
        let sz = p.metadata()?.len();
        let n = (sz.div_ceil(CLUSTER)).max(1) as u32;
        let start = alloc.alloc(n).ok_or_else(|| {
            io::Error::other(format!("FAT32: out of free clusters for file (need {n})"))
        })?;
        file_starts.push(start);
        file_sizes.push(sz);
    }

    // Root dir: vol label, ".", "..", EFI subdir
    let mut area = vec![0u8; CLUSTER as usize];
    area[..32].copy_from_slice(&vol_entry(&vol_label));
    area[32..96].copy_from_slice(&dot_entries(root, 0));
    area[96..128].copy_from_slice(&entry_83(&pack_83(b"EFI", b""), 0x10, efi, 0));
    img[alloc.sector_of(root) as usize * 512..][..CLUSTER as usize].copy_from_slice(&area);

    // EFI dir: ".", "..", BOOT subdir
    area.fill(0);
    area[..64].copy_from_slice(&dot_entries(efi, root));
    area[64..96].copy_from_slice(&entry_83(&pack_83(b"BOOT", b""), 0x10, boot, 0));
    img[alloc.sector_of(efi) as usize * 512..][..CLUSTER as usize].copy_from_slice(&area);

    // BOOT dir + file data
    let mut dir = Vec::<u8>::new();
    dir.extend_from_slice(&dot_entries(boot, efi));
    for (idx, (dest_name, source_path)) in files.iter().enumerate() {
        let file_size = file_sizes[idx] as u32;
        let first_clus = file_starts[idx];

        let upper = dest_name.to_uppercase();
        let (stem, ext) = upper
            .rsplit_once('.')
            .map_or((upper.as_bytes(), b"".as_ref()), |(s, e)| {
                (s.as_bytes(), e.as_bytes())
            });
        let short = pack_83(stem, ext);

        if let Some((lfn, sfn)) = make_lfn(dest_name, &short, 0x20, first_clus, file_size) {
            dir.extend_from_slice(&lfn);
            dir.extend_from_slice(&sfn);
        } else {
            dir.extend_from_slice(&entry_83(&short, 0x20, first_clus, file_size));
        }

        let mut src = File::open(source_path)?;
        let mut cur = first_clus;
        let mut remaining = file_size as u64;
        while remaining > 0 {
            let chunk = remaining.min(CLUSTER) as usize;
            let off = (alloc.sector_of(cur) * SECTOR) as usize;
            src.read_exact(&mut img[off..off + chunk])?;
            remaining = remaining.saturating_sub(chunk as u64);
            if remaining == 0 {
                break;
            }
            let next = alloc.fat[cur as usize];
            if next == 0x0FFFFFFF {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "FAT cluster chain too short",
                ));
            }
            cur = next;
        }
    }
    if dir.len() > CLUSTER as usize {
        return Err(io::Error::other(format!(
            "BOOT dir ({} bytes) exceeds cluster limit ({CLUSTER})",
            dir.len()
        )));
    }
    dir.resize(CLUSTER as usize, 0);
    img[alloc.sector_of(boot) as usize * 512..][..CLUSTER as usize].copy_from_slice(&dir);

    // FAT tables
    let fat_bytes: Vec<u8> = alloc.fat.iter().flat_map(|v| v.to_le_bytes()).collect();
    let fat0 = (RESERVED * SECTOR) as usize;
    let fat1 = ((RESERVED + fat_sectors as u64) * SECTOR) as usize;
    img[fat0..fat0 + fat_bytes.len()].copy_from_slice(&fat_bytes);
    img[fat1..fat1 + fat_bytes.len()].copy_from_slice(&fat_bytes);

    // FSInfo
    let total_clusters = alloc.clusters as u32;
    let used = alloc.fat.iter().filter(|&&v| v != 0).count() as u32 - 2;
    let free = total_clusters - used;
    let next_free = alloc.fat.iter().position(|&v| v == 0).unwrap_or(2) as u32;
    write_fsinfo(&mut img, 1, free, next_free);
    write_fsinfo(&mut img, 7, free, next_free);

    // Backup BPB at sector 6
    write_bpb(
        &mut img,
        6 * SECTOR,
        total_sectors,
        fat_sectors,
        hidden,
        serial,
    );

    Ok((img, total_sectors))
}

// ── Public API ──

pub fn create_fat_image(
    fat_img_path: &Path,
    files: &[(&str, &Path)],
    hidden: u32,
) -> io::Result<u32> {
    let (img, total_sectors) = build_image(files, hidden)?;
    let mut file = File::options()
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
        let (fat, data) = calc_layout(532480, 32, 8);
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
        create_fat_image(
            &img,
            &[("BOOTX64.EFI", l.as_path()), ("KERNEL.EFI", k.as_path())],
            0,
        )?;
        assert!(img.exists());
        let r = File::open(&img)?;
        let fs = fatfs::FileSystem::new(r, fatfs::FsOptions::new())?;
        let root = fs.root_dir();
        let mut v = Vec::new();
        root.open_file("EFI/BOOT/BOOTX64.EFI")?
            .read_to_end(&mut v)?;
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
        assert_eq!(
            u32::from_le_bytes(bytes[0x1C..0x20].try_into().unwrap()),
            2048
        );
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
        assert_eq!(lfn_checksum(&pack_83(b"BOOTX64", b"EFI")), 0x1D);
    }

    #[test]
    fn test_no_lfn() {
        assert!(make_lfn("EFI", &pack_83(b"EFI", b""), 0x10, 3, 0).is_none());
    }

    #[test]
    fn test_lfn() {
        let r = make_lfn("BOOTX64.EFI", &pack_83(b"BOOTX64", b"EFI"), 0x20, 5, 1024).unwrap();
        assert_eq!(r.0.len(), 32);
        assert_eq!(r.1.len(), 32);
    }

    #[test]
    fn test_lfn2() {
        let r = make_lfn(
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
