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
    pub boot_image_sectors: u16,
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

    // Find the first bootable entry to get its sector count for the default boot header Nsect.
    // Nsect in the default header is a single byte and can only represent up to 255 sectors.
    // If the calculated sectors exceed 255, we cap it at 255.
    // If sectors is 0 (e.g., empty boot image), we set Nsect to 1.
    let mut default_nsect: u16 = 1; // Default to 1 if no bootable entry or size is 0
    if let Some(first_bootable_entry) = entries.iter().find(|e| e.bootable) {
        let sectors = first_bootable_entry.boot_image_sectors;
        if sectors > 0 {
            default_nsect = sectors.min(255); // Cap at 255 for 1-byte field
        }
    }
    val[27] = default_nsect as u8; // Nsect is 1 byte at offset 27

    // Set Bootoff (LBA of the boot catalog)
    val[28..30].copy_from_slice(&LBA_BOOT_CATALOG.to_le_bytes());

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
        entry[0] = if entry_data.bootable {
            BOOT_CATALOG_BOOT_ENTRY_HEADER_ID
        } else {
            0x00
        };
        entry[1] = 0x00; // No Emulation
        entry[2..4].copy_from_slice(&0u16.to_le_bytes());
        entry[4] = entry_data.platform_id;
        entry[8..12].copy_from_slice(&entry_data.boot_image_lba.to_le_bytes());
        entry[12..14].copy_from_slice(&entry_data.boot_image_sectors.to_le_bytes());
        catalog[offset..offset + 32].copy_from_slice(&entry);
        offset += 32;
    }

    iso.write_all(&catalog)?;

    Ok(())
}
