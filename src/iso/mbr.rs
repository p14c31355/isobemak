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
