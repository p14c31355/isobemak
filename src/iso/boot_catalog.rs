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
    let mut catalog = [0u8; ISO_SECTOR_SIZE];
    let mut offset = 0;

    // Validation Entry (32 bytes)
    let mut val = [0u8; 32];
    val[0] = BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID;
    let first_platform = entries.first().map_or(0u8, |e| e.platform_id);
    val[1] = first_platform;
    let id_bytes = if first_platform == BOOT_CATALOG_EFI_PLATFORM_ID {
        [0u8; 24]
    } else {
        let mut bytes = [0u8; 24];
        let spec = b"EL TORITO SPECIFICATION";
        bytes[0..spec.len()].copy_from_slice(spec);
        bytes
    };
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

    // Boot Entries
    for entry_data in entries {
        let mut entry = [0u8; 32];
        let boot_indicator = if entry_data.bootable {
            BOOT_CATALOG_BOOT_ENTRY_HEADER_ID // 0x88
        } else {
            0x00u8
        };
        entry[0] = boot_indicator;
        entry[1] = 0x00; // No Emulation
        entry[2..4].copy_from_slice(&0u16.to_le_bytes()); // Load segment
        entry[4] = entry_data.platform_id; // System type (0xEF for UEFI)

        // Sector count is a u16 at offset 6. An upstream check should ensure this doesn't overflow.
        let sectors = entry_data.boot_image_sectors;
        entry[6..8].copy_from_slice(&sectors.to_le_bytes());

        // Load RBA (LBA in 512-byte sectors) is a u32 at offset 8.
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
            bootable: true,
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
            bootable: false,
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
