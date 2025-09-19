// isobemak/src/iso/volume_descriptor.rs
use crate::iso::dir_record::IsoDirEntry;
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba};
use std::fs::File;
use std::io::{self, Write};

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

pub fn write_primary_volume_descriptor(
    iso: &mut File,
    total_sectors: u32,
    root_entry: &IsoDirEntry,
) -> io::Result<()> {
    pad_to_lba(iso, 16)?;
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

    let record_bytes = root_entry.to_bytes();
    pvd[PVD_ROOT_DIR_RECORD_OFFSET..PVD_ROOT_DIR_RECORD_OFFSET + record_bytes.len()]
        .copy_from_slice(&record_bytes);

    iso.write_all(&pvd)
}

pub fn write_boot_record_volume_descriptor(
    iso: &mut File,
    boot_catalog_lba: u32,
) -> io::Result<()> {
    pad_to_lba(iso, 17)?;
    let mut brvd = [0u8; ISO_SECTOR_SIZE];
    brvd[0] = ISO_VOLUME_DESCRIPTOR_BOOT_RECORD;
    brvd[1..6].copy_from_slice(ISO_ID);
    brvd[6] = ISO_VERSION;
    let spec_name = b"EL TORITO SPECIFICATION";
    brvd[7..7 + spec_name.len()].copy_from_slice(spec_name);
    brvd[71..75].copy_from_slice(&boot_catalog_lba.to_le_bytes());
    iso.write_all(&brvd)
}

pub fn write_volume_descriptor_terminator(iso: &mut File) -> io::Result<()> {
    pad_to_lba(iso, 18)?;
    let mut term = [0u8; ISO_SECTOR_SIZE];
    term[0] = ISO_VOLUME_DESCRIPTOR_TERMINATOR;
    term[1..6].copy_from_slice(ISO_ID);
    term[6] = ISO_VERSION;
    iso.write_all(&term)
}

/// A combined function to write all necessary volume descriptors in sequence.
pub fn write_volume_descriptors(
    iso: &mut File,
    total_sectors: u32,
    boot_catalog_lba: u32,
    root_entry: &IsoDirEntry,
) -> io::Result<()> {
    write_primary_volume_descriptor(iso, total_sectors, root_entry)?;
    write_boot_record_volume_descriptor(iso, boot_catalog_lba)?;
    write_volume_descriptor_terminator(iso)?;
    Ok(())
}