use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

use isobemak::build_iso;
use tempfile::tempdir;

use crate::integration_tests::common::run_command;

/// Read PVD Volume Space Size (offset 80, 4 bytes LE + 4 bytes BE) from LBA 16.
fn read_pvd_volume_space_size(file: &mut File) -> io::Result<u32> {
    let lba16_offset = 16 * 2048;
    file.seek(SeekFrom::Start(lba16_offset + 80))?;
    let mut le_bytes = [0u8; 4];
    file.read_exact(&mut le_bytes)?;
    Ok(u32::from_le_bytes(le_bytes))
}

#[test]
fn test_iso_integrity_and_boot_modes() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let temp_dir_path = temp_dir.path();
    println!("Temp dir for integrity test: {:?}", &temp_dir_path);

    // Setup files and paths for an ISO with both BIOS and UEFI boot
    let bios_boot_image_path = temp_dir_path.join("isolinux.bin");
    let mut bios_boot_image = vec![0u8; 512];
    bios_boot_image[510..512].copy_from_slice(&0xAA55u16.to_le_bytes());
    std::fs::write(&bios_boot_image_path, bios_boot_image)?; // A dummy 512-byte boot image with signature
    let bios_cfg_path = temp_dir_path.join("isolinux.cfg");
    std::fs::write(&bios_cfg_path, b"default menu.c32")?;

    let bootx64_path = temp_dir_path.join("bootx64.efi");
    std::fs::write(&bootx64_path, vec![0u8; 64 * 1024])?;

    let kernel_path = temp_dir_path.join("kernel.elf");
    std::fs::write(&kernel_path, vec![0u8; 16 * 1024])?;

    let iso_path = temp_dir_path.join("integrity_test.iso");

    let iso_image = isobemak::IsoImage {
        volume_id: None,
        files: vec![
            isobemak::IsoImageFile {
                source: bios_cfg_path.clone(),
                destination: "isolinux/isolinux.cfg".to_string(),
            },
            isobemak::IsoImageFile {
                source: bootx64_path.clone(),
                destination: "EFI/BOOT/BOOTX64.EFI".to_string(),
            },
            isobemak::IsoImageFile {
                source: kernel_path.clone(),
                destination: "EFI/BOOT/KERNEL.EFI".to_string(),
            },
        ],
        boot_info: isobemak::BootInfo {
            bios_boot: Some(isobemak::BiosBootInfo {
                boot_image: bios_boot_image_path.clone(),
                destination_in_iso: "isolinux/isolinux.bin".to_string(),
            }),
            uefi_boot: Some(isobemak::UefiBootInfo {
                boot_image: bootx64_path.clone(),
                kernel_image: kernel_path.clone(),
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
                additional_efi_boot_files: Vec::new(),
                grub_cfg_content: None,
            }),
        },
        layout_profile: isobemak::IsoLayoutProfile::default(),
    };

    // Build the ISO
    build_iso(&iso_path, &iso_image, true)?;
    assert!(iso_path.exists());

    // 1. Verify ISO integrity using md5sum
    let md5sum_output = run_command("md5sum", &[iso_path.to_str().unwrap()])?;
    println!("md5sum output:\n{}", md5sum_output);
    assert!(md5sum_output.contains(iso_path.file_name().unwrap().to_str().unwrap()));
    // A more robust check would be to compare against a known good checksum,
    // but for a generated ISO, simply ensuring the command runs and produces output is a start.

    // 2. Verify BIOS (El Torito) boot entry
    let isoinfo_d_output = run_command("isoinfo", &["-d", "-i", iso_path.to_str().unwrap()])?;
    println!("isoinfo -d output (integrity test):\n{}", isoinfo_d_output);
    assert!(isoinfo_d_output.contains("El Torito VD version 1 found"));
    // Validation Entry platform ID is 0xEF (EFI) for Ventoy/UEFI firmware compat.
    // isoinfo reports this as "Arch 239 (Unknown Arch)" because it doesn't have
    // a symbolic name for 0xEF in the architecture table.
    assert!(isoinfo_d_output.contains("Arch 239 (Unknown Arch)"));
    // Single-entry UEFI boot catalog: 0x88 entry, No Emulation (0x00), system_type=0xEF.
    // This is the canonical El Torito UEFI layout recognised by OVMF and real firmware.
    assert!(isoinfo_d_output.contains("Boot media 0 (No Emulation Boot)"));
    assert!(isoinfo_d_output.contains("Sys type EF"));
    // Removed assertion for "EFI boot entry is present" as isoinfo -d does not output this string directly.
    // Detailed UEFI boot entry verification is handled in `test_create_isohybrid_uefi_iso`.

    // 7z may fail to extract from isohybrid images (offset ISO9660 start),
    // so this check is best-effort only. Structural verification is done above.
    let extract_dir = temp_dir_path.join("extracted_bios_boot");
    let _ = std::fs::create_dir_all(&extract_dir);
    let _ = run_command(
        "7z",
        &[
            "x",
            iso_path.to_str().unwrap(),
            &format!("-o{}", extract_dir.to_str().unwrap()),
            "isolinux/isolinux.bin",
        ],
    );
    let extracted_bios_boot_path = extract_dir.join("isolinux/isolinux.bin");
    if extracted_bios_boot_path.exists() {
        let mut boot_image_file = File::open(&extracted_bios_boot_path)?;
        let mut boot_sector = [0u8; 512];
        boot_image_file.read_exact(&mut boot_sector)?;
        // Check for boot signature 0xAA55 at offset 510-511
        assert_eq!(
            u16::from_le_bytes([boot_sector[510], boot_sector[511]]),
            0xAA55,
            "BIOS boot image does not have the expected boot signature (0xAA55)"
        );
        println!("Verified BIOS boot image signature (0xAA55)");
    } else {
        println!(
            "Warning: isolinux/isolinux.bin not extracted or found for BIOS boot signature check (expected for isohybrid)."
        );
    }

    // 3. Verify UEFI boot entry
    // The `test_create_isohybrid_uefi_iso` already performs detailed UEFI boot entry verification.
    // Removed assertion for "EFI boot entry is present" as isoinfo -d does not output this string directly.

    // 4. Verify MBR boot signature (xorriso-compatible, no GPT)
    let mut iso_file = File::open(&iso_path)?;
    let mut mbr_sector = [0u8; 512];
    iso_file.read_exact(&mut mbr_sector)?;

    // MBR boot signature at bytes 510-511 must be 0xAA55
    let mbr_sig = u16::from_le_bytes([mbr_sector[510], mbr_sector[511]]);
    assert_eq!(mbr_sig, 0xAA55, "MBR boot signature mismatch");
    println!("Verified MBR boot signature: 0x{:04X}", mbr_sig);

    // MBR Partition Entry 0 at offset 0x1BE: type 0xEE (GPT Protective), LBA 1.
    // This is the standard protective MBR per UEFI spec §5.2.3,
    // matching Ubuntu/xorriso layout.  0xEE tells UEFI firmware that
    // the disk uses GPT partitioning.
    let entry0_type = mbr_sector[0x1BE + 4];
    let entry0_start =
        u32::from_le_bytes(mbr_sector[(0x1BE + 8)..(0x1BE + 12)].try_into().unwrap());
    assert_eq!(entry0_type, 0xEE, "MBR entry 0 should be type 0xEE (GPT Protective, UEFI spec)");
    assert_eq!(entry0_start, 1, "MBR entry 0 should start at LBA 1 (LBA 0 is the MBR itself)");
    println!("MBR entry 0: type=0x{:02X}, start={}", entry0_type, entry0_start);

    // MBR Partition Entry 1 at offset 0x1CE: type 0xEF (ESP), bootable=0x00
    let entry1_bootable = mbr_sector[0x1CE];
    let entry1_type = mbr_sector[0x1CE + 4];
    let entry1_start =
        u32::from_le_bytes(mbr_sector[(0x1CE + 8)..(0x1CE + 12)].try_into().unwrap());
    assert_eq!(entry1_bootable, 0x00, "MBR entry 1 should not be bootable");
    assert_eq!(entry1_type, 0xEF, "MBR entry 1 should be type 0xEF (ESP)");
    println!("MBR entry 1: type=0x{:02X}, start={}", entry1_type, entry1_start);

    Ok(())
}

/// Verify ISO9660 PVD Volume Space Size matches the actual file size.
///
/// When building an isohybrid ISO, backup GPT structures (33 sectors)
/// are appended after the initial ISO data.  If the PVD Volume Space Size
/// is not updated after this append, Ventoy and other tools will report
/// the ISO as broken because the ISO9660 metadata says the file is smaller
/// than it actually is.
///
/// This test directly catches the regression where finalize_iso writes
/// the PVD before write_hybrid_structures extends the file.
#[test]
fn test_iso9660_volume_space_size_matches_file_size() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let temp_dir_path = temp_dir.path();

    let bootx64_path = temp_dir_path.join("bootx64.efi");
    std::fs::write(&bootx64_path, vec![0u8; 64 * 1024])?;

    let kernel_path = temp_dir_path.join("kernel.elf");
    std::fs::write(&kernel_path, vec![0u8; 16 * 1024])?;

    let iso_path = temp_dir_path.join("volume_size_test.iso");

    let iso_image = isobemak::IsoImage {
        volume_id: None,
        files: vec![],
        boot_info: isobemak::BootInfo {
            bios_boot: None,
            uefi_boot: Some(isobemak::UefiBootInfo {
                boot_image: bootx64_path.clone(),
                kernel_image: kernel_path.clone(),
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
                additional_efi_boot_files: Vec::new(),
                grub_cfg_content: None,
            }),
        },
        layout_profile: isobemak::IsoLayoutProfile::default(),
    };

    build_iso(&iso_path, &iso_image, true)?;

    // Read PVD Volume Space Size (in 2048-byte sectors)
    let mut iso_file = File::open(&iso_path)?;
    let pvd_total_sectors = read_pvd_volume_space_size(&mut iso_file)?;

    // Actual file size must be a multiple of 2048 (ISO sector size).
    // Backup GPT structures are 33×512 = 16896 bytes, which is NOT 2048-aligned
    // (16896 % 2048 = 512).  If the builder doesn't re-pad after appending
    // GPT, the ISO file won't end on a 2048-byte boundary, which breaks
    // Ventoy and other tools that expect ISO9660 filesystems to be
    // sector-aligned.
    let actual_size = iso_file.metadata()?.len();
    assert_eq!(
        actual_size % 2048,
        0,
        "ISO file size ({actual_size}) must be a multiple of 2048 (ISO sector size); \
         remainder={} bytes = {} GPT sectors",
        actual_size % 2048,
        (actual_size % 2048) / 512,
    );

    let expected_sectors = u32::try_from(actual_size / 2048).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "ISO too large for u32 sectors")
    })?;

    assert_eq!(
        pvd_total_sectors, expected_sectors,
        "PVD Volume Space Size ({pvd_total_sectors} sectors) must match actual file size \
         ({actual_size} bytes / 2048 = {expected_sectors} sectors).  \
         The difference is {} bytes ({} GPT sectors).",
        actual_size.saturating_sub(pvd_total_sectors as u64 * 2048),
        actual_size.saturating_sub(pvd_total_sectors as u64 * 2048) / 512,
    );
    Ok(())
}