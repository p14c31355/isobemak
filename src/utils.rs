use std::fs::File;
use std::io::{self, Seek, SeekFrom};

pub const ISO_SECTOR_SIZE: usize = 2048;

pub fn seek_to_lba(file: &mut File, lba: u32) -> io::Result<u64> {
    let target_pos = lba as u64 * ISO_SECTOR_SIZE as u64;
    file.seek(SeekFrom::Start(target_pos))
}

#[cfg(test)]
pub mod test_utils {
    use std::fs;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};

    /// Creates a dummy file with the specified size in a temporary directory.
    pub fn create_dummy_file(
        temp_dir: &Path,
        name: &str,
        size_kb: usize,
    ) -> io::Result<PathBuf> {
        let path = temp_dir.join(name);
        let mut file = fs::File::create(&path)?;
        file.write_all(&vec![0u8; size_kb * 1024])?;
        Ok(path)
    }

    /// A macro to simplify the creation of multiple dummy files.
    #[macro_export]
    macro_rules! create_dummy_files {
        ($temp_dir:expr, $($name:expr => $size_kb:expr),*) => {
            {
                let mut paths = std::collections::HashMap::new();
                $(
                    let path = $crate::utils::test_utils::create_dummy_file($temp_dir, $name, $size_kb).unwrap();
                    paths.insert($name.to_string(), path);
                )*
                paths
            }
        };
    }
}
