// src/iso/mbr.rs

use std::io::{self, Seek, Write};

/// Disk geometry used for CHS calculations.
/// Must match the BPB geometry written in fat.rs (heads=64, spt=32).
const CHS_HEADS: u32 = 64;
const CHS_SPT: u32 = 32;
const CHS_SECTORS_PER_CYLINDER: u32 = CHS_HEADS * CHS_SPT; // 2048

/// Maximum values for CHS fields (when LBA exceeds CHS addressable range).
const CHS_MAX_CYL: u32 = 1023;
const CHS_MAX_HEAD: u32 = 254; // heads are 0-indexed, max valid is 255
const CHS_MAX_SECTOR: u32 = 63;

/// Encode an LBA into a 3-byte CHS tuple `[head, sector_cyl, cyl_low]`
/// using H=64, S=32 geometry.
///
/// CHS encoding (per MBR / INT 13h):
///   byte 0 = head
///   byte 1 = sector (bits 0–5) | cylinder bits 8–9 (bits 6–7)
///   byte 2 = cylinder bits 0–7
///
/// When `lba` exceeds the CHS-addressable range, all fields are saturated
/// to their maximum values.
fn lba_to_chs(lba: u64) -> [u8; 3] {
    let max_lba = (CHS_MAX_CYL as u64 + 1) * (CHS_MAX_HEAD as u64 + 1) * CHS_MAX_SECTOR as u64;

    if lba == 0 || lba >= max_lba {
        // LBA 0 or beyond CHS range → saturate
        return [
            (CHS_MAX_HEAD + 1) as u8, // head = 255
            0xFF,                     // sector 63 | cylinder bits 8-9 = 11b → 0xFF
            0xFF,                     // cylinder bits 0-7 = 0xFF
        ];
    }

    let cylinder = (lba / CHS_SECTORS_PER_CYLINDER as u64) as u32;
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
/// When `isohybrid` and ESP params are provided, the MBR gets:
///   - Partition 1: type 0x83 (Linux native) covering the whole disk
///   - Partition 2: type 0xEF (EFI System Partition) pointing to the ESP
/// Older UEFI firmware (e.g. NEC 2015) requires the 0x83+0xEF pattern
/// for USB-HDD boot compatibility, as 0xEE (GPT Protective) causes some
/// firmware to skip MBR parsing entirely.
pub fn create_mbr_for_gpt_hybrid(
    total_lbas: u32,
    is_isohybrid: bool,
    esp_start_lba: Option<u32>,
    esp_size_lba: Option<u32>,
) -> io::Result<Mbr> {
    let mut mbr = Mbr::new();

    if is_isohybrid {
        // xorriso-compatible hybrid MBR layout for real hardware UEFI boot.
        //
        // Two partitions:
        //   Entry 0: type 0x83 (Linux native), covers the entire disk from LBA 0.
        //   Entry 1: type 0xEF (EFI System Partition), points to the ESP.
        //
        // This dual-entry pattern is critical for real hardware (NEC/Insyde/old AMI)
        // because firmware that boots in USB-HDD mode sees the 0xEF partition
        // directly in MBR and loads the bootloader from the ESP without needing
        // GPT parsing.  xorriso uses this exact layout with type 0x83 + 0xEF,
        // and changing type 0xEE→0x83 fixes "No bootfile found for UEFI!" on
        // real hardware (e.g. InsydeH2O, Lenovo, Panasonic).
        //
        // When GPT is also present, this provides a dual fallback path:
        // GPT-aware firmware uses GPT, MBR-only firmware falls back to 0xEF.
        mbr.partition_table[0].bootable = 0x00;
        mbr.partition_table[0].partition_type = 0x83; // Linux native (xorriso-compatible)
        mbr.partition_table[0].starting_lba = 0;
        mbr.partition_table[0].size_in_lba = total_lbas.min(0xFFFF_FFFF);
        // Populate CHS fields — InsydeH2O and older AMI firmware verify
        // CHS/LBA consistency and ignore the MBR when CHS is all-zero.
        mbr.partition_table[0].starting_chs = lba_to_chs(0);
        let linux_end_lba = (total_lbas as u64).saturating_sub(1);
        mbr.partition_table[0].ending_chs = lba_to_chs(linux_end_lba);

        // EFI System Partition in MBR entry 1 (backward compatibility)
        // for firmware that checks MBR before GPT.
        if let (Some(start), Some(size)) = (esp_start_lba, esp_size_lba)
            && size > 0
        {
            let esp_start = start;
            let esp_size = (size as u64).min(0xFFFF_FFFF) as u32;
            mbr.partition_table[1].bootable = 0x00;
            mbr.partition_table[1].partition_type = 0xEF; // EFI System Partition
            mbr.partition_table[1].starting_lba = esp_start;
            mbr.partition_table[1].size_in_lba = esp_size;
            mbr.partition_table[1].starting_chs = lba_to_chs(esp_start as u64);
            mbr.partition_table[1].ending_chs = lba_to_chs(
                (esp_start as u64).saturating_add(esp_size as u64).saturating_sub(1),
            );
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

        // Part 1: type 0x83 (Linux native), covers whole disk from LBA 0
        {
            let bootable = mbr.partition_table[0].bootable;
            let ptype = mbr.partition_table[0].partition_type;
            let start = mbr.partition_table[0].starting_lba;
            let size = mbr.partition_table[0].size_in_lba;
            assert_eq!(bootable, 0x00);
            assert_eq!(ptype, 0x83);
            assert_eq!(start, 0);
            assert_eq!(size, total_lbas);
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
