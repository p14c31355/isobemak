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

}