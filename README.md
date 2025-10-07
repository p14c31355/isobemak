# isobemak

**isobemak** is a Rust crate for creating bootable ISO 9660 images with UEFI and BIOS support. It can generate standard ISO images or hybrid isohybrid images that can boot from both optical media and USB drives.

## Features

- **ISO 9660 Filesystem Creation**: Generates standard ISO 9660 filesystem structures
- **UEFI Boot Support**: Creates UEFI-bootable images with proper ESP (EFI System Partition) handling
- **BIOS/El Torito Boot Support**: Provides legacy BIOS boot capability
- **Hybrid Isohybrid Images**: Generates hybrid images that can boot both as optical media and USB drives with GPT/MBR structures
- **FAT32 ESP Creation**: Automatically creates FAT32-formatted EFI System Partitions for UEFI booting

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
isobemak = "0.2.2"
```

## Usage

The primary function is `build_iso`, which takes a configured `IsoImage` and generates the ISO file.

### Basic Example

```rust
use isobemak::{build_iso, IsoImage, IsoImageFile, BootInfo, UefiBootInfo};
use std::path::PathBuf;

let isolinux_bin_path = PathBuf::from("path/to/isolinux.bin");
let kernel_path = PathBuf::from("path/to/kernel");
let bootx64_efi_path = PathBuf::from("path/to/BOOTX64.EFI");
let iso_output_path = PathBuf::from("bootable.iso");

let iso_image = IsoImage {
    files: vec![
        IsoImageFile {
            source: kernel_path.clone(),
            destination: "kernel".to_string(),
        },
    ],
    boot_info: BootInfo {
        bios_boot: None,
        uefi_boot: Some(UefiBootInfo {
            boot_image: bootx64_efi_path.clone(),
            kernel_image: kernel_path.clone(),
            destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
        }),
    },
};

// Create a standard UEFI-bootable ISO
let (iso_path, _temp_fat, _iso_file, _fat_size) = build_iso(&iso_output_path, &iso_image, false)?;
```

### Hybrid Isohybrid Example

```rust
use isobemak::{build_iso, IsoImage, IsoImageFile, BootInfo, BiosBootInfo, UefiBootInfo};
use std::path::PathBuf;

let isolinux_bin_path = PathBuf::from("path/to/isolinux.bin");
let kernel_path = PathBuf::from("path/to/kernel");
let bootx64_efi_path = PathBuf::from("path/to/BOOTX64.EFI");
let iso_output_path = PathBuf::from("hybrid.iso");

let iso_image = IsoImage {
    files: vec![
        IsoImageFile {
            source: kernel_path.clone(),
            destination: "kernel".to_string(),
        },
    ],
    boot_info: BootInfo {
        bios_boot: Some(BiosBootInfo {
            boot_catalog: PathBuf::from("BOOT.CAT"),
            boot_image: isolinux_bin_path.clone(),
            destination_in_iso: "isolinux/isolinux.bin".to_string(),
        }),
        uefi_boot: Some(UefiBootInfo {
            boot_image: bootx64_efi_path.clone(),
            kernel_image: kernel_path.clone(),
            destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
        }),
    },
};

// Create a hybrid isohybrid ISO that can boot from both CD/DVD and USB
let (iso_path, _temp_fat, _iso_file, _fat_size) = build_iso(&iso_output_path, &iso_image, true)?;
```

## How It Works

### Standard ISO Creation

1. **Filesystem Preparation**: Creates an ISO 9660 filesystem structure with directories and file records
2. **Boot Catalog Creation**: Generates an El Torito boot catalog pointing to boot images
3. **Volume Descriptors**: Writes Primary Volume Descriptor, Boot Record Volume Descriptor, and Volume Descriptor Set Terminator
4. **File Copying**: Copies all specified files into the ISO at their designated locations

### Isohybrid (Hybrid) Creation

For hybrid images, the process additionally includes:

1. **EFI System Partition Creation**: Generates a FAT32-formatted disk image containing the UEFI bootloader and kernel
2. **ESP Embedding**: Embeds the EFI System Partition into the ISO at a specific location (LBA 34)
3. **MBR/GPT Structures**: Adds Master Boot Record and GUID Partition Table structures to make the image USB-bootable
4. **Hybrid Boot Catalog**: Creates boot catalog entries for both BIOS (MBR) and UEFI (ESP) booting

## API Overview

### Core Functions

- `build_iso(iso_path: &Path, image: &IsoImage, is_isohybrid: bool)` - Main ISO creation function

### Configuration Structures

- `IsoImage` - Top-level configuration containing files and boot information
- `IsoImageFile` - Specifies source file and destination path in ISO
- `BootInfo` - Contains optional BIOS and UEFI boot configurations
- `BiosBootInfo` - BIOS/El Torito boot settings
- `UefiBootInfo` - UEFI boot settings including ESP creation

### Builder Pattern Alternative

For more control, you can use the `IsoBuilder`:

```rust
use isobemak::{IsoBuilder, BootInfo, BiosBootInfo, UefiBootInfo};
use std::fs::File;
use std::path::{Path, PathBuf};

let mut builder = IsoBuilder::new();
builder.set_isohybrid(true);

builder.add_file("kernel", PathBuf::from("my_kernel"))?;
builder.add_file("initrd.img", PathBuf::from("my_initrd"))?;

let boot_info = BootInfo {
    bios_boot: Some(BiosBootInfo {
        boot_catalog: PathBuf::from("BOOT.CAT"),
        boot_image: PathBuf::from("isolinux.bin"),
        destination_in_iso: "isolinux/isolinux.bin".to_string(),
    }),
    uefi_boot: Some(UefiBootInfo {
        boot_image: PathBuf::from("BOOTX64.EFI"),
        kernel_image: PathBuf::from("kernel"),
        destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
    }),
};

builder.set_boot_info(boot_info);

let mut iso_file = File::create("output.iso")?;
builder.build(&mut iso_file, Path::new("output.iso"), None, None)?;
```

## Project Structure

- `src/lib.rs` - Main library exports and definitions
- `src/iso/` - ISO 9660 filesystem implementation
  - `builder.rs` - Core ISO building logic
  - `iso_writer.rs` - File writing and descriptor creation
  - `fs_node.rs` - Filesystem node representations
  - `volume_descriptor.rs` - Volume descriptor structures
  - `gpt/` - GUID Partition Table support for hybrid images
- `src/fat.rs` - FAT32 ESP creation utilities
- `src/utils.rs` - Utility functions and constants

## Dependencies

- `crc32fast` - CRC32 checksum calculation
- `fatfs` - FAT filesystem manipulation
- `uuid` - UUID generation for GPT partitions
- `tempfile` - Temporary file handling
- `regex` - Text processing utilities

## License

Licensed under both MIT and Apache 2.0 licenses.

## Contributing

Contributions are welcome! Please see the contributing guide at `docs/CONTRIBUTING.md`.
