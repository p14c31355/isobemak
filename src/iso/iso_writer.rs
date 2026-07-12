use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::iso::boot_catalog::{BootCatalogEntry, write_boot_catalog};
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::fs_node::{IsoDirectory, IsoFsNode};
use crate::iso::volume_descriptor::{update_total_sectors_in_pvd, write_volume_descriptors};
use crate::utils::{ISO_SECTOR_SIZE, seek_to_lba};

/// Writes all ISO volume descriptors.
pub fn write_descriptors(
    iso_file: &mut File,
    volume_id: Option<&str>,
    root_lba: u32,
    total_sectors: u32,
) -> io::Result<()> {
    let root_entry = IsoDirEntry {
        lba: root_lba,
        size: ISO_SECTOR_SIZE as u32,
        flags: 0x02,
        name: ".",
    };
    write_volume_descriptors(iso_file, volume_id, total_sectors, &root_entry)
}

/// Writes the El Torito boot catalog.
pub fn write_boot_catalog_to_iso(
    iso_file: &mut File,
    boot_catalog_lba: u32,
    boot_entries: Vec<BootCatalogEntry>,
) -> io::Result<()> {
    if !boot_entries.is_empty() {
        iso_file.seek(SeekFrom::Start(
            (boot_catalog_lba as u64) * ISO_SECTOR_SIZE as u64,
        ))?;
        write_boot_catalog(iso_file, boot_entries)?;
    }
    Ok(())
}

/// Writes the directory records for the ISO filesystem.
pub fn write_directories(
    iso_file: &mut File,
    dir: &IsoDirectory,
    parent_lba: u32,
) -> io::Result<()> {
    seek_to_lba(iso_file, dir.lba)?;

    let mut dir_entries = Vec::new();
    // Self-reference
    dir_entries.push(IsoDirEntry {
        lba: dir.lba,
        size: ISO_SECTOR_SIZE as u32,
        flags: 0x02,
        name: ".",
    });
    // Parent directory
    dir_entries.push(IsoDirEntry {
        lba: parent_lba,
        size: ISO_SECTOR_SIZE as u32,
        flags: 0x02,
        name: "..",
    });

    for_sorted_children!(dir, |name, node| {
        let (lba, size, flags) = match node {
            IsoFsNode::File(file) => {
                let file_size_u32 = u32::try_from(file.size).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "File '{}' is too large for ISO9660 (exceeds u32::MAX bytes)",
                            name
                        ),
                    )
                })?;
                (file.lba, file_size_u32, 0x00)
            }
            IsoFsNode::Directory(subdir) => (subdir.lba, ISO_SECTOR_SIZE as u32, 0x02),
        };
        dir_entries.push(IsoDirEntry {
            lba,
            size,
            flags,
            name: name.as_str(),
        });
    });

    let mut dir_sector = [0u8; ISO_SECTOR_SIZE];
    let mut offset = 0;

    for entry in &dir_entries {
        let entry_bytes = entry.to_bytes();
        dir_sector[offset..offset + entry_bytes.len()].copy_from_slice(&entry_bytes);
        offset += entry_bytes.len();
    }
    iso_file.write_all(&dir_sector)?;

    for_sorted_children!(dir, |_name, node| {
        if let IsoFsNode::Directory(subdir) = node {
            write_directories(iso_file, subdir, dir.lba)?;
        }
    });

    Ok(())
}

/// Copies all file contents to the ISO image.
pub fn copy_files(iso_file: &mut File, dir: &IsoDirectory) -> io::Result<()> {
    for_sorted_children!(dir, |_name, node| {
        match node {
            IsoFsNode::File(file) => {
                seek_to_lba(iso_file, file.lba)?;
                let mut real_file = File::open(&file.path)?;
                io::copy(&mut real_file, iso_file)?;
            }
            IsoFsNode::Directory(subdir) => {
                copy_files(iso_file, subdir)?;
            }
        }
    });

    Ok(())
}

const PVD_LBA: u32 = 16;

/// Writes the boot information table into the BIOS boot image at offsets 8–63.
///
/// The boot information table (a.k.a. `-boot-info-table` in mkisofs/xorriso) tells
/// BIOS-stage‑1 bootloaders (e.g. ISOLINUX, Limine) where the PVD, the boot image
/// itself, and the rest of the filesystem are located.  Without this table the
/// bootloader cannot proceed past "Booting from DVD/CD…".
///
/// Layout (56 bytes at file offset 8):
///
/// | Offset | Size | Field              |
/// |--------|------|--------------------|
/// |  8     | 4    | PVD LBA            |
/// | 12     | 4    | Boot image LBA     |
/// | 16     | 4    | Boot image length  |
/// | 20     | 4    | Checksum of bytes 64+ |
/// | 24     | 32   | Reserved (zero)    |
pub fn write_boot_info_table(
    iso_file: &mut File,
    boot_image_lba: u32,
    boot_image_size: u64,
) -> io::Result<()> {
    let sector_base = boot_image_lba as u64 * ISO_SECTOR_SIZE as u64;
    let checksum_start = sector_base + 64;

    // Compute checksum of all full u32 LE words from byte 64 to end-of-file.
    let mut checksum = 0u32;
    if boot_image_size > 64 {
        iso_file.seek(SeekFrom::Start(checksum_start))?;
        let mut buf = [0u8; 4096];
        let mut remaining = boot_image_size - 64;
        while remaining > 0 {
            let to_read = buf.len().min(remaining as usize);
            iso_file.read_exact(&mut buf[..to_read])?;
            for chunk in buf[..to_read].chunks_exact(4) {
                checksum = checksum.wrapping_add(u32::from_le_bytes(chunk.try_into().unwrap()));
            }
            remaining -= to_read as u64;
        }
    }

    // Write the 56-byte table at offset 8 within the boot image's extent.
    let table_offset = sector_base + 8;
    iso_file.seek(SeekFrom::Start(table_offset))?;
    let mut table = [0u8; 56];
    table[0..4].copy_from_slice(&PVD_LBA.to_le_bytes());
    table[4..8].copy_from_slice(&boot_image_lba.to_le_bytes());
    table[8..12].copy_from_slice(&(boot_image_size as u32).to_le_bytes());
    table[12..16].copy_from_slice(&checksum.to_le_bytes());
    iso_file.write_all(&table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::NamedTempFile;

    fn read_sector(file: &mut File, lba: u32) -> io::Result<[u8; ISO_SECTOR_SIZE]> {
        let mut buf = [0u8; ISO_SECTOR_SIZE];
        file.seek(SeekFrom::Start(lba as u64 * ISO_SECTOR_SIZE as u64))?;
        file.read_exact(&mut buf)?;
        Ok(buf)
    }

    #[test]
    fn test_boot_info_table_structure() -> io::Result<()> {
        let mut f = NamedTempFile::new()?;

        // Write one sector of dummy boot image data at LBA 50.
        let boot_lba = 50u32;
        let boot_size: u64 = 2048;
        let boot_offset = boot_lba as u64 * ISO_SECTOR_SIZE as u64;

        let mut boot_data = vec![0u8; boot_size as usize];
        // Fill bytes 64.. with a known pattern for checksum verification.
        for i in 64..boot_size as usize {
            boot_data[i] = (i as u8).wrapping_mul(3).wrapping_add(0xAB);
        }
        f.seek(SeekFrom::Start(boot_offset))?;
        f.write_all(&boot_data)?;

        write_boot_info_table(f.as_file_mut(), boot_lba, boot_size)?;

        // Read back the 56-byte table at offset 8.
        let mut table = [0u8; 56];
        f.seek(SeekFrom::Start(boot_offset + 8))?;
        f.read_exact(&mut table)?;

        // PVD LBA
        assert_eq!(
            u32::from_le_bytes(table[0..4].try_into().unwrap()),
            16,
            "PVD LBA should be 16"
        );
        // Boot image LBA
        assert_eq!(
            u32::from_le_bytes(table[4..8].try_into().unwrap()),
            boot_lba,
            "boot image LBA mismatch"
        );
        // Boot image size
        assert_eq!(
            u32::from_le_bytes(table[8..12].try_into().unwrap()),
            boot_size as u32,
            "boot image size mismatch"
        );
        // Reserved bytes 24..63 (table[16..56]) must be zero
        assert_eq!(&table[16..56], &[0u8; 40], "reserved bytes not zero");

        // Manually compute expected checksum: sum of all u32 LE words at bytes 64..
        let mut expected_csum = 0u32;
        for chunk in boot_data[64..].chunks_exact(4) {
            expected_csum =
                expected_csum.wrapping_add(u32::from_le_bytes(chunk.try_into().unwrap()));
        }
        let actual_csum = u32::from_le_bytes(table[12..16].try_into().unwrap());
        assert_eq!(actual_csum, expected_csum, "checksum mismatch");

        Ok(())
    }

    #[test]
    fn test_boot_info_table_exact_64_bytes() -> io::Result<()> {
        // Edge case: boot image is exactly 64 bytes.
        // The table fills offsets 8..63, and there are zero checksum bytes.
        let mut f = NamedTempFile::new()?;
        let boot_lba = 10u32;
        let boot_size: u64 = 64;

        let boot_offset = boot_lba as u64 * ISO_SECTOR_SIZE as u64;
        f.seek(SeekFrom::Start(boot_offset))?;
        f.write_all(&[0xFFu8; 2048])?;

        write_boot_info_table(f.as_file_mut(), boot_lba, boot_size)?;

        let mut table = [0u8; 56];
        f.seek(SeekFrom::Start(boot_offset + 8))?;
        f.read_exact(&mut table)?;

        assert_eq!(
            u32::from_le_bytes(table[12..16].try_into().unwrap()),
            0,
            "checksum should be 0 when file has no data beyond offset 64"
        );
        Ok(())
    }

    #[test]
    fn test_boot_info_table_does_not_overflow_sector() -> io::Result<()> {
        // The table must be fully contained within the first sector.
        // Even for tiny boot images, writing at offset 8 must not corrupt
        // data in adjacent sectors.
        let mut f = NamedTempFile::new()?;
        let boot_lba = 99u32;

        // Pre-fill the sector with 0xAA so we can detect unintended writes.
        let boot_offset = boot_lba as u64 * ISO_SECTOR_SIZE as u64;
        let sector = [0xAAu8; ISO_SECTOR_SIZE as usize];
        f.seek(SeekFrom::Start(boot_offset))?;
        f.write_all(&sector)?;

        write_boot_info_table(f.as_file_mut(), boot_lba, 128)?;

        let sector_read = read_sector(f.as_file_mut(), boot_lba)?;
        // Bytes 8..63 may have changed, but byte 64+ should still be 0xAA.
        assert_eq!(
            &sector_read[64..128],
            &[0xAAu8; 64],
            "boot-info-table wrote beyond offset 63"
        );
        Ok(())
    }
}

/// Finalizes the ISO image by padding and updating the total sector count in the PVD.
pub fn finalize_iso(iso_file: &mut File, total_sectors: &mut u32) -> io::Result<()> {
    let current_pos = iso_file.stream_position()?;
    let remainder = current_pos % ISO_SECTOR_SIZE as u64;
    if remainder != 0 {
        let padding_bytes = ISO_SECTOR_SIZE as u64 - remainder;
        io::copy(&mut io::repeat(0).take(padding_bytes), iso_file)?;
    }

    let final_pos = iso_file.stream_position()?;
    let total_sectors_u64 = final_pos.div_ceil(ISO_SECTOR_SIZE as u64);
    *total_sectors = u32::try_from(total_sectors_u64)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "ISO image too large"))?;
    update_total_sectors_in_pvd(iso_file, *total_sectors)?;

    Ok(())
}
