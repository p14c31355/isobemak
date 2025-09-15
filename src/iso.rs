// isobemak/src/iso.rs
// ISO + El Torito
use crate::utils::{SECTOR_SIZE, pad_sector};
use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    path::Path,
};

/// Constants for ISO 9660 structure to improve readability.
const ISO_VOLUME_DESCRIPTOR_TERMINATOR: u8 = 255;
const ISO_VOLUME_DESCRIPTOR_PRIMARY: u8 = 1;
const ISO_VOLUME_DESCRIPTOR_BOOT_RECORD: u8 = 0;
const ISO_ID: &[u8] = b"CD001";
const ISO_VERSION: u8 = 1;
const ISO_DIRECTORY_FLAG: u8 = 0x02;

const BOOT_CATALOG_HEADER_SIGNATURE: u16 = 0xAA55;
const BOOT_CATALOG_BOOTABLE_INDICATOR: u8 = 0x88;
const BOOT_CATALOG_EFI_PLATFORM_ID: u8 = 0xEF;

/// Creates a general ISO 9660 directory or file record.
/// This function handles the common fields for a directory record.
fn create_dir_record(lba: u32, size: u32, is_dir: bool, name: &[u8]) -> Vec<u8> {
    let name_len = name.len() as u8;
    let record_len = 33 + name_len;
    let mut record = vec![0u8; record_len as usize];

    record[0] = record_len; // Length of Directory Record
    record[1] = 0; // Extended Attribute Record Length
    record[2..6].copy_from_slice(&lba.to_le_bytes()); // Location of Extent (LE)
    record[6..10].copy_from_slice(&lba.to_be_bytes()); // Location of Extent (BE)
    record[10..14].copy_from_slice(&size.to_le_bytes()); // Data Length (LE)
    record[14..18].copy_from_slice(&size.to_be_bytes()); // Data Length (BE)
    // Date and time (bytes 18-24) - left as 0
    record[25] = if is_dir { ISO_DIRECTORY_FLAG } else { 0x00 }; // File Flags (Directory or File)
    // File Unit Size (byte 26) - 0
    // Interleave Gap Size (byte 27) - 0
    record[28..30].copy_from_slice(&1u16.to_le_bytes()); // Volume Sequence Number (LE)
    record[30..32].copy_from_slice(&1u16.to_be_bytes()); // Volume Sequence Number (BE)
    record[32] = name_len; // Length of File Identifier
    record[33..33 + name_len as usize].copy_from_slice(name); // File Identifier

    record
}

/// Creates a Directory Record for the current ('.') or parent ('..') directory.
fn create_relative_dir_entry(lba: u32, size: u32, is_parent: bool) -> [u8; 34] {
    let name_byte = if is_parent { 0x01 } else { 0x00 };
    let vec_record = create_dir_record(lba, size, true, &[name_byte]);
    let mut record = [0u8; 34];
    record.copy_from_slice(&vec_record);
    record
}

/// Pads the ISO file with zeros to align to a specific LBA.
/// This helper function reduces code duplication in the main logic.
fn pad_to_lba(iso: &mut File, lba: u32) -> io::Result<()> {
    let target_pos = lba as u64 * SECTOR_SIZE as u64;
    let current_pos = iso.stream_position()?;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        io::copy(&mut io::repeat(0).take(padding_bytes), iso)?;
    }
    Ok(())
}

/// Appends a directory record to a buffer, with a panic guard.
fn append_dir_record(buffer: &mut Vec<u8>, record: &[u8]) -> io::Result<()> {
    let next_offset = buffer.len() + record.len();
    if next_offset > SECTOR_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Buffer overflow: Directory record exceeds sector size.",
        ));
    }
    buffer.extend_from_slice(record);
    Ok(())
}

pub fn create_iso(path: &Path, bellows_file: &mut File, kernel_file: &mut File) -> io::Result<()> {
    println!("create_iso: Creating ISO with bellows and kernel from file handles.");

    let mut iso = File::create(path)?;
    io::copy(&mut io::repeat(0).take(SECTOR_SIZE as u64 * 16), &mut iso)?; // System Area

    // Read EFI files from the provided file handles
    let mut bellows_file_content = Vec::new();
    bellows_file.read_to_end(&mut bellows_file_content)?;
    let mut kernel_file_content = Vec::new();
    kernel_file.read_to_end(&mut kernel_file_content)?;

    let bellows_sectors = (bellows_file_content.len() as u32).div_ceil(SECTOR_SIZE as u32);
    let kernel_sectors = (kernel_file_content.len() as u32).div_ceil(SECTOR_SIZE as u32);

    // Calculate LBAs for EFI files and directory structure
    const LBA_EFI_DIR: u32 = 20;
    const LBA_BOOT_DIR: u32 = LBA_EFI_DIR + 1;
    let bellows_efi_lba = LBA_BOOT_DIR + 1;
    let kernel_efi_lba = bellows_efi_lba + bellows_sectors;

    let total_sectors = kernel_efi_lba + kernel_sectors;

    // Primary Volume Descriptor (LBA 16)
    const LBA_PVD: u32 = 16;
    pad_to_lba(&mut iso, LBA_PVD)?;
    let mut pvd = [0u8; SECTOR_SIZE];
    pvd[0] = ISO_VOLUME_DESCRIPTOR_PRIMARY;
    pvd[1..6].copy_from_slice(ISO_ID);
    pvd[6] = ISO_VERSION;
    let mut volume_id = [0u8; 32];
    let project_name = b"FULLERENE";
    volume_id[..project_name.len()].copy_from_slice(project_name);
    volume_id[project_name.len()..].fill(b' ');
    pvd[40..72].copy_from_slice(&volume_id);

    pvd[80..84].copy_from_slice(&total_sectors.to_le_bytes());
    pvd[84..88].copy_from_slice(&total_sectors.to_be_bytes());
    pvd[128..132].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes());

    // Root Directory Record (for PVD)
    let root_dir_lba = LBA_EFI_DIR;
    let root_dir_size = SECTOR_SIZE as u32;
    let root_dir_record = create_relative_dir_entry(root_dir_lba, root_dir_size, false);
    pvd[156..190].copy_from_slice(&root_dir_record);
    iso.write_all(&pvd)?;

    // Boot Record Volume Descriptor (LBA 17)
    const LBA_BRVD: u32 = 17;
    pad_to_lba(&mut iso, LBA_BRVD)?;
    let mut brvd = [0u8; SECTOR_SIZE];
    brvd[0] = ISO_VOLUME_DESCRIPTOR_BOOT_RECORD;
    brvd[1..6].copy_from_slice(ISO_ID);
    brvd[6] = ISO_VERSION;
    let mut el_torito_spec = [0u8; 32];
    let spec_name = b"EL TORITO SPECIFICATION";
    el_torito_spec[..spec_name.len()].copy_from_slice(spec_name);
    brvd[7..39].copy_from_slice(&el_torito_spec);

    const LBA_BOOT_CATALOG: u32 = 19;
    brvd[71..75].copy_from_slice(&LBA_BOOT_CATALOG.to_le_bytes()); // Boot Catalog LBA
    iso.write_all(&brvd)?;

    // Volume Descriptor Terminator (LBA 18)
    const LBA_VDT: u32 = 18;
    pad_to_lba(&mut iso, LBA_VDT)?;
    let mut term = [0u8; SECTOR_SIZE];
    term[0] = ISO_VOLUME_DESCRIPTOR_TERMINATOR;
    term[1..6].copy_from_slice(ISO_ID);
    term[6] = ISO_VERSION;
    iso.write_all(&term)?;

    // Boot Catalog (LBA 19)
    pad_to_lba(&mut iso, LBA_BOOT_CATALOG)?;
    let mut cat = [0u8; SECTOR_SIZE];
    cat[0] = 1;
    cat[1] = BOOT_CATALOG_EFI_PLATFORM_ID;
    cat[2..4].copy_from_slice(&0u16.to_le_bytes());
    cat[30..32].copy_from_slice(&BOOT_CATALOG_HEADER_SIGNATURE.to_le_bytes());

    let mut sum: u16 = 0;
    for i in (0..32).step_by(2) {
        sum = sum.wrapping_add(u16::from_le_bytes([cat[i], cat[i + 1]]));
    }
    let checksum = 0u16.wrapping_sub(sum);
    cat[28..30].copy_from_slice(&checksum.to_le_bytes());

    let mut entry = [0u8; 32];
    entry[0] = BOOT_CATALOG_BOOTABLE_INDICATOR; // Bootable, EFI
    entry[1] = 0x00; // Boot media type (no emulation)
    entry[2..4].copy_from_slice(&0u16.to_le_bytes()); // Load segment (0)
    entry[4] = 0x00; // System type (0)
    entry[5] = 0x00; // Unused

    let sector_count_512 = bellows_file_content.len().div_ceil(512);
    let sector_count_u16 = if sector_count_512 > 0xFFFF {
        0xFFFF
    } else {
        sector_count_512 as u16
    };

    entry[6..8].copy_from_slice(&sector_count_u16.to_le_bytes()); // Sector count (512-byte units)
    entry[8..12].copy_from_slice(&bellows_efi_lba.to_le_bytes()); // LBA of boot image (bellows.efi)
    cat[32..64].copy_from_slice(&entry);
    iso.write_all(&cat)?;

    // EFI Directory (LBA 20)
    pad_to_lba(&mut iso, LBA_EFI_DIR)?;
    let mut efi_dir_data = Vec::new();

    // Current directory entry ('.')
    let current_dir_record = create_relative_dir_entry(LBA_EFI_DIR, SECTOR_SIZE as u32, false);
    append_dir_record(&mut efi_dir_data, &current_dir_record)?;

    // Parent directory entry ('..')
    let parent_dir_record = create_relative_dir_entry(LBA_EFI_DIR, SECTOR_SIZE as u32, true);
    append_dir_record(&mut efi_dir_data, &parent_dir_record)?;

    // BOOT directory entry
    let boot_dir_record = create_dir_record(LBA_BOOT_DIR, SECTOR_SIZE as u32, true, b"BOOT");
    append_dir_record(&mut efi_dir_data, &boot_dir_record)?;

    iso.write_all(&efi_dir_data)?;
    pad_sector(&mut iso)?; // Pad to full sector

    // BOOT Directory (LBA 21)
    pad_to_lba(&mut iso, LBA_BOOT_DIR)?;
    let mut boot_dir_data = Vec::new();

    // Current directory entry ('.')
    let current_dir_record = create_relative_dir_entry(LBA_BOOT_DIR, SECTOR_SIZE as u32, false);
    append_dir_record(&mut boot_dir_data, &current_dir_record)?;

    // Parent directory entry ('..')
    let parent_dir_record = create_relative_dir_entry(LBA_EFI_DIR, SECTOR_SIZE as u32, true);
    append_dir_record(&mut boot_dir_data, &parent_dir_record)?;

    // BOOTX64.EFI entry
    let bootx64_record = create_dir_record(
        bellows_efi_lba,
        bellows_file_content.len() as u32,
        false,
        b"BOOTX64.EFI",
    );
    append_dir_record(&mut boot_dir_data, &bootx64_record)?;

    // KERNEL.EFI entry
    let kernel_record = create_dir_record(
        kernel_efi_lba,
        kernel_file_content.len() as u32,
        false,
        b"KERNEL.EFI",
    );
    append_dir_record(&mut boot_dir_data, &kernel_record)?;

    iso.write_all(&boot_dir_data)?;
    pad_sector(&mut iso)?; // Pad to full sector

    // Write bellows.efi
    pad_to_lba(&mut iso, bellows_efi_lba)?;
    iso.write_all(&bellows_file_content)?;
    pad_sector(&mut iso)?;

    // Write kernel.efi
    pad_to_lba(&mut iso, kernel_efi_lba)?;
    iso.write_all(&kernel_file_content)?;
    pad_sector(&mut iso)?;

    Ok(())
}
