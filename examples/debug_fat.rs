use isobemak::fat::create_fat_image;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

fn main() -> std::io::Result<()> {
    let tmp = std::env::temp_dir().join("fat_debug");
    fs::create_dir_all(&tmp)?;

    let loader = tmp.join("loader.efi");
    let kernel = tmp.join("kernel.elf");
    fs::write(&loader, b"UEFI loader")?;
    fs::write(&kernel, b"ELF kernel content here for testing")?;

    let fat_img = tmp.join("esp.img");
    let files: Vec<(&str, &Path)> = vec![
        ("BOOTX64.EFI", loader.as_path()),
        ("KERNEL.EFI", kernel.as_path()),
    ];
    let sectors = create_fat_image(&fat_img, &files, 0)?;
    println!(
        "Created FAT image at {:?} ({} sectors, {} bytes)",
        fat_img,
        sectors,
        fat_img.metadata()?.len()
    );

    // Verify
    let mut f = File::open(&fat_img)?;
    let mut buf = [0u8; 96];
    f.seek(SeekFrom::Start(6 * 512))?;
    f.read_exact(&mut buf)?;
    println!("Sector 6 first byte: {:02x} (should be eb, not 00)", buf[0]);
    println!(
        "Sector 6 OEM: {:?}",
        std::str::from_utf8(&buf[3..11]).unwrap_or("???")
    );
    Ok(())
}
