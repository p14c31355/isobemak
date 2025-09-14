<div align="center">
  <h1>isobemak</h1>
</div>

**isobemak** is a Rust crate designed to create bootable disk images. It's built for generating an ISO file that contains a FAT32 filesystem, tailored for UEFI booting.

## How it works

The crate handles two main tasks to produce a bootable ISO:

1.  **Creating a FAT32 Disk Image**: It creates a new file, sizes it to 32 MiB, and formats it as FAT32. The crate then copies two essential files, a bootloader and a kernel, into the `EFI/BOOT/` directory inside this image. It renames the bootloader to `BOOTX64.EFI` and the kernel to `KERNEL.EFI`, a standard convention for UEFI booting.

2.  **Generating the ISO File**: The crate creates a new ISO file and populates it with the necessary data structures to make it bootable according to the El Torito standard. This includes:

      * A **Primary Volume Descriptor** with information like the volume ID "FULLERENE" and the total number of sectors.
      * A **Boot Record Volume Descriptor** that identifies the image as an El Torito bootable specification.
      * A **Boot Catalog** which specifies the location and size of the boot image.

The FAT32 disk image created in the first step is then added to the ISO at a specific location (LBA 20), making it the bootable payload for the ISO.

## Project Structure

  * `src/lib.rs`: The main entry point that coordinates the creation of both the FAT32 image and the final ISO file.
  * `src/fat32.rs`: Contains the logic for creating and formatting the 32 MiB FAT32 disk image and copying the bootloader and kernel files into it.
  * `src/iso.rs`: Handles the creation of the ISO9660 filesystem with El Torito bootable extensions. It sets up the various volume descriptors and the boot catalog.
  * `src/utils.rs`: Provides utility functions, such as `pad_sector`, which aligns files to the correct sector size (2048 bytes), and `crc16`, a checksum function that's no longer used but remains in the code.

## Usage

The `create_disk_and_iso` function in `src/lib.rs` is the primary function to be used. It takes the paths for the output FAT32 and ISO images, as well as file handles for the bootloader and the kernel.

```ignore
pub fn create_disk_and_iso(
    fat32_img: &Path,
    iso: &Path,
    bellows: &mut File,
    kernel: &mut File,
) -> io::Result<()>
```