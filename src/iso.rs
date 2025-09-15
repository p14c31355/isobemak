// isobemak/src/iso.rs
// ISO + El Torito
use crate::utils::{SECTOR_SIZE, pad_sector};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, Write},
    path::Path,
};

/// Creates a general ISO 9660 directory or file record.
/// This function handles the common fields for a directory record.
fn create_dir_record(lba: u32, size: u32, is_dir: bool, name: &[u8]) -> Vec<u8> {
    let name_len = name.len() as u8;
    let record_len = 33 + name_len;
    let mut record = vec![0u8; record_len as usize];

    record[0] = record_len; // Length of Directory Record
    record[1] = 0; // Extended Attribute Record Length
    record[2..6].copy_from_slice(&lba.to_le_bytes()); // Location of Extent (LE)
    record[6..10].copy_from_slice(&lba.to_be_bytes()); // Location of Extent (BE)
    record[10..14].copy_from_slice(&size.to_le_bytes()); // Data Length (LE)
    record[14..18].copy_from_slice(&size.to_be_bytes()); // Data Length (BE)
    // Date and time (bytes 18-24) - left as 0
    record[25] = if is_dir { 0x02 } else { 0x00 }; // File Flags (Directory or File)
    // File Unit Size (byte 26) - 0
    // Interleave Gap Size (byte 27) - 0
    record[28..30].copy_from_slice(&1u16.to_le_bytes()); // Volume Sequence Number (LE)
    record[30..32].copy_from_slice(&1u16.to_be_bytes()); // Volume Sequence Number (BE)
    record[32] = name_len; // Length of File Identifier
    record[33..33 + name_len as usize].copy_from_slice(name); // File Identifier

    record
}

/// Creates a Directory Record for the current directory ('.').
fn create_dot_entry(lba: u32, size: u32) -> [u8; 34] {
    let mut record = [0u8; 34];
    record[0] = 34; // Length of Directory Record
    record[1] = 0;
    record[2..6].copy_from_slice(&lba.to_le_bytes());
    record[6..10].copy_from_slice(&lba.to_be_bytes());
    record[10..14].copy_from_slice(&size.to_le_bytes());
    record[14..18].copy_from_slice(&size.to_be_bytes());
    record[25] = 0x02; // Directory
    record[28..30].copy_from_slice(&1u16.to_le_bytes());
    record[30..32].copy_from_slice(&1u16.to_be_bytes());
    record[32] = 1;
    record[33] = 0x00; // '.'
    record
}

/// Creates a Directory Record for the parent directory ('..').
fn create_dotdot_entry(parent_lba: u32, parent_size: u32) -> [u8; 34] {
    let mut record = [0u8; 34];
    record[0] = 34;
    record[1] = 0;
    record[2..6].copy_from_slice(&parent_lba.to_le_bytes());
    record[6..10].copy_from_slice(&parent_lba.to_be_bytes());
    record[10..14].copy_from_slice(&parent_size.to_le_bytes());
    record[14..18].copy_from_slice(&parent_size.to_be_bytes());
    record[25] = 0x02; // Directory
    record[28..30].copy_from_slice(&1u16.to_le_bytes());
    record[30..32].copy_from_slice(&1u16.to_be_bytes());
    record[32] = 1;
    record[33] = 0x01; // '..'
    record
}

pub fn create_iso(path: &Path, bellows_path: &Path, kernel_path: &Path) -> io::Result<()> {
    println!(
        "create_iso: Creating ISO with bellows: {} and kernel: {}",
        bellows_path.display(),
        kernel_path.display()
    );

    if !bellows_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Bellows EFI file not found at {}", bellows_path.display()),
        ));
    }
    if !kernel_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Kernel EFI file not found at {}", kernel_path.display()),
        ));
    }

    let mut iso = File::create(path)?;
    io::copy(&mut io::repeat(0).take(SECTOR_SIZE as u64 * 16), &mut iso)?; // System Area

    // Read EFI files
    let bellows_file_content = fs::read(bellows_path)?;
    let kernel_file_content = fs::read(kernel_path)?;

    let bellows_sectors = (bellows_file_content.len() as u32).div_ceil(SECTOR_SIZE as u32);
    let kernel_sectors = (kernel_file_content.len() as u32).div_ceil(SECTOR_SIZE as u32);

    // Calculate LBAs for EFI files and directory structure
    let efi_dir_lba = 20;
    let boot_dir_lba = efi_dir_lba + 1;
    let bellows_efi_lba = boot_dir_lba + 1;
    let kernel_efi_lba = bellows_efi_lba + bellows_sectors;

    let total_sectors = kernel_efi_lba + kernel_sectors;

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

    pvd[80..84].copy_from_slice(&total_sectors.to_le_bytes());
    pvd[84..88].copy_from_slice(&total_sectors.to_be_bytes());
    pvd[128..132].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes());

    // Root Directory Record (for PVD)
    let root_dir_lba = efi_dir_lba;
    let root_dir_size = SECTOR_SIZE as u32;
    let root_dir_record = create_dot_entry(root_dir_lba, root_dir_size);
    pvd[156..190].copy_from_slice(&root_dir_record);
    iso.write_all(&pvd)?;

    // Boot Record Volume Descriptor (LBA 17)
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
    brvd[71..75].copy_from_slice(&19u32.to_le_bytes()); // Boot Catalog LBA (LBA 19)
    iso.write_all(&brvd)?;

    // Volume Descriptor Terminator (LBA 18)
    let mut term = [0u8; SECTOR_SIZE];
    term[0] = 255;
    term[1..6].copy_from_slice(b"CD001");
    term[6] = 1;
    iso.write_all(&term)?;

    // Pad to Boot Catalog LBA (LBA 19)
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
    entry[0] = 0x88; // Bootable, EFI
    entry[1] = 0x00; // Boot media type (no emulation)
    entry[2..4].copy_from_slice(&0u16.to_le_bytes()); // Load segment (0)
    entry[4] = 0x00; // System type (0)
    entry[5] = 0x00; // Unused

    let sector_count_512 = bellows_file_content.len().div_ceil(512);
    let sector_count_u16 = if sector_count_512 > 0xFFFF {
        0xFFFF
    } else {
        sector_count_512 as u16
    };

    entry[6..8].copy_from_slice(&sector_count_u16.to_le_bytes()); // Sector count (512-byte units)
    entry[8..12].copy_from_slice(&bellows_efi_lba.to_le_bytes()); // LBA of boot image (bellows.efi)
    cat[32..64].copy_from_slice(&entry);
    iso.write_all(&cat)?;

    // Pad to EFI Directory LBA (LBA 20)
    let current_pos = iso.stream_position()?;
    let target_pos = efi_dir_lba as u64 * SECTOR_SIZE as u64;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        io::copy(&mut io::repeat(0).take(padding_bytes), &mut iso)?;
    }

    // EFI Directory (LBA 20)
    let mut efi_dir_data = [0u8; SECTOR_SIZE];
    let mut offset = 0;

    // Current directory entry ('.')
    let current_dir_record = create_dot_entry(efi_dir_lba, SECTOR_SIZE as u32);
    efi_dir_data[offset..offset + current_dir_record.len()].copy_from_slice(&current_dir_record);
    offset += current_dir_record.len();

    // Parent directory entry ('..')
    let parent_dir_record = create_dotdot_entry(root_dir_lba, SECTOR_SIZE as u32);
    efi_dir_data[offset..offset + parent_dir_record.len()].copy_from_slice(&parent_dir_record);
    offset += parent_dir_record.len();

    // BOOT directory entry
    let boot_dir_record = create_dir_record(boot_dir_lba, SECTOR_SIZE as u32, true, b"BOOT");
    efi_dir_data[offset..offset + boot_dir_record.len()].copy_from_slice(&boot_dir_record);
    offset += boot_dir_record.len();

    iso.write_all(&efi_dir_data)?;

    // Pad to BOOT Directory LBA (LBA 21)
    let current_pos = iso.stream_position()?;
    let target_pos = boot_dir_lba as u64 * SECTOR_SIZE as u64;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        io::copy(&mut io::repeat(0).take(padding_bytes), &mut iso)?;
    }

    // BOOT Directory (LBA 21)
    let mut boot_dir_data = [0u8; SECTOR_SIZE];
    let mut offset = 0;

    // Current directory entry ('.')
    let current_dir_record = create_dot_entry(boot_dir_lba, SECTOR_SIZE as u32);
    boot_dir_data[offset..offset + current_dir_record.len()].copy_from_slice(&current_dir_record);
    offset += current_dir_record.len();

    // Parent directory entry ('..')
    let parent_dir_record = create_dotdot_entry(efi_dir_lba, SECTOR_SIZE as u32);
    boot_dir_data[offset..offset + parent_dir_record.len()].copy_from_slice(&parent_dir_record);
    offset += parent_dir_record.len();

    // BOOTX64.EFI entry
    let bootx64_record = create_dir_record(
        bellows_efi_lba,
        bellows_file_content.len() as u32,
        false,
        b"BOOTX64.EFI",
    );
    boot_dir_data[offset..offset + bootx64_record.len()].copy_from_slice(&bootx64_record);
    offset += bootx64_record.len();

    // KERNEL.EFI entry
    let kernel_record = create_dir_record(
        kernel_efi_lba,
        kernel_file_content.len() as u32,
        false,
        b"KERNEL.EFI",
    );
    boot_dir_data[offset..offset + kernel_record.len()].copy_from_slice(&kernel_record);
    offset += kernel_record.len();

    iso.write_all(&boot_dir_data)?;

    // Pad to bellows.efi LBA
    let current_pos = iso.stream_position()?;
    let target_pos = bellows_efi_lba as u64 * SECTOR_SIZE as u64;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        io::copy(&mut io::repeat(0).take(padding_bytes), &mut iso)?;
    }

    // Write bellows.efi
    iso.write_all(&bellows_file_content)?;
    pad_sector(&mut iso)?;

    // Pad to kernel.efi LBA
    let current_pos = iso.stream_position()?;
    let target_pos = kernel_efi_lba as u64 * SECTOR_SIZE as u64;
    if current_pos < target_pos {
        let padding_bytes = target_pos - current_pos;
        io::copy(&mut io::repeat(0).take(padding_bytes), &mut iso)?;
    }

    // Write kernel.efi
    iso.write_all(&kernel_file_content)?;
    pad_sector(&mut iso)?;

    Ok(())
}
