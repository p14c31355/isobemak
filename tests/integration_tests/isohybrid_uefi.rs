use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

use isobemak::{BootInfo, IsoImage, IsoImageFile, UefiBootInfo, build_iso};
use tempfile::tempdir;

use crate::integration_tests::common::{
    run_command, setup_integration_test_files, verify_iso_binary_structures,
};

#[test]
fn test_create_isohybrid_uefi_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let temp_dir_path = temp_dir.path();
    println!("Temp dir for isohybrid UEFI test: {:?}", &temp_dir_path);

    // Setup files and paths
    let (bootx64_path, kernel_path, iso_path) = setup_integration_test_files(&temp_dir_path)?;

    let iso_image = IsoImage {
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
    assert_eq!(
        uefi_boot_lba,
        isobemak::ESP_START_LBA,
        "UEFI boot LBA in boot catalog is incorrect"
    );

    let expected_esp_sectors = logical_fat_size_512_sectors.unwrap() as u16;

    assert_eq!(
        uefi_boot_sectors, expected_esp_sectors,
        "UEFI boot sectors in boot catalog is incorrect"
    );
    println!(
        "Verified UEFI boot entry: LBA={}, Sectors={} (expected: {})",
        uefi_boot_lba, uefi_boot_sectors, expected_esp_sectors
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
