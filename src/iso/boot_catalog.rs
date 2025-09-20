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
pub const BOOT_CATALOG_CHECKSUM_OFFSET: usize = 2;

/// Writes an El Torito boot catalog for UEFI.
pub fn write_boot_catalog(
    iso: &mut File,
    boot_img_lba: u32,
    boot_img_sectors: u16,
) -> io::Result<()> {
    pad_to_lba(iso, LBA_BOOT_CATALOG)?;

    let mut catalog = [0u8; ISO_SECTOR_SIZE];

    // --- Validation Entry (32 bytes) ---
    let mut val = [0u8; 32];
    val[0] = BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID;
    val[1] = BOOT_CATALOG_EFI_PLATFORM_ID;

    // ID string "UEFI" + zero padding (32 bytes)
    val[ID_FIELD_OFFSET..ID_FIELD_OFFSET + 24]
        .copy_from_slice(b"UEFI\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"); // 4 bytes for "UEFI" + 20 null bytes = 24 bytes

    // Signature 0xAA55 at offset 30
    val[30..32].copy_from_slice(&BOOT_CATALOG_HEADER_SIGNATURE.to_le_bytes());

    // Checksum calculation (0..32, excluding checksum field itself)
    // Temporarily zero out the checksum field for calculation
    val[BOOT_CATALOG_CHECKSUM_OFFSET..BOOT_CATALOG_CHECKSUM_OFFSET + 2].copy_from_slice(&[0, 0]);
    let mut sum: u16 = 0;
    for i in (0..32).step_by(2) { // Iterate over all 32 bytes
        sum = sum.wrapping_add(u16::from_le_bytes([val[i], val[i + 1]]));
    }
    let checksum = 0u16.wrapping_sub(sum);
    val[BOOT_CATALOG_CHECKSUM_OFFSET..BOOT_CATALOG_CHECKSUM_OFFSET + 2]
        .copy_from_slice(&checksum.to_le_bytes());
    catalog[0..32].copy_from_slice(&val);

    // --- Default Boot Entry (32 bytes) ---
    let mut entry = [0u8; 32];
    entry[0] = BOOT_CATALOG_BOOT_ENTRY_HEADER_ID; // Bootable
    entry[1] = 0x00;                              // No Emulation
    entry[2..4].copy_from_slice(&0u16.to_le_bytes()); // Load Segment
    entry[4] = BOOT_CATALOG_EFI_PLATFORM_ID;      // System Type
    entry[5] = 0x00;                              // Unused
    entry[6..8].copy_from_slice(&boot_img_sectors.to_le_bytes()); // Sector count (512-byte sectors)
    entry[8..12].copy_from_slice(&boot_img_lba.to_le_bytes());    // LBA
    catalog[32..64].copy_from_slice(&entry);

    iso.write_all(&catalog)
}
