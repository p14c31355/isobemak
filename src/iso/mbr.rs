// src/iso/mbr.rs

use std::io::{self, Seek, Write};

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

    /// Serialize the MBR to bytes using explicit little-endian conversions.
    /// This avoids `mem::transmute` which is unsafe and sensitive to
    /// compiler padding / endianness differences.
    pub fn to_bytes(&self) -> [u8; 512] {
        let mut bytes = [0u8; 512];
        let mut offset = 0;

        // Boot code: 440 bytes
        bytes[offset..offset + 440].copy_from_slice(&self.boot_code);
        offset += 440;

        // Disk signature: u32 LE
        bytes[offset..offset + 4].copy_from_slice(&self.disk_signature.to_le_bytes());
        offset += 4;

        // Reserved: u16 LE
        bytes[offset..offset + 2].copy_from_slice(&self.reserved.to_le_bytes());
        offset += 2;

        // 4 partition entries (16 bytes each)
        for entry in &self.partition_table {
            bytes[offset] = entry.bootable;
            bytes[offset + 1..offset + 4].copy_from_slice(&entry.starting_chs);
            bytes[offset + 4] = entry.partition_type;
            bytes[offset + 5..offset + 8].copy_from_slice(&entry.ending_chs);
            bytes[offset + 8..offset + 12].copy_from_slice(&entry.starting_lba.to_le_bytes());
            bytes[offset + 12..offset + 16].copy_from_slice(&entry.size_in_lba.to_le_bytes());
            offset += 16;
        }

        // Boot signature: u16 LE (0xAA55)
        bytes[offset..offset + 2].copy_from_slice(&self.boot_signature.to_le_bytes());

        bytes
    }

    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> io::Result<()> {
        let bytes = self.to_bytes();
        writer.write_all(&bytes)?;
        Ok(())
    }
}

/// Creates an MBR with protective GPT and optional EFI System Partition entries.
/// `total_lbas` is in 512-byte sectors.
/// When `isohybrid` and ESP params are provided, the MBR gets:
///   - Partition 1: type 0xEE (GPT Protective) covering the whole disk
///   - Partition 2: type 0xEF (EFI System Partition) pointing to the ESP
/// Older UEFI firmware (e.g. NEC 2015) often requires the 0xEF entry in MBR.
pub fn create_mbr_for_gpt_hybrid(
    total_lbas: u32,
    is_isohybrid: bool,
    esp_start_lba: Option<u32>,
    esp_size_lba: Option<u32>,
) -> io::Result<Mbr> {
    let mut mbr = Mbr::new();

    if is_isohybrid {
        // Protective GPT MBR layout (matching xorriso behavior).
        // Single entry: type 0xEE (GPT Protective), covers the whole disk.
        // The actual EFI System Partition is defined only in the GPT table.
        // This matches the layout that boots on NEC VersaPro (InsydeH2O, 2015).
        mbr.partition_table[0].bootable = 0x00;
        mbr.partition_table[0].partition_type = 0xEE; // GPT Protective
        mbr.partition_table[0].starting_lba = 1;
        mbr.partition_table[0].size_in_lba = total_lbas
            .saturating_sub(1)
            .min(0xFFFF_FFFF);

        // EFI System Partition in MBR entry 1 (backward compatibility)
        // for firmware that checks MBR before GPT.
        if let (Some(start), Some(size)) = (esp_start_lba, esp_size_lba)
            && size > 0
        {
            mbr.partition_table[1].bootable = 0x00;
            mbr.partition_table[1].partition_type = 0xEF; // EFI System Partition
            mbr.partition_table[1].starting_lba = start;
            mbr.partition_table[1].size_in_lba = (size as u64).min(0xFFFF_FFFF) as u32;
        }
    } else {
        // Standard MBR for El Torito (if not isohybrid)
        mbr.partition_table[0].bootable = 0x80; // Bootable
        mbr.partition_table[0].partition_type = 0xEF; // EFI System Partition (placeholder)
        mbr.partition_table[0].starting_lba = 1;
        mbr.partition_table[0].size_in_lba = total_lbas
            .saturating_sub(1)
            .min(0xFFFF_FFFF);
    }

    Ok(mbr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::mem;

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
        let total_lbas = 1000u32;
        let esp_start = 4096u32;
        let esp_size = 32768u32;
        let mbr = create_mbr_for_gpt_hybrid(total_lbas, true, Some(esp_start), Some(esp_size))?;

        // Part 1: type 0xEE (GPT Protective), covers whole disk from LBA 1
        {
            let bootable = mbr.partition_table[0].bootable;
            let ptype = mbr.partition_table[0].partition_type;
            let start = mbr.partition_table[0].starting_lba;
            let size = mbr.partition_table[0].size_in_lba;
            assert_eq!(bootable, 0x00);
            assert_eq!(ptype, 0xEE);
            assert_eq!(start, 1);
            assert_eq!(size, total_lbas - 1);
        }

        // Part 2: EFI System Partition (bootable=0x00, matches xorriso)
        {
            let bootable = mbr.partition_table[1].bootable;
            let ptype = mbr.partition_table[1].partition_type;
            let start = mbr.partition_table[1].starting_lba;
            let size = mbr.partition_table[1].size_in_lba;
            assert_eq!(bootable, 0x00);
            assert_eq!(ptype, 0xEF);
            assert_eq!(start, esp_start);
            assert_eq!(size, esp_size);
        }
        Ok(())
    }

    #[test]
    fn test_create_mbr_no_isohybrid() -> io::Result<()> {
        let total_lbas = 2000;
        let mbr = create_mbr_for_gpt_hybrid(total_lbas, false, None, None)?;

        let bootable = mbr.partition_table[0].bootable;
        let ptype = mbr.partition_table[0].partition_type;
        let start = mbr.partition_table[0].starting_lba;
        let size = mbr.partition_table[0].size_in_lba;
        assert_eq!(bootable, 0x80);
        assert_eq!(ptype, 0xEF);
        assert_eq!(start, 1);
        assert_eq!(size, total_lbas - 1);
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
