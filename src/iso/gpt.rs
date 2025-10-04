use crc32fast::Hasher;
use std::io::{self, Seek, SeekFrom, Write};
use std::mem;
use uuid::Uuid;

use crate::iso::ESP_START_LBA;

pub const EFI_SYSTEM_PARTITION_GUID: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";

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
            last_usable_lba: total_lbas.saturating_sub(ESP_START_LBA as u64), // total_lbas - 1 (backup header) - 32 (backup partition array) - 1 (current header)
            disk_guid: disk_guid_uuid.into_bytes(),
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

pub fn write_gpt_structures<W: Write + Seek>(
    writer: &mut W,
    total_lbas: u64,
    partitions: &[GptPartitionEntry],
) -> io::Result<()> {
    let num_partition_entries = 128; // Standard number of entries
    let partition_entry_size = mem::size_of::<GptPartitionEntry>() as u32;
    let partition_array_lba = 2; // LBA 2 for partition array

    // Main GPT Header
    let mut header = GptHeader::new(
        total_lbas,
        partition_array_lba,
        num_partition_entries,
        partition_entry_size,
    );

    // Calculate partition array CRC32
    let mut partition_array_bytes =
        vec![0u8; (num_partition_entries * partition_entry_size) as usize];
    let mut offset = 0;
    for partition in partitions {
        let partition_slice = partition.to_bytes();
        partition_array_bytes[offset..offset + mem::size_of::<GptPartitionEntry>()]
            .copy_from_slice(&partition_slice);
        offset += mem::size_of::<GptPartitionEntry>();
    }
    let mut hasher = Hasher::new();
    hasher.update(&partition_array_bytes);
    header.partition_array_crc32 = hasher.finalize();

    // Recalculate header CRC32. The CRC is calculated over the first 92 bytes of the header.
    let mut header_copy = header;
    header_copy.header_crc32 = 0;
    let header_bytes = header_copy.to_bytes();
    let header_data_for_crc = &header_bytes[0..92];
    let mut hasher = Hasher::new();
    hasher.update(header_data_for_crc);
    header.header_crc32 = hasher.finalize();

    // Write Main GPT Header
    writer.seek(SeekFrom::Start(512))?; // LBA 1
    header.write_to(writer)?;

    // Write Partition Entries
    writer.seek(SeekFrom::Start(partition_array_lba * 512))?; // LBA 2
    writer.write_all(&partition_array_bytes)?;

    // Backup GPT Header
    let mut backup_header = header;
    backup_header.current_lba = total_lbas - 1;
    backup_header.backup_lba = 1;
    backup_header.partition_entry_lba = total_lbas
        .saturating_sub(num_partition_entries as u64)
        .saturating_sub(1); // Backup partition array LBA

    // Recalculate backup header CRC32. The CRC is calculated over the first 92 bytes of the header.
    let mut backup_header_copy = backup_header;
    backup_header_copy.header_crc32 = 0;
    let backup_header_bytes = backup_header_copy.to_bytes();
    let header_data_for_crc = &backup_header_bytes[0..92];
    let mut hasher = Hasher::new();
    hasher.update(header_data_for_crc);
    backup_header.header_crc32 = hasher.finalize();

    writer.seek(SeekFrom::Start(total_lbas.saturating_sub(1) * 512))?; // Last LBA
    backup_header.write_to(writer)?;

    // Backup Partition Entries
    writer.seek(SeekFrom::Start(
        total_lbas
            .saturating_sub(num_partition_entries as u64)
            .saturating_sub(1)
            * 512,
    ))?;
    for partition in partitions {
        let partition_slice = partition.to_bytes();
        writer.write_all(&partition_slice)?;
    }
    // Pad remaining backup partition entries with zeros
    for _ in partitions.len()..num_partition_entries as usize {
        writer.write_all(&vec![0u8; partition_entry_size as usize])?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // Helper to read a packed struct safely from a byte slice.
    fn read_struct<T: Copy>(slice: &[u8], offset: usize) -> T {
        let size = mem::size_of::<T>();
        let sub_slice = &slice[offset..offset + size];
        // Using `read_unaligned` is safer and more direct for reading from a byte
        // slice that may not have the alignment required for type `T`.
        unsafe { (sub_slice.as_ptr() as *const T).read_unaligned() }
    }

    #[test]
    fn test_gpt_header_new() {
        let total_lbas = 2048;
        let header = GptHeader::new(total_lbas, 2, 128, 128);

        assert_eq!(&header.signature, b"EFI PART");
        let revision = header.revision;
        assert_eq!(revision, 0x00010000);
        let current_lba = header.current_lba;
        assert_eq!(current_lba, 1);
        let backup_lba = header.backup_lba;
        assert_eq!(backup_lba, total_lbas - 1);
        let first_usable = header.first_usable_lba;
        assert_eq!(first_usable, ESP_START_LBA as u64);
    }

    #[test]
    fn test_gpt_partition_entry_new() {
        let p_guid = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
        let u_guid = "A2A0D0D0-039B-42A0-BA42-A0D0D0D0D0A0";
        let name = "EFI System Partition";
        let entry = GptPartitionEntry::new(p_guid, u_guid, ESP_START_LBA as u64, 2048, name, 0);

        let starting_lba = entry.starting_lba;
        assert_eq!(starting_lba, ESP_START_LBA as u64);
        let ending_lba = entry.ending_lba;
        assert_eq!(ending_lba, 2048);

        let mut expected_name = [0u16; 36];
        for (i, c) in name.encode_utf16().enumerate() {
            expected_name[i] = c;
        }
        let partition_name = entry.partition_name;
        assert_eq!(partition_name, expected_name);
    }

    #[test]
    fn test_write_gpt_structures() -> io::Result<()> {
        let total_lbas = 4096;
        let num_partition_entries = 128;
        let partition_entry_size = mem::size_of::<GptPartitionEntry>();
        let mut disk = Cursor::new(vec![0; total_lbas as usize * 512]);

        let partitions = vec![GptPartitionEntry::new(
            EFI_SYSTEM_PARTITION_GUID,
            &"A2A0D0D0-039B-42A0-BA42-A0D0D0D0D0A0".to_string(),
            2048,
            4095,
            "Test Partition",
            0,
        )];

        write_gpt_structures(&mut disk, total_lbas, &partitions)?;

        let disk_bytes = disk.into_inner();

        // Verify Primary Header
        let primary_header: GptHeader = read_struct(&disk_bytes, 512);
        assert_eq!(&primary_header.signature, b"EFI PART");

        // Get the 92 bytes of the header for CRC calculation
        let mut header_bytes = primary_header.to_bytes();
        header_bytes[16..20].copy_from_slice(&[0; 4]); // Zero out CRC field for calculation
        let header_data_for_crc = &header_bytes[0..92];

        let mut hasher = Hasher::new();
        hasher.update(header_data_for_crc);
        let calculated_crc = hasher.finalize();

        let stored_crc = primary_header.header_crc32; // Read directly from the struct
        assert_eq!(stored_crc, calculated_crc, "Primary header CRC32 mismatch");

        // Verify Partition Array
        let partition_array_offset = 2 * 512;
        let partition_array_size = num_partition_entries * partition_entry_size;
        let partition_array_bytes =
            &disk_bytes[partition_array_offset..partition_array_offset + partition_array_size];
        let mut hasher = Hasher::new();
        hasher.update(partition_array_bytes);
        let calculated_array_crc = hasher.finalize();
        let stored_array_crc = primary_header.partition_array_crc32;
        assert_eq!(
            stored_array_crc, calculated_array_crc,
            "Partition array CRC32 mismatch"
        );

        // Verify Backup Header
        let backup_header_offset = ((total_lbas - 1) * 512) as usize;
        let backup_header: GptHeader = read_struct(&disk_bytes, backup_header_offset);
        assert_eq!(&backup_header.signature, b"EFI PART");
        let backup_current_lba = backup_header.current_lba;
        assert_eq!(backup_current_lba, total_lbas - 1);
        let backup_backup_lba = backup_header.backup_lba;
        assert_eq!(backup_backup_lba, 1);

        Ok(())
    }
}
