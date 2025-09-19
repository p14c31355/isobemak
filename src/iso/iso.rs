// src/iso/iso.rs
use crate::iso::boot_catalog::{LBA_BOOT_CATALOG, write_boot_catalog};
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::volume_descriptor::*;
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba, update_4byte_fields};
use std::fs::File;
use std::io::{self, Read, Seek, Write, copy};
use std::path::Path;

/// Creates an ISO image from a bootable FAT image file, ensuring both BOOTX64.EFI and KERNEL.EFI
/// inside the FAT image are included in the ISO.
///
/// # Arguments
/// * `iso_path` - The path to write the resulting ISO.
/// * `fat_img_path` - Path to the FAT32 image containing BOOTX64.EFI and KERNEL.EFI.
/// * `fat_img_actual_size` - Actual size of the FAT image, used to compute El Torito Nsect.
pub fn create_iso_from_img(
    iso_path: &Path,
    fat_img_path: &Path,
    fat_img_actual_size: u32,
) -> io::Result<()> {
    // Convert size to u64 for arithmetic.
    let fat_img_size = fat_img_actual_size as u64;

    // Compute boot image sectors in 512-byte units for El Torito.
    let boot_img_sectors = fat_img_size.div_ceil(512);
    if boot_img_sectors > u16::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Boot image too large: {} sectors", boot_img_sectors),
        ));
    }

    let mut iso = File::create(iso_path)?;

    // --- Write Volume Descriptors ---
    let root_entry = IsoDirEntry {
        lba: 20,
        size: ISO_SECTOR_SIZE as u32,
        flags: 0x02,
        name: ".",
    };
    write_volume_descriptors(&mut iso, 0, LBA_BOOT_CATALOG, &root_entry)?;

    // Start LBA of the FAT boot image (ISO 9660 LBA in 2048-byte units)
    let boot_img_lba = 23;

    // --- Write the El Torito boot catalog ---
    write_boot_catalog(&mut iso, boot_img_lba, boot_img_sectors as u16)?;

    // --- ISO9660 directories ---
    // Root directory
    pad_to_lba(&mut iso, 20)?;
    let root_dir = [
        IsoDirEntry {
            lba: 20,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        }
        .to_bytes(),
        IsoDirEntry {
            lba: 20,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "..",
        }
        .to_bytes(),
        IsoDirEntry {
            lba: 21,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "EFI",
        }
        .to_bytes(),
        IsoDirEntry {
            lba: LBA_BOOT_CATALOG,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x00,
            name: "BOOT.CATALOG",
        }
        .to_bytes(),
    ]
    .concat();
    let mut root_dir_content = root_dir;
    root_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&root_dir_content)?;

    // EFI directory
    pad_to_lba(&mut iso, 21)?;
    let efi_dir = [
        IsoDirEntry {
            lba: 21,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        }
        .to_bytes(),
        IsoDirEntry {
            lba: 20,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "..",
        }
        .to_bytes(),
        IsoDirEntry {
            lba: 22,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "BOOT",
        }
        .to_bytes(),
    ]
    .concat();
    let mut efi_dir_content = efi_dir;
    efi_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&efi_dir_content)?;

    // BOOT directory
    pad_to_lba(&mut iso, 22)?;
    let boot_dir = [
        IsoDirEntry {
            lba: 22,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        }
        .to_bytes(),
        IsoDirEntry {
            lba: 21,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "..",
        }
        .to_bytes(),
        // Include the entire FAT image as BOOTX64.EFI (for UEFI firmware)
        IsoDirEntry {
            lba: boot_img_lba,
            size: fat_img_size as u32,
            flags: 0x00,
            name: "BOOTX64.EFI",
        }
        .to_bytes(),
    ]
    .concat();
    let mut boot_dir_content = boot_dir;
    boot_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&boot_dir_content)?;

    // --- Copy the FAT boot image ---
    pad_to_lba(&mut iso, boot_img_lba)?;
    let mut fat_img_file = File::open(fat_img_path)?;
    let written_size = copy(&mut fat_img_file, &mut iso)?;
    let padded_size = boot_img_sectors * 512;
    if written_size < padded_size {
        let padding_size = padded_size - written_size;
        io::copy(&mut io::repeat(0).take(padding_size), &mut iso)?;
    }

    // Pad ISO to next 2048-byte boundary
    let current_pos = iso.stream_position()?;
    let remainder = current_pos % ISO_SECTOR_SIZE as u64;
    if remainder != 0 {
        let padding_needed = ISO_SECTOR_SIZE as u64 - remainder;
        io::copy(&mut io::repeat(0).take(padding_needed), &mut iso)?;
    }

    // --- Update total sectors in Primary Volume Descriptor ---
    let final_pos = iso.stream_position()?;
    let total_sectors = final_pos.div_ceil(ISO_SECTOR_SIZE as u64);
    if total_sectors > u32::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("ISO image too large: {} sectors", total_sectors),
        ));
    }

    update_4byte_fields(
        &mut iso,
        16,
        PVD_TOTAL_SECTORS_OFFSET,
        PVD_TOTAL_SECTORS_OFFSET + 4,
        total_sectors as u32,
    )?;

    println!(
        "create_iso_from_img: ISO created with {} sectors",
        total_sectors
    );
    Ok(())
}
