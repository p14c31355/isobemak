// isobemak/src/iso/boot_catalog.rs
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba};
use std::fs::File;
use std::io::{self, Write};

/// LBA of the boot catalog in the ISO.
pub const LBA_BOOT_CATALOG: u32 = 19;

/// Boot catalog constants
pub const BOOT_CATALOG_HEADER_SIGNATURE: u16 = 0xAA55;
pub const BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID: u8 = 1;
pub const BOOT_CATALOG_BOOT_ENTRY_HEADER_ID: u8 = 0x88;
pub const BOOT_CATALOG_EFI_PLATFORM_ID: u8 = 0xEF;
pub const ID_FIELD_OFFSET: usize = 4;
pub const BOOT_CATALOG_CHECKSUM_OFFSET: usize = 28;

pub struct BootCatalogEntry {
    pub platform_id: u8,
    pub boot_image_lba: u32,
    pub boot_image_sectors: u32,
    pub bootable: bool,
}

/// Writes an El Torito boot catalog.
pub fn write_boot_catalog(iso: &mut File, entries: Vec<BootCatalogEntry>) -> io::Result<()> {
    pad_to_lba(iso, LBA_BOOT_CATALOG)?;

    let mut catalog = [0u8; ISO_SECTOR_SIZE];
    let mut offset = 0;

    // Validation Entry (32 bytes)
    let mut val = [0u8; 32];
    val[0] = BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID;
    val[1] = BOOT_CATALOG_EFI_PLATFORM_ID;
    val[ID_FIELD_OFFSET..ID_FIELD_OFFSET + 24]
        .copy_from_slice(&b"UEFI\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"[..]);

    // No Nsect in Validation Entry (non-standard and corrupts ID string)

    // Set Bootoff (LBA of the boot catalog)
    // LBA_BOOT_CATALOG is u32, but Bootoff field is 2 bytes.
    // This location is immediately overwritten by the checksum calculation later in the function (lines 66-67),
    // because BOOT_CATALOG_CHECKSUM_OFFSET is also defined as 28.
    // If you intend to use a vendor-specific field here for Bootoff, you cannot also have a standard El Torito checksum at the same location.
    // This logic needs to be revisited to either correctly place the Bootoff value or remove this conflicting write.
    // For now, we remove this write to avoid conflict with the checksum.

    // Set Signature
    val[30..32].copy_from_slice(&BOOT_CATALOG_HEADER_SIGNATURE.to_le_bytes());

    // Checksum calculation
    let mut sum: u16 = 0;
    for i in (0..32).step_by(2) {
        sum = sum.wrapping_add(u16::from_le_bytes([val[i], val[i + 1]]));
    }

    // Calculate the checksum such that the sum of all 16 words is 0
    let checksum = 0u16.wrapping_sub(sum);
    val[BOOT_CATALOG_CHECKSUM_OFFSET..BOOT_CATALOG_CHECKSUM_OFFSET + 2]
        .copy_from_slice(&checksum.to_le_bytes());
    catalog[offset..offset + 32].copy_from_slice(&val);
    offset += 32;

    // Boot Entries
    for entry_data in entries {
        let mut entry = [0u8; 32];
        let boot_indicator = if entry_data.bootable {
            if entry_data.platform_id == BOOT_CATALOG_EFI_PLATFORM_ID {
                0x88u8
            } else {
                0x80u8
            }
        } else {
            0x00u8
        };
        entry[0] = boot_indicator;
        entry[1] = 0x00; // No Emulation
        entry[2..4].copy_from_slice(&0u16.to_le_bytes()); // Load segment
        entry[4] = 0x00; // System type (x86)
        // Bytes 5-7: unused (0)
        entry[5..8].copy_from_slice(&[0u8; 3]);
        entry[8..12].copy_from_slice(&entry_data.boot_image_sectors.to_le_bytes()); // Sector count
        entry[12..16].copy_from_slice(&(entry_data.boot_image_lba * 4).to_le_bytes()); // Load RBA (LBA in 512-byte sectors)
        // Bytes 16-31: unused (0), already zeroed
        catalog[offset..offset + 32].copy_from_slice(&entry);
        offset += 32;
    }

    iso.write_all(&catalog)?;

    Ok(())
}
