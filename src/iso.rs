// isobemak/src/iso.rs
// ISO + El Torito
use crate::utils::{FAT32_SECTOR_SIZE, ISO_SECTOR_SIZE};
use fatfs::{self, FsOptions};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

// Constants for ISO 9660 structure to improve readability.
const ISO_VOLUME_DESCRIPTOR_TERMINATOR: u8 = 255;
const ISO_VOLUME_DESCRIPTOR_PRIMARY: u8 = 1;
const ISO_VOLUME_DESCRIPTOR_BOOT_RECORD: u8 = 0;
const ISO_ID: &[u8] = b"CD001";
const ISO_VERSION: u8 = 1;
const PVD_VOLUME_ID_OFFSET: usize = 40;
const PVD_TOTAL_SECTORS_OFFSET: usize = 80;

// New constants for PVD fields
const PVD_VOL_SET_SIZE_OFFSET: usize = 120;
const PVD_VOL_SEQ_NUM_OFFSET: usize = 124;
const PVD_LOGICAL_BLOCK_SIZE_OFFSET: usize = 128;
const PVD_PATH_TABLE_SIZE_OFFSET: usize = 132;
const PVD_ROOT_DIR_RECORD_OFFSET: usize = 156;

// Constants for El Torito boot catalog.
const BOOT_CATALOG_HEADER_SIGNATURE: u16 = 0xAA55;
const BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID: u8 = 1;
const BOOT_CATALOG_BOOT_ENTRY_HEADER_ID: u8 = 0x88;
const BOOT_CATALOG_NO_EMULATION: u8 = 0x00;
const BOOT_CATALOG_EFI_PLATFORM_ID: u8 = 0xEF;

// New constants for Boot Catalog
const ID_FIELD_OFFSET: usize = 4;
const ID_FIELD_LEN: usize = 24;
const ID_STR: &[u8] = b"ISOBEMAKI EFI BOOT";
const BOOT_CATALOG_CHECKSUM_OFFSET: usize = 28;
const BOOT_CATALOG_VALIDATION_SIGNATURE_OFFSET: usize = 30;

// New constants for Directory Records
const DIR_RECORD_LEN: u8 = 34;
const DIR_RECORD_LBA_OFFSET: usize = 2;
const DIR_RECORD_DATA_LEN_OFFSET: usize = 10;
const DIR_RECORD_FLAGS_OFFSET: usize = 25;
const DIR_RECORD_VOL_SEQ_OFFSET: usize = 28;
const DIR_RECORD_ID_LEN_OFFSET: usize = 32;
const DIR_RECORD_ID_OFFSET: usize = 33;

/// Pads the ISO file with zeros to align to a specific LBA.
fn pad_to_lba(iso: &mut File, lba: u32) -> io::Result<()> {
    let target_pos = lba as u64 * ISO_SECTOR_SIZE as u64;
    let current_pos = iso.stream_position()?;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        io::copy(&mut io::repeat(0).take(padding_bytes), iso)?;
    }
    Ok(())
}

/// Helper to write a directory record.
fn write_directory_record(
    sector: &mut [u8],
    offset: usize,
    lba: u32,
    data_len: u32,
    file_id: &[u8],
    flags: u8,
) {
    sector[offset] = DIR_RECORD_LEN;
    sector[offset + DIR_RECORD_LBA_OFFSET..offset + DIR_RECORD_LBA_OFFSET + 4]
        .copy_from_slice(&lba.to_le_bytes());
    sector[offset + DIR_RECORD_LBA_OFFSET + 4..offset + DIR_RECORD_LBA_OFFSET + 8]
        .copy_from_slice(&lba.to_be_bytes());

    sector[offset + DIR_RECORD_DATA_LEN_OFFSET..offset + DIR_RECORD_DATA_LEN_OFFSET + 4]
        .copy_from_slice(&data_len.to_le_bytes());
    sector[offset + DIR_RECORD_DATA_LEN_OFFSET + 4..offset + DIR_RECORD_DATA_LEN_OFFSET + 8]
        .copy_from_slice(&data_len.to_be_bytes());

    sector[offset + DIR_RECORD_FLAGS_OFFSET] = flags; // File flags
    let vol_seq: u16 = 1;
    sector[offset + DIR_RECORD_VOL_SEQ_OFFSET..offset + DIR_RECORD_VOL_SEQ_OFFSET + 2]
        .copy_from_slice(&vol_seq.to_le_bytes());
    sector[offset + DIR_RECORD_VOL_SEQ_OFFSET + 2..offset + DIR_RECORD_VOL_SEQ_OFFSET + 4]
        .copy_from_slice(&vol_seq.to_be_bytes());

    sector[offset + DIR_RECORD_ID_LEN_OFFSET] = file_id.len() as u8;
    sector[offset + DIR_RECORD_ID_OFFSET..offset + DIR_RECORD_ID_OFFSET + file_id.len()]
        .copy_from_slice(file_id);
}

fn write_root_directory_sector(iso: &mut File, root_dir_lba: u32) -> io::Result<()> {
    pad_to_lba(iso, root_dir_lba)?;
    let mut root_dir_sector = [0u8; ISO_SECTOR_SIZE];

    // . (self) directory record
    write_directory_record(
        &mut root_dir_sector,
        0,
        root_dir_lba,
        ISO_SECTOR_SIZE as u32,
        b"\x00",
        2,
    );

    // .. (parent) directory record
    let parent_offset = DIR_RECORD_LEN as usize;
    write_directory_record(
        &mut root_dir_sector,
        parent_offset,
        root_dir_lba,
        ISO_SECTOR_SIZE as u32,
        b"\x01",
        2,
    );

    iso.write_all(&root_dir_sector)
}

fn write_primary_volume_descriptor(
    iso: &mut File,
    total_sectors: u32,
    root_dir_lba: u32,
) -> io::Result<()> {
    const LBA_PVD: u32 = 16;
    pad_to_lba(iso, LBA_PVD)?;
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
    let vol_set_size: u16 = 1;
    pvd[PVD_VOL_SET_SIZE_OFFSET..PVD_VOL_SET_SIZE_OFFSET + 2]
        .copy_from_slice(&vol_set_size.to_le_bytes());
    pvd[PVD_VOL_SET_SIZE_OFFSET + 2..PVD_VOL_SET_SIZE_OFFSET + 4]
        .copy_from_slice(&vol_set_size.to_be_bytes());

    let vol_seq_num: u16 = 1;
    pvd[PVD_VOL_SEQ_NUM_OFFSET..PVD_VOL_SEQ_NUM_OFFSET + 2]
        .copy_from_slice(&vol_seq_num.to_le_bytes());
    pvd[PVD_VOL_SEQ_NUM_OFFSET + 2..PVD_VOL_SEQ_NUM_OFFSET + 4]
        .copy_from_slice(&vol_seq_num.to_be_bytes());

    let sector_size_u16 = ISO_SECTOR_SIZE as u16;
    pvd[PVD_LOGICAL_BLOCK_SIZE_OFFSET..PVD_LOGICAL_BLOCK_SIZE_OFFSET + 2]
        .copy_from_slice(&sector_size_u16.to_le_bytes());
    pvd[PVD_LOGICAL_BLOCK_SIZE_OFFSET + 2..PVD_LOGICAL_BLOCK_SIZE_OFFSET + 4]
        .copy_from_slice(&sector_size_u16.to_be_bytes());

    let path_table_size: u32 = 0;
    pvd[PVD_PATH_TABLE_SIZE_OFFSET..PVD_PATH_TABLE_SIZE_OFFSET + 4]
        .copy_from_slice(&path_table_size.to_le_bytes());
    pvd[PVD_PATH_TABLE_SIZE_OFFSET + 4..PVD_PATH_TABLE_SIZE_OFFSET + 8]
        .copy_from_slice(&path_table_size.to_be_bytes());

    let mut root_dir_record = [0u8; 34];
    root_dir_record[0] = 34; // Directory record length
    let root_dir_lba_u32 = root_dir_lba;
    root_dir_record[2..6].copy_from_slice(&root_dir_lba_u32.to_le_bytes());
    root_dir_record[6..10].copy_from_slice(&root_dir_lba_u32.to_be_bytes());
    let sector_size_u32 = ISO_SECTOR_SIZE as u32;
    root_dir_record[10..14].copy_from_slice(&sector_size_u32.to_le_bytes());
    root_dir_record[14..18].copy_from_slice(&sector_size_u32.to_be_bytes());
    root_dir_record[25] = 2;
    let vol_seq: u16 = 1;
    root_dir_record[28..30].copy_from_slice(&vol_seq.to_le_bytes());
    root_dir_record[30..32].copy_from_slice(&vol_seq.to_be_bytes());
    root_dir_record[32] = 1;
    root_dir_record[33] = 0;

    pvd[PVD_ROOT_DIR_RECORD_OFFSET..PVD_ROOT_DIR_RECORD_OFFSET + 34]
        .copy_from_slice(&root_dir_record);
    iso.write_all(&pvd)
}

fn write_boot_record_volume_descriptor(iso: &mut File, lba_boot_catalog: u32) -> io::Result<()> {
    const LBA_BRVD: u32 = 17;
    pad_to_lba(iso, LBA_BRVD)?;
    let mut brvd = [0u8; ISO_SECTOR_SIZE];
    brvd[0] = ISO_VOLUME_DESCRIPTOR_BOOT_RECORD;
    brvd[1..6].copy_from_slice(ISO_ID);
    brvd[6] = ISO_VERSION;
    let spec_name = b"EL TORITO SPECIFICATION";
    brvd[7..7 + spec_name.len()].copy_from_slice(spec_name);
    brvd[71..75].copy_from_slice(&lba_boot_catalog.to_le_bytes()); // Boot Catalog LBA
    iso.write_all(&brvd)
}

fn write_volume_descriptor_terminator(iso: &mut File) -> io::Result<()> {
    const LBA_VDT: u32 = 18;
    pad_to_lba(iso, LBA_VDT)?;
    let mut term = [0u8; ISO_SECTOR_SIZE];
    term[0] = ISO_VOLUME_DESCRIPTOR_TERMINATOR;
    term[1..6].copy_from_slice(ISO_ID);
    term[6] = ISO_VERSION;
    iso.write_all(&term)
}

fn write_boot_catalog(iso: &mut File, fat_image_lba: u32, img_file_size: u64) -> io::Result<()> {
    const LBA_BOOT_CATALOG: u32 = 19;
    pad_to_lba(iso, LBA_BOOT_CATALOG)?;
    let mut cat = [0u8; ISO_SECTOR_SIZE];

    // Validation Entry
    cat[0] = BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID;
    cat[1] = BOOT_CATALOG_EFI_PLATFORM_ID;
    cat[2..4].copy_from_slice(&[0; 2]); // Reserved

    let mut id_field = [0u8; ID_FIELD_LEN];
    id_field[..ID_STR.len()].copy_from_slice(ID_STR);
    cat[ID_FIELD_OFFSET..ID_FIELD_OFFSET + ID_FIELD_LEN].copy_from_slice(&id_field);

    cat[BOOT_CATALOG_VALIDATION_SIGNATURE_OFFSET..BOOT_CATALOG_VALIDATION_SIGNATURE_OFFSET + 2]
        .copy_from_slice(&BOOT_CATALOG_HEADER_SIGNATURE.to_le_bytes());

    // Checksum calculation (reordered)
    let mut sum: u16 = 0;
    for i in (0..32).step_by(2) {
        sum = sum.wrapping_add(u16::from_le_bytes([cat[i], cat[i + 1]]));
    }
    let checksum = 0u16.wrapping_sub(sum);
    cat[BOOT_CATALOG_CHECKSUM_OFFSET..BOOT_CATALOG_CHECKSUM_OFFSET + 2]
        .copy_from_slice(&checksum.to_le_bytes());

    // Boot Entry
    let mut entry = [0u8; 32];
    entry[0] = BOOT_CATALOG_BOOT_ENTRY_HEADER_ID;
    entry[1] = BOOT_CATALOG_NO_EMULATION;

    // FIX: Write the correct sector count in FAT32_SECTOR_SIZE (512-byte) units
    let sector_count_512 = img_file_size.div_ceil(FAT32_SECTOR_SIZE);
    let sector_count_u16 = if sector_count_512 > 0xFFFF {
        0xFFFF
    } else {
        sector_count_512 as u16
    };
    entry[6..8].copy_from_slice(&sector_count_u16.to_le_bytes());

    entry[8..12].copy_from_slice(&fat_image_lba.to_le_bytes()); // LBA of FAT32 image
    cat[32..64].copy_from_slice(&entry);

    iso.write_all(&cat)
}

fn update_total_sectors(iso: &mut File, total_sectors: u32) -> io::Result<()> {
    const PVD_START_OFFSET: u64 = 16 * ISO_SECTOR_SIZE as u64;
    const PVD_TOTAL_SECTORS_LE_OFFSET: u64 = PVD_START_OFFSET + PVD_TOTAL_SECTORS_OFFSET as u64;
    const PVD_TOTAL_SECTORS_BE_OFFSET: u64 = PVD_START_OFFSET + PVD_TOTAL_SECTORS_OFFSET as u64 + 4;

    iso.seek(SeekFrom::Start(PVD_TOTAL_SECTORS_LE_OFFSET))?;
    iso.write_all(&total_sectors.to_le_bytes())?;

    iso.seek(SeekFrom::Start(PVD_TOTAL_SECTORS_BE_OFFSET))?;
    iso.write_all(&total_sectors.to_be_bytes())?;

    Ok(())
}

fn iso_path_name(name: &str) -> Vec<u8> {
    let mut parts = name.split('.');
    let mut base = parts.next().unwrap_or("").to_uppercase();
    let ext = parts.next().unwrap_or("").to_uppercase();

    // ISO9660 filename restrictions
    base = base.replace(|c: char| !c.is_ascii_alphanumeric(), "_");
    let mut filename = base.split('_').next().unwrap_or("").to_string();
    if !ext.is_empty() {
        filename.push('.');
        filename.push_str(&ext);
    }

    // Add version number
    filename.push_str(";1");
    filename.into_bytes()
}

/// Recursively copies a directory's contents from the FAT32 image to the ISO file,
/// creating directory records in the process.
fn copy_fat_dir_to_iso(
    iso: &mut File,
    fat32_dir: &fatfs::Dir<&mut File>,
    parent_lba: u32,
) -> io::Result<u32> {
    let dir_lba = (iso.stream_position()? / ISO_SECTOR_SIZE as u64) as u32;
    let mut dir_sector = [0u8; ISO_SECTOR_SIZE];
    let mut offset = 0;

    // . (self) directory record
    write_directory_record(
        &mut dir_sector,
        offset,
        dir_lba,
        ISO_SECTOR_SIZE as u32,
        b"\x00",
        2,
    );
    offset += 1 + 33; // Record length for '.' is 34 bytes

    // .. (parent) directory record
    write_directory_record(
        &mut dir_sector,
        offset,
        parent_lba,
        ISO_SECTOR_SIZE as u32,
        b"\x01",
        2,
    );
    offset += 1 + 33; // Record length for '..' is 34 bytes

    let entries = fat32_dir.iter().collect::<io::Result<Vec<_>>>()?;
    for entry in entries {
        let name = entry.file_name();
        let name_iso = iso_path_name(&name);

        let entry_lba: u32;
        let entry_size: u32;

        // Ensure that the file is not the parent directory
        if name == "." || name == ".." {
            continue;
        }

        if entry.is_dir() {
            let subdir_lba = copy_fat_dir_to_iso(iso, &entry.to_dir(), dir_lba)?;
            entry_lba = subdir_lba;
            entry_size = ISO_SECTOR_SIZE as u32;
            write_directory_record(&mut dir_sector, offset, entry_lba, entry_size, &name_iso, 2);
        } else {
            let mut file = entry.to_file();
            let file_size = entry.len();
            let file_lba = (iso.stream_position()? / ISO_SECTOR_SIZE as u64) as u32;
            io::copy(&mut file, iso)?;
            pad_to_lba(
                iso,
                file_lba + (file_size.div_ceil(ISO_SECTOR_SIZE as u64)) as u32,
            )?;

            entry_lba = file_lba;
            entry_size = file_size as u32;
            write_directory_record(&mut dir_sector, offset, entry_lba, entry_size, &name_iso, 0);
        }

        let record_len = DIR_RECORD_LEN as usize + name_iso.len();
        let padded_len = if record_len % 2 == 0 {
            record_len
        } else {
            record_len + 1
        };
        offset += padded_len;
    }

    iso.seek(SeekFrom::Start(dir_lba as u64 * ISO_SECTOR_SIZE as u64))?;
    iso.write_all(&dir_sector)?;
    iso.seek(SeekFrom::End(0))?;

    Ok(dir_lba)
}

/// Creates a bootable ISO image from a FAT32 image.
/// This function now also populates the ISO's file system with the contents of the FAT32 image.
pub fn create_iso_from_img(iso_path: &Path, img_path: &Path) -> io::Result<()> {
    println!("create_iso_from_img: Creating ISO from FAT32 image.");

    let mut img_file = File::open(img_path)?;
    let img_file_size = img_file.metadata()?.len();

    let mut iso = File::create(iso_path)?;
    io::copy(
        &mut io::repeat(0).take(ISO_SECTOR_SIZE as u64 * 16),
        &mut iso,
    )?; // System Area

    const FAT_IMAGE_LBA: u32 = 21;
    let root_dir_lba;

    {
        let fs = fatfs::FileSystem::new(&mut img_file, FsOptions::new())?;
        let root = fs.root_dir();
        // The root directory will start at LBA 20
        pad_to_lba(&mut iso, 20)?;
        root_dir_lba = copy_fat_dir_to_iso(&mut iso, &root, 20)?;
    }

    // Write FAT image
    pad_to_lba(&mut iso, FAT_IMAGE_LBA)?;
    img_file.seek(SeekFrom::Start(0))?;
    let mut limited_reader = img_file.take(img_file_size);
    io::copy(&mut limited_reader, &mut iso)?;

    // Calculate the final total sectors from the actual file size
    let final_pos = iso.stream_position()?;
    let total_sectors = final_pos.div_ceil(ISO_SECTOR_SIZE as u64) as u32;

    // Write ISO Volume Descriptors
    write_primary_volume_descriptor(&mut iso, total_sectors, root_dir_lba)?;
    const LBA_BOOT_CATALOG: u32 = 19;
    write_boot_record_volume_descriptor(&mut iso, LBA_BOOT_CATALOG)?;
    write_volume_descriptor_terminator(&mut iso)?;

    // Write Boot Catalog
    write_boot_catalog(&mut iso, FAT_IMAGE_LBA, img_file_size)?;

    // Write Root Directory Sector
    write_root_directory_sector(&mut iso, root_dir_lba)?;

    // Seek back and update the total_sectors field in the PVD
    update_total_sectors(&mut iso, total_sectors)?;

    // Ensure the file is truncated to the correct size
    iso.set_len(total_sectors as u64 * ISO_SECTOR_SIZE as u64)?;

    Ok(())
}
