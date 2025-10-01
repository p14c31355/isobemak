use crc32fast::Hasher;
use std::io::{self, Seek, SeekFrom, Write};
use std::mem;
use uuid::Uuid;

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
            current_lba: 1, // LBA 1
            backup_lba: total_lbas - 1,
            first_usable_lba: 34, // MBR (1) + GPT Header (1) + Partition Array (32)
            last_usable_lba: total_lbas.saturating_sub(34), // total_lbas - 1 (backup header) - 32 (backup partition array) - 1 (current header)
            disk_guid: disk_guid_bytes,
            partition_entry_lba,
            num_partition_entries,
            partition_entry_size,
            partition_array_crc32: 0, // Calculated later
            _reserved1: [0; 420],
        }
    }

    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> io::Result<()> {
        let bytes: [u8; mem::size_of::<GptHeader>()] = unsafe { mem::transmute(*self) };
        writer.write_all(&bytes)?;
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
            attributes: 0,
            partition_name: name_bytes,
        }
    }

    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> io::Result<()> {
        let bytes: [u8; mem::size_of::<GptPartitionEntry>()] = unsafe { mem::transmute(*self) };
        writer.write_all(&bytes)?;
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
        let bytes: [u8; mem::size_of::<GptPartitionEntry>()] =
            unsafe { mem::transmute(*partition) };
        partition_array_bytes[offset..offset + mem::size_of::<GptPartitionEntry>()]
            .copy_from_slice(&bytes);
        offset += mem::size_of::<GptPartitionEntry>();
    }
    let mut hasher = Hasher::new();
    hasher.update(&partition_array_bytes);
    header.partition_array_crc32 = hasher.finalize();

    // Recalculate header CRC32 with partition array CRC
    let mut header_bytes: [u8; mem::size_of::<GptHeader>()] = unsafe { mem::transmute(header) };
    header_bytes[16..20].copy_from_slice(&[0; 4]); // Zero out header_crc32 field for calculation

    let mut hasher = Hasher::new();
    hasher.update(&header_bytes);
    header.header_crc32 = hasher.finalize();

    // Write Main GPT Header
    writer.seek(SeekFrom::Start(512))?; // LBA 1
    header.write_to(writer)?;

    // Write Partition Entries
    writer.seek(SeekFrom::Start(partition_array_lba * 512))?; // LBA 2
    for partition in partitions {
        partition.write_to(writer)?;
    }
    // Pad remaining partition entries with zeros
    for _ in partitions.len()..num_partition_entries as usize {
        writer.write_all(&vec![0u8; partition_entry_size as usize])?;
    }

    // Backup GPT Header
    let mut backup_header = header;
    backup_header.current_lba = total_lbas - 1;
    backup_header.backup_lba = 1;
    backup_header.partition_entry_lba = total_lbas
        .saturating_sub(num_partition_entries as u64)
        .saturating_sub(1); // Backup partition array LBA

    // Recalculate backup header CRC32
    let mut backup_header_bytes: [u8; mem::size_of::<GptHeader>()] =
        unsafe { mem::transmute(backup_header) };
    backup_header_bytes[16..20].copy_from_slice(&[0; 4]); // Zero out header_crc32 field for calculation

    let mut hasher = Hasher::new();
    hasher.update(&backup_header_bytes);
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
        partition.write_to(writer)?;
    }
    // Pad remaining backup partition entries with zeros
    for _ in partitions.len()..num_partition_entries as usize {
        writer.write_all(&vec![0u8; partition_entry_size as usize])?;
    }

    Ok(())
}
