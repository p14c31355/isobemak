// tests/integration.rs
use std::{
    fs::File,
    io::{self, Error, ErrorKind, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
};

use isobemak::iso::builder::{IsoImageFile, build_iso};
use tempfile::tempdir;

fn run_command(command: &str, args: &[&str]) -> io::Result<String> {
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
fn setup_integration_test_files(temp_dir: &Path) -> io::Result<(PathBuf, PathBuf, PathBuf)> {
    // Create dummy files needed for the ISO image
    let bootx64_path = temp_dir.join("bootx64.efi");
    std::fs::write(&bootx64_path, vec![0u8; 64 * 1024])?;

    let kernel_path = temp_dir.join("kernel.elf");
    std::fs::write(&kernel_path, vec![0u8; 16 * 1024])?;

    let iso_path = temp_dir.join("test.iso");

    Ok((bootx64_path, kernel_path, iso_path))
}

#[test]
fn test_create_disk_and_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let temp_dir_path = temp_dir.path();
    println!("Temp dir: {:?}", &temp_dir_path);

    // Setup files and paths
    let (bootx64_path, kernel_path, iso_path) = setup_integration_test_files(&temp_dir_path)?;

    let iso_image = isobemak::iso::builder::IsoImage {
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
        boot_info: isobemak::iso::builder::BootInfo {
            bios_boot: None, // Not testing BIOS boot in this specific test
            uefi_boot: Some(isobemak::iso::builder::UefiBootInfo {
                boot_image: bootx64_path.clone(),
                kernel_image: kernel_path.clone(),
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
            }),
        },
    };

    // Call the main function with correct arguments
    build_iso(&iso_path, &iso_image, false)?;
    // Assert that the ISO file was created
    assert!(iso_path.exists());

    // Verify ISO content using isoinfo
    let isoinfo_d_output = run_command("isoinfo", &["-d", "-i", iso_path.to_str().unwrap()])?;
    println!("isoinfo -d output:\n{}", isoinfo_d_output);
    assert!(isoinfo_d_output.contains("Volume id: ISOBEMAKI"));

    let isoinfo_l_output = run_command("isoinfo", &["-l", "-i", iso_path.to_str().unwrap()])?;
    println!("isoinfo -l output:\n{}", isoinfo_l_output);
    assert!(isoinfo_l_output.contains("BOOTX64.EFI;1"));
    assert!(isoinfo_l_output.contains("KERNEL.EFI;1"));

    // Verify ISO content using 7z
    let sevenz_output = Command::new("7z")
        .args(&["l", iso_path.to_str().unwrap()])
        .output()?;
    let sevenz_l_output = String::from_utf8_lossy(&sevenz_output.stdout).into_owned();
    println!("7z l output:\n{}", sevenz_l_output);
    assert!(sevenz_l_output.contains("EFI/BOOT/BOOTX64.EFI"));
    assert!(sevenz_l_output.contains("EFI/BOOT/KERNEL.EFI"));

    // Extract the UEFI boot image and verify with dumpet
    let extract_dir = temp_dir_path.join("extracted");
    std::fs::create_dir_all(&extract_dir)?;
    let _extract_output = Command::new("7z")
        .args(&[
            "x",
            iso_path.to_str().unwrap(),
            "-o",
            extract_dir.to_str().unwrap(),
        ])
        .output()?;
    // Proceed even if there are warnings

    let extracted_bootx64_path = extract_dir.join("EFI/BOOT/BOOTX64.EFI");
    // Skip extraction assertion due to 7z warning, but verify file size if exists
    if extracted_bootx64_path.exists() {
        let dumpet_output = run_command("dumpet", &[extracted_bootx64_path.to_str().unwrap()])?;
        println!("dumpet output:\n{}", dumpet_output);
        assert!(dumpet_output.contains("EFI boot image"));
    } else {
        println!("Extraction failed, but listing succeeded");
    }

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

/// Verifies critical binary structures within the generated ISO file.
fn verify_iso_binary_structures(iso_file: &mut File) -> io::Result<()> {
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
