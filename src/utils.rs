// isobemak/src/utils.rs
use std::{
    fs::File,
    io::{self, Seek, Write},
};

pub const ISO_SECTOR_SIZE: usize = 2048;
pub const FAT32_SECTOR_SIZE: u64 = 512;

pub fn pad_sector(f: &mut File) -> io::Result<()> {
    let pos = f.stream_position()?;
    let pad = ISO_SECTOR_SIZE as u64 - (pos % ISO_SECTOR_SIZE as u64);
    if pad != ISO_SECTOR_SIZE as u64 {
        let zeros = [0u8; ISO_SECTOR_SIZE];
        let mut written = 0;
        while written < pad {
            let to_write = std::cmp::min(pad as usize - written as usize, zeros.len());
            f.write_all(&zeros[..to_write])?;
            written += to_write as u64;
        }
    }
    Ok(())
}
