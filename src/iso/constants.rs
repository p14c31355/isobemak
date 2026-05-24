/// Size of one ISO 9660 sector (logical block) in bytes.
pub const ISO_SECTOR_SIZE: u64 = 2048;

/// Size of one disk sector (used by GPT, MBR, FAT BPB) in bytes.
pub const DISK_SECTOR_SIZE: u64 = 512;

/// The starting LBA for the EFI System Partition in **ISO 2048-byte sectors**.
///
/// LBA 1024 in ISO sectors = 1024 × 2048 = 2 MiB = 512-byte sector 4096.
/// Used for El Torito catalog entries (Load RBA) and ISO filesystem layout
/// (seeking to the ESP position within the ISO image).
///
/// 2 MiB alignment is chosen for maximum compatibility with real UEFI
/// firmware (NEC, Insyde, older Lenovo) that may not handle 1 MiB alignment.
pub const ESP_START_LBA_ISO: u32 = 1024;

/// The starting LBA for the EFI System Partition in **512-byte sectors**.
///
/// 1024 ISO sectors × 4 = 4096 512-byte sectors = exactly 2 MiB.
/// Used **only** for GPT partition entries and MBR partition table,
/// which always operate in 512-byte sector units.
///
/// NEVER mix this with ESP_START_LBA_ISO — one is for on‑disk partition
/// tables, the other is for El Torito and ISO‑internal offsets.
/// Use [`iso_to_512`] / [`disk512_to_iso`] to convert when needed.
pub const ESP_START_LBA_512: u32 = 4096;

/// Number of **512-byte sectors** reserved at the start of the disk for the
/// GPT protective area.
///
/// This covers:
///   - LBA 0: protective MBR (1 sector)
///   - LBA 1: GPT header (1 sector)
///   - LBA 2–33: GPT partition entry array (32 sectors for 128 entries × 128 bytes)
///
/// Total: 34 × 512 = 17 KiB.
///
/// This is a **disk-sector** constant (512-byte units), NOT an ISO-sector
/// constant.  It exists for documentation and validation; the actual GPT
/// layout is computed at runtime from the partition entry count and size.
pub const GPT_RESERVED_512_SECTORS: u32 = 34;

/// Number of 512-byte sectors needed for the backup GPT structures:
/// 1 sector for backup header + 32 sectors for backup partition entries.
pub const BACKUP_GPT_RESERVED_512: u64 = 33;

/// Convert an ISO 2048-byte sector LBA to the equivalent 512-byte sector LBA.
///
/// 1 ISO sector = 4 × 512-byte sectors.
///
/// # Example
/// ```
/// # use isobemak::iso::constants::iso_to_512;
/// assert_eq!(iso_to_512(1024), 4096); // 2 MiB
/// ```
#[inline]
pub const fn iso_to_512(lba: u32) -> u32 {
    lba * 4
}

/// Convert a 512-byte disk sector LBA to the equivalent ISO 2048-byte sector LBA.
///
/// Rounds down; partial ISO sectors are discarded.
///
/// # Example
/// ```
/// # use isobemak::iso::constants::disk512_to_iso;
/// assert_eq!(disk512_to_iso(4096), 1024); // 2 MiB
/// assert_eq!(disk512_to_iso(4099), 1024); // fractional → floor
/// ```
#[inline]
pub const fn disk512_to_iso(lba: u32) -> u32 {
    lba / 4
}
