use std::io::{self, Seek, Write};

const H: u32 = 64;
const SPT: u32 = 32;
const SPC: u32 = H * SPT;
const MAX_CYL: u32 = 1023;

fn lba_to_chs(lba: u64) -> [u8; 3] {
    let cyl = lba / SPC as u64;
    if cyl > MAX_CYL as u64 {
        return [0xFF; 3];
    }
    let cyl = cyl as u32;
    let rem = (lba % SPC as u64) as u32;
    let head = rem / SPT;
    let sector = (rem % SPT) + 1;
    let cyl_hi = ((cyl >> 8) & 0x03) as u8;
    [
        head as u8,
        ((sector as u8) & 0x3F) | (cyl_hi << 6),
        (cyl & 0xFF) as u8,
    ]
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MbrPartitionEntry {
    pub bootable: u8,
    pub starting_chs: [u8; 3],
    pub partition_type: u8,
    pub ending_chs: [u8; 3],
    pub starting_lba: u32,
    pub size_in_lba: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct Mbr {
    pub boot_code: [u8; 440],
    pub disk_signature: u32,
    pub reserved: u16,
    pub partition_table: [MbrPartitionEntry; 4],
    pub boot_signature: u16,
}

impl Default for Mbr {
    fn default() -> Self {
        Self::new()
    }
}

impl Mbr {
    pub fn new() -> Self {
        Self {
            boot_code: [0; 440],
            disk_signature: 0,
            reserved: 0,
            partition_table: Default::default(),
            boot_signature: 0xAA55,
        }
    }

    pub fn to_bytes(&self) -> [u8; 512] {
        let mut b = [0u8; 512];
        b[..440].copy_from_slice(&self.boot_code);
        b[440..444].copy_from_slice(&self.disk_signature.to_le_bytes());
        b[444..446].copy_from_slice(&self.reserved.to_le_bytes());
        let mut off = 446;
        for e in &self.partition_table {
            b[off] = e.bootable;
            b[off + 1..off + 4].copy_from_slice(&e.starting_chs);
            b[off + 4] = e.partition_type;
            b[off + 5..off + 8].copy_from_slice(&e.ending_chs);
            b[off + 8..off + 12].copy_from_slice(&e.starting_lba.to_le_bytes());
            b[off + 12..off + 16].copy_from_slice(&e.size_in_lba.to_le_bytes());
            off += 16;
        }
        b[510..512].copy_from_slice(&self.boot_signature.to_le_bytes());
        b
    }

    pub fn write_to<W: Write + Seek>(&self, w: &mut W) -> io::Result<()> {
        w.write_all(&self.to_bytes())
    }
}

fn set_part(pe: &mut MbrPartitionEntry, bootable: u8, ptype: u8, start_lba: u32, size_lba: u32) {
    pe.bootable = bootable;
    pe.partition_type = ptype;
    pe.starting_lba = start_lba;
    pe.size_in_lba = size_lba;
    pe.starting_chs = lba_to_chs(start_lba as u64);
    pe.ending_chs = lba_to_chs(start_lba as u64 + size_lba as u64 - 1);
}

pub fn create_mbr_for_gpt_hybrid(
    total_lbas: u32,
    is_isohybrid: bool,
    esp_start: Option<u32>,
    esp_size: Option<u32>,
) -> io::Result<Mbr> {
    let mut mbr = Mbr::new();
    if is_isohybrid {
        set_part(
            &mut mbr.partition_table[0],
            0,
            0xEE,
            1,
            total_lbas.saturating_sub(1),
        );
        if let (Some(s), Some(sz)) = (esp_start, esp_size)
            && sz > 0
        {
            set_part(&mut mbr.partition_table[1], 0, 0xEF, s, sz);
        }
    } else {
        set_part(
            &mut mbr.partition_table[0],
            0x80,
            0xEF,
            1,
            total_lbas.saturating_sub(1),
        );
    }
    Ok(mbr)
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::mem;

    #[test]
    fn test_new() {
        let mbr = Mbr::new();
        assert_eq!({ mbr.boot_signature }, 0xAA55);
        assert_eq!(mbr.boot_code, [0; 440]);
    }

    #[test]
    fn test_isohybrid() -> io::Result<()> {
        let mbr = create_mbr_for_gpt_hybrid(1000, true, Some(4096), Some(32768))?;
        let p0 = &mbr.partition_table[0];
        assert_eq!({ p0.partition_type }, 0xEE);
        assert_eq!({ p0.starting_lba }, 1);
        assert_eq!({ p0.size_in_lba }, 999);
        let p1 = &mbr.partition_table[1];
        assert_eq!({ p1.partition_type }, 0xEF);
        assert_eq!({ p1.starting_lba }, 4096);
        assert_eq!({ p1.size_in_lba }, 32768);
        Ok(())
    }

    #[test]
    fn test_no_isohybrid() -> io::Result<()> {
        let mbr = create_mbr_for_gpt_hybrid(2000, false, None, None)?;
        let p0 = &mbr.partition_table[0];
        assert_eq!({ p0.bootable }, 0x80);
        assert_eq!({ p0.partition_type }, 0xEF);
        assert_eq!({ p0.starting_lba }, 1);
        assert_eq!({ p0.size_in_lba }, 1999);
        Ok(())
    }

    #[test]
    fn test_write() -> io::Result<()> {
        let mbr = Mbr::new();
        let mut c = Cursor::new(Vec::new());
        mbr.write_to(&mut c)?;
        let b = c.into_inner();
        assert_eq!(b.len(), mem::size_of::<Mbr>());
        assert_eq!(b.len(), 512);
        assert_eq!(u16::from_le_bytes([b[510], b[511]]), 0xAA55);
        Ok(())
    }
}
