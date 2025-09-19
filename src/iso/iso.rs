// isobemak/src/iso.rs

use crate::iso::boot_catalog::*;
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::volume_descriptor::*;
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba, update_4byte_fields};
use std::fs::File;
use std::io::{self, Seek, Write, copy};
use std::path::Path;

/// Creates an ISO image from a bootable image file.
pub fn create_iso_from_img(iso_path: &Path, boot_img_path: &Path) -> io::Result<()> {
    let boot_img_metadata = std::fs::metadata(boot_img_path)?;
    let boot_img_size = boot_img_metadata.len();

    // The boot image sector count must be in 512-byte units, as per the El Torito specification.
    let boot_img_sectors_u64 = boot_img_size.div_ceil(512);
    if boot_img_sectors_u64 > u16::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Boot image is too large for El Torito specification",
        ));
    }
    let boot_img_sectors = boot_img_size.div_ceil(512); // in 512-byte sectors
    let mut iso = File::create(iso_path)?;

    let root_entry = IsoDirEntry {
        lba: 20,
        size: ISO_SECTOR_SIZE as u32,
        flags: 0x02,
        name: ".",
    };
    write_primary_volume_descriptor(&mut iso, 0, &root_entry)?;
    write_boot_record_volume_descriptor(&mut iso, LBA_BOOT_CATALOG)?;
    write_volume_descriptor_terminator(&mut iso)?;

    // The start LBA for the boot image. This is an ISO 9660 LBA (2048-byte unit).
    let boot_img_lba = 23;

    // Write the El Torito boot catalog.
    // This function expects the boot image LBA (2048-byte units) and the sector count (512-byte units).
    write_boot_catalog(&mut iso, boot_img_lba, boot_img_sectors as u16)?;

    // ISO9660 directories simplified
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
        IsoDirEntry {
            // Use the correct ISO 9660 LBA for the boot image entry.
            lba: boot_img_lba,
            size: boot_img_size as u32,
            flags: 0x00,
            name: "BOOTX64.EFI",
        }
        .to_bytes(),
    ]
    .concat();
    let mut boot_dir_content = boot_dir;
    boot_dir_content.resize(ISO_SECTOR_SIZE, 0);
    iso.write_all(&boot_dir_content)?;

    // Pad to the correct LBA and copy the boot image content.
    pad_to_lba(&mut iso, boot_img_lba)?;
    let mut boot_img_file = File::open(boot_img_path)?;
    let written_size = copy(&mut boot_img_file, &mut iso)?;
    let padded_size = boot_img_sectors * 512;
    if written_size < padded_size {
        let padding = vec![0u8; (padded_size - written_size) as usize];
        iso.write_all(&padding)?;
    }

    let final_pos = iso.stream_position()?;
    let total_sectors = final_pos.div_ceil(ISO_SECTOR_SIZE as u64);
    update_4byte_fields(
        &mut iso,
        16,
        PVD_TOTAL_SECTORS_OFFSET,
        PVD_TOTAL_SECTORS_OFFSET + 4,
        total_sectors as u32,
    )?;
    iso.set_len(total_sectors * ISO_SECTOR_SIZE as u64)?;

    println!(
        "create_iso_from_img: ISO created with {} sectors",
        total_sectors
    );
    Ok(())
}
