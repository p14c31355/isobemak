// FAT12/16/32 UEFI boot image creator — hand-written to avoid fatfs LFN bugs on "."/"..".
//
// Auto-selects FAT type based on image size so that small EFI System Partitions
// (a few MB) use FAT12/FAT16 instead of the 255 MiB minimum imposed by FAT32.
use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
};

const SECTOR: u64 = 512;
const CLUSTER: u64 = 4096;
const SEC_PER_CLUS: u64 = 8;

// ── FAT type selection ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FatType {
    Fat12,
    Fat16,
    Fat32,
}

impl FatType {
    /// Choose the smallest FAT type that can hold `clusters` data clusters.
    #[allow(dead_code)]
    fn from_clusters(clusters: usize) -> Self {
        if clusters <= 4084 {
            FatType::Fat12
        } else if clusters <= 65524 {
            FatType::Fat16
        } else {
            FatType::Fat32
        }
    }

    fn reserved_sectors(self) -> u64 {
        match self {
            FatType::Fat12 | FatType::Fat16 => 1,
            FatType::Fat32 => 32,
        }
    }

    fn root_dir_entries(self) -> usize {
        match self {
            FatType::Fat12 => 224, // 14 sectors — plenty for a small ESP
            FatType::Fat16 => 512, // 32 sectors
            FatType::Fat32 => 0,   // root is a cluster chain, not a fixed region
        }
    }

    fn root_dir_sectors(self) -> u64 {
        (self.root_dir_entries() as u64 * 32).div_ceil(SECTOR)
    }

    fn eoc_marker(self) -> u32 {
        match self {
            FatType::Fat12 => 0x0FF8,
            FatType::Fat16 => 0xFFF8,
            FatType::Fat32 => 0x0FFFFFF8,
        }
    }

    fn eoc_chain_end(self) -> u32 {
        match self {
            FatType::Fat12 => 0x0FFF,
            FatType::Fat16 => 0xFFFF,
            FatType::Fat32 => 0x0FFFFFFF,
        }
    }

    fn fstype_str(self) -> &'static [u8; 8] {
        match self {
            FatType::Fat12 => b"FAT12   ",
            FatType::Fat16 => b"FAT16   ",
            FatType::Fat32 => b"FAT32   ",
        }
    }

    /// Number of bytes one FAT entry occupies on disk.
    fn entry_bytes(self) -> u64 {
        match self {
            FatType::Fat12 => 12,
            FatType::Fat16 => 16,
            FatType::Fat32 => 32,
        }
    }

    /// Whether the root directory is stored in the data-cluster area (FAT32)
    /// or in a fixed region immediately after the FATs (FAT12/16).
    fn root_is_cluster(self) -> bool {
        matches!(self, FatType::Fat32)
    }
}

// ── 8.3 names ───────────────────────────────────────────────────────────────

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
    let len = chars.len();
    let num_lfn = (len + 1).div_ceil(13); // +1 for null terminator
    chars.resize(num_lfn * 13, 0xFFFF);
    chars[len] = 0x0000;

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
        for k in 0..6 {
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
    sfn[24..26].copy_from_slice(&0x21u16.to_le_bytes());
    sfn[26..28].copy_from_slice(&(first_cluster as u16).to_le_bytes());
    sfn[28..32].copy_from_slice(&file_size.to_le_bytes());
    Some((lfn, sfn.to_vec()))
}

// ── Cluster allocator ───────────────────────────────────────────────────────

struct Alloc {
    /// Cluster chain table.  Values are stored "full width" (u32) but only
    /// the bits valid for the current FAT type are meaningful.  EOC markers
    /// are stored using `FatType::eoc_marker()` / `FatType::eoc_chain_end()`.
    fat: Vec<u32>,
    clusters: usize,
    /// LBA (in 512-byte sectors) where the cluster heap starts.
    data_start: u64,
    fat_type: FatType,
    /// Pre-computed sectors-per-FAT, taken from the layout solver.
    sectors_per_fat: u64,
}

impl Alloc {
    fn new(total_sectors: u64, sectors_per_fat: u64, fat_type: FatType) -> Self {
        let root_sectors = fat_type.root_dir_sectors();
        let data_start = fat_type.reserved_sectors() + 2 * sectors_per_fat + root_sectors;
        let clusters = ((total_sectors - data_start) / SEC_PER_CLUS) as usize;
        let mut fat = vec![0u32; clusters + 2];
        fat[0] = fat_type.eoc_marker();
        fat[1] = fat_type.eoc_chain_end();
        Self {
            fat,
            clusters,
            data_start,
            fat_type,
            sectors_per_fat,
        }
    }

    fn alloc(&mut self, count: u32) -> Option<u32> {
        let eoc = self.fat_type.eoc_chain_end();
        let mut first = None;
        let mut prev = None;
        let mut n = 0;
        for i in 2..self.fat.len() {
            if self.fat[i] == 0 {
                if first.is_none() {
                    first = Some(i as u32);
                }
                if let Some(p) = prev {
                    // Before writing the link, make sure p is marked as a
                    // valid cluster (not EOC yet).
                    self.fat[p as usize] = i as u32;
                }
                prev = Some(i as u32);
                // Temporarily mark as end-of-chain; the next iteration
                // will overwrite this if more clusters are needed.
                self.fat[i] = eoc;
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

    /// Number of sectors occupied by the root directory (0 for FAT32).
    fn root_dir_sectors(&self) -> u64 {
        self.fat_type.root_dir_sectors()
    }

    /// Where the root directory region starts (in 512-byte LBA).
    fn root_dir_start(&self) -> u64 {
        self.fat_type.reserved_sectors() + 2 * self.sectors_per_fat
    }

    #[allow(dead_code)]
    fn sectors_per_fat(&self) -> u64 {
        self.sectors_per_fat
    }
}

// ── Directory entry helpers ─────────────────────────────────────────────────

fn entry_83(short: &[u8; 11], attr: u8, first_cluster: u32, file_size: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(short);
    e[11] = attr;
    e[16..18].copy_from_slice(&0x0000u16.to_le_bytes());
    e[18..20].copy_from_slice(&0x21u16.to_le_bytes());
    e[20..22].copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
    e[24..26].copy_from_slice(&0x21u16.to_le_bytes());
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

// ── BPB / FSInfo writers ────────────────────────────────────────────────────

fn write_bpb(
    img: &mut [u8],
    off: u64,
    fat_type: FatType,
    total_sectors: u32,
    fat_sectors: u32,
    hidden: u32,
    serial: u32,
    root_dir_entries: u16,
) {
    let off = off as usize;
    let mut b = [0u8; 90];
    b[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    b[3..11].copy_from_slice(b"MSWIN4.1");
    b[11..13].copy_from_slice(&512u16.to_le_bytes()); // bytes per sector
    b[13] = SEC_PER_CLUS as u8; // sectors per cluster
    b[14..16].copy_from_slice(&(fat_type.reserved_sectors() as u16).to_le_bytes());
    b[16] = 2; // number of FATs

    // Root directory entries — 0 for FAT32, non-zero for FAT12/16
    b[17..19].copy_from_slice(&root_dir_entries.to_le_bytes());

    // Total sectors (u16 field) — 0 if >= 65536
    let total16 = if total_sectors < 65536 {
        total_sectors as u16
    } else {
        0
    };
    b[19..21].copy_from_slice(&total16.to_le_bytes());

    b[21] = 0xF8; // media descriptor (fixed disk)
    b[22..24].copy_from_slice(&0u16.to_le_bytes()); // sectors per FAT (u16, 0 for FAT32)
    b[24..26].copy_from_slice(&32u16.to_le_bytes()); // sectors per track
    b[26..28].copy_from_slice(&64u16.to_le_bytes()); // number of heads
    b[28..32].copy_from_slice(&hidden.to_le_bytes());

    match fat_type {
        FatType::Fat12 | FatType::Fat16 => {
            // FAT12/16: sectors per FAT in u16 field at offset 22
            b[22..24].copy_from_slice(&(fat_sectors as u16).to_le_bytes());
            // BPB_TotSec32 must be 0 when BPB_TotSec16 is non-zero (FAT spec)
            b[32..36].copy_from_slice(&0u32.to_le_bytes());
            b[36] = 0x80; // drive number
            // b[37] = 0; reserved
            b[38] = 0x29; // extended boot signature
            b[39..43].copy_from_slice(&serial.to_le_bytes());
            b[43..54].copy_from_slice(b"EFI        "); // volume label
            b[54..62].copy_from_slice(fat_type.fstype_str());
        }
        FatType::Fat32 => {
            // Total sectors (u32 field at offset 32)
            b[32..36].copy_from_slice(&total_sectors.to_le_bytes());
            b[36..40].copy_from_slice(&fat_sectors.to_le_bytes()); // sectors per FAT
            b[40..42].copy_from_slice(&0u16.to_le_bytes()); // flags
            b[42..44].copy_from_slice(&0u16.to_le_bytes()); // FAT version
            b[44..48].copy_from_slice(&2u32.to_le_bytes()); // root cluster
            b[48..50].copy_from_slice(&1u16.to_le_bytes()); // FSInfo sector
            b[50..52].copy_from_slice(&6u16.to_le_bytes()); // backup boot sector
            b[64] = 0x80; // drive number
            b[66] = 0x29; // extended boot signature
            b[67..71].copy_from_slice(&serial.to_le_bytes());
            b[71..82].copy_from_slice(b"EFI        "); // volume label
            b[82..90].copy_from_slice(fat_type.fstype_str());
        }
    }

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

// ── FAT table serialisation ─────────────────────────────────────────────────

/// Pack fat entries (stored as u32) into on-disk format.
fn write_fat_tables(
    img: &mut [u8],
    fat: &[u32],
    fat_type: FatType,
    sectors_per_fat: u64,
    reserved: u64,
) {
    let fat_size_bytes = (sectors_per_fat * SECTOR) as usize;
    let fat0_off = (reserved * SECTOR) as usize;
    let fat1_off = fat0_off + fat_size_bytes;

    match fat_type {
        FatType::Fat32 => {
            let bytes: Vec<u8> = fat.iter().flat_map(|v| v.to_le_bytes()).collect();
            let n = bytes.len().min(fat_size_bytes);
            img[fat0_off..fat0_off + n].copy_from_slice(&bytes[..n]);
            img[fat1_off..fat1_off + n].copy_from_slice(&bytes[..n]);
        }
        FatType::Fat16 => {
            let mut bytes = vec![0u8; fat_size_bytes];
            for (i, &v) in fat.iter().enumerate() {
                let off = i * 2;
                if off + 2 <= bytes.len() {
                    bytes[off..off + 2].copy_from_slice(&(v as u16).to_le_bytes());
                }
            }
            img[fat0_off..fat0_off + fat_size_bytes].copy_from_slice(&bytes);
            img[fat1_off..fat1_off + fat_size_bytes].copy_from_slice(&bytes);
        }
        FatType::Fat12 => {
            // 12-bit entries: two entries → three bytes.
            // Offset in bytes = i * 12 / 8 = i + i / 2.
            let mut bytes = vec![0u8; fat_size_bytes];
            for (i, &v) in fat.iter().enumerate() {
                let byte_off = i + i / 2;
                if byte_off + 1 >= bytes.len() {
                    break;
                }
                let val = (v & 0x0FFF) as u16;
                if i % 2 == 0 {
                    bytes[byte_off] = val as u8;
                    bytes[byte_off + 1] = ((val >> 8) & 0x0F) as u8;
                } else {
                    bytes[byte_off] |= ((val & 0x0F) as u8) << 4;
                    bytes[byte_off + 1] = (val >> 4) as u8;
                }
            }
            img[fat0_off..fat0_off + fat_size_bytes].copy_from_slice(&bytes);
            img[fat1_off..fat1_off + fat_size_bytes].copy_from_slice(&bytes);
        }
    }
}

// ── Layout solver ───────────────────────────────────────────────────────────

/// Iteratively compute `(sectors_per_fat, data_sectors)` given the total
/// sectors reserved for the FAT region and the root directory size (0 for
/// FAT32).  The result accounts for the space the FATs themselves occupy.
/// `entry_bits` is 12, 16, or 32 depending on the FAT type.
fn calc_layout(
    total_sectors: u64,
    reserved: u64,
    spc: u64,
    root_dir_sectors: u64,
    entry_bits: u64,
) -> (u64, u64) {
    let mut data = total_sectors - reserved - root_dir_sectors;
    loop {
        let entries = data.div_ceil(spc) + 2;
        let fat_bytes = (entries * entry_bits).div_ceil(8);
        let fat_sectors = fat_bytes.div_ceil(SECTOR);
        let new = total_sectors
            .saturating_sub(reserved + 2 * fat_sectors + root_dir_sectors)
            .max(1);
        if new >= data {
            break;
        }
        data = new;
    }
    let entries = data.div_ceil(spc) + 2;
    let fat_bytes = (entries * entry_bits).div_ceil(8);
    let fat_sectors = fat_bytes.div_ceil(SECTOR);
    (fat_sectors, data)
}

// ── Image builder ───────────────────────────────────────────────────────────
//
// Strategy (memory-efficient):
//   1. Estimate required size → determine FAT type and total sectors.
//   2. Pre-allocate a Vec<u8> of the exact final size.
//   3. Write directory entries and file payloads.
//   4. Serialise FAT tables.
//   5. Write BPB last (so no back-patching needed).
//   6. Return the buffer (already exactly sized).

fn build_image(files: &[(&str, &Path)], hidden: u32) -> io::Result<(Vec<u8>, u32)> {
    if files.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one file",
        ));
    }

    // ── 1. Determine FAT type ──────────────────────────────────────────
    let mut content_size = 0u64;
    for (_, p) in files {
        if !p.exists() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("{:?}", p)));
        }
        content_size += p.metadata()?.len();
    }

    // Rough estimate: content + directory overhead + 2×FATs.
    let overhead = CLUSTER * 5 + 256 * SECTOR; // generous upper bound
    let rough_total = content_size + overhead;

    // Clamp to a safe minimum so we always have at least a few data clusters.
    let min_sectors = (12 * SEC_PER_CLUS + 16).max(2880); // at least a 1.44 MB floppy worth
    let estimated_sectors = rough_total.div_ceil(SECTOR).max(min_sectors);

    // Pick the first candidate FAT type, then refine with a layout pass.
    let candidates = [FatType::Fat12, FatType::Fat16, FatType::Fat32];
    let mut chosen_type = FatType::Fat32; // fallback
    let mut chosen_total: u32 = 0;
    let mut chosen_fat_sectors: u32 = 0;

    for &ft in &candidates {
        let reserved = ft.reserved_sectors();
        let rds = ft.root_dir_sectors();
        // Try the current estimate; if the clusters don't fit then try FAT32.
        let (fs, ds) = calc_layout(
            estimated_sectors,
            reserved,
            SEC_PER_CLUS,
            rds,
            ft.entry_bytes(),
        );
        let data_aligned = (ds / SEC_PER_CLUS) * SEC_PER_CLUS;
        let total = (reserved + 2 * fs + rds + data_aligned) as u32;
        let clusters = data_aligned / SEC_PER_CLUS;

        // FAT12/16 volumes must fit in 65535 sectors (u16 BPB_TotSec16)
        let fits_in_u16 = total < 65536;

        // Does the computed cluster count fall within this FAT type's range?
        let max_clusters = match ft {
            FatType::Fat12 => 4084u64,
            FatType::Fat16 => 65524u64,
            FatType::Fat32 => u64::MAX,
        };
        if clusters <= max_clusters && fits_in_u16 {
            chosen_type = ft;
            chosen_total = total;
            chosen_fat_sectors = fs as u32;
            break;
        }
    }

    // If we still need FAT32, compute final layout with FAT32 parameters.
    if chosen_type == FatType::Fat32 && chosen_total == 0 {
        let reserved = FatType::Fat32.reserved_sectors();
        let (fs, ds) = calc_layout(estimated_sectors, reserved, SEC_PER_CLUS, 0, 32);
        let data_aligned = (ds / SEC_PER_CLUS) * SEC_PER_CLUS;
        chosen_total = (reserved + 2 * fs + data_aligned) as u32;
        chosen_fat_sectors = fs as u32;
    }

    let total_sectors = chosen_total;

    // ── 2. Allocate buffer ─────────────────────────────────────────────
    let serial: u32 = rand::random();
    let vol_label = pack_83(b"EFI", b"");
    let mut img = vec![0u8; total_sectors as usize * SECTOR as usize];

    // ── 3. Set up allocator ────────────────────────────────────────────
    let mut alloc = Alloc::new(total_sectors as u64, chosen_fat_sectors as u64, chosen_type);
    let err = |what| io::Error::other(format!("FAT: out of free clusters for {what}"));

    // Root directory: cluster for FAT32, fixed region for FAT12/16.
    let root = if chosen_type.root_is_cluster() {
        Some(alloc.alloc(1).ok_or_else(|| err("root directory"))?)
    } else {
        None
    };
    let efi = alloc.alloc(1).ok_or_else(|| err("EFI directory"))?;
    let boot = alloc.alloc(1).ok_or_else(|| err("BOOT directory"))?;

    let mut file_starts = Vec::with_capacity(files.len());
    let mut file_sizes = Vec::with_capacity(files.len());
    for (_name, p) in files {
        let sz = p.metadata()?.len();
        let n = (sz.div_ceil(CLUSTER)).max(1) as u32;
        let start = alloc.alloc(n).ok_or_else(|| {
            io::Error::other(format!("FAT: out of free clusters for file (need {n})"))
        })?;
        file_starts.push(start);
        file_sizes.push(sz);
    }

    // ── 4. Write directory entries & file payloads ─────────────────────

    // 4a. Root directory
    let root_parent = 0u32; // FAT12/16 convention: 0 = root
    if let Some(root_clus) = root {
        // FAT32: root is a normal cluster
        let mut area = vec![0u8; CLUSTER as usize];
        area[..32].copy_from_slice(&vol_entry(&vol_label));
        area[32..64].copy_from_slice(&entry_83(&pack_83(b"EFI", b""), 0x10, efi, 0));
        img[alloc.sector_of(root_clus) as usize * 512..][..CLUSTER as usize].copy_from_slice(&area);
    } else {
        // FAT12/16: write directly to the fixed root directory region
        let root_start = (alloc.root_dir_start() * SECTOR) as usize;
        let root_size = (alloc.root_dir_sectors() * SECTOR) as usize;
        let mut area = vec![0u8; CLUSTER as usize]; // use only as much as needed
        area[..32].copy_from_slice(&vol_entry(&vol_label));
        area[32..64].copy_from_slice(&entry_83(&pack_83(b"EFI", b""), 0x10, efi, 0));
        let copy_len = area.len().min(root_size);
        img[root_start..root_start + copy_len].copy_from_slice(&area[..copy_len]);
    }

    // 4b. EFI directory: ".", "..", BOOT subdir
    {
        let efi_parent = root.unwrap_or(root_parent);
        let mut area = vec![0u8; CLUSTER as usize];
        area[..64].copy_from_slice(&dot_entries(efi, efi_parent));
        area[64..96].copy_from_slice(&entry_83(&pack_83(b"BOOT", b""), 0x10, boot, 0));
        img[alloc.sector_of(efi) as usize * 512..][..CLUSTER as usize].copy_from_slice(&area);
    }

    // 4c. BOOT directory: ".", "..", file entries + file data
    {
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
                let eoc = chosen_type.eoc_chain_end();
                if next == eoc {
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
    }

    // ── 5. Write FAT tables ────────────────────────────────────────────
    write_fat_tables(
        &mut img,
        &alloc.fat,
        chosen_type,
        chosen_fat_sectors as u64,
        chosen_type.reserved_sectors(),
    );

    // ── 6. FSInfo (FAT32 only) ─────────────────────────────────────────
    if chosen_type == FatType::Fat32 {
        let total_clusters = alloc.clusters as u32;
        let used = alloc.fat.iter().filter(|&&v| v != 0).count() as u32 - 2;
        let free = total_clusters - used;
        let next_free = alloc.fat.iter().position(|&v| v == 0).unwrap_or(2) as u32;
        write_fsinfo(&mut img, 1, free, next_free);
        write_fsinfo(&mut img, 7, free, next_free);
    }

    // ── 7. Write BPB (last, after everything else is final) ────────────
    let root_dir_entries = chosen_type.root_dir_entries() as u16;
    write_bpb(
        &mut img,
        0,
        chosen_type,
        total_sectors,
        chosen_fat_sectors,
        hidden,
        serial,
        root_dir_entries,
    );

    // Backup BPB at sector 6 (FAT32 only)
    if chosen_type == FatType::Fat32 {
        write_bpb(
            &mut img,
            6 * SECTOR,
            chosen_type,
            total_sectors,
            chosen_fat_sectors,
            hidden,
            serial,
            root_dir_entries,
        );
    }

    Ok((img, total_sectors))
}

// ── Public API ──────────────────────────────────────────────────────────────

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

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::tempdir;

    #[test]
    fn test_layout_fat32() {
        let (fat, data) = calc_layout(532480, 32, 8, 0, 32);
        assert!(data + 2 * fat + 32 <= 532480);
        assert!(fat > 0 && fat < 4096);
        assert!(data / 8 >= 65525);
    }

    #[test]
    fn test_layout_fat16() {
        let (fat, data) = calc_layout(65536, 1, 8, 32, 16); // 32 MiB with FAT16 params
        assert!(data + 2 * fat + 1 + 32 <= 65536);
        assert!(fat > 0);
    }

    #[test]
    fn test_layout_fat12() {
        let (fat, data) = calc_layout(2880, 1, 8, 14, 12); // ~1.44 MiB floppy-sized
        assert!(data + 2 * fat + 1 + 14 <= 2880);
    }

    #[test]
    fn test_fat_type_selection() {
        assert_eq!(FatType::from_clusters(100), FatType::Fat12);
        assert_eq!(FatType::from_clusters(4084), FatType::Fat12);
        assert_eq!(FatType::from_clusters(4085), FatType::Fat16);
        assert_eq!(FatType::from_clusters(65524), FatType::Fat16);
        assert_eq!(FatType::from_clusters(65525), FatType::Fat32);
    }

    #[test]
    fn test_create_inmem_fat12() -> io::Result<()> {
        // Small files → should trigger FAT12
        let dir = tempdir()?;
        let l = dir.path().join("l.efi");
        let k = dir.path().join("k.elf");
        std::fs::write(&l, b"UEFI loader")?;
        std::fs::write(&k, b"ELF kernel")?;
        let img = dir.path().join("f.img");
        let sectors = create_fat_image(
            &img,
            &[("BOOTX64.EFI", l.as_path()), ("KERNEL.EFI", k.as_path())],
            0,
        )?;
        // Should be small — well under 255 MiB (522240 sectors)
        assert!(
            sectors < 522240,
            "FAT image is {sectors} sectors — expected < 522240 (255 MiB)"
        );
        assert!(img.exists());

        // Verify with fatfs
        let r = File::open(&img)?;
        let fs = fatfs::FileSystem::new(r, fatfs::FsOptions::new())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
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
    fn test_create_inmem_fat16() -> io::Result<()> {
        // Medium file → should trigger FAT16
        let dir = tempdir()?;
        let l = dir.path().join("l.efi");
        let k = dir.path().join("k.elf");
        // ~16 MiB — fits comfortably in FAT16 (65535 sector limit < 32 MiB total)
        std::fs::write(&l, vec![0u8; 16 * 1024 * 1024])?;
        std::fs::write(&k, b"ELF kernel")?;
        let img = dir.path().join("f.img");
        let sectors = create_fat_image(
            &img,
            &[("BOOTX64.EFI", l.as_path()), ("KERNEL.EFI", k.as_path())],
            0,
        )?;
        assert!(sectors < 65536, "FAT16 must be under 65536 sectors");
        assert!(img.exists());
        let r = File::open(&img)?;
        let fs = fatfs::FileSystem::new(r, fatfs::FsOptions::new())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let mut v = Vec::new();
        fs.root_dir()
            .open_file("EFI/BOOT/BOOTX64.EFI")?
            .read_to_end(&mut v)?;
        assert_eq!(v.len(), 16 * 1024 * 1024);
        v.clear();
        fs.root_dir()
            .open_file("EFI/BOOT/KERNEL.EFI")?
            .read_to_end(&mut v)?;
        assert_eq!(v, b"ELF kernel");
        Ok(())
    }

    #[test]
    fn test_calc_layout_fat32_threshold() {
        // Verify the layout solver works for FAT32-sized parameter sets.
        // 1 GiB image with 4K clusters → ~262k clusters → needs FAT32.
        let (fat, data) = calc_layout(2097152, 32, 8, 0, 32);
        // Layout must not overflow.
        assert!(data + 2 * fat + 32 <= 2097152);
        assert!(fat > 0);
        assert!(
            data / 8 > 65525,
            "should need > 65525 clusters (FAT32 territory)"
        );
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

    #[test]
    fn test_fat12_bpb() {
        // Create a minimal image and check the BPB fields manually.
        let dir = tempdir().unwrap();
        let f = dir.path().join("t.efi");
        std::fs::write(&f, b"hello").unwrap();
        let img = dir.path().join("t.img");
        create_fat_image(&img, &[("T.EFI", f.as_path())], 0).unwrap();

        let mut bytes = Vec::new();
        File::open(&img).unwrap().read_to_end(&mut bytes).unwrap();

        // bytes per sector = 512
        assert_eq!(u16::from_le_bytes([bytes[11], bytes[12]]), 512);
        // sectors per cluster = 8
        assert_eq!(bytes[13], 8);
        // reserved sectors = 1 (FAT12)
        assert_eq!(u16::from_le_bytes([bytes[14], bytes[15]]), 1);
        // number of FATs = 2
        assert_eq!(bytes[16], 2);
        // media = 0xF8
        assert_eq!(bytes[21], 0xF8);
        // boot signature
        assert_eq!(u16::from_le_bytes([bytes[510], bytes[511]]), 0xAA55);

        // Verify fatfs can read it
        let r = File::open(&img).unwrap();
        let fs = fatfs::FileSystem::new(r, fatfs::FsOptions::new())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
            .unwrap();
        let mut v = Vec::new();
        fs.root_dir()
            .open_file("EFI/BOOT/T.EFI")
            .unwrap()
            .read_to_end(&mut v)
            .unwrap();
        assert_eq!(v, b"hello");
    }
}
