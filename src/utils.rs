// isobemak/src/utils.rs
use fatfs::{self};
use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    path::Path,
};

/// The standard size of an ISO 9660 sector in bytes.
pub const ISO_SECTOR_SIZE: usize = 2048;

/// Pads the ISO file with zeros to align to a specific LBA (Logical Block Address).
pub fn pad_to_lba(iso: &mut File, lba: u32) -> io::Result<()> {
    let target_pos = lba as u64 * ISO_SECTOR_SIZE as u64;
    let current_pos = iso.stream_position()?;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        io::copy(&mut io::repeat(0).take(padding_bytes), iso)?;
    }
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::NamedTempFile;

    #[test]
    fn test_pad_to_lba() -> io::Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        let initial_content = b"hello";
        temp_file.write_all(initial_content)?;

        let mut file = temp_file.reopen()?;
        pad_to_lba(&mut file, 2)?;

        let file_size = file.metadata()?.len();
        assert_eq!(file_size, 2 * ISO_SECTOR_SIZE as u64);

        Ok(())
    }

    #[test]
    fn test_copy_to_fat() -> io::Result<()> {
        // Create a dummy source file
        let mut src_file = NamedTempFile::new()?;
        let src_content = b"This is a test file.";
        src_file.write_all(src_content)?;

        // Create an in-memory FAT filesystem
        let mut fat_image = vec![0; 512 * 1024]; // 512KB
        let mut cursor = Cursor::new(fat_image.as_mut_slice());
        fatfs::format_volume(&mut cursor, fatfs::FormatVolumeOptions::new())?;
        let fs = fatfs::FileSystem::new(&mut cursor, fatfs::FsOptions::new())?;
        let root_dir = fs.root_dir();

        // Copy the file
        let dest_filename = "test.txt";
        copy_to_fat(&root_dir, src_file.path(), dest_filename)?;

        // Verify the file exists and its content is correct
        let mut dest_file = root_dir.open_file(dest_filename)?;
        let mut dest_content = Vec::new();
        dest_file.read_to_end(&mut dest_content)?;

        assert_eq!(src_content, dest_content.as_slice());

        Ok(())
    }
}
