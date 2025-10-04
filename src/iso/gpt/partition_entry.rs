use std::io::{self, Seek, Write};
use std::mem;
use uuid::Uuid;

pub const EFI_SYSTEM_PARTITION_GUID: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";

// GPT Partition Entry structure
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct GptPartitionEntry {
    pub partition_type_guid: [u8; 16],
    pub unique_partition_guid: [u8; 16],
    pub starting_lba: u64,
    pub ending_lba: u64,
    pub attributes: u64,
    pub partition_name: [u16; 36], // UTF-16LE
}

impl GptPartitionEntry {
    pub fn new(
        partition_type_guid: &str,
        unique_partition_guid: &str,
        starting_lba: u64,
        ending_lba: u64,
        partition_name: &str,
        attributes: u64,
    ) -> Self {
        let partition_type_guid_bytes = Uuid::parse_str(partition_type_guid)
            .expect("Failed to parse partition type GUID")
            .into_bytes();
        let unique_partition_guid_bytes = Uuid::parse_str(unique_partition_guid)
            .expect("Failed to parse unique partition GUID")
            .into_bytes();

        let mut name_bytes = [0u16; 36];
        for (i, c) in partition_name.encode_utf16().take(36).enumerate() {
            name_bytes[i] = c;
        }

        GptPartitionEntry {
            partition_type_guid: partition_type_guid_bytes,
            unique_partition_guid: unique_partition_guid_bytes,
            starting_lba,
            ending_lba,
            attributes,
            partition_name: name_bytes,
        }
    }

    pub fn to_bytes(&self) -> [u8; mem::size_of::<GptPartitionEntry>()] {
        let mut bytes = [0u8; mem::size_of::<GptPartitionEntry>()];
        let mut offset = 0;

        bytes[offset..offset + 16].copy_from_slice(&self.partition_type_guid);
        offset += 16;
        bytes[offset..offset + 16].copy_from_slice(&self.unique_partition_guid);
        offset += 16;
        bytes[offset..offset + 8].copy_from_slice(&self.starting_lba.to_le_bytes());
        offset += 8;
        bytes[offset..offset + 8].copy_from_slice(&self.ending_lba.to_le_bytes());
        offset += 8;
        bytes[offset..offset + 8].copy_from_slice(&self.attributes.to_le_bytes());
        offset += 8;
        for i in 0..36 {
            bytes[offset..offset + 2].copy_from_slice(&self.partition_name[i].to_le_bytes());
            offset += 2;
        }

        bytes
    }

    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> io::Result<()> {
        let partition_bytes = self.to_bytes();
        writer.write_all(&partition_bytes)?;
        Ok(())
    }
}
