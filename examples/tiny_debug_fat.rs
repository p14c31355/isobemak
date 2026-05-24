use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::Command;

fn main() -> std::io::Result<()> {
    // Tiny manual FAT32 image: enough sectors for a minimal test
    let tmp = std::env::temp_dir().join("fat_debug2");
    fs::create_dir_all(&tmp)?;

    let img = tmp.join("test.img");
    // 1 MiB = 2048 sectors
    let total_sectors = 2048u32;
    let total_size = total_sectors as u64 * 512;

    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&img)?;
    f.set_len(total_size)?;
    // Zero-fill
    f.seek(SeekFrom::Start(0))?;
    let zero = vec![0u8; 65536];
    for _ in 0..(total_size / 65536 + 1) {
        f.write_all(&zero)?;
    }
    f.seek(SeekFrom::Start(0))?;

    // Write sector 0 manually
    f.seek(SeekFrom::Start(0))?;
    f.write_all(&[0xEBu8, 0x58, 0x90])?; // jump
    f.write_all(b"MSWIN4.1")?;

    // Verify sector 0
    let mut v0 = [0u8; 3];
    f.seek(SeekFrom::Start(0))?;
    f.read_exact(&mut v0)?;
    assert_eq!(v0, [0xEB, 0x58, 0x90], "Sector 0 readback fail");

    // Now write sector 6 and verify
    f.seek(SeekFrom::Start(6 * 512))?;
    f.write_all(&[0xEBu8, 0x58, 0x90])?;
    f.write_all(b"MSWIN4.1")?;

    // Verify sector 6 immediately
    let mut v6 = [0u8; 3];
    f.seek(SeekFrom::Start(6 * 512))?;
    f.read_exact(&mut v6)?;
    assert_eq!(v6, [0xEB, 0x58, 0x90], "Sector 6 readback fail after write");

    // Write 128K of data at sector 257 (1/8 into file) — well away from sector 6
    f.seek(SeekFrom::Start(257 * 512))?;
    let data = vec![0xAAu8; 512 * 256];
    f.write_all(&data)?;

    // Verify sector 6 again
    let mut v6b = [0u8; 3];
    f.seek(SeekFrom::Start(6 * 512))?;
    f.read_exact(&mut v6b)?;
    assert_eq!(
        v6b,
        [0xEB, 0x58, 0x90],
        "Sector 6 corrupted after data write"
    );

    // Now write to sector 32 (FAT table area)
    f.seek(SeekFrom::Start(32 * 512))?;
    f.write_all(&vec![0xBBu8; 512 * 10])?;

    // Verify sector 6 again
    let mut v6c = [0u8; 3];
    f.seek(SeekFrom::Start(6 * 512))?;
    f.read_exact(&mut v6c)?;
    assert_eq!(
        v6c,
        [0xEB, 0x58, 0x90],
        "Sector 6 corrupted after FAT write"
    );

    f.sync_all()?;
    drop(f);

    // Re-open and verify
    let mut f2 = File::open(&img)?;
    let mut v6d = [0u8; 3];
    f2.seek(SeekFrom::Start(6 * 512))?;
    f2.read_exact(&mut v6d)?;
    println!(
        "Final sector 6: {:02x} {:02x} {:02x} (expected eb 58 90)",
        v6d[0], v6d[1], v6d[2]
    );

    // Also verify sector 0
    let mut v0d = [0u8; 3];
    f2.seek(SeekFrom::Start(0))?;
    f2.read_exact(&mut v0d)?;
    println!(
        "Final sector 0: {:02x} {:02x} {:02x} (expected eb 58 90)",
        v0d[0], v0d[1], v0d[2]
    );

    // Dump first 16 bytes of sectors 0-7
    for s in 0..8u64 {
        let mut buf = [0u8; 16];
        f2.seek(SeekFrom::Start(s * 512))?;
        f2.read_exact(&mut buf)?;
        println!(
            "Sector {}: {}",
            s,
            buf.iter()
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    Ok(())
}
