// src/iso/disk_layout.rs
//! Disk-centric layout model for isohybrid ISO images.
//!
//! Instead of treating the ESP as an ISO9660-embedded blob ("ISO-centric"),
//! this module models the disk as:
//!
//! ```text
//! Disk
//!  ├─ MBR/GPT
//!  ├─ ESP partition (real FAT partition, not an ISO object)
//!  └─ ISO9660 region
//!       └─ El Torito (direct EFI binary, not FAT image)
//! ```
//!
//! This matches how real UEFI firmware sees a USB drive and is the
//! approach used by xorriso (`-append_partition`).

/// A physical disk partition (e.g., EFI System Partition).
///
/// This is a **real** partition in MBR/GPT, not an ISO9660 filesystem object.
/// Firmware (especially on real hardware like NEC/Insyde) looks for this
/// when the medium is treated as USB-HDD.
#[derive(Debug, Clone)]
pub struct Partition {
    /// Partition start LBA in 512-byte sectors.
    pub start_lba_512: u64,
    /// Partition size in 512-byte sectors.
    pub size_lba_512: u64,
}

/// The ISO9660 filesystem region within the disk layout.
///
/// Contains volume descriptors, El Torito boot catalog, directory records,
/// and file data. The El Torito catalog references the EFI binary directly
/// (not a FAT image), for QEMU/OVMF CD-ROM emulation.
#[derive(Debug, Clone)]
pub struct IsoRegion {
    /// LBA (in 2048-byte ISO sectors) where ISO9660 file data begins
    /// (after volume descriptors and boot catalog).
    pub data_start_lba: u32,
    /// Total ISO9660 sectors (filled in during build).
    pub total_sectors: u32,
}

/// Complete disk layout: partitions + ISO9660 region.
///
/// This is the central model that replaces the old "ISO-with-embedded-ESP"
/// approach. The key insight: ESP is a **disk partition**, not an ISO object.
#[derive(Debug, Clone)]
pub struct DiskLayout {
    /// Disk partitions (e.g., ESP). In MBR/GPT order.
    pub partitions: Vec<Partition>,
    /// ISO9660 filesystem region.
    pub iso_region: IsoRegion,
}

/// How UEFI boot is exposed to firmware.
///
/// The fundamental split recognized by xorriso and now isobemak:
///
/// - **El Torito direct EFI**: QEMU/OVMF boots via CD-ROM emulation,
///   reading the EFI binary directly from the ISO9660 filesystem through
///   the El Torito boot catalog.
///
/// - **ESP partition**: Real USB hardware (NEC, Insyde, old AMI, Lenovo,
///   Panasonic) ignores El Torito and boots via MBR→GPT→ESP, looking for
///   a real FAT partition with `EFI/BOOT/BOOTX64.EFI`.
///
/// Both strategies coexist in a hybrid ISO: El Torito for emulators,
/// ESP partition for real hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UefiBootStrategy {
    /// Expose EFI binary directly via El Torito boot catalog entry.
    /// Primary path for QEMU/OVMF which boots via CD-ROM emulation.
    ElToritoDirectEfi,
    /// Expose via a real ESP partition in MBR/GPT.
    /// Primary path for real USB-HDD hardware.
    EspPartition,
}

impl DiskLayout {
    /// Create a disk layout from partition placement parameters.
    ///
    /// `esp_alignment_lba_512`: where the ESP starts (in 512-byte sectors).
    /// `esp_size_512`: ESP size in 512-byte sectors (None if no ESP).
    /// `iso_data_start_lba`: ISO9660 data start in 2048-byte sectors.
    pub fn from_partition_params(
        esp_alignment_lba_512: u32,
        esp_size_512: Option<u32>,
        iso_data_start_lba: u32,
    ) -> Self {
        let mut partitions = Vec::new();
        if let Some(size) = esp_size_512 {
            if size > 0 {
                partitions.push(Partition {
                    start_lba_512: esp_alignment_lba_512 as u64,
                    size_lba_512: size as u64,
                });
            }
        }
        Self {
            partitions,
            iso_region: IsoRegion {
                data_start_lba: iso_data_start_lba,
                total_sectors: 0, // filled in during finalize_iso
            },
        }
    }

    /// Returns the EFI System Partition if present.
    pub fn esp_partition(&self) -> Option<&Partition> {
        self.partitions.first()
    }

    /// Returns true if the disk has an ESP partition for hardware UEFI boot.
    pub fn has_esp(&self) -> bool {
        self.esp_partition().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disk_layout_with_esp() {
        let layout = DiskLayout::from_partition_params(2048, Some(32768), 24);
        assert!(layout.has_esp());
        let esp = layout.esp_partition().unwrap();
        assert_eq!(esp.start_lba_512, 2048);
        assert_eq!(esp.size_lba_512, 32768);
        assert_eq!(layout.iso_region.data_start_lba, 24);
    }

    #[test]
    fn test_disk_layout_without_esp() {
        let layout = DiskLayout::from_partition_params(0, None, 20);
        assert!(!layout.has_esp());
        assert!(layout.esp_partition().is_none());
    }

    #[test]
    fn test_disk_layout_empty_esp() {
        let layout = DiskLayout::from_partition_params(2048, Some(0), 24);
        assert!(!layout.has_esp());
    }
}