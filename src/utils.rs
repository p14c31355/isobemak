// isobemak/src/utils.rs
use std::{
    fs::File,
    io::{self, Read, Seek},
};

pub const ISO_SECTOR_SIZE: usize = 2048;
pub const FAT32_SECTOR_SIZE: u64 = 512;

/// Pads a file with zeros to align to the ISO sector size.
pub fn pad_sector(f: &mut File) -> io::Result<()> {
    let pos = f.stream_position()?;
    let pad = ISO_SECTOR_SIZE as u64 - (pos % ISO_SECTOR_SIZE as u64);
    if pad != ISO_SECTOR_SIZE as u64 {
        io::copy(&mut io::repeat(0).take(pad), f)?;
    }
    Ok(())
}
