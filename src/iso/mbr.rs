// src/iso/mbr.rs

use std::io::{self, Seek, Write};

/// Disk geometry used for CHS calculations.
/// Must match the BPB geometry written in fat.rs (heads=64, spt=32).
const CHS_HEADS: u32 = 64;
const CHS_SPT: u32 = 32;
const CHS_SECTORS_PER_CYLINDER: u32 = CHS_HEADS * CHS_SPT; // 2048

/// Maximum cylinder value for CHS addressing with the configured geometry.
/// When cylinder exceeds this value, CHS is saturated (0xFF, 0xFF, 0xFF).
const CHS_MAX_CYL: u32 = 1023;

/// Encode an LBA into a 3-byte CHS tuple `[head, sector_cyl, cyl_low]`
/// using H=64, S=32 geometry.
///
/// CHS encoding (per MBR / INT 13h):
///   byte 0 = head
///   byte 1 = sector (bits 0–5) | cylinder bits 8–9 (bits 6–7)
///   byte 2 = cylinder bits 0–7
///
/// LBA 0 is a valid address (the MBR itself) and maps to cylinder 0,
/// head 0, sector 1 (INT 13h sectors are 1-indexed).
///
/// When `lba` exceeds the CHS-addressable range for the configured
/// geometry (H=64, SPT=32), all fields are saturated to their
/// maximum values.
fn lba_to_chs(lba: u64) -> [u8; 3] {
    let cylinder = lba / CHS_SECTORS_PER_CYLINDER as u64;

    if cylinder > CHS_MAX_CYL as u64 {
        // LBA beyond CHS addressable range for this geometry
        return [0xFF, 0xFF, 0xFF];
    }

    let cylinder = cylinder as u32;
    let remainder = (lba % CHS_SECTORS_PER_CYLINDER as u64) as u32;
    let head = remainder / CHS_SPT;
    let sector = (remainder % CHS_SPT) + 1;

    let cylinder_hi = ((cylinder >> 8) & 0x03) as u8; // bits 8-9
    let cylinder_lo = (cylinder & 0xFF) as u8;
    let head_byte = head as u8;
    let sector_byte = ((sector as u8) & 0x3F) | (cylinder_hi << 6);

    [head_byte, sector_byte, cylinder_lo]
}

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

/// Creates an MBR with xorriso-compatible hybrid partitions.
/// `total_lbas` is in 512-byte sectors.
///
/// The MBR layout uses:
///   - Partition 1: type 0xEE (GPT Protective), covers LBA 1 to end of disk.
///     This is the standard protective MBR per UEFI spec §5.2.3.
///     Ubuntu/xorriso both use 0xEE; some firmware (InsydeH2O, old AMI)
///     checks for 0xEE to detect GPT layout.
///   - Partition 2: type 0xEF (EFI System Partition), pointing to the ESP.
///     Provides backward compatibility for firmware that reads MBR
///     partitions directly (USB-HDD boot path).
///
/// Overlapping partitions are intentional and standard for hybrid MBR+GPT;
/// the protective entry covers the whole disk while the ESP entry points
/// to the EFI partition within it.
pub fn create_mbr_for_gpt_hybrid(
    total_lbas: u32,
    is_isohybrid: bool,
    esp_start_lba: Option<u32>,
    esp_size_lba: Option<u32>,
) -> io::Result<Mbr> {
    let mut mbr = Mbr::new();

    if is_isohybrid {
        // Protective MBR (type 0xEE) per UEFI spec §5.2.3.
        // Covers the entire disk from LBA 1 (LBA 0 is the MBR itself).
        // This tells UEFI firmware that the disk uses GPT partitioning.
        mbr.partition_table[0].bootable = 0x00;
        mbr.partition_table[0].partition_type = 0xEE; // GPT Protective (standard)
        mbr.partition_table[0].starting_lba = 1;
        mbr.partition_table[0].size_in_lba = total_lbas.saturating_sub(1);
        mbr.partition_table[0].starting_chs = lba_to_chs(1);
        let prot_end_lba = (total_lbas as u64).saturating_sub(1);
        mbr.partition_table[0].ending_chs = lba_to_chs(prot_end_lba);

        // EFI System Partition in MBR entry 1
        // Provides a direct MBR pointer to the ESP for firmware that
        // boots via USB-HDD and reads MBR partitions directly.
        if let (Some(start), Some(size)) = (esp_start_lba, esp_size_lba) {
            if size > 0 {
                let esp_start = start;
                let esp_size = size;
                mbr.partition_table[1].bootable = 0x00;
                mbr.partition_table[1].partition_type = 0xEF; // EFI System Partition
                mbr.partition_table[1].starting_lba = esp_start;
                mbr.partition_table[1].size_in_lba = esp_size;
                mbr.partition_table[1].starting_chs = lba_to_chs(esp_start as u64);
                mbr.partition_table[1].ending_chs = lba_to_chs(
                    (esp_start as u64)
                        .saturating_add(esp_size as u64)
                        .saturating_sub(1),
                );
            }
        }
    } else {
        // Standard MBR for El Torito (if not isohybrid)
        mbr.partition_table[0].bootable = 0x80; // Bootable
        mbr.partition_table[0].partition_type = 0xEF; // EFI System Partition (placeholder)
        mbr.partition_table[0].starting_lba = 1;
        mbr.partition_table[0].size_in_lba = total_lbas.saturating_sub(1);
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

        // Part 1: type 0xEE (GPT Protective), covers disk from LBA 1
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

        // Part 2: EFI System Partition
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
