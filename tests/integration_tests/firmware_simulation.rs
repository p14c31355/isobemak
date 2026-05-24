//! Firmware behaviour simulation tests.
//!
//! These tests go beyond "structure is correct" and verify that the
//! generated ISO would be accepted by real UEFI firmware through:
//!
//! 1. GPT GUID exact bytes + backup GPT complete CRC check
//! 2. Firmware-style ESP discovery (MBR→GPT→BPB→FAT→BOOTX64.EFI)
//! 3. Ventoy-style strict parser validation
//! 4. Linux loop partition recognition (requires root, ignored)
//! 5. blkid-based partition detection (requires root, ignored)

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

/// Validate ISO file size is 512-byte aligned (required by GPT/MBR).
fn assert_512_aligned(file_len: u64) -> u64 {
    assert_eq!(
        file_len % 512, 0,
        "ISO file size ({} bytes) must be 512-byte aligned for GPT/MBR", file_len
    );
    file_len / 512 - 1
}

/// Decode a GPT 8-byte LE field.
fn gpt_u64(header: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap())
}

/// Decode a GPT 4-byte LE field.
fn gpt_u32(header: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap())
}

/// Decode a GPT partition GUID field (16 bytes).
fn gpt_guid(entry: &[u8]) -> &[u8] {
    &entry[0..16]
}

/// Reads the Boot Record Volume Descriptor (LBA 17) and extracts the
/// Boot Catalog LBA from it.  This avoids hard-coding LBA 19.
fn read_boot_catalog_lba(iso_file: &mut File) -> io::Result<u32> {
    iso_file.seek(SeekFrom::Start(17 * 2048))?;
    let mut brvd = [0u8; 2048];
    iso_file.read_exact(&mut brvd)?;

    // BRVD: bytes 0-6 = header (0x00, CD001, version), bytes 7-38 = boot id
    // Boot Catalog LBA is at offset 71-74 (little-endian)
    assert_eq!(&brvd[0..7], &[0x00, b'C', b'D', b'0', b'0', b'1', 0x01],
        "BRVD not found at LBA 17");
    assert_eq!(&brvd[7..30], b"EL TORITO SPECIFICATION",
        "BRVD must contain 'EL TORITO SPECIFICATION'");

    let lba = u32::from_le_bytes(brvd[71..75].try_into().unwrap());
    assert!(lba > 0, "Boot catalog LBA in BRVD is invalid (0)");
    Ok(lba)
}

// ============================================================
// Test 1: GPT GUID exact bytes + backup GPT complete CRC
// ============================================================

#[test]
fn test_gpt_guid_exact_bytes() -> io::Result<()> {
    let (iso_path, _temp_dir) = build_test_iso()?;
    let mut iso_file = File::open(&iso_path)?;

    let file_len = iso_file.metadata()?.len();
    let last_lba_512 = assert_512_aligned(file_len);

    // ── Primary GPT header ──
    iso_file.seek(SeekFrom::Start(512))?;
    let mut primary = [0u8; 92];
    iso_file.read_exact(&mut primary)?;

    assert_eq!(&primary[0..8], b"EFI PART", "Primary GPT signature mismatch");
    assert_eq!(gpt_u64(&primary, 24), 1, "Primary current_lba must be 1");
    assert_eq!(gpt_u64(&primary, 32), last_lba_512,
        "Primary backup_lba must point to last sector");

    let partition_entry_lba = gpt_u64(&primary, 72);
    assert_eq!(partition_entry_lba, 2);
    let num_entries = gpt_u32(&primary, 80);
    let entry_size = gpt_u32(&primary, 84);
    assert_eq!(num_entries, 128);
    assert_eq!(entry_size, 128);

    // ── Partition entries ──
    iso_file.seek(SeekFrom::Start(2 * 512))?;
    let mut entry0 = [0u8; 128];
    let mut entry1 = [0u8; 128];
    let mut entry2 = [0u8; 128];
    iso_file.read_exact(&mut entry0)?;
    iso_file.read_exact(&mut entry1)?;
    iso_file.read_exact(&mut entry2)?;

    // Entry 0: Microsoft Basic Data (used for ISO9660 by Ubuntu/xorriso)
    let expected_iso_guid: [u8; 16] = [
        0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44,
        0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7,
    ];
    assert_eq!(gpt_guid(&entry0), &expected_iso_guid,
        "Entry 0 GUID must be Microsoft Basic Data (EBD0A0A2-...)");

    let iso_start = u64::from_le_bytes(entry0[32..40].try_into().unwrap());
    assert!(iso_start >= 34);

    // Entry 1: ESP
    let expected_esp_guid: [u8; 16] = [
        0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11,
        0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B,
    ];
    assert_eq!(gpt_guid(&entry1), &expected_esp_guid,
        "Entry 1 GUID must be EFI System Partition (C12A7328-...)");

    let esp_start = u64::from_le_bytes(entry1[32..40].try_into().unwrap());
    // ESP is file-backed, so the LBA depends on filesystem layout.
    // It should be after GPT reserved area (>=34) and within a reasonable range.
    assert!(esp_start >= 34, "ESP must start after GPT reserved area (>=34), got {}", esp_start);
    assert!(esp_start < 4096, "ESP should be a file-backed partition (<4096), got {}", esp_start);

    // Soft warn on attributes bit 0 (Ubuntu also omits)
    let esp_attrs = u64::from_le_bytes(entry1[48..56].try_into().unwrap());
    if esp_attrs & 1 == 0 {
        println!("WARNING: ESP bit 0 (System Partition) not set — Ubuntu ISOs also omit this.");
    }

    // Entry 2: Gap1 (may be zero)
    let zero_guid = [0u8; 16];
    if entry2[0..16] != zero_guid {
        let gap_end = u64::from_le_bytes(entry2[40..48].try_into().unwrap());
        let gap_start = u64::from_le_bytes(entry2[32..40].try_into().unwrap());
        assert!(gap_end > gap_start, "Gap1 must have non-zero size");
    }

    // ── Backup GPT header ──
    iso_file.seek(SeekFrom::End(-512))?;
    let mut backup = [0u8; 92];
    iso_file.read_exact(&mut backup)?;

    assert_eq!(&backup[0..8], b"EFI PART", "Backup GPT signature mismatch");
    assert_eq!(gpt_u64(&backup, 24), last_lba_512, "Backup current_lba mismatch");
    assert_eq!(gpt_u64(&backup, 32), 1, "Backup backup_lba must be 1");
    assert_eq!(gpt_u64(&backup, 72), last_lba_512.saturating_sub(32));

    // Backup UUIDs must match primary
    assert_eq!(backup[56..72], primary[56..72],
        "Backup disk GUID must match primary");
    assert_eq!(gpt_u64(&backup, 40), gpt_u64(&primary, 40),
        "Backup first_usable_lba mismatch");
    assert_eq!(gpt_u64(&backup, 48), gpt_u64(&primary, 48),
        "Backup last_usable_lba mismatch");

    // ── Backup GPT header CRC32 ──
    let backup_stored_crc = gpt_u32(&backup, 16);
    let backup_hdr_size = gpt_u32(&backup, 12) as usize;
    assert_eq!(backup_hdr_size, 92, "Backup GPT header_size must be 92");
    let mut backup_for_crc = backup;
    backup_for_crc[16..20].copy_from_slice(&[0u8; 4]);
    let backup_calc_crc = {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&backup_for_crc[..backup_hdr_size]);
        hasher.finalize()
    };
    assert_eq!(
        backup_stored_crc, backup_calc_crc,
        "Backup GPT header CRC32 mismatch — firmware would reject this ISO"
    );

    // ── Backup partition array CRC32 ──
    let backup_arr_crc_stored = gpt_u32(&backup, 88);
    let backup_entry_lba = gpt_u64(&backup, 72);
    iso_file.seek(SeekFrom::Start(backup_entry_lba * 512))?;
    let array_bytes = (num_entries as usize) * (entry_size as usize);
    let mut backup_array = vec![0u8; array_bytes];
    iso_file.read_exact(&mut backup_array)?;

    let backup_arr_calc = {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&backup_array);
        hasher.finalize()
    };
    assert_eq!(
        backup_arr_crc_stored, backup_arr_calc,
        "Backup GPT partition array CRC32 mismatch"
    );

    println!("GPT GUID + backup GPT complete CRC test PASSED");
    Ok(())
}

// ============================================================
// Test 2: Firmware-style ESP discovery
// ============================================================

#[test]
fn test_firmware_style_esp_discovery() -> io::Result<()> {
    let (iso_path, _temp_dir) = build_test_iso()?;
    let mut iso_file = File::open(&iso_path)?;

    // ── Step 1: MBR LBA 0 ──
    iso_file.seek(SeekFrom::Start(0))?;
    let mut mbr = [0u8; 512];
    iso_file.read_exact(&mut mbr)?;

    let mbr_sig = u16::from_le_bytes([mbr[510], mbr[511]]);
    assert_eq!(mbr_sig, 0xAA55, "MBR boot signature mismatch");

    let mut protective_ok = false;
    for i in 0..4 {
        let off = 0x1BE + i * 16;
        if mbr[off + 4] == 0xEE {
            let start = u32::from_le_bytes(mbr[(off + 8)..(off + 12)].try_into().unwrap());
            let size = u32::from_le_bytes(mbr[(off + 12)..(off + 16)].try_into().unwrap());
            assert_eq!(start, 1, "Protective MBR must start at LBA 1");
            assert!(size == 0xFFFFFFFF || size > 0,
                "Protective MBR size must be non-zero or 0xFFFFFFFF");
            protective_ok = true;
            break;
        }
    }
    assert!(protective_ok, "No GPT Protective MBR entry found");

    // ── Step 2: GPT header ──
    iso_file.seek(SeekFrom::Start(512))?;
    let mut gpt_header = [0u8; 92];
    iso_file.read_exact(&mut gpt_header)?;
    assert_eq!(&gpt_header[0..8], b"EFI PART");

    // CRC
    let stored_crc = gpt_u32(&gpt_header, 16);
    let hdr_sz = gpt_u32(&gpt_header, 12) as usize;
    assert_eq!(hdr_sz, 92);
    let mut hdr_crc = gpt_header;
    hdr_crc[16..20].copy_from_slice(&[0u8; 4]);
    let calc_crc = {
        let mut h = crc32fast::Hasher::new();
        h.update(&hdr_crc[..hdr_sz]);
        h.finalize()
    };
    assert_eq!(stored_crc, calc_crc, "GPT header CRC mismatch — firmware would reject");

    // ── Step 3: Scan for ESP ──
    iso_file.seek(SeekFrom::Start(gpt_u64(&gpt_header, 72) * 512))?;

    let esp_guid: [u8; 16] = [
        0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11,
        0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B,
    ];

    let mut esp_found = false;
    let mut esp_lba_512 = 0u64;
    for _ in 0..128 {
        let mut entry = [0u8; 128];
        iso_file.read_exact(&mut entry)?;
        if entry.iter().all(|&b| b == 0) { break; }
        if entry[0..16] == esp_guid {
            esp_found = true;
            esp_lba_512 = u64::from_le_bytes(entry[32..40].try_into().unwrap());
            break;
        }
    }
    assert!(esp_found, "ESP GUID not found in GPT");

    // ── Step 4: Locate ESP ──
    let esp_offset = esp_lba_512 * 512;
    iso_file.seek(SeekFrom::Start(esp_offset))?;

    // ── Step 5: FAT BPB ──
    let mut bpb = [0u8; 512];
    iso_file.read_exact(&mut bpb)?;

    // Boot sector signature (Insyde/Phoenix)
    assert_eq!(bpb[510], 0x55, "FAT boot sector sig byte 510 must be 0x55");
    assert_eq!(bpb[511], 0xAA, "FAT boot sector sig byte 511 must be 0xAA");

    let bps = u16::from_le_bytes(bpb[11..13].try_into().unwrap());
    assert_eq!(bps, 512, "FAT BPB: bytes/sector must be 512");

    let spc = bpb[13];
    assert!(spc >= 1 && spc.is_power_of_two(),
        "FAT BPB: sectors/cluster ({}) must be >= 1 and power of 2", spc);

    assert_eq!(bpb[16], 2, "FAT BPB: FAT count must be 2");

    let hidden = u32::from_le_bytes(bpb[28..32].try_into().unwrap());
    assert!(hidden == 0 || hidden == esp_lba_512 as u32,
        "FAT BPB hidden_sectors ({}) must be 0 or ESP start LBA ({})",
        hidden, esp_lba_512);

    assert_eq!(bpb[21], 0xF8, "FAT BPB: media descriptor must be 0xF8");

    // FAT type string: offset 54 for FAT12/16, offset 82 for FAT32
    // Fixed-length 8-byte field, trimmed. Firmware expects exact values.
    let t54 = String::from_utf8_lossy(&bpb[54..62]);
    let t82 = String::from_utf8_lossy(&bpb[82..90]);
    let fat_str = if t54.starts_with("FAT") { &t54 }
                  else if t82.starts_with("FAT") { &t82 }
                  else { "" };
    assert!(
        fat_str == "FAT12   " || fat_str == "FAT16   " || fat_str == "FAT32   ",
        "FAT BPB type string must be 'FAT12   ', 'FAT16   ', or 'FAT32   ', got '{}'",
        fat_str
    );
    println!("FAT BPB type string: '{}' ✓", fat_str);

    // ── Step 6: FAT traversal → BOOTX64.EFI ──
    let ts16 = u16::from_le_bytes(bpb[19..21].try_into().unwrap());
    let ts32 = u32::from_le_bytes(bpb[32..36].try_into().unwrap());
    let total_sectors = if ts16 != 0 { ts16 as u64 } else { ts32 as u64 };

    // Safety: max 256 MiB (ESP minimum size for FAT32)
    assert!(total_sectors <= 524288,
        "ESP total sectors ({}) exceeds safety limit (524288 = 256 MiB)", total_sectors);

    let read_size = ((total_sectors as usize) * 512).min(256 * 1024 * 1024);
    let mut esp_data = vec![0u8; read_size];
    iso_file.seek(SeekFrom::Start(esp_offset))?;
    iso_file.read_exact(&mut esp_data)?;

    let fs = FileSystem::new(std::io::Cursor::new(esp_data), FsOptions::new())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("FAT mount failed: {:?}", e)))?;

    let root = fs.root_dir();
    assert!(root.open_file("EFI/BOOT/BOOTX64.EFI").is_ok(),
        "BOOTX64.EFI not found — firmware would report 'No bootfile found for UEFI!'");
    assert!(root.open_file("EFI/BOOT/KERNEL.EFI").is_ok(),
        "KERNEL.EFI not found in ESP");

    println!("Firmware-style ESP discovery PASSED (ESP LBA {}, FAT {})", esp_lba_512, fat_str);
    Ok(())
}

// ============================================================
// Test 3: Ventoy-style strict parser validation
// ============================================================

#[test]
fn test_ventoy_style_strict_parser() -> io::Result<()> {
    let (iso_path, _temp_dir) = build_test_iso()?;
    let mut iso_file = File::open(&iso_path)?;

    let file_len = iso_file.metadata()?.len();
    let last_lba = assert_512_aligned(file_len);

    // ── GPT header CRC + partition array CRC ──
    iso_file.seek(SeekFrom::Start(512))?;
    let mut hdr = [0u8; 92];
    iso_file.read_exact(&mut hdr)?;
    assert_eq!(&hdr[0..8], b"EFI PART");

    let stored_crc = gpt_u32(&hdr, 16);
    let hdr_sz = gpt_u32(&hdr, 12) as usize;
    assert_eq!(hdr_sz, 92);
    let mut hdr_c = hdr;
    hdr_c[16..20].copy_from_slice(&[0u8; 4]);
    let calc_crc = { let mut hasher = crc32fast::Hasher::new(); hasher.update(&hdr_c[..hdr_sz]); hasher.finalize() };
    assert_eq!(stored_crc, calc_crc, "GPT header CRC mismatch");

    let arr_crc_stored = gpt_u32(&hdr, 88);
    let pe_lba = gpt_u64(&hdr, 72);
    let n_ent = gpt_u32(&hdr, 80) as usize;
    let e_sz = gpt_u32(&hdr, 84) as usize;
    let arr_len = n_ent * e_sz;

    iso_file.seek(SeekFrom::Start(pe_lba * 512))?;
    let mut arr = vec![0u8; arr_len];
    iso_file.read_exact(&mut arr)?;
    let arr_crc = { let mut h = crc32fast::Hasher::new(); h.update(&arr); h.finalize() };
    assert_eq!(arr_crc_stored, arr_crc, "GPT partition array CRC mismatch — firmware rejects");

    // ── Overlap check ──
    iso_file.seek(SeekFrom::Start(pe_lba * 512))?;
    let mut e0 = [0u8; 128];
    let mut e1 = [0u8; 128];
    iso_file.read_exact(&mut e0)?;
    iso_file.read_exact(&mut e1)?;

    let iso_s = u64::from_le_bytes(e0[32..40].try_into().unwrap());
    let iso_e = u64::from_le_bytes(e0[40..48].try_into().unwrap());
    let esp_s = u64::from_le_bytes(e1[32..40].try_into().unwrap());
    let esp_e = u64::from_le_bytes(e1[40..48].try_into().unwrap());
    assert!(iso_s <= esp_s && iso_e >= esp_e, "ESP must be within ISO9660 partition");

    // ── Backup GPT cross-check + CRC ──
    iso_file.seek(SeekFrom::End(-512))?;
    let mut bk = [0u8; 92];
    iso_file.read_exact(&mut bk)?;
    assert_eq!(&bk[0..8], b"EFI PART");
    assert_eq!(gpt_u64(&bk, 24), last_lba, "Backup current_lba mismatch");
    assert_eq!(gpt_u64(&bk, 32), 1, "Backup backup_lba must be 1");

    // Backup header CRC
    let b_stored = gpt_u32(&bk, 16);
    let b_sz = gpt_u32(&bk, 12) as usize;
    assert_eq!(b_sz, 92);
    let mut b_c = bk;
    b_c[16..20].copy_from_slice(&[0u8; 4]);
    let b_calc = { let mut h = crc32fast::Hasher::new(); h.update(&b_c[..b_sz]); h.finalize() };
    assert_eq!(b_stored, b_calc, "Backup GPT header CRC mismatch");

    // Backup partition array CRC
    let ba_stored = gpt_u32(&bk, 88);
    let ba_lba = gpt_u64(&bk, 72);
    iso_file.seek(SeekFrom::Start(ba_lba * 512))?;
    let mut ba = vec![0u8; arr_len];
    iso_file.read_exact(&mut ba)?;
    let ba_calc = { let mut h = crc32fast::Hasher::new(); h.update(&ba); h.finalize() };
    assert_eq!(ba_stored, ba_calc, "Backup GPT partition array CRC mismatch");

    // ── El Torito catalog (dynamic LBA from BRVD) ──
    let catalog_lba = read_boot_catalog_lba(&mut iso_file)?;
    iso_file.seek(SeekFrom::Start(catalog_lba as u64 * 2048))?;
    let mut catalog = [0u8; 32];
    iso_file.read_exact(&mut catalog)?;

    let mut sum: u16 = 0;
    for c in catalog.chunks_exact(2) {
        sum = sum.wrapping_add(u16::from_le_bytes(c.try_into().unwrap()));
    }
    assert_eq!(sum, 0, "El Torito boot catalog checksum mismatch");

    println!("Ventoy-style strict parser PASSED");
    Ok(())
}

// ============================================================
// Test 4: Linux loop partition recognition (root required, #[ignore])
// ============================================================

// RAII guard for loop device cleanup
struct LoopGuard {
    dev: String,
}

impl LoopGuard {
    fn new(dev: String) -> Self {
        Self { dev }
    }
}

impl Drop for LoopGuard {
    fn drop(&mut self) {
        let _ = run_command("sudo", &["losetup", "-d", &self.dev]);
    }
}

#[test]
#[cfg(target_os = "linux")]
#[ignore]
fn test_linux_loop_partition_recognition() -> io::Result<()> {
    let (iso_path, _temp_dir) = build_test_iso()?;

    let find_output = run_command("losetup", &["-f"])?;
    let loop_dev = find_output.trim().to_string();
    assert!(!loop_dev.is_empty(), "No free loop device found");

    // Set up loop device — RAII guard ensures cleanup
    let _guard = LoopGuard::new(loop_dev.clone());

    let _ = run_command(
        "sudo",
        &["losetup", "--partscan", &loop_dev, iso_path.to_str().unwrap()],
    )?;
    std::thread::sleep(std::time::Duration::from_millis(500));

    let loop_base = loop_dev.trim_start_matches("/dev/");
    let part1 = format!("/dev/{}p1", loop_base);
    let part2 = format!("/dev/{}p2", loop_base);

    assert!(Path::new(&part1).exists(), "Partition p1 not found at {}", part1);
    assert!(Path::new(&part2).exists(), "Partition p2 not found at {}", part2);

    let mnt = _temp_dir.path().join("mnt_esp");
    std::fs::create_dir_all(&mnt)?;

    let _ = run_command(
        "sudo",
        &["mount", "-t", "vfat", "-o", "ro,noexec,nosuid,nodev", &part2, mnt.to_str().unwrap()],
    )?;

    assert!(mnt.join("EFI/BOOT/BOOTX64.EFI").exists(), "BOOTX64.EFI missing in mounted ESP");
    assert!(mnt.join("EFI/BOOT/KERNEL.EFI").exists(), "KERNEL.EFI missing in mounted ESP");

    let _ = run_command("sudo", &["umount", mnt.to_str().unwrap()]);
    // _guard will detach loop device on drop

    println!("Linux loop partition recognition PASSED");
    Ok(())
}

// ============================================================
// Test 5: blkid partition detection (root required, #[ignore])
// ============================================================

#[test]
#[cfg(target_os = "linux")]
#[ignore]
fn test_blkid_partition_detection() -> io::Result<()> {
    let (iso_path, _temp_dir) = build_test_iso()?;

    let find_output = run_command("losetup", &["-f"])?;
    let loop_dev = find_output.trim().to_string();
    assert!(!loop_dev.is_empty(), "No free loop device found");

    let _guard = LoopGuard::new(loop_dev.clone());

    let _ = run_command(
        "sudo",
        &["losetup", "--partscan", &loop_dev, iso_path.to_str().unwrap()],
    )?;
    std::thread::sleep(std::time::Duration::from_millis(500));

    let loop_base = loop_dev.trim_start_matches("/dev/");
    let part2 = format!("/dev/{}p2", loop_base);

    // blkid TYPE must be vfat
    let blkid = run_command("sudo", &["blkid", "-p", "-o", "value", "-s", "TYPE", &part2]);
    match blkid {
        Ok(ref o) => {
            assert_eq!(o.trim(), "vfat", "ESP partition must be detected as 'vfat' by blkid, got '{:?}'", o);
            println!("blkid ESP TYPE: vfat ✓");
        }
        Err(ref e) => panic!("blkid failed for {}: {}", part2, e),
    }

    // PARTLABEL
    if let Ok(ref o) = run_command("sudo",
        &["blkid", "-p", "-o", "value", "-s", "PARTLABEL", &part2])
    {
        assert_eq!(o.trim(), "EFI System Partition", "ESP PARTLABEL mismatch");
        println!("blkid ESP PARTLABEL: '{}' ✓", o.trim());
    }

    // PARTUUID
    if let Ok(ref o) = run_command("sudo",
        &["blkid", "-p", "-o", "value", "-s", "PARTUUID", &part2])
    {
        assert!(!o.trim().is_empty(), "ESP PARTUUID must not be empty");
        println!("blkid ESP PARTUUID: {} ✓", o.trim());
    }

    // sgdisk -v
    if let Ok(ref o) = run_command("sudo", &["sgdisk", "-v", iso_path.to_str().unwrap()]) {
        let problems: Vec<&str> = o.lines()
            .filter(|l| l.contains("Problem:") || l.contains("Warning:"))
            .collect();
        let critical: Vec<&&str> = problems.iter()
            .filter(|p| !p.contains("MBR partitions 1 and 2 overlap"))
            .collect();
        assert!(critical.is_empty(), "sgdisk -v critical problems: {:?}", critical);
        println!("sgdisk -v PASSED");
    }

    println!("blkid partition detection PASSED");

    // _guard will detach loop device on drop
    Ok(())
}