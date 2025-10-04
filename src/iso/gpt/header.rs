use std::io::{self, Seek, Write};
use std::mem;
use uuid::Uuid;

use crate::iso::constants::ESP_START_LBA;

// GPT Header structure
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct GptHeader {
    pub signature: [u8; 8], // "EFI PART"
    pub revision: u32,
    pub header_size: u32,
    pub header_crc32: u32,
    _reserved0: u32,
    pub current_lba: u64,
    pub backup_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: [u8; 16],
    pub partition_entry_lba: u64,
    pub num_partition_entries: u32,
    pub partition_entry_size: u32,
    pub partition_array_crc32: u32,
    _reserved1: [u8; 420],
}

impl GptHeader {
    pub fn new(
        total_lbas: u64,
        partition_entry_lba: u64,
        num_partition_entries: u32,
        partition_entry_size: u32,
    ) -> Self {
        let disk_guid_uuid = Uuid::new_v4();
        let disk_guid_bytes = disk_guid_uuid.into_bytes();

        GptHeader {
            signature: *b"EFI PART",
            revision: 0x00010000, // Version 1.0
            header_size: mem::size_of::<GptHeader>() as u32,
            header_crc32: 0, // Calculated later
            _reserved0: 0,
            current_lba: 1, // LBA
            backup_lba: total_lbas - 1,
            first_usable_lba: ESP_START_LBA as u64, // MBR (1) + GPT Header (1) + Partition Array (32)
            last_usable_lba: total_lbas.saturating_sub(ESP_START_LBA as u64), // Last usable LBA before the backup GPT structures (33 sectors)
            disk_guid: disk_guid_bytes,
            partition_entry_lba,
            num_partition_entries,
            partition_entry_size,
            partition_array_crc32: 0, // Calculated later
            _reserved1: [0; 420],
        }
    }

    pub fn to_bytes(&self) -> [u8; mem::size_of::<GptHeader>()] {
        let mut bytes = [0u8; mem::size_of::<GptHeader>()];
        let mut offset = 0;

        bytes[offset..offset + 8].copy_from_slice(&self.signature);
        offset += 8;
        bytes[offset..offset + 4].copy_from_slice(&self.revision.to_le_bytes());
        offset += 4;
        bytes[offset..offset + 4].copy_from_slice(&self.header_size.to_le_bytes());
        offset += 4;
        bytes[offset..offset + 4].copy_from_slice(&self.header_crc32.to_le_bytes());
        offset += 4;
        bytes[offset..offset + 4].copy_from_slice(&self._reserved0.to_le_bytes());
        offset += 4;
        bytes[offset..offset + 8].copy_from_slice(&self.current_lba.to_le_bytes());
        offset += 8;
        bytes[offset..offset + 8].copy_from_slice(&self.backup_lba.to_le_bytes());
        offset += 8;
        bytes[offset..offset + 8].copy_from_slice(&self.first_usable_lba.to_le_bytes());
        offset += 8;
        bytes[offset..offset + 8].copy_from_slice(&self.last_usable_lba.to_le_bytes());
        offset += 8;
        bytes[offset..offset + 16].copy_from_slice(&self.disk_guid);
        offset += 16;
        bytes[offset..offset + 8].copy_from_slice(&self.partition_entry_lba.to_le_bytes());
        offset += 8;
        bytes[offset..offset + 4].copy_from_slice(&self.num_partition_entries.to_le_bytes());
        offset += 4;
        bytes[offset..offset + 4].copy_from_slice(&self.partition_entry_size.to_le_bytes());
        offset += 4;
        bytes[offset..offset + 4].copy_from_slice(&self.partition_array_crc32.to_le_bytes());
        offset += 4;
        bytes[offset..offset + 420].copy_from_slice(&self._reserved1);

        bytes
    }

    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> io::Result<()> {
        let header_bytes = self.to_bytes();
        writer.write_all(&header_bytes)?;
        Ok(())
    }
}
