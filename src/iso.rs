// isobemak/src/iso.rs
// ISO + El Torito
use crate::utils::ISO_SECTOR_SIZE;
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
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

// Constants for FAT32
const FAT32_SECTOR_SIZE: u16 = 512;
const FAT32_OEM_NAME: &[u8] = b"MSWIN4.1";
const FAT32_VOLUME_LABEL: &[u8] = b"NO NAME    ";
const FAT32_EFI_FILE_NAME: &[u8] = b"BOOTX64 EFI";
const FAT32_EFI_FILE_LEN: u8 = 11;

// New LBA assignments for ISO 9660 structure
const LBA_ROOT_DIR: u32 = 20;
const LBA_EFI_DIR: u32 = 21;
const LBA_BOOT_DIR: u32 = 22;
const LBA_BOOTX64_EFI_FILE: u32 = 23; // This will be the start of the FAT32 image

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

/// Creates an ISO 9660 directory record.
fn create_iso9660_dir_record(
    lba: u32,
    data_len: u32,
    flags: u8,
    file_id_str: &str, // Changed to &str for easier handling of "." and ".."
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
                file_id_vec = format!("{}.1", file_id_str).into_bytes();
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
    // Year (offset 0): 0 (0-99, 0 for 1900)
    // Month (offset 1): 0 (1-12)
    // Day (offset 2): 0 (1-31)
    // Hour (offset 3): 0 (0-23)
    // Minute (offset 4): 0 (0-59)
    // Second (offset 5): 0 (0-59)
    // Offset from GMT (offset 6): 0 (in 15-minute intervals, -48 to +52)
    // For simplicity, we'll leave these as zeros.
    record[25] = flags; // File Flags
    record[26] = 0; // File Unit Size
    record[27] = 0; // Interleave Gap Size
    record[28..30].copy_from_slice(&1u16.to_le_bytes()); // Volume Sequence Number (LE)
    record[30..32].copy_from_slice(&1u16.to_be_bytes()); // Volume Sequence Number (BE)
    record[32] = actual_file_id_len; // Length of File Identifier
    record[33..33 + actual_file_id_len as usize].copy_from_slice(file_id_bytes);

    record
}

/// Creates a minimal FAT32 partition image containing EFI/BOOT/BOOTX64.EFI.
fn create_fat32_image(efi_content: &[u8]) -> io::Result<Vec<u8>> {
    // Constants for FAT32 structure
    const CLUSTER_SIZE_SECTORS: u32 = 1; // 1 sector per cluster
    const RESERVED_SECTORS_FAT32: u32 = 32; // Typically 32 for FAT32
    const NUM_FATS: u32 = 2;
    const ROOT_DIR_CLUSTER: u32 = 2; // First data cluster is 2

    // Calculate sizes
    let efi_content_len = efi_content.len() as u32;
    let efi_data_sectors =
        (efi_content_len + FAT32_SECTOR_SIZE as u32 - 1) / FAT32_SECTOR_SIZE as u32; // ceil division

    // Directories will each take one cluster (one sector)
    let num_dir_clusters = 3; // Root, EFI, BOOT

    // Total data clusters (directories + efi content)
    let total_data_clusters = num_dir_clusters + efi_data_sectors;

    // Calculate FAT size
    let fat_entries_per_sector = FAT32_SECTOR_SIZE as u32 / 4;
    let num_clusters_in_fat = ROOT_DIR_CLUSTER + num_dir_clusters + efi_data_sectors;
    let fat_sectors_per_fat =
        (num_clusters_in_fat + fat_entries_per_sector - 1) / fat_entries_per_sector;

    // Total image sectors
    let total_image_sectors = RESERVED_SECTORS_FAT32 // Boot Sector + FSInfo + Backup Boot Sector + other reserved
        + NUM_FATS * fat_sectors_per_fat // Two FATs
        + total_data_clusters; // Data area (directories + EFI content)

    let mut image = vec![0u8; (total_image_sectors * FAT32_SECTOR_SIZE as u32) as usize];

    // Offsets in sectors (LBA)
    let boot_sector_lba = 0;
    let fsinfo_lba = 1; // FSInfo sector is typically LBA 1
    let backup_boot_sector_lba = 6; // Backup Boot Sector is typically LBA 6

    let fat1_lba = RESERVED_SECTORS_FAT32; // FAT1 starts after all reserved sectors
    let fat2_lba = fat1_lba + fat_sectors_per_fat;
    let data_area_lba = fat2_lba + fat_sectors_per_fat;

    // Write Boot Sector
    let mut boot_sector = [0u8; FAT32_SECTOR_SIZE as usize];
    boot_sector[0..3].copy_from_slice(&[0xeb, 0x58, 0x90]); // BS_JmpBoot
    boot_sector[3..11].copy_from_slice(FAT32_OEM_NAME); // BS_OEMName
    boot_sector[11..13].copy_from_slice(&FAT32_SECTOR_SIZE.to_le_bytes()); // BPB_BytsPerSec
    boot_sector[13] = CLUSTER_SIZE_SECTORS as u8; // BPB_SecPerClus
    boot_sector[14..16].copy_from_slice(&(RESERVED_SECTORS_FAT32 as u16).to_le_bytes()); // BPB_RsvdSecCnt (32 for FAT32)
    boot_sector[16] = NUM_FATS as u8; // BPB_NumFATs
    boot_sector[17..19].copy_from_slice(&0u16.to_le_bytes()); // BPB_RootEntCnt (0 for FAT32)
    boot_sector[19..21].copy_from_slice(&0u16.to_le_bytes()); // BPB_TotSec16 (0 for FAT32)
    boot_sector[21] = 0xf8; // BPB_Media
    boot_sector[22..24].copy_from_slice(&0u16.to_le_bytes()); // BPB_FATSz16 (0 for FAT32)
    boot_sector[24..26].copy_from_slice(&0u16.to_le_bytes()); // BPB_SecPerTrk
    boot_sector[26..28].copy_from_slice(&0u16.to_le_bytes()); // BPB_NumHeads
    boot_sector[28..32].copy_from_slice(&0u32.to_le_bytes()); // BPB_HiddSec

    boot_sector[32..36].copy_from_slice(&total_image_sectors.to_le_bytes()); // BPB_TotSec32
    boot_sector[36..40].copy_from_slice(&fat_sectors_per_fat.to_le_bytes()); // BPB_FATSz32
    boot_sector[40..42].copy_from_slice(&0u16.to_le_bytes()); // BPB_ExtFlags (0x0000)
    boot_sector[42..44].copy_from_slice(&0u16.to_le_bytes()); // BPB_FSVer (0x0000)
    boot_sector[44..48].copy_from_slice(&ROOT_DIR_CLUSTER.to_le_bytes()); // BPB_RootClus
    boot_sector[48..50].copy_from_slice(&(fsinfo_lba as u16).to_le_bytes()); // BPB_FSInfo (LBA 1)
    boot_sector[50..52].copy_from_slice(&(backup_boot_sector_lba as u16).to_le_bytes()); // BPB_BkBootSec (LBA 6)
    boot_sector[52..64].copy_from_slice(&[0u8; 12]); // BPB_Reserved

    boot_sector[64] = 0x80; // BS_DrvNum
    boot_sector[65] = 0x00; // BS_Reserved1
    boot_sector[66] = 0x29; // BS_BootSig (Extended boot signature)
    boot_sector[67..71].copy_from_slice(&0x12345678u32.to_le_bytes()); // BS_VolID (Fixed for reproducibility)
    boot_sector[71..82].copy_from_slice(FAT32_VOLUME_LABEL); // BS_VolLab
    boot_sector[82..90].copy_from_slice(b"FAT32   "); // BS_FilSysType
    boot_sector[510..512].copy_from_slice(&0xAA55u16.to_le_bytes()); // Boot signature
    image[(boot_sector_lba * FAT32_SECTOR_SIZE as u32) as usize..][..FAT32_SECTOR_SIZE as usize]
        .copy_from_slice(&boot_sector);

    // Write FSInfo Sector
    let mut fsinfo_sector = [0u8; FAT32_SECTOR_SIZE as usize];
    fsinfo_sector[0..4].copy_from_slice(&0x41615252u32.to_le_bytes()); // FSI_LeadSig
    fsinfo_sector[484..488].copy_from_slice(&0x61417272u32.to_le_bytes()); // FSI_StrucSig
    fsinfo_sector[488..492].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes()); // FSI_Free_Clus (unknown)
    fsinfo_sector[492..496].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes()); // FSI_Nxt_Free (unknown)
    fsinfo_sector[510..512].copy_from_slice(&0xAA55u16.to_le_bytes()); // FSI_Sig
    image[(fsinfo_lba * FAT32_SECTOR_SIZE as u32) as usize..][..FAT32_SECTOR_SIZE as usize]
        .copy_from_slice(&fsinfo_sector);

    // Write Backup Boot Sector (a copy of the main boot sector)
    image[(backup_boot_sector_lba * FAT32_SECTOR_SIZE as u32) as usize..]
        [..FAT32_SECTOR_SIZE as usize]
        .copy_from_slice(&boot_sector);

    // Write FAT tables
    let mut fat_content = vec![0u8; (fat_sectors_per_fat * FAT32_SECTOR_SIZE as u32) as usize];
    // Cluster 0: Media type
    fat_content[0..4].copy_from_slice(&0x0FFFFF8u32.to_le_bytes());
    // Cluster 1: EOC
    fat_content[4..8].copy_from_slice(&0x0FFFFFFFu32.to_le_bytes());
    // Cluster 2 (Root Dir) -> Cluster 3 (EFI Dir)
    fat_content[8..12].copy_from_slice(&3u32.to_le_bytes());
    // Cluster 3 (EFI Dir) -> Cluster 4 (BOOT Dir)
    fat_content[12..16].copy_from_slice(&4u32.to_le_bytes());
    // Cluster 4 (BOOT Dir) -> Cluster 5 (EFI Content)
    fat_content[16..20].copy_from_slice(&5u32.to_le_bytes());

    // EFI Content cluster chain
    for i in 0..efi_data_sectors {
        let current_cluster = ROOT_DIR_CLUSTER + num_dir_clusters + i; // Cluster 5 onwards
        let next_cluster = if i == efi_data_sectors - 1 {
            0x0FFFFFFF // End of chain
        } else {
            current_cluster + 1
        };
        fat_content[(current_cluster * 4) as usize..(current_cluster * 4 + 4) as usize]
            .copy_from_slice(&next_cluster.to_le_bytes());
    }

    image[(fat1_lba * FAT32_SECTOR_SIZE as u32) as usize..][..fat_content.len()]
        .copy_from_slice(&fat_content);
    image[(fat2_lba * FAT32_SECTOR_SIZE as u32) as usize..][..fat_content.len()]
        .copy_from_slice(&fat_content);

    // Write Root Directory Entry for EFI
    let root_dir_offset = (data_area_lba * FAT32_SECTOR_SIZE as u32) as usize;
    let mut efi_dir_entry = [0u8; 32];
    efi_dir_entry[0..8].copy_from_slice(b"EFI     ");
    efi_dir_entry[8] = 0x10; // Directory attribute
    efi_dir_entry[26..28].copy_from_slice(&3u16.to_le_bytes()); // First cluster of EFI dir (Cluster 3)
    image[root_dir_offset..root_dir_offset + 32].copy_from_slice(&efi_dir_entry);

    // Write EFI Directory Entry for BOOT
    let efi_dir_offset = ((data_area_lba + 1) * FAT32_SECTOR_SIZE as u32) as usize; // LBA 4
    let mut boot_dir_entry = [0u8; 32];
    boot_dir_entry[0..8].copy_from_slice(b"BOOT    ");
    boot_dir_entry[8] = 0x10; // Directory attribute
    boot_dir_entry[26..28].copy_from_slice(&4u16.to_le_bytes()); // First cluster of BOOT dir (Cluster 4)
    image[efi_dir_offset..efi_dir_offset + 32].copy_from_slice(&boot_dir_entry);

    // Write BOOT Directory Entry for BOOTX64.EFI
    let boot_dir_offset = ((data_area_lba + 2) * FAT32_SECTOR_SIZE as u32) as usize; // LBA 5
    let mut bootx64_efi_entry = [0u8; 32];
    bootx64_efi_entry[0..8].copy_from_slice(b"BOOTX64 ");
    bootx64_efi_entry[8..11].copy_from_slice(b"EFI");
    bootx64_efi_entry[26..28].copy_from_slice(&5u16.to_le_bytes()); // First cluster of BOOTX64.EFI (Cluster 5)
    bootx64_efi_entry[28..32].copy_from_slice(&efi_content_len.to_le_bytes());
    image[boot_dir_offset..boot_dir_offset + 32].copy_from_slice(&bootx64_efi_entry);

    // Write BOOTX64.EFI file content
    let efi_content_offset = ((data_area_lba + 3) * FAT32_SECTOR_SIZE as u32) as usize; // LBA 6
    image[efi_content_offset..efi_content_offset + efi_content_len as usize]
        .copy_from_slice(efi_content);

    Ok(image)
}

fn write_primary_volume_descriptor(
    iso: &mut File,
    total_sectors: u32,
    root_dir_lba: u32,
    root_dir_size: u32,
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

/// Reads the entire file from a specified path and returns its content.
fn read_file_from_path(file_path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(file_path)?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    Ok(content)
}

/// Creates an ISO image from an EFI image file.
pub fn create_iso_from_img(iso_path: &Path, efi_img_path: &Path) -> io::Result<()> {
    println!("create_iso_from_img: Starting creation of ISO.");

    // 1. Create a FAT32 partition image containing BOOTX64.EFI
    let efi_content = read_file_from_path(efi_img_path)?;
    let fat32_image = create_fat32_image(&efi_content)?;
    let _fat32_sectors = (fat32_image.len() / ISO_SECTOR_SIZE) as u32; // Unused, prefix with _

    let mut iso = File::create(iso_path)?;

    // 2. Write Volume Descriptors (PVD, BRVD, VDT)
    // We'll update total_sectors later
    write_primary_volume_descriptor(&mut iso, 0, LBA_ROOT_DIR, ISO_SECTOR_SIZE as u32)?;
    write_boot_record_volume_descriptor(&mut iso, LBA_BOOT_CATALOG)?;
    write_volume_descriptor_terminator(&mut iso)?;

    // 3. Write the boot catalog (LBA 19)
    write_boot_catalog(
        &mut iso,
        LBA_BOOTX64_EFI_FILE,
        (fat32_image.len() / FAT32_SECTOR_SIZE as usize) as u16,
    )?;

    // 4. Construct ISO 9660 directory structure
    // Root Directory (LBA 20)
    pad_to_lba(&mut iso, LBA_ROOT_DIR)?;
    let mut root_dir_content = Vec::new();
    // Self-reference ('.')
    root_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_ROOT_DIR,
        ISO_SECTOR_SIZE as u32,
        0x02,
        ".",
    ));
    // Parent-reference ('..')
    root_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_ROOT_DIR,
        ISO_SECTOR_SIZE as u32,
        0x02,
        "..",
    )); // Root's parent is itself

    // EFI Directory entry in Root
    root_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_EFI_DIR,
        ISO_SECTOR_SIZE as u32,
        0x02,
        "EFI",
    ));
    // BOOT.CATALOG file entry in Root
    root_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_BOOT_CATALOG,
        ISO_SECTOR_SIZE as u32, // BOOT.CATALOG is one sector
        0x00,
        "BOOT.CATALOG",
    ));

    // Pad root_dir_content to fill a sector
    root_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&root_dir_content)?;

    // EFI Directory (LBA 21)
    pad_to_lba(&mut iso, LBA_EFI_DIR)?;
    let mut efi_dir_content = Vec::new();
    // Self-reference ('.')
    efi_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_EFI_DIR,
        ISO_SECTOR_SIZE as u32,
        0x02,
        ".",
    ));
    // Parent-reference ('..')
    efi_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_ROOT_DIR,
        ISO_SECTOR_SIZE as u32,
        0x02,
        "..",
    ));

    // BOOT Directory entry in EFI
    efi_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_BOOT_DIR,
        ISO_SECTOR_SIZE as u32,
        0x02,
        "BOOT",
    ));

    // Pad efi_dir_content to fill a sector
    efi_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&efi_dir_content)?;

    // BOOT Directory (LBA 22)
    pad_to_lba(&mut iso, LBA_BOOT_DIR)?;
    let mut boot_dir_content = Vec::new();
    // Self-reference ('.')
    boot_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_BOOT_DIR,
        ISO_SECTOR_SIZE as u32,
        0x02,
        ".",
    ));
    // Parent-reference ('..')
    boot_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_EFI_DIR,
        ISO_SECTOR_SIZE as u32,
        0x02,
        "..",
    ));

    // BOOTX64.EFI file entry in BOOT
    boot_dir_content.extend_from_slice(&create_iso9660_dir_record(
        LBA_BOOTX64_EFI_FILE,
        efi_content.len() as u32,
        0x00,
        "BOOTX64.EFI",
    ));

    // Pad boot_dir_content to fill a sector
    boot_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&boot_dir_content)?;

    // 5. Write the FAT32 image content (LBA 23 onwards)
    pad_to_lba(&mut iso, LBA_BOOTX64_EFI_FILE)?;
    iso.write_all(&fat32_image)?;

    // 6. Finalize ISO file by updating the total number of sectors.
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
