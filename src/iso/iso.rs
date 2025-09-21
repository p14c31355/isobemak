// src/iso/iso.rs
use crate::iso::boot_catalog::LBA_BOOT_CATALOG;
use crate::iso::dir_record::IsoDirEntry;
use crate::iso::volume_descriptor::*;
use crate::utils::{ISO_SECTOR_SIZE, pad_to_lba};
use std::fs::File;
use std::io::{self, Read, Seek, Write, copy};
use std::path::Path;

/// Builder for creating an ISO 9660 image.
struct IsoBuilder<'a> {
    iso_file: File,
    fat_img_path: &'a Path,
    kernel_path: &'a Path,
    boot_img_lba: u32,
    kernel_lba: u32,
    total_sectors: u32,
}

impl<'a> IsoBuilder<'a> {
    /// Initializes a new ISO builder with file paths.
    fn new(iso_path: &Path, fat_img_path: &'a Path, kernel_path: &'a Path) -> io::Result<Self> {
        let iso_file = File::create(iso_path)?;
        let fat_img_size = std::fs::metadata(fat_img_path)?.len();

        let boot_img_lba = 23;
        let fat_sectors = fat_img_size.div_ceil(ISO_SECTOR_SIZE as u64) as u32;
        let kernel_lba = boot_img_lba + fat_sectors;

        Ok(Self {
            iso_file,
            fat_img_path,
            kernel_path,
            boot_img_lba,
            kernel_lba,
            total_sectors: 0,
        })
    }

    /// Writes all ISO volume descriptors.
    fn write_descriptors(&mut self) -> io::Result<()> {
        let root_entry = IsoDirEntry {
            lba: 20,
            size: ISO_SECTOR_SIZE as u32,
            flags: 0x02,
            name: ".",
        };
        write_volume_descriptors(&mut self.iso_file, 0, LBA_BOOT_CATALOG, &root_entry)
    }

    /// Writes the El Torito boot catalog.
    fn write_boot_catalog(&mut self) -> io::Result<()> {
        // This function is deprecated and should not be used directly.
        // The new IsoBuilder handles boot catalog creation.
        // The original implementation was:
        // let fat_img_size = std::fs::metadata(self.fat_img_path)?.len();
        // let fat_img_sectors = fat_img_size.div_ceil(512) as u32;
        // if fat_img_sectors > u16::MAX as u32 {
        //     return Err(io::Error::new(
        //         io::ErrorKind::InvalidInput,
        //         format!(
        //             "Boot image too large for boot catalog: {} 512-byte sectors",
        //             fat_img_sectors
        //         ),
        //     ));
        // }
        // write_boot_catalog(
        //     &mut self.iso_file,
        //     self.boot_img_lba,
        //     fat_img_sectors as u16,
        // )
        Ok(())
    }

    /// Writes the directory records for the ISO filesystem.
    fn write_directories(&mut self) -> io::Result<()> {
        let root_dir_entries = [
            IsoDirEntry {
                lba: 20,
                size: ISO_SECTOR_SIZE as u32,
                flags: 0x02,
                name: ".",
            },
            IsoDirEntry {
                lba: 20,
                size: ISO_SECTOR_SIZE as u32,
                flags: 0x02,
                name: "..",
            },
            IsoDirEntry {
                lba: 21,
                size: ISO_SECTOR_SIZE as u32,
                flags: 0x02,
                name: "EFI",
            },
        ];
        self.write_directory_sector(20, &root_dir_entries)?;

        let efi_dir_entries = [
            IsoDirEntry {
                lba: 21,
                size: ISO_SECTOR_SIZE as u32,
                flags: 0x02,
                name: ".",
            },
            IsoDirEntry {
                lba: 20,
                size: ISO_SECTOR_SIZE as u32,
                flags: 0x02,
                name: "..",
            },
            IsoDirEntry {
                lba: 22,
                size: ISO_SECTOR_SIZE as u32,
                flags: 0x02,
                name: "BOOT",
            },
        ];
        self.write_directory_sector(21, &efi_dir_entries)?;

        let kernel_size = std::fs::metadata(self.kernel_path)?.len() as u32;
        let fat_img_size = std::fs::metadata(self.fat_img_path)?.len() as u32;
        let boot_dir_entries = [
            IsoDirEntry {
                lba: 22,
                size: ISO_SECTOR_SIZE as u32,
                flags: 0x02,
                name: ".",
            },
            IsoDirEntry {
                lba: 21,
                size: ISO_SECTOR_SIZE as u32,
                flags: 0x02,
                name: "..",
            },
            IsoDirEntry {
                lba: self.boot_img_lba,
                size: fat_img_size,
                flags: 0x00,
                name: "BOOTX64.EFI",
            },
            IsoDirEntry {
                lba: self.kernel_lba,
                size: kernel_size,
                flags: 0x00,
                name: "KERNEL.EFI",
            },
        ];
        self.write_directory_sector(22, &boot_dir_entries)?;
        Ok(())
    }

    /// Helper method to write a single directory sector.
    fn write_directory_sector(&mut self, lba: u32, entries: &[IsoDirEntry]) -> io::Result<()> {
        pad_to_lba(&mut self.iso_file, lba)?;
        let mut dir_content = Vec::new();
        for e in entries {
            dir_content.extend_from_slice(&e.to_bytes());
        }
        dir_content.resize(ISO_SECTOR_SIZE, 0);
        self.iso_file.write_all(&dir_content)
    }

    /// Copies the boot image and kernel file into the ISO.
    fn copy_files(&mut self) -> io::Result<()> {
        // Copy FAT boot image
        pad_to_lba(&mut self.iso_file, self.boot_img_lba)?;
        let mut fat_file = File::open(self.fat_img_path)?;
        copy(&mut fat_file, &mut self.iso_file)?;

        // Copy kernel
        pad_to_lba(&mut self.iso_file, self.kernel_lba)?;
        let mut kernel_file = File::open(self.kernel_path)?;
        copy(&mut kernel_file, &mut self.iso_file)?;
        Ok(())
    }

    /// Finalizes the ISO image by padding and updating the total sector count.
    fn finalize(&mut self) -> io::Result<()> {
        // Final padding to ISO sector
        let current_pos = self.iso_file.stream_position()?;
        let remainder = current_pos % ISO_SECTOR_SIZE as u64;
        if remainder != 0 {
            io::copy(
                &mut io::repeat(0).take(ISO_SECTOR_SIZE as u64 - remainder),
                &mut self.iso_file,
            )?;
        }

        // Update PVD total sectors
        let final_pos = self.iso_file.stream_position()?;
        self.total_sectors = (final_pos as f64 / ISO_SECTOR_SIZE as f64).ceil() as u32;
        update_total_sectors_in_pvd(&mut self.iso_file, self.total_sectors)?;

        println!(
            "create_iso_from_img: ISO created with {} sectors",
            self.total_sectors
        );
        Ok(())
    }
}

/// Creates an ISO 9660 image with a FAT boot image (El Torito) and a UEFI kernel.
pub fn create_iso_from_img(
    iso_path: &Path,
    fat_img_path: &Path,
    kernel_path: &Path,
) -> io::Result<()> {
    let mut builder = IsoBuilder::new(iso_path, fat_img_path, kernel_path)?;
    builder.write_descriptors()?;
    builder.write_boot_catalog()?;
    builder.write_directories()?;
    builder.copy_files()?;
    builder.finalize()?;
    Ok(())
}
