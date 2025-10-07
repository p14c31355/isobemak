# API Documentation

This document outlines the API for `isobemak`, a Rust crate for creating bootable ISO 9660 images with UEFI and BIOS support.

## Main Functions

### `build_iso(iso_path: &Path, image: &IsoImage, is_isohybrid: bool) -> io::Result<(PathBuf, Option<NamedTempFile>, File, Option<u32>)>`

**Description:** Builds a bootable ISO 9660 image at the specified path. For hybrid isohybrid images that can boot from both optical media and USB drives, set `is_isohybrid` to `true`.

**Parameters:**
- `iso_path`: The path where the ISO image will be created
- `image`: Configuration object defining the files and boot information for the ISO image
- `is_isohybrid`: Whether to create a hybrid isohybrid image that can boot from USB drives

**Returns:**
A tuple containing:
- `PathBuf`: The path to the created ISO file
- `Option<NamedTempFile>`: Temporary FAT image file (if created for isohybrid)
- `File`: Open file handle to the ISO
- `Option<u32>`: FAT image size in 512-byte sectors (if created)

## Configuration Structures

### `IsoImage`

Top-level configuration structure for ISO images.

```rust
pub struct IsoImage {
    pub files: Vec<IsoImageFile>,
    pub boot_info: BootInfo,
}
```

### `IsoImageFile`

Represents a file to be included in the ISO.

```rust
pub struct IsoImageFile {
    pub source: PathBuf,
    pub destination: String,
}
```

### `BootInfo`

Contains boot configuration for BIOS and/or UEFI booting.

```rust
pub struct BootInfo {
    pub bios_boot: Option<BiosBootInfo>,
    pub uefi_boot: Option<UefiBootInfo>,
}
```

### `BiosBootInfo`

Configuration for BIOS/El Torito boot support.

```rust
pub struct BiosBootInfo {
    pub boot_catalog: PathBuf,
    pub boot_image: PathBuf,
    pub destination_in_iso: String,
}
```

### `UefiBootInfo`

Configuration for UEFI booting. For isohybrid images, this will create an EFI System Partition with the specified boot and kernel images.

```rust
pub struct UefiBootInfo {
    pub boot_image: PathBuf,
    pub kernel_image: PathBuf,
    pub destination_in_iso: String,
}
```

## Builder API

### `IsoBuilder`

Provides a builder pattern interface for more advanced ISO creation.

```rust
pub struct IsoBuilder { /* ... */ }
```

**Methods:**
- `new() -> Self`: Creates a new builder
- `add_file(&mut self, path_in_iso: &str, real_path: PathBuf) -> io::Result<()>`: Adds a file to the ISO
- `set_boot_info(&mut self, boot_info: BootInfo)`: Sets boot configuration
- `set_isohybrid(&mut self, is_isohybrid: bool)`: Enables hybrid isohybrid creation
- `build(&mut self, iso_file: &mut File, iso_path: &Path, esp_lba: Option<u32>, esp_size_sectors: Option<u32>) -> io::Result<()>`: Builds the ISO

## Filesystem Nodes

### `IsoFsNode`

Represents a filesystem node in the ISO.

```rust
pub enum IsoFsNode {
    File(IsoFile),
    Directory(IsoDirectory),
}
```

### `IsoFile`

Represents a file in the ISO filesystem.

```rust
pub struct IsoFile {
    pub path: PathBuf,
    pub size: u64,
    pub lba: u32,
}
```

### `IsoDirectory`

Represents a directory in the ISO filesystem.

```rust
pub struct IsoDirectory {
    pub lba: u32,
    pub children: HashMap<String, IsoFsNode>,
}
```

## Constants

### `ESP_START_LBA`

The Logical Block Address where the EFI System Partition starts in hybrid isohybrid images.

```rust
pub const ESP_START_LBA: u32 = 34;
```

## Examples

### Basic UEFI-Bootable ISO

```rust
use isobemak::{build_iso, IsoImage, IsoImageFile, BootInfo, UefiBootInfo};
use std::path::PathBuf;

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

// Create standard UEFI-bootable ISO
let (_iso_path, _temp_fat, _iso_file, _fat_size) = build_iso(&iso_output_path, &iso_image, false)?;
```

### Hybrid Isohybrid ISO (BIOS + UEFI)

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

// Create hybrid isohybrid ISO
let (_iso_path, _temp_fat, _iso_file, _fat_size) = build_iso(&iso_output_path, &iso_image, true)?;
```

### Using the Builder Pattern

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

## Error Handling

All functions return `io::Result<T>`, so handle `std::io::Error` for file I/O and validation errors.

Common errors:
- Invalid file paths
- Insufficient disk space
- Unsupported image sizes for hybrid ISOs (minimum 69 sectors)
- Missing boot files
