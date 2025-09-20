// isobemak/src/utils.rs

pub const ISO_SECTOR_SIZE: usize = 2048;
pub const FAT32_SECTOR_SIZE: u64 = 512;

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};
use fatfs::{self, ReadWriteSeek};

/// Reads the entire file from a specified path and returns its content.
pub fn read_file_from_path(file_path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(file_path)?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    Ok(content)
}

/// Pads the ISO file with zeros to align to a specific LBA.
pub fn pad_to_lba(iso: &mut File, lba: u32) -> io::Result<()> {
    let target_pos = lba as u64 * ISO_SECTOR_SIZE as u64;
    let current_pos = iso.stream_position()?;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        io::copy(&mut io::repeat(0).take(padding_bytes), iso)?;
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

/// Copies a file from the host filesystem into a FAT directory.
pub fn copy_to_fat<T: Read + Write + Seek>(
    dir: &fatfs::Dir<T>,
    src_path: &Path,
    dest: &str,
) -> io::Result<()> {
    let mut src_file = File::open(src_path)?;
    let mut f = dir.create_file(dest)?;
    io::copy(&mut src_file, &mut f)?;
    f.flush()?;
    println!("Copied {} to {} in FAT image.", src_path.display(), dest);
    Ok(())
}
