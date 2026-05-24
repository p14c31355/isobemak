use std::fs;
use std::process::Command;

fn main() -> std::io::Result<()> {
    let dir = tempfile::tempdir()?;
    let img = dir.path().join("esp.img");
    let img_s = img.to_str().unwrap();

    let l = dir.path().join("loader.efi");
    let k = dir.path().join("kernel.elf");
    fs::write(&l, b"UEFI loader test")?;
    fs::write(&k, b"ELF kernel content")?;

    isobemak::fat::create_fat_image(
        &img,
        &[("BOOTX64.EFI", l.as_path()), ("KERNEL.EFI", k.as_path())],
        0,
    )?;

    println!("Image: {img_s}");

    // Read BPB to compute cluster offsets
    let sector0 = fs::read(&img)?;
    let res = u16::from_le_bytes([sector0[14], sector0[15]]) as u64;
    let fatsz = u32::from_le_bytes([sector0[36], sector0[37], sector0[38], sector0[39]]) as u64;
    let data_start = res + 2 * fatsz;

    println!("res={res} fatsz={fatsz} data_start={data_start}");

    println!("=== fsck.fat ===");
    let _ = Command::new("fsck.fat").arg("-n").arg(img_s).status()?;

    println!("=== Root dir (cluster 2) ===");
    let _ = Command::new("hexdump")
        .args(&["-C", "-s", &format!("{}", data_start * 512), "-n", "160"])
        .arg(img_s)
        .status()?;

    println!("=== EFI dir (cluster 3) ===");
    let _ = Command::new("hexdump")
        .args(&[
            "-C",
            "-s",
            &format!("{}", (data_start + 8) * 512), // cluster 3 = data_start + 8 sectors
            "-n",
            "160",
        ])
        .arg(img_s)
        .status()?;

    println!("=== BOOT dir (cluster 4) ===");
    let _ = Command::new("hexdump")
        .args(&[
            "-C",
            "-s",
            &format!("{}", (data_start + 16) * 512),
            "-n",
            "256",
        ])
        .arg(img_s)
        .status()?;

    Ok(())
}