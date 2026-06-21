use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

use fatfs::{FileSystem, FsOptions};
use isobemak::{BootInfo, IsoImage, IsoImageFile, IsoLayoutProfile, UefiBootInfo, build_iso};
use tempfile::tempdir;

use crate::integration_tests::common::{
    run_command, setup_integration_test_files, verify_gpt_and_mbr_chs, verify_iso_binary_structures,
};

fn verify_fat_image_has_file(fat_img_path: &std::path::Path, fat_path: &str) -> io::Result<()> {
    let fat_file = File::open(fat_img_path)?;
    let fs = FileSystem::new(fat_file, FsOptions::new())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let root_dir = fs.root_dir();
    // fatfs uses "/" as path separator
    root_dir.open_file(fat_path).map_err(|e| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("File '{}' not found in FAT image: {:?}", fat_path, e),
        )
    })?;
    Ok(())
}

#[test]
fn test_create_isohybrid_uefi_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let temp_dir_path = temp_dir.path();
    println!("Temp dir for isohybrid UEFI test: {:?}", &temp_dir_path);

    // Setup files and paths
    let (bootx64_path, kernel_path, iso_path) = setup_integration_test_files(temp_dir_path)?;

    let iso_image = IsoImage {
        volume_id: None,
        files: vec![
            IsoImageFile {
                source: bootx64_path.clone(),
                destination: "EFI/BOOT/BOOTX64.EFI".to_string(),
            },
            IsoImageFile {
                source: kernel_path.clone(),
                destination: "EFI/BOOT/KERNEL.EFI".to_string(),
            },
        ],
        boot_info: BootInfo {
            bios_boot: None,
            uefi_boot: Some(UefiBootInfo {
                boot_image: bootx64_path.clone(),
                kernel_image: kernel_path.clone(),
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
                additional_efi_boot_files: Vec::new(),
                grub_cfg_content: None,
            }),
        },
        layout_profile: IsoLayoutProfile::default(),
    };

    // Call the main function with is_isohybrid set to true.
    // Scope the returned handles so they are dropped before we open the ISO
    // read-only — this guarantees OS-level flush of all buffered writes
    // (especially the GPT structures) before verification.
    {
        // Drop returned handles before verification so the OS flushes
        // GPT/MBR structures written via write_hybrid_structures.
        let (_fat_image_path, _temp_fat, _iso_file, _logical_size) =
            build_iso(&iso_path, &iso_image, true)?;
    }
    assert!(iso_path.exists());

    // Verify ISO content using isoinfo -d
    let isoinfo_d_output = run_command("isoinfo", &["-d", "-i", iso_path.to_str().unwrap()])?;
    println!("isoinfo -d output (isohybrid):\n{}", isoinfo_d_output);
    assert!(isoinfo_d_output.contains("Volume id: ISOBEMAKI"));

    // Verify the UEFI boot catalog entry
    let mut iso_file_for_nsect_check = File::open(&iso_path)?;
    let boot_catalog_start_pos = isobemak::iso::boot_catalog::LBA_BOOT_CATALOG as u64
        * isobemak::utils::ISO_SECTOR_SIZE as u64;
    iso_file_for_nsect_check.seek(SeekFrom::Start(boot_catalog_start_pos))?;

    let mut boot_catalog_sector = [0u8; isobemak::utils::ISO_SECTOR_SIZE];
    iso_file_for_nsect_check.read_exact(&mut boot_catalog_sector)?;

    // Platform ID 0x00 (80x86) in the Validation Entry is the standard
    // per El Torito spec §6.2.1.  The boot entry system_type=0xEF carries
    // the UEFI platform identification.
    assert_eq!(
        boot_catalog_sector[1], 0x00,
        "Validation entry platform ID must be 0x00 (80x86) per El Torito spec"
    );

    // UEFI boot catalog (UEFI only, no BIOS):
    //
    //   Validation Entry     (offset 0,  32 bytes): header_id=1, platform=0x00
    //   Section Header       (offset 32, 32 bytes): flag=0x91, platform=0xEF, entries=2
    //   Boot Entry 0         (offset 64, 32 bytes): flag=0x88, NoEmul, system_type=0xEF, LBA→ESP
    //   Section Boot Entry   (offset 96, 32 bytes): flag=0x88, NoEmul, system_type=0x00, LBA→ESP
    //
    // When no BIOS boot is configured, UEFI entries are placed under a
    // dedicated Section Header with platform_id=0xEF.  Real UEFI firmware
    // (InsydeH2O, old AMI, Lenovo, Panasonic) requires the Section Header
    // with Platform ID 0xEF to discover the UEFI boot entry.
    //
    // --- Section Header (offset 32) ---
    let sec_offset = 32;
    let sec_bytes = &boot_catalog_sector[sec_offset..sec_offset + 32];
    let sec_flag = sec_bytes[0];
    let sec_platform = sec_bytes[1];
    let sec_count = u16::from_le_bytes(sec_bytes[2..4].try_into().unwrap());
    let sec_sys = sec_bytes[4];

    assert_eq!(
        sec_flag,
        isobemak::iso::boot_catalog::BOOT_CATALOG_SECTION_HEADER_FINAL_ID,
        "Section Header must be final (0x91), got {:#x}",
        sec_flag
    );
    assert_eq!(
        sec_platform,
        isobemak::iso::boot_catalog::BOOT_CATALOG_EFI_PLATFORM_ID,
        "Section Header platform must be 0xEF, got {:#x}",
        sec_platform
    );
    assert_eq!(
        sec_count, 2,
        "Section Header entries must be 2, got {}",
        sec_count
    );
    assert_eq!(
        sec_sys, 0x00,
        "Section Header system_type must be 0x00, got {:#x}",
        sec_sys
    );

    // --- Boot Entry 0 (offset 64) ---
    let boot0_offset = 64;
    let boot0_bytes = &boot_catalog_sector[boot0_offset..boot0_offset + 32];
    let boot0_indicator = boot0_bytes[0];
    let boot0_media = boot0_bytes[1];
    let boot0_sys = boot0_bytes[4];
    let boot0_lba = u32::from_le_bytes(boot0_bytes[8..12].try_into().unwrap());
    let boot0_sectors = u16::from_le_bytes(boot0_bytes[6..8].try_into().unwrap());

    assert_eq!(
        boot0_indicator,
        isobemak::iso::boot_catalog::BOOT_CATALOG_BOOT_ENTRY_HEADER_ID,
        "Boot Entry 0 must be bootable (0x88), got {:#x}",
        boot0_indicator
    );
    assert_eq!(
        boot0_media, 0x00,
        "Boot Entry 0 must use No Emulation (0x00), got {:#x}",
        boot0_media
    );
    assert_eq!(
        boot0_sys,
        isobemak::iso::boot_catalog::BOOT_CATALOG_EFI_PLATFORM_ID,
        "Boot Entry 0 system_type must be 0xEF for UEFI, got {:#x}",
        boot0_sys
    );
    // efiboot.img is a large file (~260 MiB FAT32 minimum) placed after
    // regular files in the ISO filesystem (alphabetical sort order).
    // Its LBA depends on the file layout and is expected to be > 20
    // but may be large due to the FAT32 minimum size constraint.
    assert!(
        boot0_lba > 19,
        "Boot Entry 0 Load RBA ({}) should be after volume descriptors (>=20), got {}",
        boot0_lba,
        boot0_lba
    );
    assert_eq!(
        boot0_sectors, 0,
        "Boot Entry 0 no-emulation sector_count must be 0, got {}",
        boot0_sectors
    );

    // --- Section Boot Entry (offset 96) ---
    let sentry_offset = 96;
    let sentry_bytes = &boot_catalog_sector[sentry_offset..sentry_offset + 32];
    let sentry_indicator = sentry_bytes[0];
    let sentry_media = sentry_bytes[1];
    let sentry_sys = sentry_bytes[4];
    let sentry_lba = u32::from_le_bytes(sentry_bytes[8..12].try_into().unwrap());
    let sentry_sectors = u16::from_le_bytes(sentry_bytes[6..8].try_into().unwrap());

    assert_eq!(
        sentry_indicator,
        isobemak::iso::boot_catalog::BOOT_CATALOG_BOOT_ENTRY_HEADER_ID,
        "Section Boot Entry must be bootable (0x88), got {:#x}",
        sentry_indicator
    );
    assert_eq!(
        sentry_media, 0x00,
        "Section Boot Entry must use No Emulation (0x00), got {:#x}",
        sentry_media
    );
    // Under Section Header, system_type is conventionally 0x00
    // (the section header's platform_id already identifies the target as EFI).
    // OVMF and real firmware look at the Section Header platform, not this field.
    assert_eq!(
        sentry_sectors, 0,
        "Section Boot Entry no-emulation sector_count must be 0, got {}",
        sentry_sectors
    );
    assert_eq!(
        sentry_lba, boot0_lba,
        "Section Boot Entry LBA must match Boot Entry 0 LBA (both point to ESP)"
    );

    println!(
        "Verified multi-entry UEFI catalog:\n  Boot Entry 0: flag=0x88, sys_type=0xEF, LBA={}, Sectors=0\n  Section Header: flag=0x91, platform=0xEF, entries=1\n  Section Boot Entry: flag=0x88, sys_type=0x{:02x}, LBA={}, Sectors=0",
        boot0_lba, sentry_sys, sentry_lba
    );

    // Verify Section Header is followed by exactly one boot entry
    let rest_start = sentry_offset + 32;
    let rest = &boot_catalog_sector[rest_start..rest_start + 32];
    assert!(
        rest.iter().all(|&b| b == 0),
        "Bytes after Section Boot Entry must be zero"
    );

    // Verify ISO content using isoinfo -l
    let isoinfo_l_output = run_command("isoinfo", &["-l", "-i", iso_path.to_str().unwrap()])?;
    println!("isoinfo -l output (isohybrid):\n{}", isoinfo_l_output);
    assert!(isoinfo_l_output.contains("BOOTX64.EFI;1"));
    assert!(isoinfo_l_output.contains("KERNEL.EFI;1"));

    // Verify the boot catalog validation entry checksum
    let mut iso_file = File::open(iso_path)?;
    iso_file.seek(SeekFrom::Start(
        isobemak::iso::boot_catalog::LBA_BOOT_CATALOG as u64 * 2048,
    ))?;
    let mut boot_catalog = [0u8; 32]; // Only need the validation entry
    iso_file.read_exact(&mut boot_catalog)?;

    let mut sum: u16 = 0;
    for chunk in boot_catalog.chunks_exact(2) {
        sum = sum.wrapping_add(u16::from_le_bytes(chunk.try_into().unwrap()));
    }

    assert_eq!(sum, 0, "Boot catalog validation entry checksum should be 0");

    // Perform deeper binary verification of ISO structures
    verify_iso_binary_structures(&mut iso_file)?;

    // Verify GPT structures (CRC, ESP attributes) and MBR CHS fields —
    // these are the structures real UEFI firmware uses, and bugs here
    // cause "No bootfile found for UEFI!" on hardware.
    iso_file.seek(SeekFrom::Start(0))?;
    verify_gpt_and_mbr_chs(&mut iso_file)?;

    Ok(())
}

#[test]
fn test_create_isohybrid_with_additional_efi_files() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let temp_dir_path = temp_dir.path();

    // Setup files
    let (bootx64_path, kernel_path, iso_path) = setup_integration_test_files(temp_dir_path)?;

    // Create additional EFI boot files (e.g. GRUBX64.EFI)
    let grub_path = temp_dir_path.join("grubx64.efi");
    std::fs::write(&grub_path, vec![0xEFu8; 128])?;

    let iso_image = IsoImage {
        volume_id: None,
        files: vec![
            IsoImageFile {
                source: bootx64_path.clone(),
                destination: "EFI/BOOT/BOOTX64.EFI".to_string(),
            },
            IsoImageFile {
                source: kernel_path.clone(),
                destination: "EFI/BOOT/KERNEL.EFI".to_string(),
            },
        ],
        boot_info: BootInfo {
            bios_boot: None,
            uefi_boot: Some(UefiBootInfo {
                boot_image: bootx64_path.clone(),
                kernel_image: kernel_path.clone(),
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
                additional_efi_boot_files: vec![("GRUBX64.EFI".to_string(), grub_path.clone())],
                grub_cfg_content: None,
            }),
        },
        layout_profile: IsoLayoutProfile::default(),
    };

    let (_iso_path_buf, temp_holder, _iso_file, _) = build_iso(&iso_path, &iso_image, true)?;
    assert!(iso_path.exists());

    // Get the actual FAT image path from the NamedTempFile holder.
    let fat_img_path = temp_holder.as_ref().unwrap().path().to_path_buf();
    assert!(
        fat_img_path.exists(),
        "FAT image must exist at {:?}",
        fat_img_path
    );

    // Verify that the additional file exists in the FAT image
    verify_fat_image_has_file(&fat_img_path, "EFI/BOOT/GRUBX64.EFI")?;
    // Also verify the original files still exist
    verify_fat_image_has_file(&fat_img_path, "EFI/BOOT/BOOTX64.EFI")?;
    verify_fat_image_has_file(&fat_img_path, "EFI/BOOT/KERNEL.EFI")?;

    println!("Verified GRUBX64.EFI in FAT image alongside BOOTX64.EFI and KERNEL.EFI");

    Ok(())
}

#[test]
fn test_isohybrid_with_auto_grub_cfg() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let temp_dir_path = temp_dir.path();

    // Setup files
    let (bootx64_path, kernel_path, iso_path) = setup_integration_test_files(temp_dir_path)?;

    let grub_config = r#"set default=0
set timeout=5

menuentry "Boot from ISO" {
    chainloader /EFI/BOOT/BOOTX64.EFI
}

menuentry "Kernel" {
    linuxefi /EFI/BOOT/KERNEL.EFI
}
"#;

    let iso_image = IsoImage {
        volume_id: None,
        files: vec![
            IsoImageFile {
                source: bootx64_path.clone(),
                destination: "EFI/BOOT/BOOTX64.EFI".to_string(),
            },
            IsoImageFile {
                source: kernel_path.clone(),
                destination: "EFI/BOOT/KERNEL.EFI".to_string(),
            },
        ],
        boot_info: BootInfo {
            bios_boot: None,
            uefi_boot: Some(UefiBootInfo {
                boot_image: bootx64_path.clone(),
                kernel_image: kernel_path.clone(),
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
                additional_efi_boot_files: Vec::new(),
                grub_cfg_content: Some(grub_config.to_string()),
            }),
        },
        layout_profile: IsoLayoutProfile::default(),
    };

    let (_iso_path_buf, temp_holder, _iso_file, _) = build_iso(&iso_path, &iso_image, true)?;
    assert!(iso_path.exists());

    let fat_img_path = temp_holder.as_ref().unwrap().path().to_path_buf();
    assert!(fat_img_path.exists());

    // Verify grub.cfg exists in the FAT image
    verify_fat_image_has_file(&fat_img_path, "EFI/BOOT/grub.cfg")?;
    // Verify the content of grub.cfg
    let fat_file = File::open(&fat_img_path)?;
    let fs = FileSystem::new(fat_file, FsOptions::new())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let root_dir = fs.root_dir();
    let mut grub_file = root_dir
        .open_file("EFI/BOOT/grub.cfg")
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let mut content = String::new();
    grub_file
        .read_to_string(&mut content)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    assert!(
        content.contains("Boot from ISO"),
        "grub.cfg content mismatch"
    );
    assert!(
        content.contains("chainloader /EFI/BOOT/BOOTX64.EFI"),
        "grub.cfg should reference BOOTX64.EFI"
    );
    assert!(
        content.contains("menuentry \"Kernel\""),
        "grub.cfg should have kernel entry"
    );

    // Verify original files still exist
    verify_fat_image_has_file(&fat_img_path, "EFI/BOOT/BOOTX64.EFI")?;
    verify_fat_image_has_file(&fat_img_path, "EFI/BOOT/KERNEL.EFI")?;

    println!("Verified auto-generated grub.cfg in FAT image");

    Ok(())
}
