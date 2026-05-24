use crate::iso::boot_catalog::LBA_BOOT_CATALOG;
use crate::iso::dir_record::IsoDirEntry;
use crate::utils::{ISO_SECTOR_SIZE, seek_to_lba};
use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};

const PVD_VOL_ID: usize = 40;
const PVD_TOTAL_SEC: usize = 80;
const PVD_ROOT_DIR: usize = 156;
const PVD_VOL_SET_SIZE: usize = 120;
const PVD_VOL_SEQ_NUM: usize = 124;
const PVD_LOGICAL_BLOCK: usize = 128;
const PVD_PATH_TABLE: usize = 132;

fn write_dual(buf: &mut [u8], off: usize, val: u32, len: usize) {
    let le = val.to_le_bytes();
    let be = val.to_be_bytes();
    if len == 2 {
        buf[off..off + 2].copy_from_slice(&le[..2]);
        buf[off + 2..off + 4].copy_from_slice(&be[..2]);
    } else {
        buf[off..off + 4].copy_from_slice(&le);
        buf[off + 4..off + 8].copy_from_slice(&be);
    }
}

pub fn write_primary_volume_descriptor(
    iso: &mut File,
    volume_id: Option<&str>,
    total_sectors: u32,
    root_entry: &IsoDirEntry,
) -> io::Result<()> {
    seek_to_lba(iso, 16)?;
    let mut pvd = [0u8; ISO_SECTOR_SIZE];
    pvd[0] = 1; // primary
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[6] = 1;

    let name = volume_id.map_or(b"ISOBEMAKI" as &[u8], |id| {
        &id.as_bytes()[..id.len().min(32)]
    });
    let mut vol = [b' '; 32];
    vol[..name.len()].copy_from_slice(name);
    pvd[PVD_VOL_ID..PVD_VOL_ID + 32].copy_from_slice(&vol);

    write_dual(&mut pvd, PVD_TOTAL_SEC, total_sectors, 4);
    write_dual(&mut pvd, PVD_VOL_SET_SIZE, 1, 2);
    write_dual(&mut pvd, PVD_VOL_SEQ_NUM, 1, 2);
    write_dual(&mut pvd, PVD_LOGICAL_BLOCK, ISO_SECTOR_SIZE as u32, 2);
    write_dual(&mut pvd, PVD_PATH_TABLE, 0, 4);

    let re = root_entry.to_bytes();
    pvd[PVD_ROOT_DIR..PVD_ROOT_DIR + re.len()].copy_from_slice(&re);
    pvd[881] = 1;
    pvd[813..830].copy_from_slice(b"2024010100000000\x00");
    pvd[830..847].copy_from_slice(b"2024010100000000\x00");
    iso.write_all(&pvd)
}

pub fn update_total_sectors_in_pvd(iso: &mut File, total_sectors: u32) -> io::Result<()> {
    let base = 16 * ISO_SECTOR_SIZE as u64;
    iso.seek(SeekFrom::Start(base + PVD_TOTAL_SEC as u64))?;
    iso.write_all(&total_sectors.to_le_bytes())?;
    iso.seek(SeekFrom::Start(base + PVD_TOTAL_SEC as u64 + 4))?;
    iso.write_all(&total_sectors.to_be_bytes())
}

fn write_boot_record_vd(iso: &mut File) -> io::Result<()> {
    seek_to_lba(iso, 17)?;
    let mut brvd = [0u8; ISO_SECTOR_SIZE];
    brvd[0] = 0;
    brvd[1..6].copy_from_slice(b"CD001");
    brvd[6] = 1;
    brvd[7..30].copy_from_slice(b"EL TORITO SPECIFICATION");
    brvd[71..75].copy_from_slice(&LBA_BOOT_CATALOG.to_le_bytes());
    iso.write_all(&brvd)
}

fn write_terminator(iso: &mut File) -> io::Result<()> {
    seek_to_lba(iso, 18)?;
    let mut t = [0u8; ISO_SECTOR_SIZE];
    t[0] = 255;
    t[1..6].copy_from_slice(b"CD001");
    t[6] = 1;
    iso.write_all(&t)
}

pub fn write_volume_descriptors(
    iso: &mut File,
    volume_id: Option<&str>,
    total_sectors: u32,
    root_entry: &IsoDirEntry,
) -> io::Result<()> {
    write_primary_volume_descriptor(iso, volume_id, total_sectors, root_entry)?;
    write_boot_record_vd(iso)?;
    write_terminator(iso)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::NamedTempFile;

    fn read_sector(file: &mut File, lba: u32) -> io::Result<[u8; ISO_SECTOR_SIZE]> {
        let mut buf = [0u8; ISO_SECTOR_SIZE];
        file.seek(SeekFrom::Start(lba as u64 * ISO_SECTOR_SIZE as u64))?;
        file.read_exact(&mut buf)?;
        Ok(buf)
    }

    #[test]
    fn test_pvd() -> io::Result<()> {
        let mut f = NamedTempFile::new()?;
        let re = IsoDirEntry {
            lba: 20,
            size: 2048,
            flags: 2,
            name: ".",
        };
        write_primary_volume_descriptor(f.as_file_mut(), None, 1000, &re)?;
        let s = read_sector(f.as_file_mut(), 16)?;
        assert_eq!(s[0], 1);
        assert_eq!(&s[1..6], b"CD001");
        assert_eq!(&s[PVD_TOTAL_SEC..PVD_TOTAL_SEC + 4], &1000u32.to_le_bytes());
        let r = re.to_bytes();
        assert_eq!(&s[PVD_ROOT_DIR..PVD_ROOT_DIR + r.len()], &r);
        Ok(())
    }

    #[test]
    fn test_update_pvd() -> io::Result<()> {
        let mut f = NamedTempFile::new()?;
        let re = IsoDirEntry {
            lba: 20,
            size: 2048,
            flags: 2,
            name: ".",
        };
        write_primary_volume_descriptor(f.as_file_mut(), None, 1000, &re)?;
        update_total_sectors_in_pvd(f.as_file_mut(), 2500)?;
        let s = read_sector(f.as_file_mut(), 16)?;
        assert_eq!(
            u32::from_le_bytes(s[PVD_TOTAL_SEC..PVD_TOTAL_SEC + 4].try_into().unwrap()),
            2500
        );
        assert_eq!(
            u32::from_be_bytes(s[PVD_TOTAL_SEC + 4..PVD_TOTAL_SEC + 8].try_into().unwrap()),
            2500
        );
        Ok(())
    }

    #[test]
    fn test_all_vds() -> io::Result<()> {
        let mut f = NamedTempFile::new()?;
        let re = IsoDirEntry {
            lba: 20,
            size: 2048,
            flags: 2,
            name: ".",
        };
        write_volume_descriptors(f.as_file_mut(), None, 1234, &re)?;
        assert_eq!(read_sector(f.as_file_mut(), 16)?[0], 1);
        assert_eq!(read_sector(f.as_file_mut(), 17)?[0], 0);
        assert_eq!(read_sector(f.as_file_mut(), 18)?[0], 255);
        Ok(())
    }
}
