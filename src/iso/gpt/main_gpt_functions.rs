use crc32fast::Hasher;
use std::io::{self, Seek, SeekFrom, Write};
use std::mem;

use crate::iso::gpt::header::GptHeader;
use crate::iso::gpt::partition_entry::GptPartitionEntry;

/// Calculates the CRC32 for a GPT header.
/// The CRC covers exactly `header.header_size` bytes, per UEFI spec.
fn calculate_header_crc32(header: &mut GptHeader) -> u32 {
    header.header_crc32 = 0; // Zero out CRC field for calculation
    let header_bytes = header.to_bytes();
    let size = header.header_size as usize;
    let header_data_for_crc = &header_bytes[..size];
    let mut hasher = Hasher::new();
    hasher.update(header_data_for_crc);
    hasher.finalize()
}

/// Calculates the CRC32 for the partition entry array.
fn calculate_partition_array_crc32(
    partitions: &[GptPartitionEntry],
    num_partition_entries: u32,
    partition_entry_size: u32,
) -> u32 {
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
    hasher.finalize()
}

/// Writes the primary GPT header and partition array.
fn write_primary_gpt_structures<W: Write + Seek>(
    writer: &mut W,
    header: &GptHeader,
    partitions: &[GptPartitionEntry],
    num_partition_entries: u32,
    partition_entry_size: u32,
    partition_array_lba: u64,
) -> io::Result<()> {
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
    Ok(())
}

/// Writes the backup GPT header and partition array.
fn write_backup_gpt_structures<W: Write + Seek>(
    writer: &mut W,
    header: &GptHeader,
    partitions: &[GptPartitionEntry],
    num_partition_entries: u32,
    partition_entry_size: u32,
    total_lbas: u64,
) -> io::Result<()> {
    // Calculate partition array size in 512-byte sectors.
    // Example: 128 entries * 128 bytes = 16384 bytes → 32 sectors.
    let partition_array_sectors =
        ((num_partition_entries as u64) * (partition_entry_size as u64)).div_ceil(512);

    let mut backup_header = *header;
    backup_header.current_lba = total_lbas - 1;
    backup_header.backup_lba = 1;

    // Backup GPT: backup header at last LBA, partition array ends right before it.
    // partition_entry_lba must point to the start of the backup partition array.
    // backup_array_start = total_lbas - 1 - partition_array_sectors
    backup_header.partition_entry_lba = total_lbas
        .saturating_sub(1)
        .saturating_sub(partition_array_sectors);

    backup_header.header_crc32 = calculate_header_crc32(&mut backup_header);

    // Write backup GPT header at the last LBA.
    writer.seek(SeekFrom::Start((total_lbas - 1) * 512))?;
    backup_header.write_to(writer)?;

    // Write backup partition entries right before the backup header.
    let backup_array_start_byte = (total_lbas - 1 - partition_array_sectors) * 512;
    writer.seek(SeekFrom::Start(backup_array_start_byte))?;
    for partition in partitions {
        partition.write_to(writer)?;
    }
    // Pad remaining backup partition entries with zeros
    for _ in partitions.len()..num_partition_entries as usize {
        writer.write_all(&vec![0u8; partition_entry_size as usize])?;
    }
    Ok(())
}

pub fn write_gpt_structures<W: Write + Seek>(
    writer: &mut W,
    total_lbas: u64,
    partitions: &[GptPartitionEntry],
) -> io::Result<()> {
    let num_partition_entries: u32 = 128; // Standard number of entries
    let partition_entry_size = mem::size_of::<GptPartitionEntry>() as u32;
    let partition_array_lba: u64 = 2; // LBA 2 for partition array

    // Main GPT Header
    let mut header = GptHeader::new(
        total_lbas,
        partition_array_lba,
        num_partition_entries,
        partition_entry_size,
    );

    header.partition_array_crc32 =
        calculate_partition_array_crc32(partitions, num_partition_entries, partition_entry_size);
    header.header_crc32 = calculate_header_crc32(&mut header);

    write_primary_gpt_structures(
        writer,
        &header,
        partitions,
        num_partition_entries,
        partition_entry_size,
        partition_array_lba,
    )?;
    write_backup_gpt_structures(
        writer,
        &header,
        partitions,
        num_partition_entries,
        partition_entry_size,
        total_lbas,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::iso::constants::ESP_START_LBA_512;
    use crate::iso::gpt::partition_entry::EFI_SYSTEM_PARTITION_GUID;

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
        // GPT first_usable_lba = partition_entry_lba + partition_array_sectors
        // = 2 + 32 = 34 (independent of ESP_START_LBA, which is a filesystem-level constant)
        let first_usable = header.first_usable_lba;
        assert_eq!(first_usable, 34);
    }

    #[test]
    fn test_gpt_partition_entry_new() {
        let p_guid = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
        let u_guid = "A2A0D0D0-039B-42A0-BA42-A0D0D0D0D0A0";
        let name = "EFI System Partition";
        let entry = GptPartitionEntry::new(p_guid, u_guid, ESP_START_LBA_512 as u64, 2048, name, 0);

        let starting_lba = entry.starting_lba;
        assert_eq!(starting_lba, ESP_START_LBA_512 as u64);
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

        // Verify Primary Header (read as packed, then copy fields to locals)
        let primary_header: GptHeader = read_struct(&disk_bytes, 512);
        let sig_primary = { primary_header.signature };
        assert_eq!(&sig_primary, b"EFI PART");

        // CRC must cover header_size bytes, per UEFI spec
        let header_size = { primary_header.header_size } as usize;
        assert_eq!(header_size, 92, "GPT header_size must be 92");

        let mut header_bytes = primary_header.to_bytes();
        header_bytes[16..20].copy_from_slice(&[0; 4]);
        let header_data_for_crc = &header_bytes[0..header_size];

        let mut hasher = Hasher::new();
        hasher.update(header_data_for_crc);
        let calculated_crc = hasher.finalize();

        let stored_crc = { primary_header.header_crc32 };
        assert_eq!(stored_crc, calculated_crc, "Primary header CRC32 mismatch");

        // Verify Primary Partition Array position
        let partition_array_sectors =
            ((num_partition_entries as u64) * (partition_entry_size as u64)).div_ceil(512);
        assert_eq!(partition_array_sectors, 32, "128*128/512 = 32 sectors");
        let primary_part_entry_lba = { primary_header.partition_entry_lba };
        assert_eq!(primary_part_entry_lba, 2);

        // Verify Partition Array CRC
        let partition_array_offset = 2 * 512;
        let partition_array_size = num_partition_entries * partition_entry_size;
        let partition_array_bytes =
            &disk_bytes[partition_array_offset..partition_array_offset + partition_array_size];
        let mut hasher = Hasher::new();
        hasher.update(partition_array_bytes);
        let calculated_array_crc = hasher.finalize();
        let stored_array_crc = { primary_header.partition_array_crc32 };
        assert_eq!(
            stored_array_crc, calculated_array_crc,
            "Partition array CRC32 mismatch"
        );

        // Verify Backup Header at last LBA
        let backup_header_offset = ((total_lbas - 1) * 512) as usize;
        let backup_header: GptHeader = read_struct(&disk_bytes, backup_header_offset);
        let sig_backup = { backup_header.signature };
        assert_eq!(&sig_backup, b"EFI PART");
        let backup_current_lba = { backup_header.current_lba };
        assert_eq!(backup_current_lba, total_lbas - 1);
        let backup_backup_lba = { backup_header.backup_lba };
        assert_eq!(backup_backup_lba, 1);

        // Verify Backup Partition Entry array is right before the backup header.
        let expected_backup_array_start_lba = total_lbas - 1 - partition_array_sectors;
        let backup_part_entry_lba = { backup_header.partition_entry_lba };
        assert_eq!(
            backup_part_entry_lba, expected_backup_array_start_lba,
            "Backup partition_entry_lba must point to start of backup partition array"
        );

        // Verify backup partition array content
        let backup_array_byte_offset = (expected_backup_array_start_lba * 512) as usize;
        let backup_entry: GptPartitionEntry = read_struct(&disk_bytes, backup_array_byte_offset);
        let be_starting_lba = { backup_entry.starting_lba };
        let be_ending_lba = { backup_entry.ending_lba };
        assert_eq!(be_starting_lba, 2048);
        assert_eq!(be_ending_lba, 4095);

        Ok(())
    }
}
