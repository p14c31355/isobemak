use crate::iso::disk_layout::UefiBootStrategy;

/// ISO layout profile for firmware compatibility.
///
/// Separates "UEFI spec compliant" settings from "broken firmware workaround" settings,
/// following the xorriso/GRUB approach of dual-boot-path and per-firmware tuning.
#[derive(Debug, Clone)]
pub struct IsoLayoutProfile {
    /// Whether to write GPT header and partition entries.
    /// - On: QEMU/OVMF, modern UEFI firmware
    /// - Off: NEC/Insyde/old AMI that fail when GPT is present (xorriso MBR-only hybrid)
    pub use_gpt: bool,

    /// El Torito boot catalog UEFI entry configuration.
    pub eltorito_mode: ElToritoMode,

    /// EFI System Partition placement strategy.
    pub esp_mode: EspMode,

    /// ESP start LBA in 512-byte sector units (alignment).
    /// UEFI spec recommends 1 MiB (2048), some firmware expects 2 MiB (4096).
    pub esp_alignment_lba_512: u32,

    /// MBR partition table layout.
    pub mbr_mode: MbrMode,

    /// FAT BPB hidden_sectors policy.
    pub hidden_sectors_mode: HiddenSectorMode,

    /// UEFI boot strategy: El Torito direct EFI vs. ESP partition.
    /// Determines whether the primary boot path is via CD-ROM emulation
    /// (QEMU/OVMF) or via a real disk partition (USB-HDD hardware).
    pub uefi_boot_strategy: UefiBootStrategy,
}

/// How the El Torito catalog exposes the UEFI boot target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElToritoMode {
    /// Two entries: Entry 0 (bootable) → direct EFI binary in ISO9660,
    ///              Entry 1 (non-bootable) → ESP FAT image.
    /// Matches xorriso `-e EFI/BOOT/BOOTX64.EFI -append_partition 2 0xef esp.img`.
    Both,

    /// Single entry: Entry 0 (bootable) → direct EFI binary only.
    /// ESP FAT image is still present for USB-HDD boot path but not referenced from El Torito.
    DirectEfiOnly,
}

/// EFI System Partition placement strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EspMode {
    /// ESP is written inside the ISO image at a fixed offset,
    /// referenced by MBR partition entry (and optionally GPT).
    /// This is the standard isohybrid layout used by xorriso.
    AppendedPartition,
}

/// MBR partition table layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MbrMode {
    /// Two partitions: Entry 0 = type 0x83 (Linux/ISO9660, covers whole disk),
    ///                 Entry 1 = type 0xEF (EFI System Partition).
    /// Matches xorriso MBR-only hybrid layout.
    HybridLinuxEsp,
}

/// FAT BPB hidden_sectors policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiddenSectorMode {
    /// hidden_sectors = 0.
    /// QEMU/OVMF passes even with zero; avoids confusing firmware that rejects
    /// FAT images with non-zero hidden sectors inside an El Torito CD context.
    Zero,
    /// hidden_sectors = ESP partition start LBA (in 512-byte sectors).
    /// Required by NEC/Insyde/old AMI firmware for FAT geometry validation
    /// when the medium is treated as USB-HDD.
    PartitionOffset,
}

impl Default for IsoLayoutProfile {
    fn default() -> Self {
        Self::hardware()
    }
}

impl IsoLayoutProfile {
    /// QEMU/OVMF / modern UEFI firmware profile.
    ///
    /// - GPT enabled (UEFI spec compliant)
    /// - El Torito: both direct EFI binary and ESP image entries
    /// - ESP at 1 MiB alignment (2048 512-byte sectors)
    /// - MBR: hybrid Linux+ESP
    /// - hidden_sectors: partition offset
    /// - UEFI boot: El Torito direct EFI (primary for CD-ROM emulation)
    pub fn emulator() -> Self {
        Self {
            use_gpt: true,
            eltorito_mode: ElToritoMode::Both,
            esp_mode: EspMode::AppendedPartition,
            esp_alignment_lba_512: 2048, // 1 MiB
            mbr_mode: MbrMode::HybridLinuxEsp,
            hidden_sectors_mode: HiddenSectorMode::PartitionOffset,
            uefi_boot_strategy: UefiBootStrategy::ElToritoDirectEfi,
        }
    }

    /// Real hardware profile (NEC, Insyde, old AMI, Lenovo, Panasonic).
    ///   - GPT enabled (protective MBR + GPT, matching xorriso)
    ///   - ESP at 1 MiB alignment (2048 512-byte sectors)
    ///   - El Torito: BIOS + UEFI entries
    ///   - FAT BPB: heads=64, spt=32, hidden_sectors=0
    ///   - UEFI boot: ESP partition (primary for USB-HDD hardware path)
    pub fn hardware() -> Self {
        Self {
            use_gpt: true,
            eltorito_mode: ElToritoMode::Both,
            esp_mode: EspMode::AppendedPartition,
            esp_alignment_lba_512: 2048, // 1 MiB (Redox OS compatible)
            mbr_mode: MbrMode::HybridLinuxEsp,
            // PartitionOffset is required for real hardware to correctly
            // locate the FAT volume within the disk layout.
            // Firmware (NEC/Insyde/AMI) uses hidden_sectors to translate
            // between partition-relative and absolute LBA addressing.
            hidden_sectors_mode: HiddenSectorMode::PartitionOffset,
            uefi_boot_strategy: UefiBootStrategy::EspPartition,
        }
    }
}