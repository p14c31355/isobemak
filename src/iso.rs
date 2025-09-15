// isobemak/src/iso.rs
// ISO + El Torito
use crate::utils::{pad_sector, SECTOR_SIZE};
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

const BOOT_CATALOG_HEADER_SIGNATURE: u16 = 0xAA55;
const BOOT_CATALOG_BOOTABLE_INDICATOR: u8 = 0x88;
const BOOT_CATALOG_EFI_PLATFORM_ID: u8 = 0xEF;

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

pub fn create_iso_from_img(iso_path: &Path, img_path: &Path) -> io::Result<()> {
    println!("create_iso_from_img: Creating ISO from FAT32 image.");

    // 1. Read the FAT32 image from disk
    let mut img_file = File::open(img_path)?;
    let mut fat_image = Vec::new();
    img_file.read_to_end(&mut fat_image)?;

    // 2. Create the ISO with the FAT image embedded
    let mut iso = File::create(iso_path)?;
    io::copy(&mut io::repeat(0).take(SECTOR_SIZE as u64 * 16), &mut iso)?; // System Area

    let fat_image_sectors = (fat_image.len() as u32).div_ceil(SECTOR_SIZE as u32);
    const FAT_IMAGE_LBA: u32 = 20;
    let total_sectors = FAT_IMAGE_LBA + fat_image_sectors;

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
    // Minimal root directory record (not really used, but required)
    let root_dir_record = [ 34, 0, 19, 0, 0, 0, 19, 0, 0, 0, 0, 8, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 1, 0, 0, 1, 1, 0 ];
    pvd[156..190].copy_from_slice(&root_dir_record);
    iso.write_all(&pvd)?;

    // Boot Record Volume Descriptor (LBA 17)
    const LBA_BRVD: u32 = 17;
    pad_to_lba(&mut iso, LBA_BRVD)?;
    let mut brvd = [0u8; SECTOR_SIZE];
    brvd[0] = ISO_VOLUME_DESCRIPTOR_BOOT_RECORD;
    brvd[1..6].copy_from_slice(ISO_ID);
    brvd[6] = ISO_VERSION;
    let spec_name = b"EL TORITO SPECIFICATION";
    brvd[7..7 + spec_name.len()].copy_from_slice(spec_name);
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
    // Validation Entry
    cat[0] = 1;
    cat[1] = BOOT_CATALOG_EFI_PLATFORM_ID;
    cat[30..32].copy_from_slice(&BOOT_CATALOG_HEADER_SIGNATURE.to_le_bytes());
    let mut sum: u16 = 0;
    for i in (0..32).step_by(2) {
        sum = sum.wrapping_add(u16::from_le_bytes([cat[i], cat[i + 1]]));
    }
    let checksum = 0u16.wrapping_sub(sum);
    cat[28..30].copy_from_slice(&checksum.to_le_bytes());

    // Boot Entry
    let mut entry = [0u8; 32];
    entry[0] = BOOT_CATALOG_BOOTABLE_INDICATOR; // Bootable, EFI
    entry[1] = 0x00; // Boot media type (no emulation)
    let sector_count_512 = (fat_image.len() as u64).div_ceil(512);
    let sector_count_u16 = if sector_count_512 > 0xFFFF {
        0xFFFF
    } else {
        sector_count_512 as u16
    };
    entry[6..8].copy_from_slice(&sector_count_u16.to_le_bytes()); // Sector count (512-byte units)
    entry[8..12].copy_from_slice(&FAT_IMAGE_LBA.to_le_bytes()); // LBA of FAT32 image
    cat[32..64].copy_from_slice(&entry);
    iso.write_all(&cat)?;

    // Write FAT image to the ISO
    pad_to_lba(&mut iso, FAT_IMAGE_LBA)?;
    iso.write_all(&fat_image)?;
    pad_sector(&mut iso)?;

    Ok(())
}