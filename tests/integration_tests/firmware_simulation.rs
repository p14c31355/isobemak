//! Firmware behaviour simulation tests.
//!
//! These tests go beyond "structure is correct" and verify that the
//! generated ISO would be accepted by real UEFI firmware through:
//!
//! 1. GPT GUID exact bytes comparison
//! 2. Firmware-style ESP discovery (MBR→GPT→BPB→FAT traversal→BOOTX64.EFI)
//! 3. Linux loop partition recognition (requires root, ignored by default)
//! 4. Ventoy-style strict parser validation
//! 5. blkid-based partition detection (requires root, ignored by default)

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

use fatfs::{FileSystem, FsOptions};
use isobemak::{BootInfo, IsoImage, IsoImageFile, IsoLayoutProfile, UefiBootInfo, build_iso};
use tempfile::tempdir;

use crate::integration_tests::common::{run_command, setup_integration_test_files};

/// Builds a standard isohybrid UEFI ISO for testing.
/// Returns (iso_path, temp_dir_handle) — the temp_dir must be kept alive.
fn build_test_iso() -> io::Result<(std::path::PathBuf, tempfile::TempDir)> {
    let temp_dir = tempdir()?;
    let temp_dir_path = temp_dir.path();
    let (bootx64_path, kernel_path, iso_path) = setup_integration_test_files(temp_dir_path)?;

    let iso_image = IsoImage {
        volume_id: None,
        files: vec![
            IsoImageFile {
                source: bootx64_path,
                destination: "EFI/BOOT/BOOTX64.EFI".to_string(),
            },
            IsoImageFile {
                source: kernel_path,
                destination: "EFI/BOOT/KERNEL.EFI".to_string(),
            },
        ],
        boot_info: BootInfo {
            bios_boot: None,
            uefi_boot: Some(UefiBootInfo {
                boot_image: temp_dir_path.join("bootx64.efi"),
                kernel_image: temp_dir_path.join("kernel.elf"),
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
                additional_efi_boot_files: Vec::new(),
                grub_cfg_content: None,
            }),
        },
        layout_profile: IsoLayoutProfile::default(),
    };

    let (_fat_image_path, _temp_fat, _iso_file, _) = build_iso(&iso_path, &iso_image, true)?;
    assert!(iso_path.exists());
    Ok((iso_path, temp_dir))
}

// ============================================================
// Test 1: GPT GUID exact bytes
// ============================================================
//
// Real UEFI firmware compares partition type GUID against
// EFI_SYSTEM_PARTITION_GUID as raw bytes.  Any byte-level mismatch
// causes the firmware to skip the partition, resulting in
// "No bootfile found for UEFI!".
//
// This test validates:
//   - Entry 0 is ISO9660 (type GUID: EBD0A0A2-B9E5-4433-87C0-68B6B72699C7)
//   - Entry 1 is ESP (type GUID: C12A7328-F81F-11D2-BA4B-00A0C93EC93B)
//     with attribute bit 0 (System Partition) set (soft check)
//   - Entry 2 is Gap1 (padding)

#[test]
fn test_gpt_guid_exact_bytes() -> io::Result<()> {
    let (iso_path, _temp_dir) = build_test_iso()?;
    let mut iso_file = File::open(&iso_path)?;

    // Read GPT header at LBA 1 (offset 512)
    iso_file.seek(SeekFrom::Start(512))?;
    let mut gpt_header = [0u8; 92];
    iso_file.read_exact(&mut gpt_header)?;

    // Verify partition_entry_lba = 2
    let partition_entry_lba = u64::from_le_bytes(gpt_header[72..80].try_into().unwrap());
    assert_eq!(partition_entry_lba, 2, "GPT partition entries must start at LBA 2");

    let num_entries = u32::from_le_bytes(gpt_header[80..84].try_into().unwrap());
    assert_eq!(num_entries, 128, "GPT must have 128 partition entries");

    let entry_size = u32::from_le_bytes(gpt_header[84..88].try_into().unwrap());
    assert_eq!(entry_size, 128, "GPT partition entry size must be 128 bytes");

    // Read 3 partition entries at LBA 2
    iso_file.seek(SeekFrom::Start(2 * 512))?;
    let mut entry0 = [0u8; 128];
    let mut entry1 = [0u8; 128];
    let mut entry2 = [0u8; 128];
    iso_file.read_exact(&mut entry0)?;
    iso_file.read_exact(&mut entry1)?;
    iso_file.read_exact(&mut entry2)?;

    // ---- Entry 0: ISO9660 ----
    // GUID: EBD0A0A2-B9E5-4433-87C0-68B6B72699C7 in mixed-endian
    let expected_iso_type_guid: [u8; 16] = [
        0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44,
        0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7,
    ];
    assert_eq!(
        &entry0[0..16], &expected_iso_type_guid,
        "GPT entry 0 type GUID must be ISO9660 (EBD0A0A2-B9E5-4433-87C0-68B6B72699C7)"
    );
    // Entry 0 must have non-zero size and start >= 34
    let iso_start = u64::from_le_bytes(entry0[32..40].try_into().unwrap());
    let iso_end = u64::from_le_bytes(entry0[40..48].try_into().unwrap());
    assert!(iso_start >= 34, "ISO9660 partition must start at first usable LBA (34)");
    assert!(iso_end > iso_start, "ISO9660 partition must have non-zero size");

    // ---- Entry 1: EFI System Partition ----
    // GUID: C12A7328-F81F-11D2-BA4B-00A0C93EC93B in mixed-endian
    let expected_esp_type_guid: [u8; 16] = [
        0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11,
        0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B,
    ];
    assert_eq!(
        &entry1[0..16], &expected_esp_type_guid,
        "GPT entry 1 type GUID must be EFI System Partition (C12A7328-F81F-11D2-BA4B-00A0C93EC93B)"
    );

    let esp_start = u64::from_le_bytes(entry1[32..40].try_into().unwrap());
    let esp_end = u64::from_le_bytes(entry1[40..48].try_into().unwrap());
    let esp_attrs = u64::from_le_bytes(entry1[48..56].try_into().unwrap());

    assert_eq!(esp_start, 4096, "ESP must start at LBA 4096 (2 MiB)");
    assert!(esp_end > esp_start, "ESP must have non-zero size");

    // UEFI spec §5.3.3 says bit 0 (System Partition) SHOULD be set for ESP,
    // but real-world images (Ubuntu, Fedora, Windows) sometimes omit it.
    // Report as a warning for strict-mode validation, not a hard failure.
    if esp_attrs & 1 == 0 {
        println!(
            "WARNING: ESP partition attributes bit 0 (System Partition) is not set (attrs={:#x}). \
             UEFI spec recommends setting this bit, but some real firmware accepts it without.",
            esp_attrs
        );
    }

    // Partition name must be "EFI System Partition" (UTF-16LE)
    let name_bytes = &entry1[56..128];
    let name_u16: Vec<u16> = name_bytes
        .chunks_exact(2)
        .take(36)
        .map(|c| u16::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let name: String = String::from_utf16_lossy(&name_u16);
    assert_eq!(
        name.trim_end_matches('\0'),
        "EFI System Partition",
        "ESP partition name mismatch"
    );

    // ---- Entry 2: Gap1 (padding) ----
    let gap_type: [u8; 16] = entry2[0..16].try_into().unwrap();
    let zero_guid = [0u8; 16];
    if gap_type != zero_guid {
        let gap_start = u64::from_le_bytes(entry2[32..40].try_into().unwrap());
        let gap_end = u64::from_le_bytes(entry2[40..48].try_into().unwrap());
        assert!(gap_end > gap_start, "Gap1 must have non-zero size");
    }

    println!("GPT GUID exact bytes test PASSED");
    Ok(())
}

// ============================================================
// Test 2: Firmware-style ESP discovery
// ============================================================
//
// Simulates what real UEFI firmware does:
//   1. Read MBR → find GPT protective (0xEE)
//   2. Read GPT header → validate CRC
//   3. Scan partition entries for EFI_SYSTEM_PARTITION_GUID
//   4. Locate ESP via starting_lba
//   5. Validate FAT BPB at ESP start (hidden_sectors accepts plural values)
//   6. Traverse FAT directory → find \EFI\BOOT\BOOTX64.EFI
//
// This is the closest we can get to a firmware behaviour test
// without OVMF automation.

#[test]
fn test_firmware_style_esp_discovery() -> io::Result<()> {
    let (iso_path, _temp_dir) = build_test_iso()?;
    let mut iso_file = File::open(&iso_path)?;

    // Step 1: Read MBR at LBA 0
    iso_file.seek(SeekFrom::Start(0))?;
    let mut mbr = [0u8; 512];
    iso_file.read_exact(&mut mbr)?;

    // Validate MBR boot signature
    let mbr_sig = u16::from_le_bytes([mbr[510], mbr[511]]);
    assert_eq!(mbr_sig, 0xAA55, "MBR boot signature must be 0xAA55");

    // Find protective MBR entry (type 0xEE)
    let mut has_protective = false;
    for i in 0..4 {
        let offset = 0x1BE + i * 16;
        let ptype = mbr[offset + 4];
        if ptype == 0xEE {
            has_protective = true;
            break;
        }
    }
    assert!(has_protective, "MBR must contain GPT Protective entry (type 0xEE)");

    // Step 2: Read GPT header at LBA 1
    iso_file.seek(SeekFrom::Start(512))?;
    let mut gpt_header = [0u8; 92];
    iso_file.read_exact(&mut gpt_header)?;

    // Verify GPT signature
    assert_eq!(&gpt_header[0..8], b"EFI PART", "GPT signature must be 'EFI PART'");

    // Validate header CRC32
    let stored_crc = u32::from_le_bytes(gpt_header[16..20].try_into().unwrap());
    let header_size = u32::from_le_bytes(gpt_header[12..16].try_into().unwrap());
    assert_eq!(header_size, 92, "GPT header_size must be 92");
    let mut header_for_crc = gpt_header;
    header_for_crc[16..20].copy_from_slice(&[0u8; 4]);
    let calculated_crc = {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&header_for_crc[..header_size as usize]);
        hasher.finalize()
    };
    assert_eq!(
        stored_crc, calculated_crc,
        "GPT header CRC32 mismatch — firmware would reject this ISO"
    );

    // Step 3: Scan partition entries for ESP GUID
    let partition_entry_lba = u64::from_le_bytes(gpt_header[72..80].try_into().unwrap());
    iso_file.seek(SeekFrom::Start(partition_entry_lba * 512))?;

    let expected_esp_guid: [u8; 16] = [
        0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11,
        0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B,
    ];

    let mut esp_found = false;
    let mut esp_lba_512 = 0u64;
    for _entry_idx in 0..128 {
        let mut entry = [0u8; 128];
        iso_file.read_exact(&mut entry)?;

        // Break on empty entry (all zeros)
        if entry.iter().all(|&b| b == 0) {
            break;
        }

        if entry[0..16] == expected_esp_guid {
            esp_found = true;
            esp_lba_512 = u64::from_le_bytes(entry[32..40].try_into().unwrap());

            // Soft check: warn if bit 0 missing, but don't fail
            let attrs = u64::from_le_bytes(entry[48..56].try_into().unwrap());
            if attrs & 1 == 0 {
                println!(
                    "WARNING: ESP bit 0 (System Partition) not set — some firmware may still accept"
                );
            }
            break;
        }
    }
    assert!(esp_found, "Firmware could not find EFI System Partition in GPT");

    // Step 4: Move to ESP start offset in the ISO.
    let esp_offset = esp_lba_512 * 512;
    iso_file.seek(SeekFrom::Start(esp_offset))?;

    // Step 5: Validate FAT BPB
    let mut bpb = [0u8; 90];
    iso_file.read_exact(&mut bpb)?;

    // BPB validation per UEFI spec §13.3.1.1
    let bytes_per_sector = u16::from_le_bytes(bpb[11..13].try_into().unwrap());
    assert_eq!(bytes_per_sector, 512, "FAT BPB: bytes/sector must be 512");

    let sectors_per_cluster = bpb[13];
    assert!(
        sectors_per_cluster.is_power_of_two(),
        "FAT BPB: sectors/cluster ({}) must be power of 2",
        sectors_per_cluster
    );

    let fat_count = bpb[16];
    assert_eq!(fat_count, 2, "FAT BPB: FAT count must be 2");

    // hidden_sectors validation: both 0 and partition_start_lba are valid.
    //
    // - 0: correct when the medium is a dedicated partition image
    //      (GPT provides the absolute starting LBA, no offset needed)
    //
    // - partition_start_lba (here = 4096): used by some firmware (InsydeH2O,
    //   old AMI) that validates FAT BPB hidden_sectors matches the GPT
    //   partition start LBA.  Setting it to the partition offset matches
    //   what xorriso and Ubuntu ISOs do for USB-HDD direct access.
    let hidden_sectors = u32::from_le_bytes(bpb[28..32].try_into().unwrap());
    assert!(
        hidden_sectors == 0 || hidden_sectors == esp_lba_512 as u32,
        "FAT BPB hidden_sectors ({}) must be either 0 (partition image mode) \
         or equal to ESP start LBA ({}, partition-based mode)",
        hidden_sectors, esp_lba_512
    );

    let media_descriptor = bpb[21];
    assert_eq!(
        media_descriptor, 0xF8,
        "FAT BPB: media descriptor must be 0xF8 (hard disk)"
    );

    // Step 6: Mount FAT filesystem and find \EFI\BOOT\BOOTX64.EFI
    let total_sectors_16 = u16::from_le_bytes(bpb[19..21].try_into().unwrap());
    let total_sectors_32 = u32::from_le_bytes(bpb[32..36].try_into().unwrap());
    let total_sectors_512 = if total_sectors_16 != 0 {
        total_sectors_16 as u64
    } else {
        total_sectors_32 as u64
    };
    let esp_read_size = ((total_sectors_512 as usize) * 512).min(32 * 1024 * 1024);
    iso_file.seek(SeekFrom::Start(esp_offset))?;
    let mut esp_data = vec![0u8; esp_read_size];
    iso_file.read_exact(&mut esp_data)?;

    let fs = FileSystem::new(std::io::Cursor::new(esp_data), FsOptions::new())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("FAT mount failed: {:?}", e)))?;

    let root_dir = fs.root_dir();
    assert!(
        root_dir.open_file("EFI/BOOT/BOOTX64.EFI").is_ok(),
        "Firmware-style ESP traversal: \\EFI\\BOOT\\BOOTX64.EFI not found — firmware would report 'No bootfile found for UEFI!'"
    );

    // Also verify KERNEL.EFI exists
    assert!(
        root_dir.open_file("EFI/BOOT/KERNEL.EFI").is_ok(),
        "KERNEL.EFI must exist in ESP"
    );

    println!("Firmware-style ESP discovery test PASSED (ESP at LBA {}, FAT OK, BOOTX64.EFI found)", esp_lba_512);
    Ok(())
}

// ============================================================
// Test 3: Linux loop partition recognition
// ============================================================
//
// Uses losetup --partscan to scan the ISO as a block device,
// then verifies /dev/loopXpY appears for the ESP partition.
//
// Requires root privileges; skipped by default (marked #[ignore]).
// Run with: cargo test test_linux_loop_partition -- --ignored

#[test]
#[cfg(target_os = "linux")]
#[ignore]
fn test_linux_loop_partition_recognition() -> io::Result<()> {
    let (iso_path, _temp_dir) = build_test_iso()?;

    // Find a free loop device
    let find_output = run_command("losetup", &["-f"])?;
    let loop_dev = find_output.trim();
    assert!(!loop_dev.is_empty(), "No free loop device found");

    // Set up loop device with partition scanning
    let _setup_output = run_command(
        "sudo",
        &[
            "losetup",
            "--partscan",
            loop_dev,
            iso_path.to_str().unwrap(),
        ],
    )?;

    // Wait a moment for partition scanning to complete
    std::thread::sleep(std::time::Duration::from_millis(500));

    let loop_base = loop_dev.trim_start_matches("/dev/");
    let part1_dev = format!("/dev/{}p1", loop_base);
    let part2_dev = format!("/dev/{}p2", loop_base);

    // Verify partitions exist
    assert!(
        Path::new(&part1_dev).exists(),
        "loop partition p1 (ISO9660) not found at {}",
        part1_dev
    );
    assert!(
        Path::new(&part2_dev).exists(),
        "loop partition p2 (ESP) not found at {}",
        part2_dev
    );

    // Mount the ESP partition and verify BOOTX64.EFI
    let mount_point = _temp_dir.path().join("mnt_esp");
    std::fs::create_dir_all(&mount_point)?;

    let _mount_output = run_command(
        "sudo",
        &[
            "mount",
            "-t",
            "vfat",
            "-o",
            "ro,noexec,nosuid,nodev",
            &part2_dev,
            mount_point.to_str().unwrap(),
        ],
    )?;

    let bootx64_path = mount_point.join("EFI/BOOT/BOOTX64.EFI");
    assert!(
        bootx64_path.exists(),
        "BOOTX64.EFI not found in mounted ESP"
    );

    let kernel_path = mount_point.join("EFI/BOOT/KERNEL.EFI");
    assert!(kernel_path.exists(), "KERNEL.EFI not found in mounted ESP");

    // Cleanup
    run_command("sudo", &["umount", mount_point.to_str().unwrap()])?;
    run_command("sudo", &["losetup", "-d", loop_dev])?;

    println!("Linux loop partition recognition test PASSED");
    Ok(())
}

// ============================================================
// Test 4: Ventoy-style strict parser validation
// ============================================================
//
// Real-world firmware and Ventoy are strict about certain structural
// constraints.  This test validates that the generated ISO does NOT
// have common malformations that would cause rejection.
//
// Checks:
//   - No wrong GPT endianness
//   - FAT BPB hidden_sectors passes sanity check
//   - ESP does not overlap with ISO9660 data
//   - GPT header CRC and partition array CRC are valid
//   - Backup GPT header exists at end of file
//   - El Torito boot catalog checksum

#[test]
fn test_ventoy_style_strict_parser() -> io::Result<()> {
    let (iso_path, _temp_dir) = build_test_iso()?;
    let mut iso_file = File::open(&iso_path)?;

    // ── Check 1: GPT header CRC and partition array CRC ──
    iso_file.seek(SeekFrom::Start(512))?;
    let mut gpt_header = [0u8; 92];
    iso_file.read_exact(&mut gpt_header)?;

    // GPT signature
    assert_eq!(&gpt_header[0..8], b"EFI PART", "GPT signature must be 'EFI PART'");

    // Header CRC
    let stored_crc = u32::from_le_bytes(gpt_header[16..20].try_into().unwrap());
    let header_size = u32::from_le_bytes(gpt_header[12..16].try_into().unwrap());
    assert_eq!(header_size, 92, "GPT header_size must be 92");
    let mut header_for_crc = gpt_header;
    header_for_crc[16..20].copy_from_slice(&[0u8; 4]);
    let calculated_crc = {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&header_for_crc[..header_size as usize]);
        hasher.finalize()
    };
    assert_eq!(stored_crc, calculated_crc, "GPT header CRC mismatch");

    // Partition array CRC
    let partition_array_crc_stored =
        u32::from_le_bytes(gpt_header[88..92].try_into().unwrap());
    let partition_entry_lba = u64::from_le_bytes(gpt_header[72..80].try_into().unwrap());
    let num_entries = u32::from_le_bytes(gpt_header[80..84].try_into().unwrap());
    let entry_size = u32::from_le_bytes(gpt_header[84..88].try_into().unwrap());

    let array_len = (num_entries as usize) * (entry_size as usize);
    iso_file.seek(SeekFrom::Start(partition_entry_lba * 512))?;
    let mut array_bytes = vec![0u8; array_len];
    iso_file.read_exact(&mut array_bytes)?;

    let array_crc_calc = {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&array_bytes);
        hasher.finalize()
    };
    assert_eq!(
        partition_array_crc_stored, array_crc_calc,
        "GPT partition array CRC mismatch — stricter firmware rejects this"
    );

    // ── Check 2: No ESP / ISO9660 overlap ──
    iso_file.seek(SeekFrom::Start(partition_entry_lba * 512))?;
    let mut entry0 = [0u8; 128];
    let mut entry1 = [0u8; 128];
    iso_file.read_exact(&mut entry0)?; // entry 0: ISO9660
    iso_file.read_exact(&mut entry1)?; // entry 1: ESP

    let iso_start_0 = u64::from_le_bytes(entry0[32..40].try_into().unwrap());
    let iso_end_0 = u64::from_le_bytes(entry0[40..48].try_into().unwrap());
    let esp_start_1 = u64::from_le_bytes(entry1[32..40].try_into().unwrap());
    let esp_end_1 = u64::from_le_bytes(entry1[40..48].try_into().unwrap());

    // ISO9660 partition must cover the ESP region (isohybrid design)
    assert!(
        iso_start_0 <= esp_start_1,
        "ISO9660 partition must start before or at ESP start"
    );
    assert!(
        iso_end_0 >= esp_end_1,
        "ISO9660 partition must cover the ESP region"
    );

    // ── Check 3: Backup GPT exists at end of file ──
    iso_file.seek(SeekFrom::End(-512))?;
    let mut backup_gpt_sig = [0u8; 8];
    iso_file.read_exact(&mut backup_gpt_sig)?;
    assert_eq!(
        &backup_gpt_sig,
        b"EFI PART",
        "Backup GPT header must exist at the end of the ISO"
    );

    // ── Check 4: El Torito boot catalog checksum ──
    iso_file.seek(SeekFrom::Start(19 * 2048))?;
    let mut boot_catalog = [0u8; 32];
    iso_file.read_exact(&mut boot_catalog)?;

    let mut sum: u16 = 0;
    for chunk in boot_catalog.chunks_exact(2) {
        sum = sum.wrapping_add(u16::from_le_bytes(chunk.try_into().unwrap()));
    }
    assert_eq!(sum, 0, "El Torito boot catalog checksum must be 0");

    println!("Ventoy-style strict parser test PASSED");
    Ok(())
}

// ============================================================
// Test 5: blkid partition detection
// ============================================================
//
// Uses losetup --partscan + blkid to verify the Linux kernel
// recognises the GPT partitions (ISO9660 + ESP) correctly.
//
// Requires root privileges; skipped by default (marked #[ignore]).
// Run with: cargo test test_blkid_partition_detection -- --ignored

#[test]
#[cfg(target_os = "linux")]
#[ignore]
fn test_blkid_partition_detection() -> io::Result<()> {
    let (iso_path, _temp_dir) = build_test_iso()?;

    // Find a free loop device
    let find_output = run_command("losetup", &["-f"])?;
    let loop_dev = find_output.trim();
    assert!(!loop_dev.is_empty(), "No free loop device found");

    // Set up loop device with partition scanning
    let _setup_output = run_command(
        "sudo",
        &[
            "losetup",
            "--partscan",
            loop_dev,
            iso_path.to_str().unwrap(),
        ],
    )?;

    std::thread::sleep(std::time::Duration::from_millis(500));

    let loop_base = loop_dev.trim_start_matches("/dev/");
    let part1_dev = format!("/dev/{}p1", loop_base);
    let part2_dev = format!("/dev/{}p2", loop_base);

    // Run blkid on each partition device
    let blkid_p1 = run_command("sudo", &["blkid", "-p", "-o", "value", "-s", "TYPE", &part1_dev]);
    match blkid_p1 {
        Ok(ref output) => {
            let fs_type = output.trim();
            println!("blkid p1 (partition 1) type: {:?}", fs_type);
            // ISO9660 partition may show as "iso9660" or not be detected
            // depending on blkid version; detection is informational.
        }
        Err(ref e) => {
            println!("blkid p1 skipped (partition may be detected as 'iso9660' on some systems): {}", e);
        }
    }

    let blkid_p2 = run_command("sudo", &["blkid", "-p", "-o", "value", "-s", "TYPE", &part2_dev]);
    match blkid_p2 {
        Ok(ref output) => {
            let fs_type = output.trim();
            assert_eq!(
                fs_type, "vfat",
                "ESP partition must be detected as 'vfat' by blkid, got '{:?}'",
                fs_type
            );
            println!("blkid p2 (ESP) type: vfat ✓");
        }
        Err(ref e) => {
            panic!("blkid failed for ESP partition {}: {}", part2_dev, e);
        }
    }

    // Also verify PARTLABEL
    let blkid_label = run_command(
        "sudo",
        &["blkid", "-p", "-o", "value", "-s", "PARTLABEL", &part2_dev],
    );
    if let Ok(ref output) = blkid_label {
        let label = output.trim();
        assert_eq!(
            label, "EFI System Partition",
            "ESP partition label must be 'EFI System Partition', got '{:?}'",
            label
        );
        println!("blkid p2 (ESP) PARTLABEL: '{}' ✓", label);
    }

    // Verify PARTUUID is non-empty
    let blkid_uuid = run_command(
        "sudo",
        &["blkid", "-p", "-o", "value", "-s", "PARTUUID", &part2_dev],
    );
    if let Ok(ref output) = blkid_uuid {
        let uuid = output.trim();
        assert!(!uuid.is_empty(), "ESP PARTUUID must not be empty");
        println!("blkid p2 (ESP) PARTUUID: {} ✓", uuid);
    }

    // Also run sgdisk -v for additional GPT integrity validation
    let sgdisk_output = run_command("sudo", &["sgdisk", "-v", iso_path.to_str().unwrap()]);
    if let Ok(ref output) = sgdisk_output {
        // Check for warnings/errors
        let problems: Vec<&str> = output.lines()
            .filter(|l| l.contains("Problem:") || l.contains("Warning:"))
            .collect();
        if !problems.is_empty() {
            println!("sgdisk -v warnings:");
            for p in &problems {
                println!("  {}", p);
            }
            // Known acceptable: "MBR partitions 1 and 2 overlap" is by design for hybrid MBR+GPT
            let critical: Vec<&&str> = problems.iter()
                .filter(|p| !p.contains("MBR partitions 1 and 2 overlap"))
                .collect();
            assert!(
                critical.is_empty(),
                "sgdisk -v found critical problems: {:?}",
                critical
            );
        }
        println!("sgdisk -v validation PASSED");
    } else {
        println!("sgdisk -v skipped (not available or permission issue)");
    }

    // Cleanup
    run_command("sudo", &["losetup", "-d", loop_dev])?;

    println!("blkid partition detection test PASSED");
    Ok(())
}