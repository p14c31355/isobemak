// src/iso/iso.rs
use crate::iso::boot_catalog::{LBA_BOOT_CATALOG, write_boot_catalog};
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::volume_descriptor::*;
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba, update_4byte_fields};
use std::fs::File;
use std::io::{self, copy, Seek, SeekFrom, Write, Read};
use std::path::Path;

/// Creates a minimal ISO containing a FAT boot image (BOOTX64.EFI) and kernel (KERNEL.EFI)
/// in the same BOOT directory, with proper El Torito catalog.
pub fn create_iso_from_img(
    iso_path: &Path,
    fat_img_path: &Path,
    kernel_path: &Path,
    fat_img_actual_size: u32,
) -> io::Result<()> {
    let fat_img_size = fat_img_actual_size as u64;
    let boot_img_sectors = fat_img_size.div_ceil(512);
    if boot_img_sectors > u16::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Boot image too large: {} sectors", boot_img_sectors),
        ));
    }

    let mut iso = File::create(iso_path)?;

    // --- Write Primary Volume Descriptor ---
    let root_entry = IsoDirEntry {
        lba: 20,
        size: ISO_SECTOR_SIZE as u32,
        flags: 0x02,
        name: ".",
    };
    write_volume_descriptors(&mut iso, 0, LBA_BOOT_CATALOG, &root_entry)?;

    let boot_img_lba = 23;

    // --- Write El Torito Boot Catalog ---
    write_boot_catalog(&mut iso, boot_img_lba, boot_img_sectors as u16)?;

    // --- Root Directory ---
    pad_to_lba(&mut iso, 20)?;
    let root_dir_entries = [
        IsoDirEntry { lba: 20, size: ISO_SECTOR_SIZE as u32, flags: 0x02, name: "." }.to_bytes(),
        IsoDirEntry { lba: 20, size: ISO_SECTOR_SIZE as u32, flags: 0x02, name: ".." }.to_bytes(),
        IsoDirEntry { lba: 21, size: ISO_SECTOR_SIZE as u32, flags: 0x02, name: "EFI" }.to_bytes(),
        IsoDirEntry { lba: LBA_BOOT_CATALOG, size: ISO_SECTOR_SIZE as u32, flags: 0x00, name: "BOOT.CATALOG" }.to_bytes(),
    ].concat();
    let mut root_dir_content = root_dir_entries;
    root_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&root_dir_content)?;

    // --- EFI Directory ---
    pad_to_lba(&mut iso, 21)?;
    let efi_dir_entries = [
        IsoDirEntry { lba: 21, size: ISO_SECTOR_SIZE as u32, flags: 0x02, name: "." }.to_bytes(),
        IsoDirEntry { lba: 20, size: ISO_SECTOR_SIZE as u32, flags: 0x02, name: ".." }.to_bytes(),
        IsoDirEntry { lba: 22, size: ISO_SECTOR_SIZE as u32, flags: 0x02, name: "BOOT" }.to_bytes(),
    ].concat();
    let mut efi_dir_content = efi_dir_entries;
    efi_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&efi_dir_content)?;

    // --- BOOT Directory (initial) ---
    pad_to_lba(&mut iso, 22)?;
    let mut boot_dir_entries = vec![
        IsoDirEntry { lba: 22, size: ISO_SECTOR_SIZE as u32, flags: 0x02, name: "." }.to_bytes(),
        IsoDirEntry { lba: 21, size: ISO_SECTOR_SIZE as u32, flags: 0x02, name: ".." }.to_bytes(),
        IsoDirEntry { lba: boot_img_lba, size: fat_img_size as u32, flags: 0x00, name: "BOOTX64.EFI" }.to_bytes(),
    ];

    // Reserve space for BOOT directory
    let mut boot_dir_content = boot_dir_entries.concat();
    boot_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&boot_dir_content)?;

    // --- Copy FAT Boot Image ---
    pad_to_lba(&mut iso, boot_img_lba)?;
    let mut fat_file = File::open(fat_img_path)?;
    let written_fat = copy(&mut fat_file, &mut iso)?;
    let fat_padded = boot_img_sectors * 512;
    if written_fat < fat_padded {
        io::copy(&mut io::repeat(0).take(fat_padded - written_fat), &mut iso)?;
    }

    // --- Copy Kernel.EFI ---
    let kernel_lba = boot_img_lba + boot_img_sectors as u32; // Next free LBA after FAT image
    pad_to_lba(&mut iso, kernel_lba)?;
    let mut kernel_file = File::open(kernel_path)?;
    let kernel_size = copy(&mut kernel_file, &mut iso)? as u32;

    // --- Update BOOT Directory with Kernel entry ---
    boot_dir_entries.push(
        IsoDirEntry {
            lba: kernel_lba,
            size: kernel_size,
            flags: 0x00,
            name: "KERNEL.EFI",
        }.to_bytes()
    );
    // Re-pad BOOT directory
    let mut final_boot_dir = boot_dir_entries.concat();
    let pad_sectors = (final_boot_dir.len() + ISO_SECTOR_SIZE - 1) / ISO_SECTOR_SIZE;
    final_boot_dir.resize(pad_sectors * ISO_SECTOR_SIZE, 0);
    iso.seek(SeekFrom::Start(22 * ISO_SECTOR_SIZE as u64))?;
    iso.write_all(&final_boot_dir)?;

    // --- Final ISO padding ---
    let current_pos = iso.stream_position()?;
    let remainder = current_pos % ISO_SECTOR_SIZE as u64;
    if remainder != 0 {
        io::copy(&mut io::repeat(0).take(ISO_SECTOR_SIZE as u64 - remainder), &mut iso)?;
    }

    // --- Update Total Sectors in PVD ---
    let final_pos = iso.stream_position()?;
    let total_sectors = final_pos.div_ceil(ISO_SECTOR_SIZE as u64);
    if total_sectors > u32::MAX as u64 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "ISO image too large"));
    }
    update_4byte_fields(&mut iso, 16, PVD_TOTAL_SECTORS_OFFSET, PVD_TOTAL_SECTORS_OFFSET + 4, total_sectors as u32)?;

    println!("create_iso_from_img: ISO created with {} sectors", total_sectors);
    Ok(())
}
