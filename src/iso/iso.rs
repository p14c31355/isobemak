// src/iso/iso.rs
use crate::iso::boot_catalog::{LBA_BOOT_CATALOG, write_boot_catalog};
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::volume_descriptor::*;
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write, copy};
use std::path::Path;

/// Creates a minimal ISO with a FAT boot image and a kernel.
pub fn create_iso_from_img(
    iso_path: &Path,
    fat_img_path: &Path,
    kernel_path: &Path,
    fat_img_padded_size: u32,
) -> io::Result<()> {
    let fat_img_size_u64 = fat_img_padded_size as u64;
    let boot_img_sectors_512 = fat_img_size_u64.div_ceil(512) as u32;

    let mut iso = File::create(iso_path)?;
    let boot_img_lba = 23;

    // Get kernel size
    let kernel_metadata = std::fs::metadata(kernel_path)?;
    let kernel_size = kernel_metadata.len() as u32;

    // Calculate kernel LBA
    let kernel_lba = boot_img_lba + fat_img_size_u64.div_ceil(ISO_SECTOR_SIZE as u64) as u32;

    // Define directory entries with calculated LBAs and sizes
    let boot_dir_entries_structs_with_kernel = vec![
        IsoDirEntry {
            lba: 22,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        },
        IsoDirEntry {
            lba: 21,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "..",
        },
        IsoDirEntry {
            lba: boot_img_lba,
            size: fat_img_padded_size,
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

    let efi_dir_entries_structs = [
        IsoDirEntry {
            lba: 21,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        },
        IsoDirEntry {
            lba: 20,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "..",
        },
        IsoDirEntry {
            lba: 22,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "BOOT",
        },
    ];

    let root_dir_entries_structs = [
        IsoDirEntry {
            lba: 20,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        },
        IsoDirEntry {
            lba: 20,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "..",
        },
        IsoDirEntry {
            lba: 21,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: "EFI",
        },
        IsoDirEntry {
            lba: LBA_BOOT_CATALOG,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x00,
            name: "BOOT.CATALOG",
        },
    ];

    // Define the root directory entry for the PVD
    let root_entry = IsoDirEntry {
        lba: 20,
        size: ISO_SECTOR_SIZE as u32,
        flags: 0x02,
        name: ".",
    };
    write_volume_descriptors(&mut iso, 0, LBA_BOOT_CATALOG, &root_entry)?;

    // --- El Torito Boot Catalog ---
    if boot_img_sectors_512 > u16::MAX as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Boot image too large for boot catalog: {} 512-byte sectors",
                boot_img_sectors_512
            ),
        ));
    }
    write_boot_catalog(&mut iso, boot_img_lba, boot_img_sectors_512 as u16)?;

    // --- Write Root Directory ---
    pad_to_lba(&mut iso, 20)?;
    let root_dir_entries_bytes = root_dir_entries_structs
        .iter()
        .flat_map(|e| e.to_bytes())
        .collect::<Vec<u8>>();
    let mut root_dir_content = root_dir_entries_bytes;
    root_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&root_dir_content)?;

    // --- Write EFI Directory ---
    pad_to_lba(&mut iso, 21)?;
    let efi_dir_entries_bytes = efi_dir_entries_structs
        .iter()
        .flat_map(|e| e.to_bytes())
        .collect::<Vec<u8>>();
    let mut efi_dir_content = efi_dir_entries_bytes;
    efi_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&efi_dir_content)?;

    // --- Write BOOT Directory ---
    pad_to_lba(&mut iso, 22)?;
    let boot_dir_entries_structs_final = boot_dir_entries_structs_with_kernel;

    let boot_dir_content_bytes = boot_dir_entries_structs_final
        .iter()
        .flat_map(|e| e.to_bytes())
        .collect::<Vec<u8>>();
    let mut boot_dir_content = boot_dir_content_bytes;
    boot_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&boot_dir_content)?;

    // --- Copy FAT Boot Image ---
    pad_to_lba(&mut iso, boot_img_lba)?;
    let mut fat_file = File::open(fat_img_path)?;
    let written_fat = copy(&mut fat_file, &mut iso)?;
    let fat_padded = fat_img_padded_size as u64;
    if written_fat < fat_padded {
        io::copy(&mut io::repeat(0).take(fat_padded - written_fat), &mut iso)?;
    }

    // --- Copy Kernel.EFI ---
    pad_to_lba(&mut iso, kernel_lba)?;
    let mut kernel_file = File::open(kernel_path)?;
    copy(&mut kernel_file, &mut iso)?;

    // --- Final ISO Padding ---
    let current_pos = iso.stream_position()?;
    let remainder = current_pos % ISO_SECTOR_SIZE as u64;
    if remainder != 0 {
        io::copy(
            &mut io::repeat(0).take(ISO_SECTOR_SIZE as u64 - remainder),
            &mut iso,
        )?;
    }

    // --- Update Total Sectors in PVD ---
    iso.seek(SeekFrom::End(0))?;
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
