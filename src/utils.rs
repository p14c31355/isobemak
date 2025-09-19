// isobemak/src/utils.rs

pub const ISO_SECTOR_SIZE: usize = 2048;
pub const FAT32_SECTOR_SIZE: u64 = 512;

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

/// Reads the entire file from a specified path and returns its content.
pub fn read_file_from_path(file_path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(file_path)?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    Ok(content)
}

/// Pads the ISO file with zeros to align to a specific LBA.
pub fn pad_to_lba(f: &mut File, lba: u32) -> io::Result<()> {
    let pos = f.stream_position()?;
    let target = lba as u64 * ISO_SECTOR_SIZE as u64;
    if pos < target {
        let pad = vec![0u8; (target - pos) as usize];
        f.write_all(&pad)?;
    }
    Ok(())
}

/// A helper function to update two 4-byte fields at different offsets
/// within a single ISO sector (2048 bytes). This is used for the
/// total sector count in the PVD.
pub fn update_4byte_fields(
    iso: &mut File,
    base_lba: u32,
    offset1: usize,
    offset2: usize,
    value: u32,
) -> io::Result<()> {
    let base_offset = base_lba as u64 * ISO_SECTOR_SIZE as u64;

    iso.seek(SeekFrom::Start(base_offset + offset1 as u64))?;
    iso.write_all(&value.to_le_bytes())?;

    iso.seek(SeekFrom::Start(base_offset + offset2 as u64))?;
    iso.write_all(&value.to_be_bytes())?;

    Ok(())
}
