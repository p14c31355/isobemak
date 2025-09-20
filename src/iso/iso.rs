// src/iso/iso.rs
use crate::iso::boot_catalog::{LBA_BOOT_CATALOG, write_boot_catalog};
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::volume_descriptor::*;
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba};
use std::fs::File;
use std::io::{self, Read, Seek, Write, copy};
use std::path::Path;

const LBA_ROOT_DIR: u32 = 20;
const LBA_EFI_DIR: u32 = 21;
const LBA_BOOT_DIR: u32 = 22;

fn write_directory(iso: &mut File, lba: u32, entries: &[IsoDirEntry]) -> io::Result<()> {
    pad_to_lba(iso, lba)?;
    let mut dir_content = Vec::new();
    for e in entries {
        dir_content.extend_from_slice(&e.to_bytes());
    }
    dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&dir_content)
}

/// Creates an ISO 9660 image with a FAT boot image (El Torito) and a UEFI kernel.
pub fn create_iso_from_img(
    iso_path: &Path,
    fat_img_path: &Path,
    kernel_path: &Path,
) -> io::Result<()> {
    // Open ISO file for writing
    let mut iso = File::create(iso_path)?;

    // Create FAT image and get padded size
    let fat_img_size = std::fs::metadata(fat_img_path)?.len();
    let fat_img_sectors = fat_img_size.div_ceil(512) as u32;

    // Define LBAs
    let boot_img_lba = 23; // LBA where Boot-NoEmul.img will be placed
    let kernel_metadata = std::fs::metadata(kernel_path)?;
    let kernel_size = kernel_metadata.len() as u32;
    let kernel_lba = boot_img_lba + fat_img_size.div_ceil(ISO_SECTOR_SIZE as u64) as u32;

    // Write Primary Volume Descriptor
    let root_entry = IsoDirEntry {
        lba: LBA_ROOT_DIR,
        size: ISO_SECTOR_SIZE as u32,
        flags: 0x02,
        name: ".",
    };
    write_volume_descriptors(&mut iso, 0, LBA_BOOT_CATALOG, &root_entry)?;

    // Write El Torito Boot Catalog
    if fat_img_sectors > u16::MAX as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Boot image too large for boot catalog: {} 512-byte sectors",
                fat_img_sectors
            ),
        ));
    }
    write_boot_catalog(&mut iso, boot_img_lba, fat_img_sectors as u16)?;

    // Prepare directory entries
    let root_dir_entries = [
        IsoDirEntry {
            lba: LBA_ROOT_DIR,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        },
        IsoDirEntry {
            lba: LBA_ROOT_DIR,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "..",
        },
        IsoDirEntry {
            lba: LBA_EFI_DIR,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "EFI",
        },
    ];
    let efi_dir_entries = [
        IsoDirEntry {
            lba: LBA_EFI_DIR,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        },
        IsoDirEntry {
            lba: LBA_ROOT_DIR,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "..",
        },
        IsoDirEntry {
            lba: LBA_BOOT_DIR,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "BOOT",
        },
    ];
    let boot_dir_entries = [
        IsoDirEntry {
            lba: LBA_BOOT_DIR,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        },
        IsoDirEntry {
            lba: LBA_EFI_DIR,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "..",
        },
        IsoDirEntry {
            lba: boot_img_lba,
            size: fat_img_size as u32,
            flags: 0x00,
            name: "BOOTX64.EFI",
        },
        IsoDirEntry {
            lba: kernel_lba,
            size: kernel_size,
            flags: 0x00,
            name: "KERNEL.EFI",
        },
    ];

    // Write directories to ISO
    write_directory(&mut iso, LBA_ROOT_DIR, &root_dir_entries)?;
    write_directory(&mut iso, LBA_EFI_DIR, &efi_dir_entries)?;
    write_directory(&mut iso, LBA_BOOT_DIR, &boot_dir_entries)?;

    // Copy FAT boot image
    pad_to_lba(&mut iso, boot_img_lba)?;
    let mut fat_file = File::open(fat_img_path)?;
    copy(&mut fat_file, &mut iso)?;

    // Copy kernel
    pad_to_lba(&mut iso, kernel_lba)?;
    let mut kernel_file = File::open(kernel_path)?;
    copy(&mut kernel_file, &mut iso)?;

    // Final padding to ISO sector
    let current_pos = iso.stream_position()?;
    let remainder = current_pos % ISO_SECTOR_SIZE as u64;
    if remainder != 0 {
        io::copy(
            &mut io::repeat(0).take(ISO_SECTOR_SIZE as u64 - remainder),
            &mut iso,
        )?;
    }

    // Update PVD total sectors
    let final_pos = iso.stream_position()?;
    let total_sectors = final_pos.div_ceil(ISO_SECTOR_SIZE as u64);
    if total_sectors > u32::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ISO image too large",
        ));
    }
    update_total_sectors_in_pvd(&mut iso, total_sectors as u32)?;

    println!(
        "create_iso_from_img: ISO created with {} sectors",
        total_sectors
    );
    Ok(())
}
