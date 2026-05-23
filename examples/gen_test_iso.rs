use std::fs;
use std::io;
use std::path::PathBuf;

fn main() -> io::Result<()> {
    let dir = PathBuf::from("/tmp/isotest");
    fs::create_dir_all(&dir)?;
    let boot = dir.join("BOOTX64.EFI");
    let kern = dir.join("KERNEL.EFI");
    fs::write(&boot, vec![0xEFu8; 64 * 1024])?;
    fs::write(&kern, vec![0xEFu8; 16 * 1024])?;

    let iso_path = PathBuf::from("/tmp/test_iso.iso");
    let img = isobemak::IsoImage {
        volume_id: None,
        files: vec![
            isobemak::IsoImageFile {
                source: boot.clone(),
                destination: "EFI/BOOT/BOOTX64.EFI".into(),
            },
            isobemak::IsoImageFile {
                source: kern.clone(),
                destination: "EFI/BOOT/KERNEL.EFI".into(),
            },
        ],
        boot_info: isobemak::BootInfo {
            bios_boot: None,
            uefi_boot: Some(isobemak::UefiBootInfo {
                boot_image: boot,
                kernel_image: kern,
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".into(),
                additional_efi_boot_files: vec![],
                grub_cfg_content: None,
            }),
        },
    };
    isobemak::build_iso(&iso_path, &img, true)?;
    println!("ISO: {:?} size={}", iso_path, iso_path.metadata()?.len());
    Ok(())
}