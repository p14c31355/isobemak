/// The starting LBA for the EFI System Partition in **ISO 2048-byte sectors**.
///
/// LBA 512 in ISO sectors = 512 × 2048 = 1 MiB = 512-byte sector 2048.
/// Used for El Torito catalog entries (Load RBA) and ISO filesystem layout
/// (seeking to the ESP position within the ISO image).
///
/// This satisfies the 1 MiB alignment requirement that many real UEFI
/// firmwares (AMI, Insyde, older Lenovo, NEC, Panasonic) expect for ESP.
pub const ESP_START_LBA_ISO: u32 = 512;

/// The starting LBA for the EFI System Partition in **512-byte sectors**.
///
/// 512 ISO sectors × 4 = 2048 512-byte sectors = exactly 1 MiB.
/// Used **only** for GPT partition entries and MBR partition table,
/// which always operate in 512-byte sector units.
///
/// NEVER mix this with ESP_START_LBA_ISO — one is for on‑disk partition
/// tables, the other is for El Torito and ISO‑internal offsets.
pub const ESP_START_LBA_512: u32 = 2048;

/// Number of ISO sectors reserved for the system area (MBR at LBA 0
/// plus GPT header at LBA 1 and partition entries at LBA 2-33).
/// GPT partition entries occupy 32 sectors (128 entries × 128 bytes ÷ 512).
pub const GPT_RESERVED_ISO_SECTORS: u32 = 34;

/// Number of 512-byte sectors needed for the backup GPT structures:
/// 1 sector for backup header + 32 sectors for backup partition entries.
pub const BACKUP_GPT_RESERVED_512: u64 = 33;