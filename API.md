# API Documentation

This document outlines the API endpoints and their usage for creating bootable ISO 9660 images with UEFI support.

## Core Functionality

The primary function for creating ISO images is `build_iso`.

### `build_iso(output_path: &Path, iso_image: &IsoImage) -> io::Result<()>`

**Description:** Builds a bootable ISO 9660 image at the specified `output_path` using the provided `iso_image` configuration.

**Parameters:**
- `output_path`: The path where the ISO image will be created.
- `iso_image`: A configuration object defining the files and boot information for the ISO image.

## Configuration Structures

The `IsoImage` struct and its related structs define the configuration for the ISO image.

### `IsoImage`

Represents the overall configuration for an ISO image.

```rust
pub struct IsoImage {
    pub files: Vec<IsoImageFile>,
    pub boot_info: BootInfo,
}
```

### `IsoImageFile`

Represents a file to be included in the ISO image.

```rust
pub struct IsoImageFile {
    pub source: PathBuf,
    pub destination: String,
}
```

### `BootInfo`

Contains information for both BIOS and UEFI booting.

```rust
pub struct BootInfo {
    pub bios_boot: Option<BiosBootInfo>,
    pub uefi_boot: Option<UefiBootInfo>,
}
```

### `BiosBootInfo`

Configuration for BIOS booting.

```rust
pub struct BiosBootInfo {
    pub boot_catalog: PathBuf,
    pub boot_image: PathBuf,
    pub destination_in_iso: String,
}
```

### `UefiBootInfo`

Configuration for UEFI booting.

```rust
pub struct UefiBootInfo {
    pub boot_image: PathBuf,
    pub destination_in_iso: String,
}
```

## Example Usage

```rust
use isobemak::iso::builder::{build_iso, IsoImage, IsoImageFile, BootInfo, BiosBootInfo, UefiBootInfo};
use std::path::PathBuf;
use std::io;

fn main() -> io::Result<()> {
    let isolinux_bin_path = PathBuf::from("path/to/isolinux.bin");
    let kernel_path = PathBuf::from("path/to/kernel");
    let initrd_img_path = PathBuf::from("path/to/initrd.img");
    let iso_output_path = PathBuf::from("my_bootable.iso");

    let iso_image = IsoImage {
        files: vec![
            IsoImageFile {
                source: isolinux_bin_path.clone(),
                destination: "isolinux/isolinux.bin".to_string(),
            },
            IsoImageFile {
                source: kernel_path.clone(),
                destination: "kernel".to_string(),
            },
            IsoImageFile {
                source: initrd_img_path.clone(),
                destination: "initrd.img".to_string(),
            },
        ],
        boot_info: BootInfo {
            bios_boot: Some(BiosBootInfo {
                boot_catalog: PathBuf::from("BOOT.CAT"),
                boot_image: isolinux_bin_path,
                destination_in_iso: "isolinux/isolinux.bin".to_string(),
            }),
            uefi_boot: Some(UefiBootInfo {
                boot_image: PathBuf::from("path/to/BOOTX64.EFI"),
                destination_in_iso: "EFI/BOOT/EFI.img".to_string(),
            }),
        },
    };

    build_iso(&iso_output_path, &iso_image)?;

    println!("ISO image created successfully at: {:?}", iso_output_path);

    Ok(())
}
