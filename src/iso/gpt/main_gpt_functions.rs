use crate::iso::gpt::header::GptHeader;
use crate::iso::gpt::partition_entry::GptPartitionEntry;
use crc32fast::Hasher;
use std::io::{self, Seek, SeekFrom, Write};

fn crc_header(h: &mut GptHeader) -> u32 {
    h.header_crc32 = 0;
    let b = h.to_bytes();
    let mut hasher = Hasher::new();
    hasher.update(&b[..h.header_size as usize]);
    hasher.finalize()
}

fn crc_parts(parts: &[GptPartitionEntry], n: u32, es: u32) -> u32 {
    let mut arr = vec![0u8; (n * es) as usize];
    let mut off = 0;
    for p in parts {
        let pb = p.to_bytes();
        arr[off..off + pb.len()].copy_from_slice(&pb);
        off += pb.len();
    }
    let mut hasher = Hasher::new();
    hasher.update(&arr);
    hasher.finalize()
}

fn write_primary<W: Write + Seek>(
    w: &mut W,
    h: &GptHeader,
    parts: &[GptPartitionEntry],
    n: u32,
    es: u32,
    alba: u64,
) -> io::Result<()> {
    w.seek(SeekFrom::Start(512))?;
    h.write_to(w)?;
    w.seek(SeekFrom::Start(alba * 512))?;
    for p in parts {
        p.write_to(w)?;
    }
    for _ in parts.len()..n as usize {
        w.write_all(&vec![0u8; es as usize])?;
    }
    Ok(())
}

fn write_backup<W: Write + Seek>(
    w: &mut W,
    h: &GptHeader,
    parts: &[GptPartitionEntry],
    n: u32,
    es: u32,
    total: u64,
) -> io::Result<()> {
    let arr_sectors = ((n as u64) * (es as u64)).div_ceil(512);
    let mut bh = *h;
    bh.current_lba = total - 1;
    bh.backup_lba = 1;
    bh.partition_entry_lba = total.saturating_sub(1).saturating_sub(arr_sectors);
    bh.header_crc32 = crc_header(&mut bh);
    w.seek(SeekFrom::Start((total - 1) * 512))?;
    bh.write_to(w)?;
    w.seek(SeekFrom::Start((total - 1 - arr_sectors) * 512))?;
    for p in parts {
        p.write_to(w)?;
    }
    for _ in parts.len()..n as usize {
        w.write_all(&vec![0u8; es as usize])?;
    }
    Ok(())
}

pub fn write_gpt_structures<W: Write + Seek>(
    w: &mut W,
    total_lbas: u64,
    partitions: &[GptPartitionEntry],
) -> io::Result<()> {
    let n: u32 = 128;
    let es = std::mem::size_of::<GptPartitionEntry>() as u32;
    let alba: u64 = 2;
    let mut h = GptHeader::new(total_lbas, alba, n, es);
    h.partition_array_crc32 = crc_parts(partitions, n, es);
    h.header_crc32 = crc_header(&mut h);
    write_primary(w, &h, partitions, n, es, alba)?;
    write_backup(w, &h, partitions, n, es, total_lbas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iso::constants::ESP_START_LBA_512;
    use crate::iso::gpt::partition_entry::EFI_SYSTEM_PARTITION_GUID;
    use std::io::Cursor;
    use std::mem;

    fn read_struct<T: Copy>(s: &[u8], off: usize) -> T {
        unsafe { (s[off..off + mem::size_of::<T>()].as_ptr() as *const T).read_unaligned() }
    }

    #[test]
    fn test_gpt_header_new() {
        let h = GptHeader::new(2048, 2, 128, 128);
        assert_eq!(&h.signature, b"EFI PART");
        assert_eq!({ h.revision }, 0x00010000);
        assert_eq!({ h.current_lba }, 1);
        assert_eq!({ h.backup_lba }, 2047);
        assert_eq!({ h.first_usable_lba }, 34);
    }

    #[test]
    fn test_gpt_partition_entry_new() {
        let e = GptPartitionEntry::new(
            "C12A7328-F81F-11D2-BA4B-00A0C93EC93B",
            "A2A0D0D0-039B-42A0-BA42-A0D0D0D0D0A0",
            ESP_START_LBA_512 as u64,
            2048,
            "EFI System Partition",
            0,
        );
        assert_eq!({ e.starting_lba }, ESP_START_LBA_512 as u64);
        assert_eq!({ e.ending_lba }, 2048);
    }

    #[test]
    fn test_write_gpt() -> io::Result<()> {
        let total = 4096u64;
        let n = 128;
        let es = mem::size_of::<GptPartitionEntry>();
        let mut disk = Cursor::new(vec![0; total as usize * 512usize]);
        let parts = vec![GptPartitionEntry::new(
            EFI_SYSTEM_PARTITION_GUID,
            &"A2A0D0D0-039B-42A0-BA42-A0D0D0D0D0A0",
            2048,
            4095,
            "Test",
            0,
        )];
        write_gpt_structures(&mut disk, total, &parts)?;
        let d = disk.into_inner();

        let ph: GptHeader = read_struct(&d, 512);
        assert_eq!(&ph.signature, b"EFI PART");
        assert_eq!({ ph.header_size } as usize, 92);
        let mut hb = ph.to_bytes();
        hb[16..20].copy_from_slice(&[0; 4]);
        let mut hh = Hasher::new();
        hh.update(&hb[..92]);
        assert_eq!({ ph.header_crc32 }, hh.finalize());

        let arr_offset = 2 * 512;
        let arr_size = n * es;
        let mut hh2 = Hasher::new();
        hh2.update(&d[arr_offset..arr_offset + arr_size]);
        assert_eq!({ ph.partition_array_crc32 }, hh2.finalize());

        let bh: GptHeader = read_struct(&d, (total as usize - 1) * 512);
        assert_eq!(&bh.signature, b"EFI PART");
        assert_eq!({ bh.current_lba }, total - 1);
        assert_eq!({ bh.backup_lba }, 1);

        let arr_sectors = (n as u64 * es as u64).div_ceil(512);
        let b_arr = (total as usize - 1 - arr_sectors as usize) * 512;
        let be: GptPartitionEntry = read_struct(&d, b_arr);
        assert_eq!({ be.starting_lba }, 2048);
        assert_eq!({ be.ending_lba }, 4095);
        Ok(())
    }
}
