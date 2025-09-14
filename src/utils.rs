// isobemak/src/utils.rs
use std::{
    fs::File,
    io::{self, Seek, SeekFrom, Write},
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

// Simple CRC16 (for Validation Entry) - No longer used for El Torito checksum
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            if (crc & 0x8000) != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}
