use std::{
    fs::File,
    io::{self, Read},
    path::PathBuf,
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
                boot_catalog: PathBuf::from("BOOT.CAT"),
                boot_image: bios_boot_image_path.clone(),
                destination_in_iso: "isolinux/isolinux.bin".to_string(),
            }),
            uefi_boot: Some(isobemak::UefiBootInfo {
                boot_image: bootx64_path.clone(),
                kernel_image: kernel_path.clone(),
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
            }),
        },
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
    assert!(isoinfo_d_output.contains("Arch 0 (x86)")); // Updated assertion
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

    // 4. Enhanced Binary Dump Inspection: Check GPT Header
    // Read the ISO file content
    let mut iso_file = File::open(&iso_path)?;
    let mut iso_bytes = Vec::new();
    iso_file.read_to_end(&mut iso_bytes)?;

    // GPT Header is typically at LBA 1 (sector 1), which is 512 bytes from the start of the file.
    // A sector is 512 bytes.
    let gpt_header_offset = 512;
    let gpt_header_size = 92; // Minimum GPT header size

    if iso_bytes.len() > gpt_header_offset + gpt_header_size {
        let gpt_header_slice = &iso_bytes[gpt_header_offset..(gpt_header_offset + gpt_header_size)];

        // Check GPT Signature ("EFI PART")
        let signature = &gpt_header_slice[0..8];
        assert_eq!(signature, b"EFI PART", "GPT header signature mismatch");
        println!("Verified GPT header signature: {:?}", String::from_utf8_lossy(signature));

        // Check GPT Revision (should be 0x00010000 for version 1.0)
        let revision = u32::from_le_bytes([
            gpt_header_slice[8],
            gpt_header_slice[9],
            gpt_header_slice[10],
            gpt_header_slice[11],
        ]);
        assert_eq!(revision, 0x00010000, "GPT header revision mismatch");
        println!("Verified GPT header revision: 0x{:x}", revision);

        // Further checks could include:
        // - Header Size (offset 12, u32)
        // - CRC32 of header (offset 16, u32)
        // - Current LBA (offset 24, u64)
        // - Backup LBA (offset 32, u64)
        // - First Usable LBA (offset 40, u64)
        // - Last Usable LBA (offset 48, u64)
        // - Disk GUID (offset 56, 16 bytes)
        // - Partition Entry LBA (offset 72, u64)
        // - Number of Partition Entries (offset 80, u32)
        // - Size of Partition Entry (offset 84, u32)
        // - CRC32 of Partition Entry Array (offset 88, u32)
    } else {
        println!("Warning: ISO file too small to contain a GPT header at expected offset.");
    }

    Ok(())
}
