use crate::iso::disk_layout::UefiBootStrategy;

#[derive(Debug, Clone)]
pub struct IsoLayoutProfile {
    pub use_gpt: bool,
    pub eltorito_mode: ElToritoMode,
    pub esp_mode: EspMode,
    pub esp_alignment_lba_512: u32,
    pub mbr_mode: MbrMode,
    pub hidden_sectors_mode: HiddenSectorMode,
    pub uefi_boot_strategy: UefiBootStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElToritoMode {
    Both,
    DirectEfiOnly,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EspMode {
    AppendedPartition,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MbrMode {
    HybridLinuxEsp,
    EspOnly,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiddenSectorMode {
    Zero,
    PartitionOffset,
}

impl Default for IsoLayoutProfile {
    fn default() -> Self {
        Self::hardware()
    }
}

impl IsoLayoutProfile {
    pub fn emulator() -> Self {
        Self {
            use_gpt: true,
            eltorito_mode: ElToritoMode::Both,
            esp_mode: EspMode::AppendedPartition,
            esp_alignment_lba_512: 4096,
            mbr_mode: MbrMode::HybridLinuxEsp,
            hidden_sectors_mode: HiddenSectorMode::PartitionOffset,
            uefi_boot_strategy: UefiBootStrategy::ElToritoDirectEfi,
        }
    }
    pub fn hardware() -> Self {
        Self {
            use_gpt: true,
            eltorito_mode: ElToritoMode::Both,
            esp_mode: EspMode::AppendedPartition,
            esp_alignment_lba_512: 4096,
            mbr_mode: MbrMode::HybridLinuxEsp,
            hidden_sectors_mode: HiddenSectorMode::Zero,
            uefi_boot_strategy: UefiBootStrategy::EspPartition,
        }
    }

    /// Ventoy-compatible layout: GPT off, MBR-only ESP partition.
    ///
    /// Ventoy's default 2048-byte block mode cannot read GPT headers (they
    /// reside at byte 512 = LBA 1 in 512-byte addressing, but Ventoy maps
    /// LBA 1 → byte 2048).  This layout writes a plain MBR with type 0xEF
    /// (ESP), which the UEFI partition driver *can* discover because the MBR
    /// lives at byte 0 regardless of block size.
    pub fn ventoy_compat() -> Self {
        Self {
            use_gpt: false,
            eltorito_mode: ElToritoMode::Both,
            esp_mode: EspMode::AppendedPartition,
            esp_alignment_lba_512: 2048,
            mbr_mode: MbrMode::EspOnly,
            hidden_sectors_mode: HiddenSectorMode::PartitionOffset,
            uefi_boot_strategy: UefiBootStrategy::EspPartition,
        }
    }

    /// Flat MBR layout: no GPT, ESP partition starts at alignment (LBA 2048 or user-set).
    ///
    /// This maximizes Ventoy compatibility: the MBR lives at byte 0, GPT is absent,
    /// and the FAT BPB's hidden-sectors field matches the partition start so the
    /// UEFI partition driver can mount the filesystem regardless of block size.
    ///
    /// Use `esp_alignment_lba_512: 0` only if you know the ESP must start at
    /// LBA 0 (e.g. for certain embedded or floppy-emulation scenarios).
    pub fn flat() -> Self {
        Self {
            use_gpt: false,
            eltorito_mode: ElToritoMode::Both,
            esp_mode: EspMode::AppendedPartition,
            esp_alignment_lba_512: 0,
            mbr_mode: MbrMode::EspOnly,
            hidden_sectors_mode: HiddenSectorMode::PartitionOffset,
            uefi_boot_strategy: UefiBootStrategy::EspPartition,
        }
    }
}
