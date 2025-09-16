// isobemak/src/utils.rs
use std::{
    fs::File,
    io::{self, Seek, Write},
};

pub const SECTOR_SIZE: usize = 2048;

pub fn pad_sector(f: &mut File) -> io::Result<()> {
    let pos = f.stream_position()?;
    let pad = SECTOR_SIZE as u64 - (pos % SECTOR_SIZE as u64);
    if pad != SECTOR_SIZE as u64 {
        f.write_all(&vec![0u8; pad as usize])?;
    }
    Ok(())
}
