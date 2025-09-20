// src/iso/iso.rs
use crate::iso::boot_catalog::{LBA_BOOT_CATALOG, write_boot_catalog};
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::volume_descriptor::*;
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba, update_4byte_fields};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write, copy};
use std::path::Path;

/// Creates a minimal ISO with FAT boot image (BOOTX64.EFI) and kernel (KERNEL.EFI)
/// in the same BOOT directory.
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
    let boot_img_lba = 23;

    // Get kernel size early
    let kernel_metadata = std::fs::metadata(kernel_path)?;
    let kernel_size = kernel_metadata.len() as u32;
    let kernel_sectors = kernel_size.div_ceil(ISO_SECTOR_SIZE as u32);

    let kernel_lba = boot_img_lba + boot_img_sectors as u32;

    // Define structs with placeholder sizes first
    let mut boot_dir_entries_structs_with_kernel = vec![
        IsoDirEntry {
            lba: 22,
            size: 0,
            flags: 0x02,
            name: ".",
        },
        IsoDirEntry {
            lba: 21,
            size: 0,
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

    // Calculate sizes based on the defined structs
    let boot_dir_final_size = boot_dir_entries_structs_with_kernel
        .iter()
        .map(|e| e.to_bytes().len())
        .sum::<usize>() as u32;

    let mut efi_dir_entries_structs = [
        IsoDirEntry {
            lba: 21,
            size: 0,
            flags: 0x02,
            name: ".",
        },
        IsoDirEntry {
            lba: 20,
            size: 0,
            flags: 0x02,
            name: "..",
        },
        IsoDirEntry {
            lba: 22,
            size: boot_dir_final_size,
            flags: 0x02,
            name: "BOOT",
        },
    ];
    let efi_dir_size = efi_dir_entries_structs
        .iter()
        .flat_map(|e| e.to_bytes())
        .collect::<Vec<u8>>()
        .len() as u32;

    let mut root_dir_entries_structs = [
        IsoDirEntry {
            lba: 20,
            size: 0,
            flags: 0x02,
            name: ".",
        },
        IsoDirEntry {
            lba: 20,
            size: 0,
            flags: 0x02,
            name: "..",
        },
        IsoDirEntry {
            lba: 21,
            size: efi_dir_size,
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
    let root_dir_size = root_dir_entries_structs
        .iter()
        .flat_map(|e| e.to_bytes())
        .collect::<Vec<u8>>()
        .len() as u32;

    // Update sizes in the structs
    boot_dir_entries_structs_with_kernel[0].size = boot_dir_final_size;
    boot_dir_entries_structs_with_kernel[1].size = efi_dir_size; // Parent is EFI dir

    efi_dir_entries_structs[0].size = efi_dir_size;
    efi_dir_entries_structs[1].size = root_dir_size; // Parent is root dir
    efi_dir_entries_structs[2].size = boot_dir_final_size; // Child is BOOT dir

    root_dir_entries_structs[0].size = root_dir_size;
    root_dir_entries_structs[1].size = root_dir_size; // Parent is itself
    root_dir_entries_structs[2].size = efi_dir_size; // Child is EFI dir

    let root_entry = IsoDirEntry {
        lba: 20,
        size: root_dir_size,
        flags: 0x02,
        name: ".",
    };
    write_volume_descriptors(&mut iso, 0, LBA_BOOT_CATALOG, &root_entry)?;

    // --- El Torito Boot Catalog ---
    write_boot_catalog(&mut iso, boot_img_lba, boot_img_sectors as u16)?;

    // --- Root Directory ---
    pad_to_lba(&mut iso, 20)?;
    let root_dir_entries_bytes = root_dir_entries_structs
        .iter()
        .flat_map(|e| e.to_bytes())
        .collect::<Vec<u8>>();
    let mut root_dir_content = root_dir_entries_bytes;
    root_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&root_dir_content)?;

    // --- EFI Directory ---
    pad_to_lba(&mut iso, 21)?;
    let efi_dir_entries_bytes = efi_dir_entries_structs
        .iter()
        .flat_map(|e| e.to_bytes())
        .collect::<Vec<u8>>();
    let mut efi_dir_content = efi_dir_entries_bytes;
    efi_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&efi_dir_content)?;

    // --- BOOT Directory ---
    pad_to_lba(&mut iso, 22)?;
    let boot_dir_entries_structs_final = boot_dir_entries_structs_with_kernel; // Use the structs directly

    let mut boot_dir_content = boot_dir_entries_structs_final
        .iter()
        .flat_map(|e| e.to_bytes())
        .collect::<Vec<u8>>();

    let required_size = boot_dir_content.len().div_ceil(ISO_SECTOR_SIZE) * ISO_SECTOR_SIZE;
    boot_dir_content.resize(required_size, 0);
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
    pad_to_lba(&mut iso, kernel_lba)?;
    let mut kernel_file = File::open(kernel_path)?;
    copy(&mut kernel_file, &mut iso)?;

    // Re-calculate final BOOT directory size and re-pad
    let final_boot_dir_bytes = boot_dir_entries_structs_final
        .iter()
        .flat_map(|e| e.to_bytes())
        .collect::<Vec<u8>>();
    let pad_sectors = final_boot_dir_bytes.len().div_ceil(ISO_SECTOR_SIZE);
    let mut final_boot_dir = final_boot_dir_bytes;
    final_boot_dir.resize(pad_sectors * ISO_SECTOR_SIZE, 0);
    iso.seek(SeekFrom::Start(22 * ISO_SECTOR_SIZE as u64))?;
    iso.write_all(&final_boot_dir)?;

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
    iso.seek(SeekFrom::End(0))?; // Seek to end to get accurate final_pos
    let final_pos = iso.stream_position()?;
    let total_sectors = final_pos.div_ceil(ISO_SECTOR_SIZE as u64);
    if total_sectors > u32::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ISO image too large",
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
