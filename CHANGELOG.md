## [unreleased]
- Implement boot information table (`-boot-info-table`) patching for BIOS boot images. The 56-byte structure (PVD LBA, boot image LBA, file length, checksum) is now automatically written at offsets 8–63 of the BIOS boot image, fixing boot for stage‑1 loaders such as ISOLINUX and Limine
- **Breaking:** `IsoBuilder::build()` now requires the `iso_file` to be opened with **read + write** access. Use `OpenOptions::new().read(true).write(true).create(true).truncate(true).open(...)` instead of `File::create(...)`. The `build_iso()` convenience function handles this automatically

## [0.2.5] - 2026-05-23
- Add `UefiBootInfo::additional_efi_boot_files` field for including extra EFI binaries (e.g. GRUBX64.EFI) in the ESP FAT image
- Remove unused `BiosBootInfo::boot_catalog` field
- Refactor `create_fat_image` to accept a generic list of (filename, source_path) pairs
- Add `IsoBuilder::add_file` now accepts `&Path` instead of `PathBuf`
- Add comprehensive test for GRUBX64.EFI integration
- Update API documentation and README

## [0.2.4] - 2026-04-25
## [0.2.3] - 2025-10-07
## [0.2.2] - 2025-09-20
## [0.2.1] - 2025-09-18
## [0.2.0] - 2025-09-17