// isobemak/src/utils.rs
use std::{
    fs::File,
    io::{self, Read, Seek},
};

pub const ISO_SECTOR_SIZE: usize = 2048;

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
