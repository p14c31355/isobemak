use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

use fatfs::{FileSystem, FsOptions};
use isobemak::{BootInfo, IsoImage, IsoImageFile, UefiBootInfo, build_iso};
use tempfile::tempdir;

use crate::integration_tests::common::{
    run_command, setup_integration_test_files, verify_iso_binary_structures,
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
    };

    // Call the main function with is_isohybrid set to true
    let (fat_image_path, _temp_fat_file_holder, _iso_file, logical_fat_size_512_sectors) =
        build_iso(&iso_path, &iso_image, true)?;
    assert!(iso_path.exists());
    assert!(fat_image_path.exists());
    // _iso_file is kept in scope to ensure the ISO file remains open for verification
    // _temp_fat_file_holder is kept in scope to prevent the temporary file from being deleted

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

    // Verify Validation Entry's platform ID
    assert_eq!(
        boot_catalog_sector[1],
        isobemak::iso::boot_catalog::BOOT_CATALOG_EFI_PLATFORM_ID,
        "Validation entry platform ID is not EFI"
    );

    // Verify the first Boot Entry (which should be the UEFI one in this test case)
    let boot_entry_offset = 32; // After the 32-byte validation entry
    let boot_entry_bytes = &boot_catalog_sector[boot_entry_offset..boot_entry_offset + 32];

    let uefi_boot_indicator = boot_entry_bytes[0];
    let uefi_boot_sectors = u16::from_le_bytes(boot_entry_bytes[6..8].try_into().unwrap());
    let uefi_boot_lba = u32::from_le_bytes(boot_entry_bytes[8..12].try_into().unwrap());

    assert_eq!(
        uefi_boot_indicator,
        isobemak::iso::boot_catalog::BOOT_CATALOG_BOOT_ENTRY_HEADER_ID,
        "UEFI boot entry is not marked bootable"
    );
    // El Torito boot catalog on CD-ROM media uses 2048-byte ISO sector LBA values.
    assert_eq!(
        uefi_boot_lba,
        isobemak::ESP_START_LBA,
        "UEFI boot LBA in boot catalog is incorrect"
    );

    // The expected number of sectors in the boot catalog is the logical FAT size in 512-byte sectors,
    // rounded up to the nearest multiple of 4 (to match the ISO sector alignment of the ESP).
    let expected_esp_sectors = logical_fat_size_512_sectors.unwrap().div_ceil(4) * 4;

    assert_eq!(
        uefi_boot_sectors, expected_esp_sectors as u16,
        "UEFI boot sectors in boot catalog is incorrect"
    );
    println!(
        "Verified UEFI boot entry: LBA={} (expected: {}), Sectors={} (expected: {})",
        uefi_boot_lba,
        isobemak::ESP_START_LBA * 4,
        uefi_boot_sectors,
        expected_esp_sectors
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
