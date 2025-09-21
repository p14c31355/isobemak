// src/iso/mbr.rs

use std::io::{self, Write, Seek, SeekFrom};
use std::fs::File;

// MBR constants
const MBR_SIZE: usize = 512;
const BOOT_CODE_SIZE: usize = 440;
const PARTITION_TABLE_OFFSET: usize = 446;
const BOOT_SIGNATURE_OFFSET: usize = 510;
const BOOT_SIGNATURE: [u8; 2] = [0x55, 0xAA]; // Little-endian

// Partition entry structure (16 bytes)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct PartitionEntry {
    /// Status of the partition (e.g., 0x80 for active)
    pub status: u8,
    /// Starting sector of the partition (CHS addressing)
    pub start_chs: [u8; 3],
    /// Partition type (e.g., 0x07 for HPFS/NTFS/exFAT, 0x0C for FAT32 LBA)
    pub partition_type: u8,
    /// Ending sector of the partition (CHS addressing)
    pub end_chs: [u8; 3],
    /// Starting LBA of the partition (32-bit)
    pub start_lba: u32,
    /// Number of sectors in the partition (32-bit)
    pub sector_count: u32,
}

impl Default for PartitionEntry {
    fn default() -> Self {
        Self {
            status: 0,
            start_chs: [0; 3],
            partition_type: 0,
            end_chs: [0; 3],
            start_lba: 0,
            sector_count: 0,
        }
    }
}

/// Represents the Master Boot Record (MBR).
#[derive(Debug, Clone)] // Removed Default here as we'll implement it manually
pub struct Mbr {
    /// The boot code (e.g., bootloader).
    pub boot_code: [u8; BOOT_CODE_SIZE],
    /// The disk signature.
    pub disk_signature: u32,
    /// Reserved bytes.
    pub reserved: u16,
    /// The partition table entries.
    pub partition_table: [PartitionEntry; 4],
    /// The boot signature (0xAA55).
    pub boot_signature: [u8; 2],
}

// Manual implementation of Default for Mbr to satisfy the trait bound.
impl Default for Mbr {
    fn default() -> Self {
        Self {
            boot_code: [0u8; BOOT_CODE_SIZE], // Explicitly initialize boot_code with zeros
            disk_signature: 0,
            reserved: 0,
            partition_table: [PartitionEntry::default(); 4],
            boot_signature: BOOT_SIGNATURE,
        }
    }
}

impl Mbr {
    /// Creates a new MBR with default values.
    pub fn new() -> Self {
        // Use the default implementation for Mbr
        Mbr::default()
    }

    /// Writes the MBR to the given file at the current position.
    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> io::Result<()> {
        // Ensure we are at the beginning of the MBR area (LBA 0)
        writer.seek(SeekFrom::Start(0))?;

        // Write boot code
        writer.write_all(&self.boot_code)?;

        // Write disk signature and reserved bytes
        writer.write_all(&self.disk_signature.to_le_bytes())?;
        writer.write_all(&self.reserved.to_le_bytes())?;

        // Write partition table entries
        for entry in &self.partition_table {
            let entry_bytes = unsafe {
                std::slice::from_raw_parts(
                    entry as *const PartitionEntry as *const u8,
                    std::mem::size_of::<PartitionEntry>(),
                )
            };
            writer.write_all(entry_bytes)?;
        }

        // Write boot signature
        writer.write_all(&self.boot_signature)?;

        // Ensure the total size is 512 bytes.
        // If any part was written incorrectly, this might fail or write extra data.
        // For simplicity, we assume the above writes fill exactly 512 bytes.
        // A more robust implementation would check the current position.
        let current_pos = writer.stream_position()?;
        if current_pos != MBR_SIZE as u64 {
            // This should ideally not happen if all fields are written correctly.
            // If it does, it indicates a problem with the MBR structure or writing logic.
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("MBR write error: expected {} bytes, wrote {}", MBR_SIZE, current_pos),
            ));
        }

        Ok(())
    }
}

// Placeholder for boot code. In a real scenario, this would be actual bootloader code.
// For now, we'll just use zeros.
pub fn get_default_boot_code() -> [u8; BOOT_CODE_SIZE] {
    [0u8; BOOT_CODE_SIZE]
}
