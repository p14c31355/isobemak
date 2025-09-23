// isobemak/src/iso/volume_descriptor.rs
use crate::iso::boot_catalog::LBA_BOOT_CATALOG;
use crate::iso::dir_record::IsoDirEntry;
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba};
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

pub fn write_primary_volume_descriptor(
    iso: &mut File,
    total_sectors: u32,
    root_entry: &IsoDirEntry,
    base_lba: u32, // PVD's LBA
) -> io::Result<()> {
    pad_to_lba(iso, base_lba)?;
    let mut pvd = [0u8; ISO_SECTOR_SIZE];
    pvd[0] = ISO_VOLUME_DESCRIPTOR_PRIMARY;
    pvd[1..6].copy_from_slice(ISO_ID);
    pvd[6] = ISO_VERSION;

    let project_name = b"ISOBEMAKI";
    let mut volume_id = [b' '; 32];
    volume_id[..project_name.len()].copy_from_slice(project_name);
    pvd[PVD_VOLUME_ID_OFFSET..PVD_VOLUME_ID_OFFSET + 32].copy_from_slice(&volume_id);

    pvd[PVD_TOTAL_SECTORS_OFFSET..PVD_TOTAL_SECTORS_OFFSET + 4]
        .copy_from_slice(&total_sectors.to_le_bytes());
    pvd[PVD_TOTAL_SECTORS_OFFSET + 4..PVD_TOTAL_SECTORS_OFFSET + 8]
        .copy_from_slice(&total_sectors.to_be_bytes());

    pvd[PVD_VOL_SET_SIZE_OFFSET..PVD_VOL_SET_SIZE_OFFSET + 2].copy_from_slice(&1u16.to_le_bytes());
    pvd[PVD_VOL_SET_SIZE_OFFSET + 2..PVD_VOL_SET_SIZE_OFFSET + 4]
        .copy_from_slice(&1u16.to_be_bytes());

    pvd[PVD_VOL_SEQ_NUM_OFFSET..PVD_VOL_SEQ_NUM_OFFSET + 2].copy_from_slice(&1u16.to_le_bytes());
    pvd[PVD_VOL_SEQ_NUM_OFFSET + 2..PVD_VOL_SEQ_NUM_OFFSET + 4]
        .copy_from_slice(&1u16.to_be_bytes());

    pvd[PVD_LOGICAL_BLOCK_SIZE_OFFSET..PVD_LOGICAL_BLOCK_SIZE_OFFSET + 2]
        .copy_from_slice(&(ISO_SECTOR_SIZE as u16).to_le_bytes());
    pvd[PVD_LOGICAL_BLOCK_SIZE_OFFSET + 2..PVD_LOGICAL_BLOCK_SIZE_OFFSET + 4]
        .copy_from_slice(&(ISO_SECTOR_SIZE as u16).to_be_bytes());

    pvd[PVD_PATH_TABLE_SIZE_OFFSET..PVD_PATH_TABLE_SIZE_OFFSET + 4]
        .copy_from_slice(&0u32.to_le_bytes());
    pvd[PVD_PATH_TABLE_SIZE_OFFSET + 4..PVD_PATH_TABLE_SIZE_OFFSET + 8]
        .copy_from_slice(&0u32.to_be_bytes());

    let root_entry_bytes = root_entry.to_bytes();
    pvd[PVD_ROOT_DIR_RECORD_OFFSET..PVD_ROOT_DIR_RECORD_OFFSET + root_entry_bytes.len()]
        .copy_from_slice(&root_entry_bytes);

    iso.write_all(&pvd)?;

    // Update total sectors in PVD, passing the PVD's LBA
    update_total_sectors_in_pvd(iso, base_lba, total_sectors)?;

    Ok(())
}

pub fn update_total_sectors_in_pvd(iso: &mut File, base_lba: u32, total_sectors: u32) -> io::Result<()> {
    update_4byte_fields(
        iso,
        base_lba,
        PVD_TOTAL_SECTORS_OFFSET,
        PVD_TOTAL_SECTORS_OFFSET + 4,
        total_sectors,
    )
}

pub fn write_boot_record_volume_descriptor(
    iso: &mut File,
    boot_catalog_lba: u32, // LBA of the boot catalog
    base_lba: u32, // BRVD's LBA
) -> io::Result<()> {
    pad_to_lba(iso, base_lba)?;
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

pub fn write_volume_descriptor_terminator(iso: &mut File, base_lba: u32) -> io::Result<()> {
    pad_to_lba(iso, base_lba)?;
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
    base_lba: u32, // The starting LBA for VDs
) -> io::Result<()> {
    // PVD at base_lba
    write_primary_volume_descriptor(iso, total_sectors, root_entry, base_lba)?;
    // BRVD at base_lba + 1
    // The boot_catalog_lba needs to be dynamic. For now, let's assume it's base_lba + 1 + 1 = base_lba + 2
    // But the LBA_BOOT_CATALOG constant is 19. This needs to be fixed.
    // Let's assume the boot catalog is always after the VDs.
    let boot_catalog_lba = base_lba + 3; // PVD(1) + BRVD(1) + Terminator(1) = 3 sectors
    write_boot_record_volume_descriptor(iso, boot_catalog_lba, base_lba + 1)?;
    // Terminator at base_lba + 2
    write_volume_descriptor_terminator(iso, base_lba + 2)?;
    Ok(())
}
