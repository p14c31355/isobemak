// Simulates Choosable's ISO9660 parser logic to verify that an ISO
// produced by isobemak can be correctly parsed via the same
// algorithm that Choosable uses at boot time.
//
// This catches regressions that would cause the Choosable menu to
// return after selecting an ISO (i.e. PVD not found, directory walk
// failure, etc.).

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
};

use isobemak::{BootInfo, IsoImage, IsoImageFile, IsoLayoutProfile, UefiBootInfo, build_iso};
use tempfile::tempdir;

use crate::integration_tests::common::setup_integration_test_files;

const ISO_SECTOR_SIZE: usize = 2048;

/// Read a 2048-byte ISO sector from a flat file (4 × 512 byte slices).
fn read_file_iso_sector(file: &mut File, iso_sector: u64) -> io::Result<[u8; ISO_SECTOR_SIZE]> {
    let mut buf = [0u8; ISO_SECTOR_SIZE];
    file.seek(SeekFrom::Start(iso_sector * ISO_SECTOR_SIZE as u64))?;
    file.read_exact(&mut buf)?;
    Ok(buf)
}

/// ISO9660 directory record parser (matches Choosable's logic)
fn find_in_dir_flat(
    file: &mut File,
    dir_lba: u32,
    dir_size: u32,
    name: &[u8],
    scratch: &mut [u8; ISO_SECTOR_SIZE],
) -> Option<(u32, u32)> {
    let total_sectors = ((dir_size as u64 + 2047) / 2048) as u32;
    for s in 0..total_sectors {
        *scratch = read_file_iso_sector(file, (dir_lba + s) as u64).ok()?;
        let mut offset: usize = 0;
        while offset + 34 <= ISO_SECTOR_SIZE {
            let record_len = scratch[offset] as usize;
            if record_len == 0 {
                break;
            }
            if offset + record_len > ISO_SECTOR_SIZE {
                break;
            }
            let name_len = scratch[offset + 32] as usize;
            let name_offset = offset + 33;
            if name_offset + name_len > ISO_SECTOR_SIZE {
                break;
            }
            let effective_len = if name_len >= 2 && scratch[name_offset + name_len - 2] == b';' {
                name_len - 2
            } else {
                name_len
            };
            if effective_len == name.len() {
                let mut matched = true;
                for i in 0..name.len() {
                    let a = scratch[name_offset + i].to_ascii_uppercase();
                    let b = name[i].to_ascii_uppercase();
                    if a != b {
                        matched = false;
                        break;
                    }
                }
                if matched {
                    let child_extent =
                        u32::from_le_bytes(scratch[offset + 2..offset + 6].try_into().unwrap());
                    let child_size =
                        u32::from_le_bytes(scratch[offset + 10..offset + 14].try_into().unwrap());
                    return Some((child_extent, child_size));
                }
            }
            offset += record_len;
        }
    }
    None
}

/// Full Choosable-style path resolver: PVD → root → /EFI/BOOT/BOOTX64.EFI
fn resolve_efi_boot_flat(file: &mut File) -> io::Result<(u32, u32)> {
    let pvd = read_file_iso_sector(file, 16)?;
    if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid PVD signature",
        ));
    }
    let root_lba = u32::from_le_bytes(pvd[158..162].try_into().unwrap());
    let root_size = u32::from_le_bytes(pvd[166..170].try_into().unwrap());
    let mut scratch = [0u8; ISO_SECTOR_SIZE];

    let (efi_lba, efi_size) = find_in_dir_flat(file, root_lba, root_size, b"EFI", &mut scratch)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "/EFI not found"))?;
    let (boot_lba, boot_size) =
        find_in_dir_flat(file, efi_lba, efi_size, b"BOOT", &mut scratch).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "/EFI/BOOT not found")
        })?;
    find_in_dir_flat(file, boot_lba, boot_size, b"BOOTX64.EFI", &mut scratch).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "/EFI/BOOT/BOOTX64.EFI not found",
        )
    })
}

#[test]
fn test_choosable_can_resolve_efi_boot_in_isohybrid_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let temp_dir_path = temp_dir.path();

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
        layout_profile: IsoLayoutProfile::hardware(),
    };

    // Build isohybrid ISO (same settings as fullerene)
    {
        let (_iso_path_buf, _temp_holder, _iso_file, _) =
            build_iso(&iso_path, &iso_image, true)?;
    }
    assert!(iso_path.exists());

    let mut file = File::open(&iso_path)?;

    // Simulate Choosable's PVD→/EFI/BOOT/BOOTX64.EFI resolution
    let (efi_lba, efi_size) = resolve_efi_boot_flat(&mut file).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!(
                "Choosable would fail to boot this ISO: {e}. \
                 (PVD→/EFI/BOOT/BOOTX64.EFI resolution failed)"
            ),
        )
    })?;

    assert!(efi_lba > 0, "BOOTX64.EFI LBA must be non-zero");
    assert_eq!(
        efi_size, 64 * 1024,
        "BOOTX64.EFI size must match test fixture"
    );

    // Also verify that we can read the file content
    let sector_count = ((efi_size as u64 + 2047) / 2048) as u32;
    let mut content = vec![0u8; sector_count as usize * ISO_SECTOR_SIZE];
    for s in 0..sector_count {
        let sec = read_file_iso_sector(&mut file, (efi_lba + s) as u64)?;
        content[s as usize * ISO_SECTOR_SIZE..(s as usize + 1) * ISO_SECTOR_SIZE]
            .copy_from_slice(&sec);
    }
    assert_eq!(&content[..efi_size as usize], vec![0u8; 64 * 1024].as_slice());

    println!(
        "Choosable simulation PASSED: resolved /EFI/BOOT/BOOTX64.EFI at LBA {}, size {}",
        efi_lba, efi_size
    );

    Ok(())
}

#[test]
fn test_choosable_can_resolve_efi_boot_in_flat_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let temp_dir_path = temp_dir.path();

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
        layout_profile: IsoLayoutProfile::hardware(),
    };

    // Build flat (non-isohybrid) ISO
    {
        let (_iso_path_buf, _temp_holder, _iso_file, _) =
            build_iso(&iso_path, &iso_image, false)?;
    }
    assert!(iso_path.exists());

    let mut file = File::open(&iso_path)?;
    let (efi_lba, efi_size) = resolve_efi_boot_flat(&mut file).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!(
                "Choosable would fail to boot this ISO: {e}. \
                 (PVD→/EFI/BOOT/BOOTX64.EFI resolution failed)"
            ),
        )
    })?;

    assert!(efi_lba > 0);
    assert_eq!(efi_size, 64 * 1024);

    println!(
        "Choosable simulation (flat) PASSED: resolved /EFI/BOOT/BOOTX64.EFI at LBA {}, size {}",
        efi_lba, efi_size
    );

    Ok(())
}