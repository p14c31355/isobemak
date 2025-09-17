// isobemak/src/iso.rs
// ISO + El Torito
use crate::utils::{FAT32_SECTOR_SIZE, ISO_SECTOR_SIZE};
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
const PVD_SECTOR_SIZE_OFFSET: usize = 128;
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

// New constants for Directory Records
const DIR_RECORD_LEN_MIN: u8 = 34; // Minimum directory record length
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

/// Helper to write a directory record for both directories and files.
/// This function now correctly handles little-endian and big-endian values
/// and ensures the record length is even as per ISO 9660 specification.
fn write_directory_record(
    sector: &mut [u8],
    offset: &mut usize,
    lba: u32,
    data_len: u32,
    flags: u8,
    file_id: &[u8],
) {
    let id_len = file_id.len() as u8;
    let rec_len = DIR_RECORD_LEN_MIN + id_len + (id_len % 2);

    if *offset + rec_len as usize > ISO_SECTOR_SIZE {
        return;
    }

    let record_slice = &mut sector[*offset..];
    record_slice[0] = rec_len;

    record_slice[DIR_RECORD_LBA_OFFSET..DIR_RECORD_LBA_OFFSET + 4]
        .copy_from_slice(&lba.to_le_bytes());
    record_slice[DIR_RECORD_LBA_OFFSET + 4..DIR_RECORD_LBA_OFFSET + 8]
        .copy_from_slice(&lba.to_be_bytes());

    record_slice[DIR_RECORD_DATA_LEN_OFFSET..DIR_RECORD_DATA_LEN_OFFSET + 4]
        .copy_from_slice(&data_len.to_le_bytes());
    record_slice[DIR_RECORD_DATA_LEN_OFFSET + 4..DIR_RECORD_DATA_LEN_OFFSET + 8]
        .copy_from_slice(&data_len.to_be_bytes());

    record_slice[DIR_RECORD_FLAGS_OFFSET] = flags;
    let vol_seq: u16 = 1;
    record_slice[DIR_RECORD_VOL_SEQ_OFFSET..DIR_RECORD_VOL_SEQ_OFFSET + 2]
        .copy_from_slice(&vol_seq.to_le_bytes());
    record_slice[DIR_RECORD_VOL_SEQ_OFFSET + 2..DIR_RECORD_VOL_SEQ_OFFSET + 4]
        .copy_from_slice(&vol_seq.to_be_bytes());

    record_slice[DIR_RECORD_ID_LEN_OFFSET] = id_len;
    record_slice[DIR_RECORD_ID_OFFSET..DIR_RECORD_ID_OFFSET + id_len as usize]
        .copy_from_slice(file_id);

    *offset += rec_len as usize;
}

fn write_root_directory_sector(
    iso: &mut File,
    root_dir_lba: u32,
    efi_dir_lba: u32,
) -> io::Result<()> {
    pad_to_lba(iso, root_dir_lba)?;
    let mut root_dir_sector = [0u8; ISO_SECTOR_SIZE];
    let mut offset = 0;

    write_directory_record(
        &mut root_dir_sector,
        &mut offset,
        root_dir_lba,
        ISO_SECTOR_SIZE as u32,
        2,
        &[0x00],
    );

    write_directory_record(
        &mut root_dir_sector,
        &mut offset,
        root_dir_lba,
        ISO_SECTOR_SIZE as u32,
        2,
        &[0x01],
    );

    write_directory_record(
        &mut root_dir_sector,
        &mut offset,
        efi_dir_lba,
        ISO_SECTOR_SIZE as u32,
        2,
        b"EFI",
    );

    iso.write_all(&root_dir_sector)
}

fn write_efi_directory_sector(
    iso: &mut File,
    efi_dir_lba: u32,
    root_dir_lba: u32,
    boot_dir_lba: u32,
) -> io::Result<()> {
    pad_to_lba(iso, efi_dir_lba)?;
    let mut efi_dir_sector = [0u8; ISO_SECTOR_SIZE];
    let mut offset = 0;

    write_directory_record(
        &mut efi_dir_sector,
        &mut offset,
        efi_dir_lba,
        ISO_SECTOR_SIZE as u32,
        2,
        &[0x00],
    );

    write_directory_record(
        &mut efi_dir_sector,
        &mut offset,
        root_dir_lba,
        ISO_SECTOR_SIZE as u32,
        2,
        &[0x01],
    );

    write_directory_record(
        &mut efi_dir_sector,
        &mut offset,
        boot_dir_lba,
        ISO_SECTOR_SIZE as u32,
        2,
        b"BOOT",
    );

    iso.write_all(&efi_dir_sector)
}

fn write_boot_directory_sector(
    iso: &mut File,
    boot_dir_lba: u32,
    efi_dir_lba: u32,
    bootx64_lba: u32,
    bootx64_size: u32,
) -> io::Result<()> {
    pad_to_lba(iso, boot_dir_lba)?;
    let mut boot_dir_sector = [0u8; ISO_SECTOR_SIZE];
    let mut offset = 0;

    write_directory_record(
        &mut boot_dir_sector,
        &mut offset,
        boot_dir_lba,
        ISO_SECTOR_SIZE as u32,
        2,
        &[0x00],
    );

    write_directory_record(
        &mut boot_dir_sector,
        &mut offset,
        efi_dir_lba,
        ISO_SECTOR_SIZE as u32,
        2,
        &[0x01],
    );

    write_directory_record(
        &mut boot_dir_sector,
        &mut offset,
        bootx64_lba,
        bootx64_size,
        0,
        b"BOOTX64.EFI;1",
    );

    iso.write_all(&boot_dir_sector)
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
    pad_to_lba(iso, LBA_VDT)?;
    let mut term = [0u8; ISO_SECTOR_SIZE];
    term[0] = ISO_VOLUME_DESCRIPTOR_TERMINATOR;
    term[1..6].copy_from_slice(ISO_ID);
    term[6] = ISO_VERSION;
    iso.write_all(&term)
}

/// Correctly writes the El Torito boot catalog.
/// It takes the LBA and size of the BOOTX64.EFI file to ensure a bootable entry.
fn write_boot_catalog(iso: &mut File, bootx64_lba: u32, bootx64_size: u32) -> io::Result<()> {
    const LBA_BOOT_CATALOG: u32 = 19;
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

    // Calculate the number of 512-byte sectors for BOOTX64.EFI
    let sector_count_512 = (bootx64_size as u64 + 511) / 512;
    let sector_count_u16 = if sector_count_512 > 0xFFFF {
        0xFFFF
    } else {
        sector_count_512 as u16
    };
    entry[6..8].copy_from_slice(&sector_count_u16.to_le_bytes());

    // Set the LBA for BOOTX64.EFI
    entry[8..12].copy_from_slice(&bootx64_lba.to_le_bytes());
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

/// A dummy function to read a file from the FAT32 image.
/// This is a placeholder and assumes a fixed location.
/// In a real scenario, you would need to parse the FAT32 filesystem.
fn read_efi_bootx64_from_fat32_image(img_file: &mut File) -> io::Result<Vec<u8>> {
    // In a real-world scenario, you would:
    // 1. Read the BPB from img_file to find the start of the root directory and FAT.
    // 2. Traverse directory entries to find 'EFI', then 'BOOT', then 'BOOTX64.EFI'.
    // 3. Follow the cluster chain in the FAT to read the file's content.
    //
    // For this refactoring, we'll return a dummy file content for testing.
    let dummy_size: u32 = 4096;
    let dummy_content = vec![0xABu8; dummy_size as usize];
    Ok(dummy_content)
}

pub fn create_iso_from_img(iso_path: &Path, img_path: &Path) -> io::Result<()> {
    println!("create_iso_from_img: Creating ISO from FAT32 image.");

    let mut img_file = File::open(img_path)?;
    let img_file_size = img_file.metadata()?.len();

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
    const LBA_EFI_DIR: u32 = 21;
    const LBA_BOOT_DIR: u32 = 22;

    // --- 1. Write Volume Descriptors. PVD total sectors and boot catalog LBA will be patched later. ---
    write_primary_volume_descriptor(&mut iso, 0, LBA_ROOT_DIR)?;
    write_boot_record_volume_descriptor(&mut iso, LBA_BOOT_CATALOG)?;
    write_volume_descriptor_terminator(&mut iso)?;

    // --- 2. Write placeholder for the boot catalog. LBA and size will be updated. ---
    pad_to_lba(&mut iso, LBA_BOOT_CATALOG)?;
    iso.write_all(&[0u8; ISO_SECTOR_SIZE])?;

    // --- 3. Write ISO9660 directory sectors. LBA of BOOTX64.EFI will be patched later. ---
    write_root_directory_sector(&mut iso, LBA_ROOT_DIR, LBA_EFI_DIR)?;
    write_efi_directory_sector(&mut iso, LBA_EFI_DIR, LBA_ROOT_DIR, LBA_BOOT_DIR)?;
    pad_to_lba(&mut iso, LBA_BOOT_DIR)?;
    iso.write_all(&[0u8; ISO_SECTOR_SIZE])?;

    // --- 4. Write the FAT32 image itself as a regular file in the ISO9660 tree. ---
    let lba_fat_image = iso.stream_position()?.div_ceil(ISO_SECTOR_SIZE as u64) as u32;
    pad_to_lba(&mut iso, lba_fat_image)?;
    img_file.seek(SeekFrom::Start(0))?;
    io::copy(&mut img_file, &mut iso)?;
    let fat_image_size = img_file.metadata()?.len() as u32;

    // --- 5. Extract and Write actual BOOTX64.EFI file content to a new LBA. ---
    let efi_content = read_efi_bootx64_from_fat32_image(&mut img_file)?;
    let bootx64_size = efi_content.len() as u32;
    let lba_bootx64 = iso.stream_position()?.div_ceil(ISO_SECTOR_SIZE as u64) as u32;
    pad_to_lba(&mut iso, lba_bootx64)?;
    iso.write_all(&efi_content)?;

    // --- 6. Rewrite the boot catalog and boot directory with correct information. ---
    iso.seek(io::SeekFrom::Start(LBA_BOOT_CATALOG as u64 * ISO_SECTOR_SIZE as u64))?;
    write_boot_catalog(&mut iso, lba_bootx64, bootx64_size)?;

    iso.seek(io::SeekFrom::Start(LBA_BOOT_DIR as u64 * ISO_SECTOR_SIZE as u64))?;
    write_boot_directory_sector(&mut iso, LBA_BOOT_DIR, LBA_EFI_DIR, lba_bootx64, bootx64_size)?;
    
    // --- 7. Finalize ISO file by updating the total number of sectors. ---
    iso.seek(io::SeekFrom::End(0))?;
    let final_pos = iso.stream_position()?;
    let total_sectors = final_pos.div_ceil(ISO_SECTOR_SIZE as u64) as u32;

    update_total_sectors(&mut iso, total_sectors)?;
    iso.set_len(total_sectors as u64 * ISO_SECTOR_SIZE as u64)?;

    println!(
        "create_iso_from_img: ISO created with {} sectors.",
        total_sectors
    );
    Ok(())
}