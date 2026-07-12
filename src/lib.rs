//! A library for creating bootable ISO 9660 images with UEFI support.

// Public modules for interacting with the library's core functionalities.
#[macro_use]
pub mod utils;
pub mod fat;
pub mod iso;

// Re-export the main function for external use.
pub use iso::boot_info::{BiosBootInfo, BootInfo, UefiBootInfo};
pub use iso::builder::IsoBuilder;
pub use iso::builder::build_iso;
pub use iso::constants::BACKUP_GPT_RESERVED_512;
pub use iso::constants::DISK_SECTOR_SIZE;
pub use iso::constants::ESP_START_LBA_512;
pub use iso::constants::GPT_RESERVED_512_SECTORS;
pub use iso::constants::ISO_SECTOR_SIZE;
pub use iso::constants::disk512_to_iso;
pub use iso::constants::iso_to_512;
pub use iso::disk_layout::{DiskLayout, IsoRegion, Partition, UefiBootStrategy};
pub use iso::fs_node::{IsoDirectory, IsoFile, IsoFsNode};
pub use iso::iso_image::{IsoImage, IsoImageFile}; // Re-export ESP_START_LBA
pub use iso::layout_profile::{ElToritoMode, EspMode, HiddenSectorMode, IsoLayoutProfile, MbrMode};

#[cfg(test)]
mod tests {
    use super::{
        BiosBootInfo, BootInfo, IsoImage, IsoImageFile, IsoLayoutProfile, UefiBootInfo, build_iso,
    };
    use std::io;
    use std::path::Path;
    use tempfile::tempdir;

    use crate::create_dummy_files;
    /// Helper function to create dummy files and IsoImage for testing.
    fn setup_iso_creation(temp_dir: &Path) -> io::Result<IsoImage> {
        let files = create_dummy_files!(
            temp_dir,
            "isolinux.bin" => 256,
            "isolinux.cfg" => 1,
            "BOOTX64.EFI" => 64,
            "kernel" => 16,
            "initrd.img" => 16
        );

        let isolinux_bin_path = files.get("isolinux.bin").unwrap().clone();
        let isolinux_cfg_path = files.get("isolinux.cfg").unwrap().clone();
        let bootx64_efi_path = files.get("BOOTX64.EFI").unwrap().clone();
        let kernel_path = files.get("kernel").unwrap().clone();
        let initrd_img_path = files.get("initrd.img").unwrap().clone();

        // Create the IsoImage configuration
        let iso_image = IsoImage {
            volume_id: None,
            files: vec![
                IsoImageFile {
                    source: isolinux_cfg_path.clone(),
                    destination: "isolinux/isolinux.cfg".to_string(),
                },
                IsoImageFile {
                    source: kernel_path.clone(),
                    destination: "kernel".to_string(),
                },
                IsoImageFile {
                    source: initrd_img_path.clone(),
                    destination: "initrd.img".to_string(),
                },
                IsoImageFile {
                    source: bootx64_efi_path.clone(),
                    destination: "EFI/BOOT/BOOTX64.EFI".to_string(),
                },
            ],
            boot_info: BootInfo {
                bios_boot: Some(BiosBootInfo {
                    boot_image: isolinux_bin_path.clone(),
                    destination_in_iso: "isolinux/isolinux.bin".to_string(),
                }),
                uefi_boot: Some(UefiBootInfo {
                    boot_image: bootx64_efi_path.clone(),
                    kernel_image: kernel_path.clone(),
                    destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
                    additional_efi_boot_files: Vec::new(),
                    grub_cfg_content: None,
                }),
            },
            layout_profile: IsoLayoutProfile::default(),
        };

        Ok(iso_image)
    }

    #[test]
    fn test_create_custom_iso_example() -> io::Result<()> {
        let temp_dir = tempdir()?;
        let iso_output_path = temp_dir.path().join("custom_boot.iso");

        let iso_image = setup_iso_creation(temp_dir.path())?;

        // Create the ISO
        build_iso(&iso_output_path, &iso_image, true)?;

        // Assert that the ISO file was created and is not empty
        assert!(iso_output_path.exists());
        assert!(iso_output_path.metadata()?.len() > 0);

        // Verify the boot information table in the BIOS boot image
        // by reading the boot catalog (LBA 19) to find the boot image LBA.
        use std::io::{Read, Seek, SeekFrom};
        use crate::iso::constants::ISO_SECTOR_SIZE;
        use crate::iso::boot_catalog::LBA_BOOT_CATALOG;

        let mut iso_file = std::fs::File::open(&iso_output_path)?;
        let mut catalog_sector = [0u8; ISO_SECTOR_SIZE as usize];
        iso_file.seek(SeekFrom::Start(
            LBA_BOOT_CATALOG as u64 * ISO_SECTOR_SIZE as u64,
        ))?;
        iso_file.read_exact(&mut catalog_sector)?;

        // The Initial/Default Entry is at offset 32 in the catalog.
        // Bytes 40..44 (offset 8 within the entry) = boot image LBA (LE u32).
        let boot_image_lba =
            u32::from_le_bytes(catalog_sector[40..44].try_into().unwrap());

        assert!(
            boot_image_lba > 0,
            "boot image LBA must be non-zero, got {boot_image_lba}"
        );

        // Read the boot info table at offset 8 within the boot image's sector.
        let mut table = [0u8; 56];
        iso_file.seek(SeekFrom::Start(
            boot_image_lba as u64 * ISO_SECTOR_SIZE as u64 + 8,
        ))?;
        iso_file.read_exact(&mut table)?;

        // PVD is always at LBA 16.
        assert_eq!(
            u32::from_le_bytes(table[0..4].try_into().unwrap()),
            16,
            "PVD LBA should be 16"
        );
        // Boot image LBA matches what the boot catalog says.
        assert_eq!(
            u32::from_le_bytes(table[4..8].try_into().unwrap()),
            boot_image_lba,
            "boot image LBA mismatch in boot info table"
        );
        // Boot image size is positive.
        let size = u32::from_le_bytes(table[8..12].try_into().unwrap());
        assert!(size > 0, "boot image size must be non-zero");
        // Reserved bytes are zeroed.
        assert_eq!(&table[16..56], &[0u8; 40], "reserved bytes not zero");

        // Verify the checksum: read bytes 64..size from the boot image and sum u32 LE words.
        let boot_image_size = size as u64;
        let mut expected_checksum = 0u32;
        if boot_image_size > 64 {
            let sample_offset =
                boot_image_lba as u64 * ISO_SECTOR_SIZE as u64 + 64;
            let mut buf = vec![0u8; (boot_image_size - 64) as usize];
            iso_file.seek(SeekFrom::Start(sample_offset))?;
            iso_file.read_exact(&mut buf)?;
            for chunk in buf.chunks(4) {
                if chunk.len() == 4 {
                    expected_checksum = expected_checksum.wrapping_add(
                        u32::from_le_bytes(chunk.try_into().unwrap()),
                    );
                }
            }
        }
        let actual_checksum = u32::from_le_bytes(table[12..16].try_into().unwrap());
        assert_eq!(
            actual_checksum, expected_checksum,
            "boot info table checksum mismatch"
        );

        Ok(())
    }
}
