use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::for_sorted_children;
use crate::iso::boot_catalog::{BootCatalogEntry, write_boot_catalog};
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::fs_node::{IsoDirectory, IsoFsNode};
use crate::iso::volume_descriptor::{update_total_sectors_in_pvd, write_volume_descriptors};
use crate::utils::{ISO_SECTOR_SIZE, seek_to_lba};

/// Writes all ISO volume descriptors.
pub fn write_descriptors(iso_file: &mut File, root_lba: u32, total_sectors: u32) -> io::Result<()> {
    let root_entry = IsoDirEntry {
        lba: root_lba,
        size: ISO_SECTOR_SIZE as u32,
        flags: 0x02,
        name: ".",
    };
    write_volume_descriptors(iso_file, total_sectors, &root_entry)
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
