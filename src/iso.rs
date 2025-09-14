// isobemak/src/iso.rs
// ISO + El Torito
use crate::utils::{SECTOR_SIZE, pad_sector};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, Write},
    path::Path,
};

pub fn create_iso(path: &Path, fat32_img: &Path) -> io::Result<()> {
    println!(
        "create_iso: Checking if FAT32 image exists at: {}",
        fat32_img.display()
    );
    if !fat32_img.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "FAT32 image not found in create_iso",
        ));
    }
    let mut iso = File::create(path)?;
    io::copy(&mut io::repeat(0).take(SECTOR_SIZE as u64 * 16), &mut iso)?; // System Area

    // Primary Volume Descriptor
    let mut pvd = [0u8; SECTOR_SIZE];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[6] = 1;
    let mut volume_id = [0u8; 32];
    let project_name = b"FULLERENE";
    volume_id[..project_name.len()].copy_from_slice(project_name);
    volume_id[project_name.len()..].fill(b' ');
    pvd[40..72].copy_from_slice(&volume_id);

    let fat32_img_sectors = (fs::metadata(fat32_img)?.len() as u32).div_ceil(SECTOR_SIZE as u32);

    let total_sectors = 16 + 1 + 1 + 1 + 1 + fat32_img_sectors;

    pvd[80..84].copy_from_slice(&total_sectors.to_le_bytes());
    pvd[84..88].copy_from_slice(&total_sectors.to_be_bytes());
    pvd[128..132].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes());

    let mut root_dir_record = [0u8; 34];
    root_dir_record[0] = 34;
    root_dir_record[1] = 0;
    root_dir_record[2..6].copy_from_slice(&20u32.to_le_bytes());
    root_dir_record[6..10].copy_from_slice(&20u32.to_be_bytes());
    root_dir_record[10..14].copy_from_slice(&fat32_img_sectors.to_le_bytes());
    root_dir_record[14..18].copy_from_slice(&fat32_img_sectors.to_be_bytes());
    root_dir_record[25] = 0x02;
    root_dir_record[26] = 0;
    root_dir_record[27] = 0;
    root_dir_record[28..30].copy_from_slice(&1u16.to_le_bytes());
    root_dir_record[30..32].copy_from_slice(&1u16.to_be_bytes());
    root_dir_record[32] = 1;
    root_dir_record[33] = 0x00;
    pvd[156..190].copy_from_slice(&root_dir_record);
    iso.write_all(&pvd)?;

    // Boot Record Volume Descriptor
    let mut brvd = [0u8; SECTOR_SIZE];
    brvd[0] = 0;
    brvd[1..6].copy_from_slice(b"CD001");
    brvd[6] = 1;
    let mut el_torito_spec = [0u8; 32];
    let spec_name = b"EL TORITO SPECIFICATION";
    el_torito_spec[..spec_name.len()].copy_from_slice(spec_name);
    for i in spec_name.len()..32 {
        el_torito_spec[i] = 0x00;
    }
    brvd[7..39].copy_from_slice(&el_torito_spec);
    brvd[71..75].copy_from_slice(&19u32.to_le_bytes());
    iso.write_all(&brvd)?;

    // Volume Descriptor Terminator
    let mut term = [0u8; SECTOR_SIZE];
    term[0] = 255;
    term[1..6].copy_from_slice(b"CD001");
    term[6] = 1;
    iso.write_all(&term)?;

    let current_pos = iso.stream_position()?;
    let target_pos = 19 * SECTOR_SIZE as u64;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        io::copy(&mut io::repeat(0).take(padding_bytes), &mut iso)?;
    }

    // Boot Catalog (LBA 19)
    let mut cat = [0u8; SECTOR_SIZE];
    cat[0] = 1;
    cat[1] = 0xEF;
    cat[2..4].copy_from_slice(&0u16.to_le_bytes());
    cat[30] = 0x55;
    cat[31] = 0xAA;
    let mut sum: u16 = 0;
    for i in (0..32).step_by(2) {
        sum = sum.wrapping_add(u16::from_le_bytes([cat[i], cat[i + 1]]));
    }
    let checksum = 0u16.wrapping_sub(sum);
    cat[28..30].copy_from_slice(&checksum.to_le_bytes());

    let mut entry = [0u8; 32];
    entry[0] = 0x88;
    entry[1] = 0x00;
    entry[2..4].copy_from_slice(&0u16.to_le_bytes());
    entry[4] = 0x00;
    entry[5] = 0x00;

    let fat32_img_bytes = fs::metadata(fat32_img)?.len();
    let sector_count_512 = fat32_img_bytes.div_ceil(512);

    let sector_count_u16 = if sector_count_512 > 0xFFFF {
        0xFFFF
    } else {
        sector_count_512 as u16
    };

    entry[6..8].copy_from_slice(&sector_count_u16.to_le_bytes());
    entry[8..12].copy_from_slice(&20u32.to_le_bytes());
    cat[32..64].copy_from_slice(&entry);
    iso.write_all(&cat)?;

    let current_pos = iso.stream_position()?;
    let target_pos = 20 * SECTOR_SIZE as u64;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        io::copy(&mut io::repeat(0).take(padding_bytes), &mut iso)?;
    }

    let mut f = File::open(fat32_img)?;
    io::copy(&mut f, &mut iso)?;
    pad_sector(&mut iso)?;
    Ok(())
}
