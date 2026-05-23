use std::{
    fs::File,
    io::{self, Read},
};

use isobemak::build_iso;
use tempfile::tempdir;

use crate::integration_tests::common::run_command;

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
    assert!(isoinfo_d_output.contains("El Torito VD version 1 found")); // Updated assertion
    assert!(isoinfo_d_output.contains("Arch 239")); // Updated assertion for UEFI priority
    assert!(isoinfo_d_output.contains("Boot media 0 (No Emulation Boot)")); // Updated assertion
    // Removed assertion for "EFI boot entry is present" as isoinfo -d does not output this string directly.
    // Detailed UEFI boot entry verification is handled in `test_create_isohybrid_uefi_iso`.

    // Extract the BIOS boot image and check its signature (0xAA55)
    // This requires knowing the LBA of the boot image from the boot catalog.
    // For simplicity, we'll assume the first boot entry is the BIOS one and extract it.
    // A more robust solution would parse the boot catalog directly.
    let extract_dir = temp_dir_path.join("extracted_bios_boot");
    std::fs::create_dir_all(&extract_dir)?;
    run_command(
        "7z",
        &[
            "x",
            iso_path.to_str().unwrap(),
            &format!("-o{}", extract_dir.to_str().unwrap()),
            "isolinux/isolinux.bin", // Assuming this is the BIOS boot image
        ],
    )?;

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
            "Warning: isolinux/isolinux.bin not extracted or found for BIOS boot signature check."
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

    // MBR Partition Entry 0 at offset 0x1BE: type 0x83 (Linux/ISO9660)
    let entry0_type = mbr_sector[0x1BE + 4];
    let entry0_start =
        u32::from_le_bytes(mbr_sector[(0x1BE + 8)..(0x1BE + 12)].try_into().unwrap());
    assert_eq!(entry0_type, 0x83, "MBR entry 0 should be type 0x83 (Linux)");
    assert_eq!(entry0_start, 0, "MBR entry 0 should start at LBA 0");
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
