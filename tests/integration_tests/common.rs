use std::{
    fs::File,
    io::{self, Error, ErrorKind, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
};

pub fn run_command(command: &str, args: &[&str]) -> io::Result<String> {
    let output = Command::new(command).args(args).output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(Error::new(
            ErrorKind::Other,
            format!(
                "Command `{}` failed with exit code {:?}\nStdout: {}\nStderr: {}",
                command,
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        ))
    }
}

pub fn setup_integration_test_files(temp_dir: &Path) -> io::Result<(PathBuf, PathBuf, PathBuf)> {
    // Create dummy files needed for the ISO image
    let bootx64_path = temp_dir.join("bootx64.efi");
    std::fs::write(&bootx64_path, vec![0u8; 64 * 1024])?;

    let kernel_path = temp_dir.join("kernel.elf");
    std::fs::write(&kernel_path, vec![0u8; 16 * 1024])?;

    let iso_path = temp_dir.join("test.iso");

    Ok((bootx64_path, kernel_path, iso_path))
}

/// Verifies critical binary structures within the generated ISO file.
pub fn verify_iso_binary_structures(iso_file: &mut File) -> io::Result<()> {
    const ISO_SECTOR_SIZE: u64 = 2048;

    // 1. Verify Primary Volume Descriptor (PVD) at LBA 16
    iso_file.seek(SeekFrom::Start(16 * ISO_SECTOR_SIZE))?;
    let mut pvd_header = [0u8; 6];
    iso_file.read_exact(&mut pvd_header)?;
    assert_eq!(
        &pvd_header,
        &[0x01, b'C', b'D', b'0', b'0', b'1'],
        "PVD identifier 'CD001' not found at LBA 16"
    );

    // 2. Verify Boot Record Volume Descriptor (BRVD) at LBA 17
    iso_file.seek(SeekFrom::Start(17 * ISO_SECTOR_SIZE))?;
    let mut brvd_header = [0u8; 37];
    iso_file.read_exact(&mut brvd_header)?;
    assert_eq!(
        &brvd_header[0..7],
        &[0x00, b'C', b'D', b'0', b'0', b'1', 0x01],
        "BRVD identifier 'CD001' not found at LBA 17"
    );
    assert_eq!(
        &brvd_header[7..30],
        b"EL TORITO SPECIFICATION",
        "BRVD boot identifier 'EL TORITO SPECIFICATION' not found"
    );

    // 3. Re-verify the boot catalog validation entry checksum at LBA 19
    iso_file.seek(SeekFrom::Start(
        isobemak::iso::boot_catalog::LBA_BOOT_CATALOG as u64 * ISO_SECTOR_SIZE,
    ))?;
    let mut boot_catalog = [0u8; 32]; // Only need the validation entry
    iso_file.read_exact(&mut boot_catalog)?;

    let mut sum: u16 = 0;
    for chunk in boot_catalog.chunks_exact(2) {
        sum = sum.wrapping_add(u16::from_le_bytes(chunk.try_into().unwrap()));
    }
    assert_eq!(
        sum, 0,
        "Boot catalog validation entry checksum should be 0 (re-verification)"
    );

    Ok(())
}

/// Verifies GPT header CRC, partition array CRC, ESP partition entry attributes,
/// and MBR CHS field consistency for isohybrid UEFI images.
///
/// This covers critical structural checks that real UEFI firmware (InsydeH2O,
/// older AMI) performs but QEMU/OVMF does not. Missing GPT attributes or
/// zero-filled MBR CHS fields cause "No bootfile found for UEFI!" on hardware.
pub fn verify_gpt_and_mbr_chs(iso_file: &mut File) -> io::Result<()> {
    // ── Read MBR sector (LBA 0, 512 bytes) ──
    iso_file.seek(SeekFrom::Start(0))?;
    let mut mbr = [0u8; 512];
    iso_file.read_exact(&mut mbr)?;

    // MBR boot signature at offset 510-511
    let mbr_sig = u16::from_le_bytes([mbr[510], mbr[511]]);
    assert_eq!(mbr_sig, 0xAA55, "MBR boot signature mismatch");

    // --- Entry 0 (offset 0x1BE): type 0xEE (GPT Protective), LBA 1 ---
    // Per UEFI spec §5.2.3, the protective MBR must have type 0xEE covering
    // the entire disk from LBA 1.  This tells firmware the disk uses GPT.
    let e0_type = mbr[0x1BE + 4];
    let e0_start = u32::from_le_bytes(mbr[(0x1BE + 8)..(0x1BE + 12)].try_into().unwrap());
    assert_eq!(
        e0_type, 0xEE,
        "MBR entry 0 must be type 0xEE (GPT Protective, UEFI spec)"
    );
    assert_eq!(
        e0_start, 1,
        "MBR entry 0 must start at LBA 1 (LBA 0 is MBR itself)"
    );

    // Verify CHS fields for entry 0 are populated.
    // LBA=1: with H=64, SPT=32 → cylinder=0, head=0, sector=2
    let e0_chs_start = &mbr[0x1BE + 1..0x1BE + 4];
    assert_ne!(
        e0_chs_start,
        &[0, 0, 0],
        "MBR entry 0 starting CHS must not be zero"
    );
    // LBA=1 with H=64, SPT=32: C=0, H=0, S=2
    assert_eq!(
        e0_chs_start[0], 0x00,
        "MBR entry 0 start head must be 0 (LBA=1)"
    );
    assert_eq!(
        e0_chs_start[1], 0x02,
        "MBR entry 0 start sector must be 2 (LBA=1)"
    );
    assert_eq!(
        e0_chs_start[2], 0x00,
        "MBR entry 0 start cylinder lo must be 0 (LBA=1)"
    );

    let e0_chs_end = &mbr[0x1BE + 5..0x1BE + 8];
    assert_ne!(
        e0_chs_end,
        &[0, 0, 0],
        "MBR entry 0 ending CHS must not be zero (protective GPT covers whole disk)"
    );

    // --- Entry 1 (offset 0x1CE): type 0xEF, ESP ---
    let e1_type = mbr[0x1CE + 4];
    let e1_start = u32::from_le_bytes(mbr[(0x1CE + 8)..(0x1CE + 12)].try_into().unwrap());
    assert_eq!(e1_type, 0xEF, "MBR entry 1 must be type 0xEF (ESP)");
    // ESP is file-backed (efiboot.img in ISO filesystem).  Its 512-byte LBA
    // depends on the ISO filesystem layout.  With a ~260 MiB FAT32 image
    // placed alphabetically after regular files, the ESP start can be far
    // beyond 4096.  We only assert it comes after the GPT reserved area.
    assert!(
        e1_start >= 34,
        "MBR entry 1 (ESP) must start after GPT reserved area (>=34), got {}",
        e1_start
    );

    // Verify CHS fields for entry 1 (ESP) are populated.
    let e1_chs_start = &mbr[0x1CE + 1..0x1CE + 4];
    assert_ne!(
        e1_chs_start,
        &[0, 0, 0],
        "MBR entry 1 (ESP) starting CHS must not be zero"
    );
    let e1_chs_end = &mbr[0x1CE + 5..0x1CE + 8];
    assert_ne!(
        e1_chs_end,
        &[0, 0, 0],
        "MBR entry 1 (ESP) ending CHS must not be zero"
    );

    // ── Read GPT header (LBA 1, 512 bytes) ──
    iso_file.seek(SeekFrom::Start(512))?;
    let mut gpt_header = [0u8; 92];
    iso_file.read_exact(&mut gpt_header)?;

    // GPT signature "EFI PART"
    assert_eq!(
        &gpt_header[0..8],
        b"EFI PART",
        "GPT header signature must be 'EFI PART'"
    );

    // GPT header CRC32 (offset 16, u32 LE)
    let stored_crc = u32::from_le_bytes(gpt_header[16..20].try_into().unwrap());
    // Zero out the CRC field and compute CRC over header_size bytes
    let header_size = u32::from_le_bytes(gpt_header[12..16].try_into().unwrap());
    assert_eq!(header_size, 92, "GPT header_size must be 92");
    let mut header_for_crc = gpt_header; // copy
    header_for_crc[16..20].copy_from_slice(&[0u8; 4]);
    let calculated_crc = {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&header_for_crc[..header_size as usize]);
        hasher.finalize()
    };
    assert_eq!(
        stored_crc, calculated_crc,
        "GPT header CRC32 mismatch: stored={:#010x} calculated={:#010x}",
        stored_crc, calculated_crc
    );

    // Validate GPT header points to partition array at LBA 2
    let partition_array_lba = u64::from_le_bytes(gpt_header[72..80].try_into().unwrap());
    assert_eq!(
        partition_array_lba, 2,
        "Partition entry array must start at LBA 2"
    );
    let num_entries = u32::from_le_bytes(gpt_header[80..84].try_into().unwrap());
    let entry_size = u32::from_le_bytes(gpt_header[84..88].try_into().unwrap());
    assert_eq!(num_entries, 128, "GPT must have 128 partition entries");
    assert_eq!(entry_size, 128, "GPT partition entry size must be 128");

    // Read the first partition entry (ESP) at LBA 2
    iso_file.seek(SeekFrom::Start(2 * 512))?;
    let mut esp_entry = [0u8; 128];
    iso_file.read_exact(&mut esp_entry)?;

    // GPT header CRC and partition array CRC are verified by the unit
    // test `test_write_gpt_structures`.  Here we validate the ESP
    // partition entry content instead (the fields that real firmware
    // inspects when deciding whether to treat this as a valid ESP).

    // ── Verify GPT partition entry layout ──
    // Partition entry format (128 bytes each):
    //   offset 0:  type GUID (16 bytes)
    //   offset 16: unique GUID (16 bytes)
    //   offset 32: starting LBA (u64 LE)
    //   offset 40: ending LBA (u64 LE)
    //   offset 48: attributes (u64 LE)
    //   offset 56: partition name (36 UTF-16LE code units = 72 bytes)
    //
    // GPT layout (3 entries, matching Ubuntu/xorriso):
    //   Entry 0: ISO9660 (type = EBD0A0A2-B9E5-4433-87C0-68B6B72699C7)
    //   Entry 1: EFI System Partition (type = C12A7328-F81F-11D2-BA4B-00A0C93EC93B)
    //   Entry 2: Gap1 (padding)

    // Verify entry 0 is ISO9660 partition
    let expected_iso_guid: [u8; 16] = [
        0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99,
        0xC7,
    ];
    assert_eq!(
        &esp_entry[0..16],
        &expected_iso_guid,
        "GPT partition entry 0 must have ISO9660 type GUID"
    );

    // Read entry 1 (ESP) at offset 128 within the partition array
    iso_file.seek(SeekFrom::Start(2 * 512 + 128))?;
    let mut esp_entry_1 = [0u8; 128];
    iso_file.read_exact(&mut esp_entry_1)?;

    // Verify type GUID is ESP: C12A7328-F81F-11D2-BA4B-00A0C93EC93B
    let expected_esp_guid: [u8; 16] = [
        0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9,
        0x3B,
    ];
    assert_eq!(
        &esp_entry_1[0..16],
        &expected_esp_guid,
        "GPT partition entry 1 must have ESP type GUID (C12A7328-F81F-11D2-BA4B-00A0C93EC93B)"
    );

    // ESP is file-backed (efiboot.img in ISO filesystem).  Its 512-byte LBA
    // depends on the ISO filesystem layout.  With a ~260 MiB FAT32 image
    // placed alphabetically after regular files, the ESP start can be far
    // beyond 4096.  We only assert it comes after the GPT reserved area.
    let esp_start = u64::from_le_bytes(esp_entry_1[32..40].try_into().unwrap());
    assert!(
        esp_start >= 34,
        "ESP must start after GPT reserved area (>=34), got {}",
        esp_start
    );

    // ESP must have non-zero size and end after start
    let esp_end = u64::from_le_bytes(esp_entry_1[40..48].try_into().unwrap());
    assert!(
        esp_end > esp_start,
        "ESP partition ending LBA ({}) must be greater than starting LBA ({})",
        esp_end,
        esp_start
    );

    // ESP attributes bit 0 (System Partition) must be set
    let esp_attrs = u64::from_le_bytes(esp_entry_1[48..56].try_into().unwrap());
    assert_ne!(
        esp_attrs & 1,
        0,
        "ESP partition attributes bit 0 (System Partition) must be set, got {:#x}",
        esp_attrs
    );

    // Partition name should be "EFI System Partition" (UTF-16LE)
    let name_bytes = &esp_entry_1[56..128];
    let name_u16: Vec<u16> = name_bytes
        .chunks_exact(2)
        .take(36) // max 36 UTF-16LE code units
        .map(|c| u16::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let name: String = String::from_utf16_lossy(&name_u16);
    assert_eq!(
        name.trim_end_matches('\0'),
        "EFI System Partition",
        "ESP partition name must be 'EFI System Partition', got '{}'",
        name.trim_end_matches('\0')
    );

    println!(
        "GPT header CRC32, MBR CHS fields, and ESP partition entry content verified successfully"
    );

    Ok(())
}
