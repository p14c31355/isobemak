## [unreleased]
## [unreleased]
## [unreleased]
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