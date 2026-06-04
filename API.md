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
    pub volume_id: Option<String>,
    pub files: Vec<IsoImageFile>,
    pub boot_info: BootInfo,
    /// ISO layout profile for firmware compatibility.
    /// Default: [IsoLayoutProfile::hardware] (GPT enabled, 2 MiB ESP alignment).
    /// For QEMU/OVMF, use [IsoLayoutProfile::emulator].
    pub layout_profile: IsoLayoutProfile,
}
```

**`layout_profile`**: Controls GPT/MBR partitioning, El Torito mode, ESP alignment, and UEFI boot strategy. Defaults to `IsoLayoutProfile::hardware()` (GPT enabled, 2 MiB ESP alignment, `HiddenSectorMode::Zero`). Use `IsoLayoutProfile::emulator()` for QEMU/OVMF compatibility (GPT enabled, `HiddenSectorMode::PartitionOffset`).

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
    pub additional_efi_boot_files: Vec<(String, PathBuf)>,
    pub grub_cfg_content: Option<String>,
}
```

**`additional_efi_boot_files`**: A list of (destination_filename, source_path) pairs for additional EFI boot files to include in the FAT ESP image (isohybrid only). For example, to add GRUBX64.EFI, set `additional_efi_boot_files: vec![("GRUBX64.EFI".to_string(), PathBuf::from("path/to/grubx64.efi"))]`.

**`grub_cfg_content`**: Optional string content for an auto-generated `grub.cfg` file placed at `EFI/BOOT/grub.cfg` in the FAT ESP image. When set, a grub.cfg with the specified content is automatically created in the ESP. Set to `None` to skip.

## Builder API

### `IsoBuilder`

Provides a builder pattern interface for more advanced ISO creation.

```rust
pub struct IsoBuilder { /* ... */ }
```

**Methods:**
- `new() -> Self`: Creates a new builder
- `set_volume_id(&mut self, v: Option<String>)`: Sets the volume ID
- `add_file(&mut self, path_in_iso: &str, real_path: &Path) -> io::Result<()>`: Adds a file to the ISO
- `set_boot_info(&mut self, boot_info: BootInfo)`: Sets boot configuration
- `set_profile(&mut self, profile: IsoLayoutProfile)`: Sets the layout profile
- `set_isohybrid(&mut self, is_isohybrid: bool)`: Enables hybrid isohybrid creation
- `set_disk_layout(&mut self, layout: DiskLayout)`: Sets a manual disk layout
- `build(&mut self, iso_file: &mut File, iso_path: &Path, esp_lba: Option<u32>, esp_size_sectors: Option<u32>) -> io::Result<()>`: Builds the ISO

**Public fields:**
- `esp_lba: Option<u32>` — ESP partition starting LBA (set automatically during build if not specified)
- `esp_size_sectors: Option<u32>` — ESP partition size in sectors (set automatically during build if not specified)

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

### `ISO_SECTOR_SIZE`

Size of one ISO 9660 sector (logical block) in bytes.

```rust
pub const ISO_SECTOR_SIZE: u64 = 2048;
```

### `DISK_SECTOR_SIZE`

Size of one disk sector (used by GPT, MBR, FAT BPB) in bytes.

```rust
pub const DISK_SECTOR_SIZE: u64 = 512;
```

### `ESP_START_LBA_ISO`

The starting LBA for the EFI System Partition in **ISO 2048-byte sectors** (LBA 1024 = 2 MiB). Used for El Torito catalog entries and ISO filesystem layout.

```rust
pub const ESP_START_LBA_ISO: u32 = 1024;
```

### `ESP_START_LBA_512`

The starting LBA for the EFI System Partition in **512-byte sectors** (LBA 4096 = 2 MiB). Used only for GPT partition entries and MBR partition table.

```rust
pub const ESP_START_LBA_512: u32 = 4096;
```

### `GPT_RESERVED_512_SECTORS`

Number of 512-byte sectors reserved at the start of the disk for the GPT protective area (MBR + GPT header + partition entry array = 34 sectors).

```rust
pub const GPT_RESERVED_512_SECTORS: u32 = 34;
```

### `BACKUP_GPT_RESERVED_512`

Number of 512-byte sectors needed for the backup GPT structures (1 header + 32 partition entries).

```rust
pub const BACKUP_GPT_RESERVED_512: u64 = 33;
```

### `iso_to_512(lba: u32) -> u32`

Converts an ISO 2048-byte sector LBA to the equivalent 512-byte sector LBA (multiply by 4).

### `disk512_to_iso(lba: u32) -> u32`

Converts a 512-byte disk sector LBA to the equivalent ISO 2048-byte sector LBA (divide by 4, rounding down).

## Layout Configuration

### `IsoLayoutProfile`

Controls multiple aspects of the ISO layout for firmware compatibility.

```rust
pub struct IsoLayoutProfile {
    pub use_gpt: bool,
    pub eltorito_mode: ElToritoMode,
    pub esp_mode: EspMode,
    pub esp_alignment_lba_512: u32,
    pub mbr_mode: MbrMode,
    pub hidden_sectors_mode: HiddenSectorMode,
    pub uefi_boot_strategy: UefiBootStrategy,
}
```

**Factory methods:**
- `IsoLayoutProfile::hardware()` — The default. GPT enabled, 2 MiB ESP alignment, `HiddenSectorMode::Zero`, `UefiBootStrategy::EspPartition`. Best for real hardware (NEC, Insyde, older Lenovo).
- `IsoLayoutProfile::emulator()` — GPT enabled, 2 MiB ESP alignment, `HiddenSectorMode::PartitionOffset`, `UefiBootStrategy::ElToritoDirectEfi`. Best for QEMU/OVMF.

### `ElToritoMode`

```rust
pub enum ElToritoMode {
    Both,
    DirectEfiOnly,
}
```

### `EspMode`

```rust
pub enum EspMode {
    AppendedPartition,
}
```

### `MbrMode`

```rust
pub enum MbrMode {
    HybridLinuxEsp,
}
```

### `HiddenSectorMode`

Controls the `hidden_sectors` field in the FAT BPB.

```rust
pub enum HiddenSectorMode {
    Zero,
    PartitionOffset,
}
```

### `UefiBootStrategy`

```rust
pub enum UefiBootStrategy {
    ElToritoDirectEfi,
    EspPartition,
}
```

## Disk Layout Structures

### `DiskLayout`

Manually-specified disk layout for the ISO image. Use `DiskLayout::from_partition_params` to construct.

```rust
pub struct DiskLayout {
    pub partitions: Vec<Partition>,
    pub iso_region: IsoRegion,
}
```

**Methods:**
- `from_partition_params(esp_align: u32, esp_size: Option<u32>, iso_data_lba: u32) -> Self`: Creates a `DiskLayout` with an optional ESP partition
- `esp_partition(&self) -> Option<&Partition>`: Returns the ESP partition if present
- `has_esp(&self) -> bool`: Returns `true` if the layout includes an ESP partition

### `Partition`

```rust
pub struct Partition {
    pub start_lba_512: u64,
    pub size_lba_512: u64,
}
```

### `IsoRegion`

```rust
pub struct IsoRegion {
    pub data_start_lba: u32,
    pub total_sectors: u32,
}
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
    volume_id: Some("label".to_string()),
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
            additional_efi_boot_files: Vec::new(),
            grub_cfg_content: None,
        }),
    },
    layout_profile: IsoLayoutProfile::default(),
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
    volume_id: Some("label".to_string()),
    files: vec![
        IsoImageFile {
            source: kernel_path.clone(),
            destination: "kernel".to_string(),
        },
    ],
    boot_info: BootInfo {
        bios_boot: Some(BiosBootInfo {
            boot_image: isolinux_bin_path.clone(),
            destination_in_iso: "isolinux/isolinux.bin".to_string(),
        }),
        uefi_boot: Some(UefiBootInfo {
            boot_image: bootx64_efi_path.clone(),
            kernel_image: kernel_path.clone(),
            destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
            additional_efi_boot_files: Vec::new(),
            grub_cfg_content: None,
        }),
    },
    layout_profile: IsoLayoutProfile::default(),
};

// Create hybrid isohybrid ISO
let (_iso_path, _temp_fat, _iso_file, _fat_size) = build_iso(&iso_output_path, &iso_image, true)?;
```

### Isohybrid ISO with GRUBX64.EFI

```rust
use isobemak::{build_iso, IsoImage, IsoImageFile, BootInfo, UefiBootInfo};
use std::path::PathBuf;

let bootx64_path = PathBuf::from("path/to/BOOTX64.EFI");
let grubx64_path = PathBuf::from("path/to/GRUBX64.EFI");
let kernel_path = PathBuf::from("path/to/kernel");
let iso_output_path = PathBuf::from("hybrid_grub.iso");

let iso_image = IsoImage {
    volume_id: Some("hybrid".to_string()),
    files: vec![
        IsoImageFile {
            source: kernel_path.clone(),
            destination: "kernel".to_string(),
        },
    ],
    boot_info: BootInfo {
        bios_boot: None,
        uefi_boot: Some(UefiBootInfo {
            boot_image: bootx64_path.clone(),
            kernel_image: kernel_path.clone(),
            destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
            additional_efi_boot_files: vec![
                ("GRUBX64.EFI".to_string(), grubx64_path.clone()),
            ],
            grub_cfg_content: None,
        }),
    },
    layout_profile: IsoLayoutProfile::default(),
};

// Create hybrid isohybrid ISO with GRUBX64.EFI in the ESP
let (_iso_path, _temp_fat, _iso_file, _fat_size) = build_iso(&iso_output_path, &iso_image, true)?;
```

### Isohybrid ISO with Auto-Generated grub.cfg

```rust
use isobemak::{build_iso, IsoImage, IsoImageFile, BootInfo, UefiBootInfo};
use std::path::PathBuf;

let bootx64_path = PathBuf::from("path/to/BOOTX64.EFI");
let kernel_path = PathBuf::from("path/to/kernel");
let iso_output_path = PathBuf::from("hybrid_grub_cfg.iso");

let grub_config = r#"set default=0
set timeout=5

menuentry "Boot from ISO" {
    chainloader /EFI/BOOT/BOOTX64.EFI
}

menuentry "Kernel" {
    linuxefi /EFI/BOOT/KERNEL.EFI
}
"#;

let iso_image = IsoImage {
    volume_id: Some("hybrid".to_string()),
    files: vec![
        IsoImageFile {
            source: kernel_path.clone(),
            destination: "kernel".to_string(),
        },
    ],
    boot_info: BootInfo {
        bios_boot: None,
        uefi_boot: Some(UefiBootInfo {
            boot_image: bootx64_path.clone(),
            kernel_image: kernel_path.clone(),
            destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
            additional_efi_boot_files: Vec::new(),
            grub_cfg_content: Some(grub_config.to_string()),
        }),
    },
    layout_profile: IsoLayoutProfile::default(),
};

// Create hybrid isohybrid ISO with auto-generated EFI/BOOT/grub.cfg in the ESP
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
        boot_image: PathBuf::from("isolinux.bin"),
        destination_in_iso: "isolinux/isolinux.bin".to_string(),
    }),
    uefi_boot: Some(UefiBootInfo {
        boot_image: PathBuf::from("BOOTX64.EFI"),
        kernel_image: PathBuf::from("kernel"),
        destination_in_iso: "EFI/BOOT/BOOTX64.EFI".to_string(),
        additional_efi_boot_files: vec![
            ("GRUBX64.EFI".to_string(), PathBuf::from("grubx64.efi")),
        ],
        grub_cfg_content: Some("set default=0\nset timeout=5\nmenuentry \"Boot\" {\n  chainloader /EFI/BOOT/BOOTX64.EFI\n}".to_string()),
    }),
};

builder.set_boot_info(boot_info);
builder.set_profile(IsoLayoutProfile::default());

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