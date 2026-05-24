fn main() -> std::io::Result<()> {
    // Direct call to create_fat_image from lib.rs tests — avoids any external overwrites
    let dir = tempfile::tempdir()?;
    let l = dir.path().join("l.efi");
    let k = dir.path().join("k.elf");
    std::fs::write(&l, b"UEFI loader")?;
    std::fs::write(&k, b"ELF kernel content here for testing")?;
    let img = dir.path().join("f.img");
    isobemak::fat::create_fat_image(&img, &[("BOOTX64.EFI", l.as_path()), ("KERNEL.EFI", k.as_path())], 0)?;
    // Read back immediately without re-creating
    let mut f = std::fs::File::open(&img)?;
    use std::io::{Read, Seek, SeekFrom};
    let root_sector = 32 + 2 * 519;
    let cluster2 = root_sector * 512;
    let mut buf = [0u8; 32];
    f.seek(SeekFrom::Start(cluster2))?;
    f.read_exact(&mut buf)?;
    println!("Root dir: {:02x?}", buf);
    // Also dump via shell for certainty
    std::process::Command::new("hexdump").args(&["-C", "-s", &format!("{}", cluster2), "-n", "128", img.to_str().unwrap()]).status()?;
    Ok(())
}