// isobemak/src/iso.rs
// ISO + El Torito
use crate::utils::{SECTOR_SIZE, pad_sector};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, Write},
    path::Path,
};

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
    let mut bellows_file_content = fs::read(bellows_path)?;
    let mut kernel_file_content = fs::read(kernel_path)?;

    let bellows_sectors = (bellows_file_content.len() as u32).div_ceil(SECTOR_SIZE as u32);
    let kernel_sectors = (kernel_file_content.len() as u32).div_ceil(SECTOR_SIZE as u32);

    // Calculate LBAs for EFI files and directory structure
    let efi_dir_lba = 20; // EFI directory starts at LBA 20
    let boot_dir_lba = efi_dir_lba + 1; // BOOT directory starts after EFI dir
    let bellows_efi_lba = boot_dir_lba + 1; // bellows.efi starts after BOOT dir
    let kernel_efi_lba = bellows_efi_lba + bellows_sectors; // kernel.efi starts after bellows.efi

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

    pvd[80..84].copy_from_slice(&(total_sectors as u32).to_le_bytes());
    pvd[84..88].copy_from_slice(&(total_sectors as u32).to_be_bytes());
    pvd[128..132].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes());

    // Root Directory Record (for PVD)
    // This record describes the root directory itself, which will contain the EFI directory.
    let root_dir_lba = efi_dir_lba; // Root directory is effectively the EFI directory for UEFI boot
    let root_dir_size = SECTOR_SIZE as u32; // For simplicity, assume one sector for root dir
    let mut root_dir_record = [0u8; 34];
    root_dir_record[0] = 34; // Length of Directory Record
    root_dir_record[1] = 0;  // Extended Attribute Record Length
    root_dir_record[2..6].copy_from_slice(&root_dir_lba.to_le_bytes()); // Location of Extent (LE)
    root_dir_record[6..10].copy_from_slice(&root_dir_lba.to_be_bytes()); // Location of Extent (BE)
    root_dir_record[10..14].copy_from_slice(&root_dir_size.to_le_bytes()); // Data Length (LE)
    root_dir_record[14..18].copy_from_slice(&root_dir_size.to_be_bytes()); // Data Length (BE)
    // Date and time (bytes 18-24) - leave as 0 for now
    root_dir_record[25] = 0x02; // File Flags (Directory)
    // File Unit Size (byte 26) - 0
    // Interleave Gap Size (byte 27) - 0
    root_dir_record[28..30].copy_from_slice(&1u16.to_le_bytes()); // Volume Sequence Number (LE)
    root_dir_record[30..32].copy_from_slice(&1u16.to_be_bytes()); // Volume Sequence Number (BE)
    root_dir_record[32] = 1; // Length of File Identifier (for root, usually 1 for '.')
    root_dir_record[33] = 0x00; // File Identifier ('.')

    pvd[156..190].copy_from_slice(&root_dir_record);
    iso.write_all(&pvd)?;

    // Boot Record Volume Descriptor (LBA 17)
    let brvd_lba = 17;
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
    brvd[71..75].copy_from_slice(&18u32.to_le_bytes()); // Boot Catalog LBA (LBA 18)
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
    let mut current_dir_record = [0u8; 34];
    current_dir_record[0] = 34;
    current_dir_record[1] = 0;
    current_dir_record[2..6].copy_from_slice(&efi_dir_lba.to_le_bytes());
    current_dir_record[6..10].copy_from_slice(&efi_dir_lba.to_be_bytes());
    current_dir_record[10..14].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes()); // Size of EFI dir
    current_dir_record[14..18].copy_from_slice(&(SECTOR_SIZE as u32).to_be_bytes());
    current_dir_record[25] = 0x02; // Directory
    current_dir_record[28..30].copy_from_slice(&1u16.to_le_bytes());
    current_dir_record[30..32].copy_from_slice(&1u16.to_be_bytes());
    current_dir_record[32] = 1;
    current_dir_record[33] = 0x00; // '.'
    efi_dir_data[offset..offset + 34].copy_from_slice(&current_dir_record);
    offset += 34;

    // Parent directory entry ('..')
    let mut parent_dir_record = [0u8; 34];
    parent_dir_record[0] = 34;
    parent_dir_record[1] = 0;
    parent_dir_record[2..6].copy_from_slice(&root_dir_lba.to_le_bytes()); // Parent is root
    parent_dir_record[6..10].copy_from_slice(&root_dir_lba.to_be_bytes());
    parent_dir_record[10..14].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes()); // Size of root dir
    parent_dir_record[14..18].copy_from_slice(&(SECTOR_SIZE as u32).to_be_bytes());
    parent_dir_record[25] = 0x02; // Directory
    parent_dir_record[28..30].copy_from_slice(&1u16.to_le_bytes());
    parent_dir_record[30..32].copy_from_slice(&1u16.to_be_bytes());
    parent_dir_record[32] = 1;
    parent_dir_record[33] = 0x01; // '..'
    efi_dir_data[offset..offset + 34].copy_from_slice(&parent_dir_record);
    offset += 34;

    // BOOT directory entry
    let boot_dir_name = b"BOOT";
    let boot_dir_name_len = boot_dir_name.len() as u8;
    let boot_dir_record_len = 33 + boot_dir_name_len;

    let mut boot_dir_record = [0u8; 40];
    boot_dir_record[0] = boot_dir_record_len;
    boot_dir_record[1] = 0;
    boot_dir_record[2..6].copy_from_slice(&boot_dir_lba.to_le_bytes());
    boot_dir_record[6..10].copy_from_slice(&boot_dir_lba.to_be_bytes());
    boot_dir_record[10..14].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes()); // Size of BOOT dir
    boot_dir_record[14..18].copy_from_slice(&(SECTOR_SIZE as u32).to_be_bytes());
    boot_dir_record[25] = 0x02; // Directory
    boot_dir_record[28..30].copy_from_slice(&1u16.to_le_bytes());
    boot_dir_record[30..32].copy_from_slice(&1u16.to_be_bytes());
    boot_dir_record[32] = boot_dir_name_len;
    boot_dir_record[33..33 + boot_dir_name_len as usize].copy_from_slice(boot_dir_name);
    efi_dir_data[offset..offset + boot_dir_record_len as usize].copy_from_slice(&boot_dir_record[..boot_dir_record_len as usize]);
    offset += boot_dir_record_len as usize;

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
    let mut current_dir_record = [0u8; 34];
    current_dir_record[0] = 34;
    current_dir_record[1] = 0;
    current_dir_record[2..6].copy_from_slice(&boot_dir_lba.to_le_bytes());
    current_dir_record[6..10].copy_from_slice(&boot_dir_lba.to_be_bytes());
    current_dir_record[10..14].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes()); // Size of BOOT dir
    current_dir_record[14..18].copy_from_slice(&(SECTOR_SIZE as u32).to_be_bytes());
    current_dir_record[25] = 0x02; // Directory
    current_dir_record[28..30].copy_from_slice(&1u16.to_le_bytes());
    current_dir_record[30..32].copy_from_slice(&1u16.to_be_bytes());
    current_dir_record[32] = 1;
    current_dir_record[33] = 0x00; // '.'
    boot_dir_data[offset..offset + 34].copy_from_slice(&current_dir_record);
    offset += 34;

    // Parent directory entry ('..')
    let mut parent_dir_record = [0u8; 34];
    parent_dir_record[0] = 34;
    parent_dir_record[1] = 0;
    parent_dir_record[2..6].copy_from_slice(&efi_dir_lba.to_le_bytes()); // Parent is EFI dir
    parent_dir_record[6..10].copy_from_slice(&efi_dir_lba.to_be_bytes());
    parent_dir_record[10..14].copy_from_slice(&(SECTOR_SIZE as u32).to_le_bytes()); // Size of EFI dir
    parent_dir_record[14..18].copy_from_slice(&(SECTOR_SIZE as u32).to_be_bytes());
    parent_dir_record[25] = 0x02; // Directory
    parent_dir_record[28..30].copy_from_slice(&1u16.to_le_bytes());
    parent_dir_record[30..32].copy_from_slice(&1u16.to_be_bytes());
    parent_dir_record[32] = 1;
    parent_dir_record[33] = 0x01; // '..'
    boot_dir_data[offset..offset + 34].copy_from_slice(&parent_dir_record);
    offset += 34;

    // BOOTX64.EFI entry
    let bootx64_name = b"BOOTX64.EFI";
    let bootx64_name_len = bootx64_name.len() as u8;
    let bootx64_record_len = 33 + bootx64_name_len;

    let mut bootx64_record = [0u8; 48]; // Max size for BOOTX64.EFI entry
    bootx64_record[0] = bootx64_record_len;
    bootx64_record[1] = 0;
    bootx64_record[2..6].copy_from_slice(&bellows_efi_lba.to_le_bytes());
    bootx64_record[6..10].copy_from_slice(&bellows_efi_lba.to_be_bytes());
    bootx64_record[10..14].copy_from_slice(&(bellows_file_content.len() as u32).to_le_bytes());
    bootx64_record[14..18].copy_from_slice(&(bellows_file_content.len() as u32).to_be_bytes());
    bootx64_record[25] = 0x00; // File
    bootx64_record[28..30].copy_from_slice(&1u16.to_le_bytes());
    bootx64_record[30..32].copy_from_slice(&1u16.to_be_bytes());
    bootx64_record[32] = bootx64_name_len;
    bootx64_record[33..33 + bootx64_name_len as usize].copy_from_slice(bootx64_name);
    boot_dir_data[offset..offset + bootx64_record_len as usize].copy_from_slice(&bootx64_record[..bootx64_record_len as usize]);
    offset += bootx64_record_len as usize;

    // KERNEL.EFI entry
    let kernel_name = b"KERNEL.EFI";
    let kernel_name_len = kernel_name.len() as u8;
    let kernel_record_len = 33 + kernel_name_len;

    let mut kernel_record = [0u8; 48]; // Max size for KERNEL.EFI entry
    kernel_record[0] = kernel_record_len;
    kernel_record[1] = 0;
    kernel_record[2..6].copy_from_slice(&kernel_efi_lba.to_le_bytes());
    kernel_record[6..10].copy_from_slice(&kernel_efi_lba.to_be_bytes());
    kernel_record[10..14].copy_from_slice(&(kernel_file_content.len() as u32).to_le_bytes());
    kernel_record[14..18].copy_from_slice(&(kernel_file_content.len() as u32).to_be_bytes());
    kernel_record[25] = 0x00; // File
    kernel_record[28..30].copy_from_slice(&1u16.to_le_bytes());
    kernel_record[30..32].copy_from_slice(&1u16.to_be_bytes());
    kernel_record[32] = kernel_name_len;
    kernel_record[33..33 + kernel_name_len as usize].copy_from_slice(kernel_name);
    boot_dir_data[offset..offset + kernel_record_len as usize].copy_from_slice(&kernel_record[..kernel_record_len as usize]);
    offset += kernel_record_len as usize;

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
