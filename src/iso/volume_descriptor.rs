// isobemak/src/iso/volume_descriptor.rs
use crate::iso::boot_catalog::LBA_BOOT_CATALOG;
use crate::iso::dir_record::IsoDirEntry;
use crate::utils::{ISO_SECTOR_SIZE, seek_to_lba};
use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};

pub const ISO_VOLUME_DESCRIPTOR_TERMINATOR: u8 = 255;
pub const ISO_VOLUME_DESCRIPTOR_PRIMARY: u8 = 1;
pub const ISO_VOLUME_DESCRIPTOR_BOOT_RECORD: u8 = 0;
pub const ISO_ID: &[u8] = b"CD001";
pub const ISO_VERSION: u8 = 1;
pub const PVD_VOLUME_ID_OFFSET: usize = 40;
pub const PVD_TOTAL_SECTORS_OFFSET: usize = 80;
pub const PVD_ROOT_DIR_RECORD_OFFSET: usize = 156;
pub const PVD_VOL_SET_SIZE_OFFSET: usize = 120;
pub const PVD_VOL_SEQ_NUM_OFFSET: usize = 124;
pub const PVD_LOGICAL_BLOCK_SIZE_OFFSET: usize = 128;
pub const PVD_PATH_TABLE_SIZE_OFFSET: usize = 132;

/// A helper function to update two 4-byte fields at different offsets
/// within a single ISO sector (2048 bytes).
fn update_4byte_fields(
    iso: &mut File,
    base_lba: u32,
    offset1: usize,
    offset2: usize,
    value: u32,
) -> io::Result<()> {
    let base_offset = base_lba as u64 * ISO_SECTOR_SIZE as u64;

    iso.seek(SeekFrom::Start(base_offset + offset1 as u64))?;
    iso.write_all(&value.to_le_bytes())?;

    iso.seek(SeekFrom::Start(base_offset + offset2 as u64))?;
    iso.write_all(&value.to_be_bytes())?;

    Ok(())
}

/// Helper to write a value in both little-endian and big-endian to a buffer at given offset and length.
fn write_dual_endian(buf: &mut [u8], offset: usize, value: u32, byte_len: usize) {
    match byte_len {
        2 => {
            buf[offset..offset + 2].copy_from_slice(&(value as u16).to_le_bytes());
            buf[offset + 2..offset + 4].copy_from_slice(&(value as u16).to_be_bytes());
        }
        4 => {
            buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            buf[offset + 4..offset + 8].copy_from_slice(&value.to_be_bytes());
        }
        _ => panic!("Unsupported byte length for dual endian write"),
    }
}

pub fn write_primary_volume_descriptor(
    iso: &mut File,
    total_sectors: u32,
    root_entry: &IsoDirEntry,
) -> io::Result<()> {
    seek_to_lba(iso, 16)?;
    let mut pvd = [0u8; ISO_SECTOR_SIZE];
    pvd[0] = ISO_VOLUME_DESCRIPTOR_PRIMARY;
    pvd[1..6].copy_from_slice(ISO_ID);
    pvd[6] = ISO_VERSION;

    let project_name = b"ISOBEMAKI";
    let mut volume_id = [b' '; 32];
    volume_id[..project_name.len()].copy_from_slice(project_name);
    pvd[PVD_VOLUME_ID_OFFSET..PVD_VOLUME_ID_OFFSET + 32].copy_from_slice(&volume_id);

    write_dual_endian(&mut pvd, PVD_TOTAL_SECTORS_OFFSET, total_sectors, 4);
    write_dual_endian(&mut pvd, PVD_VOL_SET_SIZE_OFFSET, 1, 2);
    write_dual_endian(&mut pvd, PVD_VOL_SEQ_NUM_OFFSET, 1, 2);
    write_dual_endian(
        &mut pvd,
        PVD_LOGICAL_BLOCK_SIZE_OFFSET,
        ISO_SECTOR_SIZE as u32,
        2,
    );
    write_dual_endian(&mut pvd, PVD_PATH_TABLE_SIZE_OFFSET, 0, 4);

    let root_entry_bytes = root_entry.to_bytes();
    pvd[PVD_ROOT_DIR_RECORD_OFFSET..PVD_ROOT_DIR_RECORD_OFFSET + root_entry_bytes.len()]
        .copy_from_slice(&root_entry_bytes);

    iso.write_all(&pvd)?;

    Ok(())
}

pub fn update_total_sectors_in_pvd(iso: &mut File, total_sectors: u32) -> io::Result<()> {
    update_4byte_fields(
        iso,
        16,
        PVD_TOTAL_SECTORS_OFFSET,
        PVD_TOTAL_SECTORS_OFFSET + 4,
        total_sectors,
    )
}

pub fn write_boot_record_volume_descriptor(
    iso: &mut File,
    boot_catalog_lba: u32,
) -> io::Result<()> {
    seek_to_lba(iso, 17)?;
    let mut brvd = [0u8; ISO_SECTOR_SIZE];
    brvd[0] = ISO_VOLUME_DESCRIPTOR_BOOT_RECORD;
    brvd[1..6].copy_from_slice(ISO_ID);
    brvd[6] = ISO_VERSION;
    let spec_name = b"EL TORITO SPECIFICATION";
    brvd[7..7 + spec_name.len()].copy_from_slice(spec_name);
    brvd[71..75].copy_from_slice(&boot_catalog_lba.to_le_bytes());
    iso.write_all(&brvd)?;
    Ok(())
}

pub fn write_volume_descriptor_terminator(iso: &mut File) -> io::Result<()> {
    seek_to_lba(iso, 18)?;
    let mut term = [0u8; ISO_SECTOR_SIZE];
    term[0] = ISO_VOLUME_DESCRIPTOR_TERMINATOR;
    term[1..6].copy_from_slice(ISO_ID);
    term[6] = ISO_VERSION;
    iso.write_all(&term)?;
    Ok(())
}

/// A combined function to write all necessary volume descriptors in sequence.
pub fn write_volume_descriptors(
    iso: &mut File,
    total_sectors: u32,
    root_entry: &IsoDirEntry,
) -> io::Result<()> {
    // Primary Volume Descriptor at LBA 16
    write_primary_volume_descriptor(iso, total_sectors, root_entry)?;
    // Boot Record Volume Descriptor at LBA 17, pointing to boot catalog at LBA 19
    write_boot_record_volume_descriptor(iso, LBA_BOOT_CATALOG)?;
    // Volume Descriptor Terminator at LBA 18
    write_volume_descriptor_terminator(iso)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::NamedTempFile;

    fn read_sector(file: &mut File, lba: u32) -> io::Result<[u8; ISO_SECTOR_SIZE]> {
        let mut buffer = [0u8; ISO_SECTOR_SIZE];
        file.seek(SeekFrom::Start(lba as u64 * ISO_SECTOR_SIZE as u64))?;
        file.read_exact(&mut buffer)?;
        Ok(buffer)
    }

    #[test]
    fn test_write_pvd() -> io::Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        let root_entry = IsoDirEntry {
            lba: 20,
            size: 2048,
            flags: 2,
            name: ".",
        };
        let total_sectors = 1000;

        write_primary_volume_descriptor(temp_file.as_file_mut(), total_sectors, &root_entry)?;

        let pvd_sector = read_sector(temp_file.as_file_mut(), 16)?;
        assert_eq!(pvd_sector[0], ISO_VOLUME_DESCRIPTOR_PRIMARY);
        assert_eq!(&pvd_sector[1..6], ISO_ID);
        let mut expected_sectors = [0u8; 8];
        expected_sectors[0..4].copy_from_slice(&total_sectors.to_le_bytes());
        expected_sectors[4..8].copy_from_slice(&total_sectors.to_be_bytes());
        assert_eq!(
            &pvd_sector[PVD_TOTAL_SECTORS_OFFSET..PVD_TOTAL_SECTORS_OFFSET + 8],
            &expected_sectors
        );
        let root_bytes = root_entry.to_bytes();
        assert_eq!(
            &pvd_sector[PVD_ROOT_DIR_RECORD_OFFSET..PVD_ROOT_DIR_RECORD_OFFSET + root_bytes.len()],
            &root_bytes
        );

        Ok(())
    }

    #[test]
    fn test_update_pvd_sectors() -> io::Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        let root_entry = IsoDirEntry {
            lba: 20,
            size: 2048,
            flags: 2,
            name: ".",
        };
        write_primary_volume_descriptor(temp_file.as_file_mut(), 1000, &root_entry)?;

        let new_total_sectors = 2500;
        update_total_sectors_in_pvd(temp_file.as_file_mut(), new_total_sectors)?;

        let pvd_sector = read_sector(temp_file.as_file_mut(), 16)?;
        let read_sectors_le = u32::from_le_bytes(
            pvd_sector[PVD_TOTAL_SECTORS_OFFSET..PVD_TOTAL_SECTORS_OFFSET + 4]
                .try_into()
                .unwrap(),
        );
        let read_sectors_be = u32::from_be_bytes(
            pvd_sector[PVD_TOTAL_SECTORS_OFFSET + 4..PVD_TOTAL_SECTORS_OFFSET + 8]
                .try_into()
                .unwrap(),
        );
        assert_eq!(read_sectors_le, new_total_sectors);
        assert_eq!(read_sectors_be, new_total_sectors);

        Ok(())
    }

    #[test]
    fn test_write_brvd() -> io::Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        let boot_catalog_lba = 99;
        write_boot_record_volume_descriptor(temp_file.as_file_mut(), boot_catalog_lba)?;

        let brvd_sector = read_sector(temp_file.as_file_mut(), 17)?;
        assert_eq!(brvd_sector[0], ISO_VOLUME_DESCRIPTOR_BOOT_RECORD);
        assert_eq!(&brvd_sector[1..6], ISO_ID);
        assert_eq!(&brvd_sector[7..30], b"EL TORITO SPECIFICATION");
        assert_eq!(&brvd_sector[71..75], &boot_catalog_lba.to_le_bytes());

        Ok(())
    }

    #[test]
    fn test_write_terminator() -> io::Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        write_volume_descriptor_terminator(temp_file.as_file_mut())?;

        let term_sector = read_sector(temp_file.as_file_mut(), 18)?;
        assert_eq!(term_sector[0], ISO_VOLUME_DESCRIPTOR_TERMINATOR);
        assert_eq!(&term_sector[1..6], ISO_ID);

        Ok(())
    }

    #[test]
    fn test_write_all_descriptors() -> io::Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        let root_entry = IsoDirEntry {
            lba: 20,
            size: 2048,
            flags: 2,
            name: ".",
        };
        let total_sectors = 1234;

        write_volume_descriptors(temp_file.as_file_mut(), total_sectors, &root_entry)?;

        // Verify PVD
        let pvd_sector = read_sector(temp_file.as_file_mut(), 16)?;
        assert_eq!(pvd_sector[0], ISO_VOLUME_DESCRIPTOR_PRIMARY);

        // Verify BRVD
        let brvd_sector = read_sector(temp_file.as_file_mut(), 17)?;
        assert_eq!(brvd_sector[0], ISO_VOLUME_DESCRIPTOR_BOOT_RECORD);
        assert_eq!(&brvd_sector[71..75], &LBA_BOOT_CATALOG.to_le_bytes());

        // Verify Terminator
        let term_sector = read_sector(temp_file.as_file_mut(), 18)?;
        assert_eq!(term_sector[0], ISO_VOLUME_DESCRIPTOR_TERMINATOR);

        Ok(())
    }
}
