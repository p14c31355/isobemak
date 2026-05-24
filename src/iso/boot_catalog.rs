use crate::utils::ISO_SECTOR_SIZE;
use std::fs::File;
use std::io::{self, Write};

pub const LBA_BOOT_CATALOG: u32 = 19;
pub const BOOT_CATALOG_HEADER_SIGNATURE: u16 = 0xAA55;
pub const BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID: u8 = 1;
pub const BOOT_CATALOG_BOOT_ENTRY_HEADER_ID: u8 = 0x88;
pub const BOOT_CATALOG_SECTION_HEADER_MORE_ID: u8 = 0x90;
pub const BOOT_CATALOG_SECTION_HEADER_FINAL_ID: u8 = 0x91;
pub const BOOT_CATALOG_EFI_PLATFORM_ID: u8 = 0xEF;
const CHECKSUM_OFFSET: usize = 28;
const ID_OFFSET: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootCatalogEntryType {
    BootEntry { bootable: bool },
    SectionHeader { more_follow: bool },
}

pub struct BootCatalogEntry {
    pub platform_id: u8,
    pub boot_image_lba: u32,
    pub boot_image_sectors: u16,
    pub entry_type: BootCatalogEntryType,
}

pub fn write_boot_catalog(iso: &mut File, entries: Vec<BootCatalogEntry>) -> io::Result<()> {
    let mut catalog = [0u8; ISO_SECTOR_SIZE];
    let mut offset = 0;

    // Validation Entry
    let mut val = [0u8; 32];
    val[0] = BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID;
    val[1] = 0x00;
    let mut id = [0u8; 24];
    id[..23].copy_from_slice(b"EL TORITO SPECIFICATION");
    val[ID_OFFSET..ID_OFFSET + 24].copy_from_slice(&id);
    val[30..32].copy_from_slice(&BOOT_CATALOG_HEADER_SIGNATURE.to_le_bytes());
    let sum: u16 = (0..32)
        .step_by(2)
        .filter(|&i| i != CHECKSUM_OFFSET)
        .fold(0u16, |s, i| {
            s.wrapping_add(u16::from_le_bytes(val[i..i + 2].try_into().unwrap()))
        });
    val[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 2]
        .copy_from_slice(&(0u16.wrapping_sub(sum)).to_le_bytes());
    catalog[offset..offset + 32].copy_from_slice(&val);
    offset += 32;

    // Pre-compute section entry counts
    let section_counts: Vec<u16> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if matches!(e.entry_type, BootCatalogEntryType::SectionHeader { .. }) {
                entries[i + 1..]
                    .iter()
                    .take_while(|n| {
                        !matches!(n.entry_type, BootCatalogEntryType::SectionHeader { .. })
                    })
                    .count() as u16
            } else {
                0
            }
        })
        .collect();

    for (idx, entry_data) in entries.iter().enumerate() {
        let mut e = [0u8; 32];
        let (flag, media_type) = match entry_data.entry_type {
            BootCatalogEntryType::BootEntry { bootable } => (
                if bootable {
                    BOOT_CATALOG_BOOT_ENTRY_HEADER_ID
                } else {
                    0x00
                },
                0x00,
            ),
            BootCatalogEntryType::SectionHeader { more_follow } => (
                if more_follow {
                    BOOT_CATALOG_SECTION_HEADER_MORE_ID
                } else {
                    BOOT_CATALOG_SECTION_HEADER_FINAL_ID
                },
                entry_data.platform_id,
            ),
        };
        e[0] = flag;
        e[1] = media_type;
        let f23 = if matches!(
            entry_data.entry_type,
            BootCatalogEntryType::SectionHeader { .. }
        ) {
            section_counts[idx]
        } else {
            0
        };
        e[2..4].copy_from_slice(&f23.to_le_bytes());
        e[4] = match entry_data.entry_type {
            BootCatalogEntryType::SectionHeader { .. } => 0x00,
            BootCatalogEntryType::BootEntry { .. } => entry_data.platform_id,
        };
        e[6..8].copy_from_slice(&entry_data.boot_image_sectors.to_le_bytes());
        e[8..12].copy_from_slice(&entry_data.boot_image_lba.to_le_bytes());
        catalog[offset..offset + 32].copy_from_slice(&e);
        offset += 32;
    }
    iso.write_all(&catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};
    use tempfile::NamedTempFile;

    fn verify_checksum(ve: &[u8; 32]) {
        let s = (0..32).step_by(2).fold(0u16, |a, i| {
            a.wrapping_add(u16::from_le_bytes([ve[i], ve[i + 1]]))
        });
        assert_eq!(s, 0);
    }

    #[test]
    fn test_single_efi() -> io::Result<()> {
        let mut f = NamedTempFile::new()?;
        write_boot_catalog(
            f.as_file_mut(),
            vec![BootCatalogEntry {
                platform_id: BOOT_CATALOG_EFI_PLATFORM_ID,
                boot_image_lba: 100,
                boot_image_sectors: 50,
                entry_type: BootCatalogEntryType::BootEntry { bootable: true },
            }],
        )?;
        let mut buf = [0u8; ISO_SECTOR_SIZE];
        f.seek(SeekFrom::Start(0))?;
        f.read_exact(&mut buf)?;
        let ve: &[u8; 32] = &buf[0..32].try_into().unwrap();
        assert_eq!(ve[0], 1);
        assert_eq!(ve[1], 0x00);
        assert_eq!(&ve[30..32], &0xAA55u16.to_le_bytes());
        verify_checksum(ve);
        let be = &buf[32..64];
        assert_eq!(be[0], 0x88);
        assert_eq!(be[1], 0x00);
        assert_eq!(&be[6..8], &50u16.to_le_bytes());
        assert_eq!(&be[8..12], &100u32.to_le_bytes());
        Ok(())
    }

    #[test]
    fn test_non_bootable() -> io::Result<()> {
        let mut f = NamedTempFile::new()?;
        write_boot_catalog(
            f.as_file_mut(),
            vec![BootCatalogEntry {
                platform_id: 0,
                boot_image_lba: 200,
                boot_image_sectors: 20,
                entry_type: BootCatalogEntryType::BootEntry { bootable: false },
            }],
        )?;
        let mut buf = [0u8; ISO_SECTOR_SIZE];
        f.seek(SeekFrom::Start(0))?;
        f.read_exact(&mut buf)?;
        assert_eq!(buf[32], 0x00);
        Ok(())
    }
}
