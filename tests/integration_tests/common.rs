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
