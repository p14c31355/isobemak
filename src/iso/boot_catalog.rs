// isobemak/src/iso/boot_catalog.rs
use crate::utils::ISO_SECTOR_SIZE;
use std::fs::File;
use std::io::{self, Write};

/// LBA of the boot catalog in the ISO.
pub const LBA_BOOT_CATALOG: u32 = 19;

/// Boot catalog constants
pub const BOOT_CATALOG_HEADER_SIGNATURE: u16 = 0xAA55;
pub const BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID: u8 = 1;
pub const BOOT_CATALOG_BOOT_ENTRY_HEADER_ID: u8 = 0x88;
/// Initial/Default entry flag (El Torito §6.2.1)
pub const BOOT_CATALOG_INITIAL_ENTRY_HEADER_ID: u8 = 0x90;
/// Final/Section Header entry flag (El Torito §6.2.1, Table 8)
pub const BOOT_CATALOG_FINAL_ENTRY_HEADER_ID: u8 = 0x91;
pub const BOOT_CATALOG_EFI_PLATFORM_ID: u8 = 0xEF;
pub const ID_FIELD_OFFSET: usize = 4;
pub const BOOT_CATALOG_CHECKSUM_OFFSET: usize = 28;

/// Type of boot catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootCatalogEntryType {
    /// Standard boot entry (flag=0x88 or 0x00)
    BootEntry { bootable: bool },
    /// Initial/Default entry (flag=0x90)
    InitialDefault,
    /// Section Header / Final entry (flag=0x91, per El Torito Table 8 for UEFI)
    SectionHeader,
}

pub struct BootCatalogEntry {
    pub platform_id: u8,
    pub boot_image_lba: u32,
    pub boot_image_sectors: u16,
    pub entry_type: BootCatalogEntryType,
}

/// Writes an El Torito boot catalog.
pub fn write_boot_catalog(iso: &mut File, entries: Vec<BootCatalogEntry>) -> io::Result<()> {
    let mut catalog = [0u8; ISO_SECTOR_SIZE];
    let mut offset = 0;

    // Validation Entry (32 bytes)
    let mut val = [0u8; 32];
    val[0] = BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID;
    let first_platform = entries.first().map_or(0u8, |e| e.platform_id);
    val[1] = first_platform;
    // ID string must always be "EL TORITO SPECIFICATION" per El Torito spec,
    // regardless of platform ID.  Some real UEFI firmware rejects the boot
    // catalog when this field is zero-filled.
    let mut bytes = [0u8; 24];
    let spec = b"EL TORITO SPECIFICATION";
    bytes[0..spec.len()].copy_from_slice(spec);
    let id_bytes = bytes;
    val[ID_FIELD_OFFSET..ID_FIELD_OFFSET + 24].copy_from_slice(&id_bytes);

    // No Nsect in Validation Entry (non-standard and corrupts ID string)

    // Set Signature
    val[30..32].copy_from_slice(&BOOT_CATALOG_HEADER_SIGNATURE.to_le_bytes());

    // Checksum calculation
    // The sum of all 16-bit words in the 32-byte entry must be zero.
    // First, calculate the sum of all words *except* the checksum word.
    let mut sum: u16 = 0;
    for i in (0..32).step_by(2) {
        // Skip the checksum field itself
        if i == BOOT_CATALOG_CHECKSUM_OFFSET {
            continue;
        }
        let word = u16::from_le_bytes(val[i..i + 2].try_into().unwrap());
        sum = sum.wrapping_add(word);
    }

    // The checksum is the value that, when added to the sum, results in zero.
    // This is equivalent to the two's complement of the sum.
    let checksum = 0u16.wrapping_sub(sum);
    val[BOOT_CATALOG_CHECKSUM_OFFSET..BOOT_CATALOG_CHECKSUM_OFFSET + 2]
        .copy_from_slice(&checksum.to_le_bytes());

    catalog[offset..offset + 32].copy_from_slice(&val);
    offset += 32;

    // Pre-compute section entry counts for each SectionHeader.
    // A SectionHeader (flag=0x91) at position i needs to know how many
    // non-header entries follow it in the same section (up to the next
    // SectionHeader or end of list).  Real UEFI firmware (OVMF, InsydeH2O)
    // uses this value to locate boot entries.
    let section_counts: Vec<u16> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if matches!(e.entry_type, BootCatalogEntryType::SectionHeader) {
                entries[i + 1..]
                    .iter()
                    .take_while(|next| {
                        !matches!(next.entry_type, BootCatalogEntryType::SectionHeader)
                    })
                    .count() as u16
            } else {
                0
            }
        })
        .collect();

    // Boot Entries
    for (idx, entry_data) in entries.iter().enumerate() {
        let mut entry = [0u8; 32];

        let (flag, media_type) = match entry_data.entry_type {
            BootCatalogEntryType::BootEntry { bootable } => {
                let indicator = if bootable {
                    BOOT_CATALOG_BOOT_ENTRY_HEADER_ID // 0x88
                } else {
                    0x00u8
                };
                (indicator, 0x00u8) // No Emulation
            }
            BootCatalogEntryType::InitialDefault => {
                // Initial/Default: flag=0x90, media=4 (Hard Disk)
                (BOOT_CATALOG_INITIAL_ENTRY_HEADER_ID, 4u8)
            }
            BootCatalogEntryType::SectionHeader => {
                // Section Header / Final: flag=0x91, media=platform_id (0xEF for UEFI)
                (BOOT_CATALOG_FINAL_ENTRY_HEADER_ID, entry_data.platform_id)
            }
        };

        entry[0] = flag;
        entry[1] = media_type;

        // Bytes 2–3: Load segment for boot entries; "Number of section entries"
        // for Section Header entries (El Torito §7.2.4 Table 8).
        let field_2_3: u16 = match entry_data.entry_type {
            BootCatalogEntryType::SectionHeader => section_counts[idx],
            _ => 0,
        };
        entry[2..4].copy_from_slice(&field_2_3.to_le_bytes());

        // System type:
        //   Section Header (0x91): always 0x00 (El Torito Table 8)
        //   Boot Entry in a section (0x88): 0x00 (platform defined by Section Header, §7.2.3)
        //   Initial/Default (0x90): platform_id (standalone, not in a section)
        entry[4] = match entry_data.entry_type {
            BootCatalogEntryType::SectionHeader => 0x00,
            BootCatalogEntryType::BootEntry { .. } => 0x00,
            BootCatalogEntryType::InitialDefault => entry_data.platform_id,
        };

        // Sector count is a u16 at offset 6.
        let sectors = entry_data.boot_image_sectors;
        entry[6..8].copy_from_slice(&sectors.to_le_bytes());

        // Load RBA is a u32 at offset 8.
        let load_rba = entry_data.boot_image_lba;
        entry[8..12].copy_from_slice(&load_rba.to_le_bytes());

        // Bytes 12-31 are unused and already zeroed
        catalog[offset..offset + 32].copy_from_slice(&entry);
        offset += 32;
    }

    iso.write_all(&catalog)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};
    use tempfile::NamedTempFile;

    fn verify_checksum(validation_entry: &[u8; 32]) {
        let mut sum: u16 = 0;
        for i in (0..32).step_by(2) {
            sum = sum.wrapping_add(u16::from_le_bytes([
                validation_entry[i],
                validation_entry[i + 1],
            ]));
        }
        assert_eq!(sum, 0, "Boot catalog validation entry checksum is invalid");
    }

    #[test]
    fn test_single_efi_boot_entry() -> io::Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        let entries = vec![BootCatalogEntry {
            platform_id: BOOT_CATALOG_EFI_PLATFORM_ID,
            boot_image_lba: 100,
            boot_image_sectors: 50,
            entry_type: BootCatalogEntryType::BootEntry { bootable: true },
        }];

        write_boot_catalog(temp_file.as_file_mut(), entries)?;

        let mut buffer = [0u8; ISO_SECTOR_SIZE];
        temp_file.seek(SeekFrom::Start(0))?;
        temp_file.read_exact(&mut buffer)?;

        // Verify Validation Entry
        let val_entry: &[u8; 32] = &buffer[0..32].try_into().unwrap();
        assert_eq!(val_entry[0], BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID);
        assert_eq!(val_entry[1], BOOT_CATALOG_EFI_PLATFORM_ID);
        assert_eq!(
            &val_entry[30..32],
            &BOOT_CATALOG_HEADER_SIGNATURE.to_le_bytes()
        );
        verify_checksum(val_entry);

        // Verify Boot Entry
        let boot_entry: &[u8; 32] = &buffer[32..64].try_into().unwrap();
        assert_eq!(boot_entry[0], BOOT_CATALOG_BOOT_ENTRY_HEADER_ID);
        assert_eq!(boot_entry[1], 0x00); // No Emulation
        assert_eq!(&boot_entry[6..8], &50u16.to_le_bytes()); // Sector count
        assert_eq!(&boot_entry[8..12], &100u32.to_le_bytes()); // LBA

        Ok(())
    }

    #[test]
    fn test_non_bootable_entry() -> io::Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        let entries = vec![BootCatalogEntry {
            platform_id: 0, // BIOS
            boot_image_lba: 200,
            boot_image_sectors: 20,
            entry_type: BootCatalogEntryType::BootEntry { bootable: false },
        }];

        write_boot_catalog(temp_file.as_file_mut(), entries)?;

        let mut buffer = [0u8; ISO_SECTOR_SIZE];
        temp_file.seek(SeekFrom::Start(0))?;
        temp_file.read_exact(&mut buffer)?;

        // Verify Boot Entry
        let boot_entry: &[u8; 32] = &buffer[32..64].try_into().unwrap();
        assert_eq!(boot_entry[0], 0x00); // Not bootable

        Ok(())
    }
}
