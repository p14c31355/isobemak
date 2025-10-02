// src/iso/mbr.rs

use std::io::{self, Seek, Write};
use std::mem;

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
        Mbr {
            boot_code: [0; 440],
            disk_signature: 0,
            reserved: 0,
            partition_table: [MbrPartitionEntry::default(); 4],
            boot_signature: 0xAA55,
        }
    }

    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> io::Result<()> {
        let bytes: [u8; mem::size_of::<Mbr>()] = unsafe { mem::transmute(*self) };
        writer.write_all(&bytes)?;
        Ok(())
    }
}

pub fn create_mbr_for_gpt_hybrid(total_lbas: u32, is_isohybrid: bool) -> io::Result<Mbr> {
    let mut mbr = Mbr::new();

    if is_isohybrid {
        // Protective MBR for GPT
        mbr.partition_table[0].bootable = 0x00; // Not bootable
        mbr.partition_table[0].partition_type = 0xEE; // GPT Protective MBR
        mbr.partition_table[0].starting_lba = 1; // Starts after MBR itself
        mbr.partition_table[0].size_in_lba = total_lbas - 1; // Spans rest of the disk
    } else {
        // Standard MBR for El Torito (if not isohybrid)
        // This part might need more specific logic depending on your El Torito implementation
        // For now, a simple bootable partition covering the whole disk (excluding MBR)
        mbr.partition_table[0].bootable = 0x80; // Bootable
        mbr.partition_table[0].partition_type = 0xEF; // EFI System Partition (placeholder, adjust as needed)
        mbr.partition_table[0].starting_lba = 1; // Starts after MBR itself
        mbr.partition_table[0].size_in_lba = total_lbas - 1; // Spans rest of the disk
    }

    Ok(mbr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_mbr_new() {
        let mbr = Mbr::new();
        let boot_sig = mbr.boot_signature;
        assert_eq!(boot_sig, 0xAA55);
        assert_eq!(mbr.boot_code, [0; 440]);
        let disk_sig = mbr.disk_signature;
        assert_eq!(disk_sig, 0);
    }

    #[test]
    fn test_create_mbr_isohybrid() -> io::Result<()> {
        let total_lbas = 1000;
        let mbr = create_mbr_for_gpt_hybrid(total_lbas, true)?;
        let part = mbr.partition_table[0];

        assert_eq!(part.bootable, 0x00);
        assert_eq!(part.partition_type, 0xEE);
        let starting_lba = part.starting_lba;
        assert_eq!(starting_lba, 1);
        let size_in_lba = part.size_in_lba;
        assert_eq!(size_in_lba, total_lbas - 1);
        Ok(())
    }

    #[test]
    fn test_create_mbr_no_isohybrid() -> io::Result<()> {
        let total_lbas = 2000;
        let mbr = create_mbr_for_gpt_hybrid(total_lbas, false)?;
        let part = mbr.partition_table[0];

        assert_eq!(part.bootable, 0x80);
        assert_eq!(part.partition_type, 0xEF);
        let starting_lba = part.starting_lba;
        assert_eq!(starting_lba, 1);
        let size_in_lba = part.size_in_lba;
        assert_eq!(size_in_lba, total_lbas - 1);
        Ok(())
    }

    #[test]
    fn test_mbr_write_to() -> io::Result<()> {
        let mbr = Mbr::new();
        let mut buffer = Cursor::new(Vec::new());
        mbr.write_to(&mut buffer)?;

        let bytes = buffer.into_inner();
        assert_eq!(bytes.len(), mem::size_of::<Mbr>());
        assert_eq!(bytes.len(), 512);

        // Check the boot signature at the end
        let signature = u16::from_le_bytes([bytes[510], bytes[511]]);
        assert_eq!(signature, 0xAA55);
        Ok(())
    }
}
