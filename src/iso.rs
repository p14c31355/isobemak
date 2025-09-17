// isobemak/src/iso.rs
// ISO + El Torito
use crate::utils::ISO_SECTOR_SIZE;
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
const PVD_ROOT_DIR_RECORD_OFFSET: usize = 156;

// New constants for PVD fields
const PVD_VOL_SET_SIZE_OFFSET: usize = 120;
const PVD_VOL_SEQ_NUM_OFFSET: usize = 124;
const PVD_LOGICAL_BLOCK_SIZE_OFFSET: usize = 128;
const PVD_PATH_TABLE_SIZE_OFFSET: usize = 132;

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

/// Pads the ISO file with zeros to align to a specific LBA.
fn pad_to_lba(iso: &mut File, lba: u32) -> io::Result<()> {
    let target_pos = lba as u64 * ISO_SECTOR_SIZE as u64;
    let current_pos = iso.stream_position()?;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        println!(
            "pad_to_lba: Padding {} bytes to reach LBA {}.",
            padding_bytes, lba
        );
        io::copy(&mut io::repeat(0).take(padding_bytes), iso)?;
    } else if current_pos > target_pos {
        eprintln!(
            "Warning: Current position is past the target LBA. This might indicate an error."
        );
    }
    Ok(())
}

fn write_primary_volume_descriptor(
    iso: &mut File,
    total_sectors: u32,
    root_dir_lba: u32,
) -> io::Result<()> {
    const LBA_PVD: u32 = 16;
    println!(
        "write_primary_volume_descriptor: Writing PVD at LBA {}.",
        LBA_PVD
    );
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
    root_dir_record[0] = 34;
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
    println!(
        "write_boot_record_volume_descriptor: Writing BRVD at LBA {}.",
        LBA_BRVD
    );
    pad_to_lba(iso, LBA_BRVD)?;
    let mut brvd = [0u8; ISO_SECTOR_SIZE];
    brvd[0] = ISO_VOLUME_DESCRIPTOR_BOOT_RECORD;
    brvd[1..6].copy_from_slice(ISO_ID);
    brvd[6] = ISO_VERSION;
    let spec_name = b"EL TORITO SPECIFICATION";
    brvd[7..7 + spec_name.len()].copy_from_slice(spec_name);
    brvd[71..75].copy_from_slice(&lba_boot_catalog.to_le_bytes());
    iso.write_all(&brvd)
}

fn write_volume_descriptor_terminator(iso: &mut File) -> io::Result<()> {
    const LBA_VDT: u32 = 18;
    println!(
        "write_volume_descriptor_terminator: Writing VDT at LBA {}.",
        LBA_VDT
    );
    pad_to_lba(iso, LBA_VDT)?;
    let mut term = [0u8; ISO_SECTOR_SIZE];
    term[0] = ISO_VOLUME_DESCRIPTOR_TERMINATOR;
    term[1..6].copy_from_slice(ISO_ID);
    term[6] = ISO_VERSION;
    iso.write_all(&term)
}

/// Correctly writes the El Torito boot catalog.
/// It takes the LBA and size of the boot image to create a bootable entry.
fn write_boot_catalog(iso: &mut File, boot_img_lba: u32, boot_img_size: u32) -> io::Result<()> {
    const LBA_BOOT_CATALOG: u32 = 19;
    println!(
        "write_boot_catalog: Writing boot catalog at LBA {}.",
        LBA_BOOT_CATALOG
    );
    pad_to_lba(iso, LBA_BOOT_CATALOG)?;
    let mut cat = [0u8; ISO_SECTOR_SIZE];

    cat[0] = BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID;
    cat[1] = BOOT_CATALOG_EFI_PLATFORM_ID;
    cat[2..4].copy_from_slice(&[0; 2]);

    let mut id_field = [0u8; ID_FIELD_LEN];
    id_field[..ID_STR.len()].copy_from_slice(ID_STR);
    cat[ID_FIELD_OFFSET..ID_FIELD_OFFSET + ID_FIELD_LEN].copy_from_slice(&id_field);

    cat[BOOT_CATALOG_VALIDATION_SIGNATURE_OFFSET..BOOT_CATALOG_VALIDATION_SIGNATURE_OFFSET + 2]
        .copy_from_slice(&BOOT_CATALOG_HEADER_SIGNATURE.to_le_bytes());

    let mut sum: u16 = 0;
    for i in (0..32).step_by(2) {
        sum = sum.wrapping_add(u16::from_le_bytes([cat[i], cat[i + 1]]));
    }
    let checksum = 0u16.wrapping_sub(sum);
    cat[BOOT_CATALOG_CHECKSUM_OFFSET..BOOT_CATALOG_CHECKSUM_OFFSET + 2]
        .copy_from_slice(&checksum.to_le_bytes());

    let mut entry = [0u8; 32];
    entry[0] = BOOT_CATALOG_BOOT_ENTRY_HEADER_ID;
    entry[1] = BOOT_CATALOG_NO_EMULATION;

    // Boot image sector count (512-byte sectors)
    let sector_count_512 = boot_img_size.div_ceil(512);
    let sector_count_u16 = if sector_count_512 > 0xFFFF {
        0xFFFF
    } else {
        sector_count_512 as u16
    };
    entry[6..8].copy_from_slice(&sector_count_u16.to_le_bytes());

    // Set the LBA for the boot image
    entry[8..12].copy_from_slice(&boot_img_lba.to_le_bytes());
    cat[32..64].copy_from_slice(&entry);

    iso.write_all(&cat)
}

fn update_total_sectors(iso: &mut File, total_sectors: u32) -> io::Result<()> {
    const PVD_START_OFFSET: u64 = 16 * ISO_SECTOR_SIZE as u64;
    const PVD_TOTAL_SECTORS_LE_OFFSET: u64 = PVD_START_OFFSET + PVD_TOTAL_SECTORS_OFFSET as u64;
    const PVD_TOTAL_SECTORS_BE_OFFSET: u64 = PVD_START_OFFSET + PVD_TOTAL_SECTORS_OFFSET as u64 + 4;

    println!(
        "update_total_sectors: Updating total sectors to {} at PVD.",
        total_sectors
    );

    iso.seek(SeekFrom::Start(PVD_TOTAL_SECTORS_LE_OFFSET))?;
    iso.write_all(&total_sectors.to_le_bytes())?;

    iso.seek(SeekFrom::Start(PVD_TOTAL_SECTORS_BE_OFFSET))?;
    iso.write_all(&total_sectors.to_be_bytes())?;

    Ok(())
}

/// Reads the entire FAT32 image from a specified path and returns its content.
fn read_fat32_img_from_path(img_path: &Path) -> io::Result<Vec<u8>> {
    println!(
        "read_fat32_img_from_path: Reading FAT32 image from '{}'.",
        img_path.display()
    );
    let mut img_file = File::open(img_path)?;
    let mut content = Vec::new();
    img_file.read_to_end(&mut content)?;
    println!("read_fat32_img_from_path: Read {} bytes.", content.len());
    Ok(content)
}

/// Creates a special directory record for '.' and '..'
fn write_special_dir_record(name_byte: u8, lba: u32) -> io::Result<Vec<u8>> {
    let record_len = 34; // Correct record length for special directories
    let mut record = vec![0u8; record_len];
    record[0] = record_len as u8;
    record[2..6].copy_from_slice(&lba.to_le_bytes());
    record[6..10].copy_from_slice(&lba.to_be_bytes());
    record[10..14].copy_from_slice(&(ISO_SECTOR_SIZE as u32).to_le_bytes());
    record[14..18].copy_from_slice(&(ISO_SECTOR_SIZE as u32).to_be_bytes());
    record[25] = 2; // Directory flag
    record[32] = 1; // Name length
    record[33] = name_byte;
    Ok(record)
}

/// Creates a file record for an ISO 9660 filesystem.
fn write_file_record(name: &str, lba: u32, size: u32) -> io::Result<Vec<u8>> {
    let name_len = name.len();
    let record_len = 34 + name_len;
    let mut record = vec![0u8; record_len];
    record[0] = record_len as u8;
    record[2..6].copy_from_slice(&lba.to_le_bytes());
    record[6..10].copy_from_slice(&lba.to_be_bytes());
    record[10..14].copy_from_slice(&size.to_le_bytes());
    record[14..18].copy_from_slice(&size.to_be_bytes());
    record[25] = 0; // File flag
    record[32] = name_len as u8;
    record[33] = 0;
    record[34..34 + name_len].copy_from_slice(name.as_bytes());

    Ok(record)
}

/// Creates an ISO image from a FAT32 image file.
pub fn create_iso_from_img(iso_path: &Path, fat32_img_path: &Path) -> io::Result<()> {
    println!("create_iso_from_img: Creating ISO from FAT32 image.");

    let mut iso = File::create(iso_path)?;
    io::copy(
        &mut io::repeat(0).take(ISO_SECTOR_SIZE as u64 * 16),
        &mut iso,
    )?;

    const LBA_PVD: u32 = 16;
    const LBA_BRVD: u32 = 17;
    const LBA_VDT: u32 = 18;
    const LBA_BOOT_CATALOG: u32 = 19;
    const LBA_ROOT_DIR: u32 = 20;

    let fat32_content = read_fat32_img_from_path(fat32_img_path)?;
    let fat32_size = fat32_content.len() as u32;

    // Calculate LBA for FAT32 image, accounting for padding and records
    let lba_fat32 = LBA_ROOT_DIR + 1;
    println!(
        "create_iso_from_img: Calculated FAT32 image LBA: {}.",
        lba_fat32
    );

    // --- 1. Write Volume Descriptors. PVD total sectors will be patched later. ---
    write_primary_volume_descriptor(&mut iso, 0, LBA_ROOT_DIR)?;
    write_boot_record_volume_descriptor(&mut iso, LBA_BOOT_CATALOG)?;
    write_volume_descriptor_terminator(&mut iso)?;

    // --- 2. Write the boot catalog. ---
    pad_to_lba(&mut iso, LBA_BOOT_CATALOG)?;
    write_boot_catalog(&mut iso, lba_fat32, fat32_size)?;

    // --- 3. Write ISO9660 root directory and file records. ---
    pad_to_lba(&mut iso, LBA_ROOT_DIR)?;
    println!(
        "create_iso_from_img: Writing root directory records at LBA {}.",
        LBA_ROOT_DIR
    );

    let mut root_dir_records = Vec::new();

    // Self-record
    root_dir_records.write_all(&write_special_dir_record(0x00, LBA_ROOT_DIR)?)?;
    // Parent-record
    root_dir_records.write_all(&write_special_dir_record(0x01, LBA_ROOT_DIR)?)?;
    // Boot-NoEmul.img file record (FAT32 image)
    root_dir_records.write_all(&write_file_record(
        "BOOT-NOEMUL.IMG",
        lba_fat32,
        fat32_size,
    )?)?;

    // Pad the root directory sector to ensure it takes up one full sector
    let padding = ISO_SECTOR_SIZE - root_dir_records.len();
    root_dir_records.resize(ISO_SECTOR_SIZE, 0);

    iso.write_all(&root_dir_records)?;

    // --- 4. Write the FAT32 image content to the ISO. ---
    pad_to_lba(&mut iso, lba_fat32)?;
    println!(
        "create_iso_from_img: Writing FAT32 image content ({} bytes) at LBA {}.",
        fat32_size, lba_fat32
    );
    iso.write_all(&fat32_content)?;

    // --- 5. Finalize ISO file by updating the total number of sectors. ---
    iso.seek(io::SeekFrom::End(0))?;
    let final_pos = iso.stream_position()?;
    let total_sectors = (final_pos as f64 / ISO_SECTOR_SIZE as f64).ceil() as u32;

    update_total_sectors(&mut iso, total_sectors)?;
    iso.set_len(total_sectors as u64 * ISO_SECTOR_SIZE as u64)?;

    println!(
        "create_iso_from_img: ISO created with {} sectors.",
        total_sectors
    );
    Ok(())
}
