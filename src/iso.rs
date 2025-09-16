// isobemak/src/iso.rs
// ISO + El Torito
use crate::utils::{FAT32_SECTOR_SIZE, ISO_SECTOR_SIZE, pad_sector};
use std::{
    fs::File,
    io::{self, Read, Seek, Write},
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

// Constants for El Torito boot catalog.
const BOOT_CATALOG_HEADER_SIGNATURE: u16 = 0xAA55;
const BOOT_CATALOG_VALIDATION_ENTRY_HEADER_ID: u8 = 1;
const BOOT_CATALOG_BOOT_ENTRY_HEADER_ID: u8 = 0x88;
const BOOT_CATALOG_NO_EMULATION: u8 = 0x00;
const BOOT_CATALOG_EFI_PLATFORM_ID: u8 = 0xEF;

/// Pads the ISO file with zeros to align to a specific LBA.
/// This helper function reduces code duplication in the main logic.
fn pad_to_lba(iso: &mut File, lba: u32) -> io::Result<()> {
    let target_pos = lba as u64 * ISO_SECTOR_SIZE as u64;
    let current_pos = iso.stream_position()?;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        // Using io::copy with a repeat reader is efficient for writing large amounts of zeros.
        io::copy(&mut io::repeat(0).take(padding_bytes), iso)?;
    }
    Ok(())
}

fn write_root_directory_sector(iso: &mut File, root_dir_lba: u32) -> io::Result<()> {
    pad_to_lba(iso, root_dir_lba)?;
    let mut root_dir_sector = [0u8; ISO_SECTOR_SIZE];
    
    // . (self) directory record
    let self_dir_record_len = 34;
    root_dir_sector[0] = self_dir_record_len;
    root_dir_sector[2..6].copy_from_slice(&root_dir_lba.to_le_bytes());
    root_dir_sector[6..10].copy_from_slice(&root_dir_lba.to_be_bytes());
    let sector_size_u32 = ISO_SECTOR_SIZE as u32;
    root_dir_sector[10..14].copy_from_slice(&sector_size_u32.to_le_bytes());
    root_dir_sector[14..18].copy_from_slice(&sector_size_u32.to_be_bytes());
    root_dir_sector[25] = 2; // File flags: 2 for directory
    let vol_seq: u16 = 1;
    root_dir_sector[28..30].copy_from_slice(&vol_seq.to_le_bytes());
    root_dir_sector[30..32].copy_from_slice(&vol_seq.to_be_bytes());
    root_dir_sector[32] = 1; // Length of File Identifier
    root_dir_sector[33] = 0; // File Identifier: 0x00 for self

    // .. (parent) directory record
    let parent_dir_record_len = 34;
    let offset = self_dir_record_len as usize;
    root_dir_sector[offset] = parent_dir_record_len;
    root_dir_sector[offset + 2..offset + 6].copy_from_slice(&root_dir_lba.to_le_bytes());
    root_dir_sector[offset + 6..offset + 10].copy_from_slice(&root_dir_lba.to_be_bytes());
    root_dir_sector[offset + 10..offset + 14].copy_from_slice(&sector_size_u32.to_le_bytes());
    root_dir_sector[offset + 14..offset + 18].copy_from_slice(&sector_size_u32.to_be_bytes());
    root_dir_sector[offset + 25] = 2;
    root_dir_sector[offset + 28..offset + 30].copy_from_slice(&vol_seq.to_le_bytes());
    root_dir_sector[offset + 30..offset + 32].copy_from_slice(&vol_seq.to_be_bytes());
    root_dir_sector[offset + 32] = 1;
    root_dir_sector[offset + 33] = 1; // File Identifier: 0x01 for parent

    iso.write_all(&root_dir_sector)
}

fn write_primary_volume_descriptor(iso: &mut File, total_sectors: u32, root_dir_lba: u32) -> io::Result<()> {
    const LBA_PVD: u32 = 16;
    pad_to_lba(iso, LBA_PVD)?;
    let mut pvd = [0u8; ISO_SECTOR_SIZE];
    pvd[0] = ISO_VOLUME_DESCRIPTOR_PRIMARY;
    pvd[1..6].copy_from_slice(ISO_ID);
    pvd[6] = ISO_VERSION;

    let project_name = b"FULLERENE";
    let mut volume_id = [b' '; 32];
    volume_id[..project_name.len()].copy_from_slice(project_name);
    pvd[PVD_VOLUME_ID_OFFSET..PVD_VOLUME_ID_OFFSET + 32].copy_from_slice(&volume_id);

    // FIX: ISO9660 multi-endian fields
    let total = total_sectors as u32;
    pvd[80..84].copy_from_slice(&total.to_le_bytes()); // total_sectors (LE)
    pvd[84..88].copy_from_slice(&total.to_be_bytes()); // total_sectors (BE)

    let vol_set_size: u16 = 1;
    pvd[120..122].copy_from_slice(&vol_set_size.to_le_bytes()); // Volume Set Size (LE)
    pvd[122..124].copy_from_slice(&vol_set_size.to_be_bytes()); // Volume Set Size (BE)

    let vol_seq_num: u16 = 1;
    pvd[124..126].copy_from_slice(&vol_seq_num.to_le_bytes()); // Volume Sequence Number (LE)
    pvd[126..128].copy_from_slice(&vol_seq_num.to_be_bytes()); // Volume Sequence Number (BE)
        
    let sector_size_u16 = ISO_SECTOR_SIZE as u16;
    pvd[128..130].copy_from_slice(&sector_size_u16.to_le_bytes()); // Logical Block Size (LE)
    pvd[130..132].copy_from_slice(&sector_size_u16.to_be_bytes()); // Logical Block Size (BE)

    // Path table fields. Set to 0 but must be multi-endian.
    let path_table_size: u32 = 0;
    pvd[132..136].copy_from_slice(&path_table_size.to_le_bytes()); // Path Table Size (LE)
    pvd[136..140].copy_from_slice(&path_table_size.to_be_bytes()); // Path Table Size (BE)


    // Root directory record
    let mut root_dir_record = [0u8; 34];
    root_dir_record[0] = 34; // Directory record length

    // Location of extent (LBA of the root directory sector)
    let root_dir_lba_u32 = root_dir_lba as u32;
    root_dir_record[2..6].copy_from_slice(&root_dir_lba_u32.to_le_bytes());
    root_dir_record[6..10].copy_from_slice(&root_dir_lba_u32.to_be_bytes());

    // Data length (size of one sector for the root directory contents)
    let sector_size_u32 = ISO_SECTOR_SIZE as u32;
    root_dir_record[10..14].copy_from_slice(&sector_size_u32.to_le_bytes());
    root_dir_record[14..18].copy_from_slice(&sector_size_u32.to_be_bytes());

    // File flags (0x02 for directory)
    root_dir_record[25] = 2;

    // Volume sequence number (usually 1)
    let vol_seq: u16 = 1;
    root_dir_record[28..30].copy_from_slice(&vol_seq.to_le_bytes());
    root_dir_record[30..32].copy_from_slice(&vol_seq.to_be_bytes());

    // Length of File Identifier (1 for root)
    root_dir_record[32] = 1;
    // File Identifier (0x00 for root)
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
    cat[4..28].copy_from_slice(&[0; 24]); // Reserved
    cat[30..32].copy_from_slice(&BOOT_CATALOG_HEADER_SIGNATURE.to_le_bytes());
    
    // Checksum calculation (reordered)
    let mut sum: u16 = 0;
    for i in (0..32).step_by(2) {
        sum = sum.wrapping_add(u16::from_le_bytes([cat[i], cat[i + 1]]));
    }
    let checksum = 0u16.wrapping_sub(sum);
    cat[28..30].copy_from_slice(&checksum.to_le_bytes());

    // Boot Entry
    let mut entry = [0u8; 32];
    entry[0] = BOOT_CATALOG_BOOT_ENTRY_HEADER_ID;
    entry[1] = BOOT_CATALOG_NO_EMULATION;
    
    // FIX: Write the correct sector count in ISO_SECTOR_SIZE (2048-byte) units
    let sector_count_iso = (img_file_size + (ISO_SECTOR_SIZE as u64 - 1)) / ISO_SECTOR_SIZE as u64;
    let sector_count_u16 = if sector_count_iso > 0xFFFF {
        0xFFFF
    } else {
        sector_count_iso as u16
    };
    entry[6..8].copy_from_slice(&sector_count_u16.to_le_bytes());
    
    entry[8..12].copy_from_slice(&fat_image_lba.to_le_bytes()); // LBA of FAT32 image
    cat[32..64].copy_from_slice(&entry);

    iso.write_all(&cat)
}

pub fn create_iso_from_img(iso_path: &Path, img_path: &Path) -> io::Result<()> {
    println!("create_iso_from_img: Creating ISO from FAT32 image.");

    let img_file_size = img_path.metadata()?.len();
    let img_padding_size = (ISO_SECTOR_SIZE as u64 - (img_file_size % ISO_SECTOR_SIZE as u64)) % ISO_SECTOR_SIZE as u64;
    let padded_img_file_size = img_file_size + img_padding_size;

    let fat_image_sectors = (padded_img_file_size.div_ceil(ISO_SECTOR_SIZE as u64)) as u32;

    let mut iso = File::create(iso_path)?;
    io::copy(
        &mut io::repeat(0).take(ISO_SECTOR_SIZE as u64 * 16),
        &mut iso,
    )?; // System Area

    const FAT_IMAGE_LBA: u32 = 21; // Moved to a new LBA to make room for the root directory sector
    const ROOT_DIR_LBA: u32 = 20;
    let total_sectors = FAT_IMAGE_LBA + fat_image_sectors;

    // Write ISO Volume Descriptors
    write_primary_volume_descriptor(&mut iso, total_sectors, ROOT_DIR_LBA)?;
    const LBA_BOOT_CATALOG: u32 = 19;
    write_boot_record_volume_descriptor(&mut iso, LBA_BOOT_CATALOG)?;
    write_volume_descriptor_terminator(&mut iso)?;

    // Write Boot Catalog
    write_boot_catalog(&mut iso, FAT_IMAGE_LBA, padded_img_file_size)?;

    // Write Root Directory Sector
    write_root_directory_sector(&mut iso, ROOT_DIR_LBA)?;

    // Write FAT image
    pad_to_lba(&mut iso, FAT_IMAGE_LBA)?;
    let mut img_file = File::open(img_path)?;
    io::copy(&mut img_file, &mut iso)?;
    pad_to_lba(&mut iso, total_sectors)?; // Pad to the end of the total sectors

    Ok(())
}
