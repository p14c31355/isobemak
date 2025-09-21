// src/gpt.rs

use std::io::{self, Write, Seek, SeekFrom};
use std::fs::File;
use std::mem;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// GPT constants
const GPT_HEADER_SIZE: usize = 92; // Size of the GPT header
const GPT_PARTITION_ENTRY_SIZE: usize = 128; // Size of a single partition entry
const MAX_PARTITIONS: usize = 128; // Maximum number of partitions allowed by GPT
const EFI_SYSTEM_PARTITION_GUID: [u8; 16] = [
    0xC1, 0x2A, 0x73, 0x88, 0x00, 0x00, 0x00, 0x00, // Partition type GUID for EFI System Partition
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Represents the GPT Header.
#[repr(C, packed)]
pub struct GptHeader {
    /// Signature of the GPT header ("if\sre" in ASCII).
    pub signature: [u8; 8],
    /// Size of the GPT header in bytes.
    pub header_size: u32,
    /// CRC32 checksum of the GPT header.
    pub header_crc32: u32,
    /// CRC32 checksum of the partition entry array.
    pub partition_array_crc32: u32,
    /// Location of the primary GPT header (LBA).
    pub primary_header_lba: u64,
    /// Location of the secondary GPT header (LBA).
    pub secondary_header_lba: u64,
    /// Location of the primary partition entry array (LBA).
    pub primary_partition_entry_lba: u64,
    /// Location of the secondary partition entry array (LBA).
    pub secondary_partition_entry_lba: u64,
    /// Starting LBA of the partition entry array.
    pub partition_entry_lba: u64,
    /// Number of partition entries in the array.
    pub partition_count: u32,
    /// Size of each partition entry in bytes.
    pub partition_entry_size: u32,
    /// Sequence number of the last modification.
    pub sequence_number: u32,
    /// Revision number of the GPT header.
    pub revision: u32,
    /// Current LBA of the GPT header.
    pub current_lba: u64,
    /// Starting LBA of the partition table.
    pub partition_table_lba: u64,
    /// Number of partitions in the table.
    pub num_partitions: u32,
    /// Size of each partition entry.
    pub size_partition_entry: u32,
    /// CRC32 of the partition table.
    pub partition_table_crc32: u32,
}

impl GptHeader {
    /// Creates a new GPT header.
    pub fn new(
        total_sectors: u64,
        partition_entry_lba: u64,
        partition_count: u32,
        partition_entry_size: u32,
    ) -> Self {
        let mut header = Self {
            signature: *b"if\0\0re", // "ifre" signature
            header_size: GPT_HEADER_SIZE as u32,
            header_crc32: 0, // Will be calculated later
            partition_array_crc32: 0, // Will be calculated later
            primary_header_lba: 1, // Primary GPT header is at LBA 1
            secondary_header_lba: total_sectors - 1, // Secondary GPT header is at the last LBA
            primary_partition_entry_lba: partition_entry_lba, // Location of the primary partition entry array
            secondary_partition_entry_lba: total_sectors - (MAX_PARTITIONS as u64 * partition_entry_size as u64 / 512) - 1, // Location of the secondary partition entry array
            partition_entry_lba: partition_entry_lba,
            partition_count: partition_count,
            partition_entry_size: partition_entry_size,
            sequence_number: 0, // Can be updated on modification
            revision: 0x00010000, // GPT revision 1.0
            current_lba: 1, // Current LBA of this header
            partition_table_lba: partition_entry_lba,
            num_partitions: partition_count,
            size_partition_entry: partition_entry_size,
            partition_table_crc32: 0, // Will be calculated later
        };
        header
    }

    /// Calculates and sets the CRC32 checksums for the header and partition array.
    pub fn calculate_crc32(&mut self, partition_entries_data: &[u8]) {
        // Calculate header CRC32 (excluding the header_crc32 field itself)
        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                self as *const GptHeader as *const u8,
                GPT_HEADER_SIZE,
            )
        };
        let mut header_crc32_bytes = header_bytes.to_vec();
        // Zero out the header_crc32 field before calculating checksum
        header_crc32_bytes[4..8].copy_from_slice(&[0u8; 4]);
        self.header_crc32 = crc32fast::checksum(header_bytes);

        // Calculate partition array CRC32
        self.partition_array_crc32 = crc32fast::checksum(partition_entries_data);
        self.partition_table_crc32 = self.partition_array_crc32; // For simplicity, use the same CRC for partition_table_crc32
    }

    /// Writes the GPT header to the writer.
    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> io::Result<()> {
        writer.seek(SeekFrom::Start(self.current_lba * 512))?; // Seek to the header's LBA
        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                self as *const GptHeader as *const u8,
                GPT_HEADER_SIZE,
            )
        };
        writer.write_all(header_bytes)?;
        Ok(())
    }
}

/// Represents a GPT Partition Entry.
#[repr(C, packed)]
pub struct GptPartitionEntry {
    /// Partition type GUID.
    pub partition_type_guid: [u8; 16],
    /// Unique GUID for this partition.
    pub unique_partition_guid: [u8; 16],
    /// Starting LBA of the partition.
    pub start_lba: u64,
    /// Ending LBA of the partition.
    pub end_lba: u64,
    /// Partition attributes.
    pub attributes: u64,
    /// Name of the partition (UTF-16 Little Endian).
    pub name: [u16; 36], // 72 bytes for name
}

impl GptPartitionEntry {
    /// Creates a new GPT partition entry.
    pub fn new(
        partition_type_guid: &[u8; 16],
        unique_partition_guid: &[u8; 16],
        start_lba: u64,
        end_lba: u64,
        name: &str,
    ) -> Self {
        let mut partition_name_utf16 = [0u16; 36];
        let mut i = 0;
        for c in name.encode_utf16() {
            if i < 36 {
                partition_name_utf16[i] = c;
                i += 1;
            } else {
                break;
            }
        }

        Self {
            partition_type_guid: *partition_type_guid,
            unique_partition_guid: *unique_partition_guid,
            start_lba,
            end_lba,
            attributes: 0, // Default attributes
            name: partition_name_utf16,
        }
    }

    /// Writes the GPT partition entry to the writer.
    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> io::Result<()> {
        let entry_bytes = unsafe {
            std::slice::from_raw_parts(
                self as *const GptPartitionEntry as *const u8,
                GPT_PARTITION_ENTRY_SIZE,
            )
        };
        writer.write_all(entry_bytes)?;
        Ok(())
    }
}

/// Creates a GPT partition table and writes it to the ISO file.
pub fn create_gpt_partition_table<W: Write + Seek>(
    writer: &mut W,
    total_sectors: u64,
    partition_entries: &[GptPartitionEntry],
) -> io::Result<()> {
    let partition_entry_size = GPT_PARTITION_ENTRY_SIZE as u64;
    let partition_count = partition_entries.len() as u32;
    let partition_array_size = partition_count as u64 * partition_entry_size;

    // Calculate the LBA for the partition entry array.
    // It should be placed after the MBR and before the ISO9660 data.
    // A common placement is after the bootloader code and partition table,
    // leaving space for El Torito. Let's assume it starts at LBA 2.
    // For simplicity, we'll place it right after the MBR (LBA 1).
    // The actual placement needs to be carefully considered for hybrid boot.
    // For now, let's place it at LBA 1, assuming MBR is at LBA 0.
    // The partition entry array itself needs to be aligned to sector size.
    let partition_entry_lba = 1; // Start partition entries at LBA 1

    // Ensure partition array fits within the disk
    if partition_array_size > (total_sectors - partition_entry_lba) * 512 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Partition array is too large for the disk",
        ));
    }

    // Create GPT Header
    let mut header = GptHeader::new(
        total_sectors,
        partition_entry_lba,
        partition_count,
        GPT_PARTITION_ENTRY_SIZE as u32,
    );

    // Prepare partition entries data for CRC calculation
    let mut partition_entries_data = Vec::with_capacity(partition_array_size as usize);
    for entry in partition_entries {
        let entry_bytes = unsafe {
            std::slice::from_raw_parts(
                entry as *const GptPartitionEntry as *const u8,
                GPT_PARTITION_ENTRY_SIZE,
            )
        };
        partition_entries_data.extend_from_slice(entry_bytes);
    }

    // Calculate CRC32 for header and partition array
    header.calculate_crc32(&partition_entries_data);

    // Write primary GPT Header
    header.write_to(writer)?;

    // Write partition entries
    writer.seek(SeekFrom::Start(partition_entry_lba * 512))?;
    writer.write_all(&partition_entries_data)?;

    // Write secondary GPT Header (at the end of the disk)
    // We need to create a copy of the header and update its LBA fields
    let mut secondary_header = header;
    secondary_header.current_lba = total_sectors - 1;
    secondary_header.primary_header_lba = total_sectors - 1;
    secondary_header.secondary_header_lba = 1;
    secondary_header.primary_partition_entry_lba = total_sectors - (MAX_PARTITIONS as u64 * partition_entry_size as u64 / 512) - 1;
    secondary_header.secondary_partition_entry_lba = partition_entry_lba;
    secondary_header.partition_entry_lba = secondary_header.secondary_partition_entry_lba;
    secondary_header.partition_table_lba = secondary_header.secondary_partition_entry_lba;

    // Recalculate CRC32 for the secondary header (partition array CRC remains the same)
    secondary_header.calculate_crc32(&partition_entries_data);

    writer.seek(SeekFrom::Start(secondary_header.current_lba * 512))?;
    let secondary_header_bytes = unsafe {
        std::slice::from_raw_parts(
            &secondary_header as *const GptHeader as *const u8,
            GPT_HEADER_SIZE,
        )
    };
    writer.write_all(secondary_header_bytes)?;

    Ok(())
}

/// Generates a random GUID.
pub fn generate_guid() -> [u8; 16] {
    Uuid::new_v4().into_bytes()
}

/// Creates an MBR for a GPT hybrid setup.
/// If `is_isohybrid` is true, it creates an MBR with a bootable partition of type 0xEF (EFI System Partition).
/// Otherwise, it creates a protective MBR with a partition of type 0xEE (GPT Protective MBR).
pub fn create_mbr_for_gpt_hybrid(total_lbas: u32, is_isohybrid: bool) -> crate::iso::mbr::Mbr {
    let mut mbr = crate::iso::mbr::Mbr::new();
    if is_isohybrid {
        // isohybrid MBR: first partition is bootable, type 0xEF (EFI System Partition)
        mbr.partitions[0].bootable = 0x80; // Bootable
        mbr.partitions[0].partition_type = 0xEF; // EFI System Partition
        mbr.partitions[0].starting_lba = 1; // Starts after MBR itself
        mbr.partitions[0].size_in_lba = total_lbas - 1; // Spans rest of the disk
    } else {
        // Protective MBR: first partition is type 0xEE (GPT Protective MBR)
        mbr.partitions[0].bootable = 0x00; // Not bootable
        mbr.partitions[0].partition_type = 0xEE; // GPT Protective MBR
        mbr.partitions[0].starting_lba = 1; // Starts after MBR itself
        mbr.partitions[0].size_in_lba = total_lbas - 1; // Spans rest of the disk
    }
    mbr
}
