// isobemak/src/iso.rs
// ISO + El Torito
use crate::utils::{pad_to_lba, update_4byte_fields, ISO_SECTOR_SIZE};
use std::{
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    path::Path,
};

// Constants for ISO 9660 structure.
const ISO_VOLUME_DESCRIPTOR_TERMINATOR: u8 = 255;
const ISO_VOLUME_DESCRIPTOR_PRIMARY: u8 = 1;
const ISO_VOLUME_DESCRIPTOR_BOOT_RECORD: u8 = 0;
const ISO_ID: &[u8] = b"CD001";
const ISO_VERSION: u8 = 1;
const PVD_VOLUME_ID_OFFSET: usize = 40;
const PVD_TOTAL_SECTORS_OFFSET: usize = 80;
const PVD_ROOT_DIR_RECORD_OFFSET: usize = 156;

// Constants for PVD fields
const PVD_VOL_SET_SIZE_OFFSET: usize = 120;
const PVD_VOL_SEQ_NUM_OFFSET: usize = 124;
const PVD_LOGICAL_BLOCK_SIZE_OFFSET: usize = 128;
const PVD_PATH_TABLE_SIZE_OFFSET: usize = 132;

// Constants for El Torito boot catalog.
const LBA_BOOT_CATALOG: u32 = 19;
const BOOT_CATALOG_HEADER_SIGNATURE: u16 = 0xAA55;
const BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID: u8 = 1;
const BOOT_CATALOG_BOOT_ENTRY_HEADER_ID: u8 = 0x88;
const BOOT_CATALOG_EFI_PLATFORM_ID: u8 = 0xEF;

// New constants for Boot Catalog
const ID_FIELD_OFFSET: usize = 4;
const BOOT_CATALOG_CHECKSUM_OFFSET: usize = 28;
const BOOT_CATALOG_VALIDATION_SIGNATURE_OFFSET: usize = 30;

// New LBA assignments for ISO 9660 structure
const LBA_PVD: u32 = 16;
const LBA_BRVD: u32 = 17;
const LBA_VDT: u32 = 18;
const LBA_ROOT_DIR: u32 = 20;
const LBA_EFI_DIR: u32 = 21;
const LBA_BOOT_DIR: u32 = 22;
const LBA_BOOT_IMG: u32 = 23; // Start of the embedded bootable image

/// Creates an ISO 9660 directory record.
fn create_iso9660_dir_record(
    lba: u32,
    data_len: u32,
    flags: u8,
    file_id_str: &str,
) -> Vec<u8> {
    let file_id_vec: Vec<u8>;
    let file_id_bytes: &[u8];
    let actual_file_id_len: u8;

    match file_id_str {
        "." => {
            file_id_bytes = b"\x00";
            actual_file_id_len = 1;
        },
        ".." => {
            file_id_bytes = b"\x01";
            actual_file_id_len = 1;
        },
        _ => {
            if flags & 0x02 != 0 { // Directory
                file_id_bytes = file_id_str.as_bytes();
                actual_file_id_len = file_id_str.len() as u8;
            } else { // File
                let mut name = file_id_str.to_string();
                if !name.contains('.') {
                    name.push_str(".1");
                }
                file_id_vec = name.into_bytes();
                file_id_bytes = &file_id_vec;
                actual_file_id_len = file_id_vec.len() as u8;
            }
        }
    };
    // Directory records must be an even length.
    let record_len = 33 + actual_file_id_len + (actual_file_id_len % 2);
    let mut record = vec![0u8; record_len as usize];

    record[0] = record_len; // Length of Directory Record
    record[1] = 0; // Extended Attribute Record Length
    record[2..6].copy_from_slice(&lba.to_le_bytes()); // LBA (LE)
    record[6..10].copy_from_slice(&lba.to_be_bytes()); // LBA (BE)
    record[10..14].copy_from_slice(&data_len.to_le_bytes()); // Data Length (LE)
    record[14..18].copy_from_slice(&data_len.to_be_bytes()); // Data Length (BE)
    // Recording Date and Time (7 bytes, all zeros for simplicity)
    record[25] = flags; // File Flags
    record[26] = 0; // File Unit Size
    record[27] = 0; // Interleave Gap Size
    record[28..30].copy_from_slice(&1u16.to_le_bytes()); // Volume Sequence Number (LE)
    record[30..32].copy_from_slice(&1u16.to_be_bytes()); // Volume Sequence Number (BE)
    record[32] = actual_file_id_len; // Length of File Identifier
    record[33..33 + actual_file_id_len as usize].copy_from_slice(file_id_bytes);

    record
}

fn write_primary_volume_descriptor(
    iso: &mut File,
    total_sectors: u32,
    root_dir_lba: u32,
    root_dir_size: u32,
) -> io::Result<()> {
    pad_to_lba(iso, LBA_PVD)?;
    let mut pvd = [0u8; ISO_SECTOR_SIZE];
    pvd[0] = ISO_VOLUME_DESCRIPTOR_PRIMARY;
    pvd[1..6].copy_from_slice(ISO_ID);
    pvd[6] = ISO_VERSION;

    let project_name = b"ISOBEMAKI";
    let mut volume_id = [b' '; 32];
    volume_id[..project_name.len()].copy_from_slice(project_name);
    pvd[PVD_VOLUME_ID_OFFSET..PVD_VOLUME_ID_OFFSET + 32].copy_from_slice(&volume_id);

    // Total sectors will be updated later, so we write 0 for now.
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

    // Root directory record for the ISO9660 filesystem
    let mut root_dir_record = [0u8; 34];
    root_dir_record[0] = 34; // Directory Record Length
    root_dir_record[2..6].copy_from_slice(&root_dir_lba.to_le_bytes()); // LBA of root directory (LE)
    root_dir_record[6..10].copy_from_slice(&root_dir_lba.to_be_bytes()); // LBA of root directory (BE)
    root_dir_record[10..14].copy_from_slice(&root_dir_size.to_le_bytes()); // Data Length (LE)
    root_dir_record[14..18].copy_from_slice(&root_dir_size.to_be_bytes()); // Data Length (BE)
    root_dir_record[25] = 2; // Directory flag
    root_dir_record[32] = 1; // File ID Length
    root_dir_record[33] = 0; // File ID (0x00 for root)

    pvd[PVD_ROOT_DIR_RECORD_OFFSET..PVD_ROOT_DIR_RECORD_OFFSET + 34]
        .copy_from_slice(&root_dir_record);
    iso.write_all(&pvd)
}

fn write_boot_record_volume_descriptor(iso: &mut File, lba_boot_catalog: u32) -> io::Result<()> {
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
    pad_to_lba(iso, LBA_VDT)?;
    let mut term = [0u8; ISO_SECTOR_SIZE];
    term[0] = ISO_VOLUME_DESCRIPTOR_TERMINATOR;
    term[1..6].copy_from_slice(ISO_ID);
    term[6] = ISO_VERSION;
    iso.write_all(&term)
}

/// Correctly writes the El Torito boot catalog for UEFI.
/// It takes the LBA and sector count of the FAT32 boot image.
fn write_boot_catalog(iso: &mut File, boot_img_lba: u32, boot_img_sectors: u16) -> io::Result<()> {
    pad_to_lba(iso, LBA_BOOT_CATALOG)?;
    let mut cat = [0u8; ISO_SECTOR_SIZE];

    // --- Validation Entry (32 bytes) ---
    cat[0] = BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID; // Header ID
    cat[1] = BOOT_CATALOG_EFI_PLATFORM_ID; // Platform ID (0xEF for UEFI)
    cat[2..4].copy_from_slice(&[0; 2]); // Reserved

    // ID string
    cat[ID_FIELD_OFFSET..ID_FIELD_OFFSET + 4].copy_from_slice(b"UEFI");
    cat[BOOT_CATALOG_VALIDATION_SIGNATURE_OFFSET..BOOT_CATALOG_VALIDATION_SIGNATURE_OFFSET + 2]
        .copy_from_slice(&BOOT_CATALOG_HEADER_SIGNATURE.to_le_bytes());

    // Calculate checksum
    let mut sum: u16 = 0;
    for i in (0..16).step_by(2) {
        sum = sum.wrapping_add(u16::from_le_bytes([cat[i], cat[i + 1]]));
    }
    let checksum = 0u16.wrapping_sub(sum);
    cat[BOOT_CATALOG_CHECKSUM_OFFSET..BOOT_CATALOG_CHECKSUM_OFFSET + 2]
        .copy_from_slice(&checksum.to_le_bytes());

    // --- Initial/Default Boot Entry (32 bytes) ---
    let mut entry = [0u8; 32];
    entry[0] = BOOT_CATALOG_BOOT_ENTRY_HEADER_ID; // Bootable entry
    entry[1] = 0x00; // Boot media type: No emulation
    entry[2..4].copy_from_slice(&[0; 2]); // Load segment
    entry[4] = 0x00; // System type
    entry[5] = 0x00; // Unused
    entry[6..8].copy_from_slice(&boot_img_sectors.to_le_bytes()); // Number of 512-byte sectors to load
    entry[8..12].copy_from_slice(&boot_img_lba.to_le_bytes()); // LBA for the boot image
    entry[12..].copy_from_slice(&[0; 20]); // Unused

    // Write the boot entry into the catalog after the validation entry.
    cat[32..64].copy_from_slice(&entry);

    iso.write_all(&cat)
}

/// Creates an ISO image from a bootable image file.
pub fn create_iso_from_img(iso_path: &Path, boot_img_path: &Path) -> io::Result<()> {
    println!("create_iso_from_img: Starting creation of ISO.");

    // 1. Get the size of the boot image
    let boot_img_metadata = std::fs::metadata(boot_img_path)?;
    let boot_img_sectors = (boot_img_metadata.len() as u32 + 511) / 512;
    let boot_img_size = boot_img_metadata.len();
    
    let mut iso = File::create(iso_path)?;

    // 2. Write Volume Descriptors (PVD, BRVD, VDT)
    // We'll update total_sectors later
    write_primary_volume_descriptor(&mut iso, 0, LBA_ROOT_DIR, ISO_SECTOR_SIZE as u32)?;
    write_boot_record_volume_descriptor(&mut iso, LBA_BOOT_CATALOG)?;
    write_volume_descriptor_terminator(&mut iso)?;

    // 3. Write the boot catalog (LBA 19)
    write_boot_catalog(
        &mut iso,
        LBA_BOOT_IMG,
        boot_img_sectors as u16,
    )?;

    // 4. Construct ISO 9660 directory structure
    // Root Directory (LBA 20)
    pad_to_lba(&mut iso, LBA_ROOT_DIR)?;
    let mut root_dir_content = Vec::new();
    root_dir_content.extend_from_slice(&create_iso9660_dir_record(LBA_ROOT_DIR, ISO_SECTOR_SIZE as u32, 0x02, "."));
    root_dir_content.extend_from_slice(&create_iso9660_dir_record(LBA_ROOT_DIR, ISO_SECTOR_SIZE as u32, 0x02, ".."));

    root_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_EFI_DIR,
        ISO_SECTOR_SIZE as u32,
        0x02,
        "EFI",
    ));
    root_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_BOOT_CATALOG,
        ISO_SECTOR_SIZE as u32,
        0x00,
        "BOOT.CATALOG",
    ));

    root_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&root_dir_content)?;

    // EFI Directory (LBA 21)
    pad_to_lba(&mut iso, LBA_EFI_DIR)?;
    let mut efi_dir_content = Vec::new();
    efi_dir_content.extend_from_slice(&create_iso9660_dir_record(LBA_EFI_DIR, ISO_SECTOR_SIZE as u32, 0x02, "."));
    efi_dir_content.extend_from_slice(&create_iso9660_dir_record(LBA_ROOT_DIR, ISO_SECTOR_SIZE as u32, 0x02, ".."));

    efi_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_BOOT_DIR,
        ISO_SECTOR_SIZE as u32,
        0x02,
        "BOOT",
    ));

    efi_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&efi_dir_content)?;

    // BOOT Directory (LBA 22)
    pad_to_lba(&mut iso, LBA_BOOT_DIR)?;
    let mut boot_dir_content = Vec::new();
    boot_dir_content.extend_from_slice(&create_iso9660_dir_record(LBA_BOOT_DIR, ISO_SECTOR_SIZE as u32, 0x02, "."));
    boot_dir_content.extend_from_slice(&create_iso9660_dir_record(LBA_EFI_DIR, ISO_SECTOR_SIZE as u32, 0x02, ".."));

    boot_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_BOOT_IMG,
        boot_img_size as u32,
        0x00,
        "BOOTX64.EFI",
    ));

    boot_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&boot_dir_content)?;

    // 5. Write the bootable image content (LBA 23 onwards)
    pad_to_lba(&mut iso, LBA_BOOT_IMG)?;
    let mut boot_img_file = File::open(boot_img_path)?;
    io::copy(&mut boot_img_file, &mut iso)?;

    // 6. Finalize ISO file by updating the total number of sectors.
    let final_pos = iso.stream_position()?;
    let total_sectors = (final_pos as f64 / ISO_SECTOR_SIZE as f64).ceil() as u32;

    update_4byte_fields(
        &mut iso,
        LBA_PVD,
        PVD_TOTAL_SECTORS_OFFSET,
        PVD_TOTAL_SECTORS_OFFSET + 4,
        total_sectors,
    )?;

    iso.set_len(total_sectors as u64 * ISO_SECTOR_SIZE as u64)?;

    println!("create_iso_from_img: ISO created with {} sectors.", total_sectors);
    Ok(())
}