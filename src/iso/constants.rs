/// The starting LBA (in 2048-byte ISO sectors) for the EFI System Partition.
///
/// LBA 512 in ISO sectors = 512 * 2048 = 1 MiB = 512-byte sector 2048.
///
/// This satisfies the 1 MiB alignment requirement that many real UEFI
/// firmwares (AMI, Insyde, older Lenovo, NEC, Panasonic) expect for ESP.
/// Firmware that boots via GPT reads the partition table in 512-byte
/// sectors and expects the ESP to be aligned to a 1 MiB boundary.
pub const ESP_START_LBA: u32 = 512;

/// Number of ISO sectors reserved for the system area (MBR at LBA 0
/// plus GPT header at LBA 1 and partition entries at LBA 2-33).
/// GPT partition entries occupy 32 sectors (128 entries × 128 bytes ÷ 512).
pub const GPT_RESERVED_ISO_SECTORS: u32 = 34;

/// Number of 512-byte sectors needed for the backup GPT structures:
/// 1 sector for backup header + 32 sectors for backup partition entries.
pub const BACKUP_GPT_RESERVED_512: u64 = 33;