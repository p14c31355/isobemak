// Simulates Choosable's iso.rs, premount.rs, and iso_fs.rs parser logic
// to verify that an ISO produced by isobemak can be correctly parsed via
// the same algorithm that Choosable uses at boot time.
//
// Covered Choosable functions:
//   - read_iso_sector / read_file_iso_sector
//   - get_root_dir → PVD parse
//   - find_in_dir / find_in_dir_flat
//   - find_efi_boot → /EFI/BOOT/BOOTX64.EFI
//   - find_first_file_in_dir → overwritable file scan
//   - find_eod_in_dir → EOD marker walk
//   - build_premount_cpio_entry → synthetic PREMOUNT.CPIO record
//   - build_premount_script (premount.rs) → shell script with offset
//   - cpio_newc_header (premount.rs) → cpio header construction

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

use isobemak::{BootInfo, IsoImage, IsoImageFile, IsoLayoutProfile, UefiBootInfo, build_iso};
use tempfile::tempdir;

use crate::integration_tests::common::setup_integration_test_files;

const ISO_SECTOR_SIZE: usize = 2048;

// ═══════════════════════════════════════════════════════════════════════════
//  Shared helpers
// ═══════════════════════════════════════════════════════════════════════════

fn read_file_iso_sector(file: &mut File, iso_sector: u64) -> io::Result<[u8; ISO_SECTOR_SIZE]> {
    let mut buf = [0u8; ISO_SECTOR_SIZE];
    file.seek(SeekFrom::Start(iso_sector * ISO_SECTOR_SIZE as u64))?;
    file.read_exact(&mut buf)?;
    Ok(buf)
}

fn find_in_dir_flat(
    file: &mut File,
    dir_lba: u32,
    dir_size: u32,
    name: &[u8],
    scratch: &mut [u8; ISO_SECTOR_SIZE],
) -> Option<(u32, u32)> {
    let total_sectors = ((dir_size as u64 + 2047) / 2048) as u32;
    for s in 0..total_sectors {
        *scratch = read_file_iso_sector(file, (dir_lba + s) as u64).ok()?;
        let mut offset: usize = 0;
        while offset + 34 <= ISO_SECTOR_SIZE {
            let record_len = scratch[offset] as usize;
            if record_len == 0 { break; }
            if offset + record_len > ISO_SECTOR_SIZE { break; }
            let name_len = scratch[offset + 32] as usize;
            let name_offset = offset + 33;
            if name_offset + name_len > ISO_SECTOR_SIZE { break; }
            let effective_len = if name_len >= 2 && scratch[name_offset + name_len - 2] == b';' {
                name_len - 2
            } else {
                name_len
            };
            if effective_len == name.len()
                && scratch[name_offset..name_offset + effective_len]
                    .iter()
                    .zip(name.iter())
                    .all(|(a, b)| a.to_ascii_uppercase() == b.to_ascii_uppercase())
            {
                let child_extent =
                    u32::from_le_bytes(scratch[offset + 2..offset + 6].try_into().unwrap());
                let child_size =
                    u32::from_le_bytes(scratch[offset + 10..offset + 14].try_into().unwrap());
                return Some((child_extent, child_size));
            }
            offset += record_len;
        }
    }
    None
}

fn resolve_efi_boot_flat(file: &mut File) -> io::Result<(u32, u32)> {
    let pvd = read_file_iso_sector(file, 16)?;
    if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid PVD signature"));
    }
    let root_lba = u32::from_le_bytes(pvd[158..162].try_into().unwrap());
    let root_size = u32::from_le_bytes(pvd[166..170].try_into().unwrap());
    let mut scratch = [0u8; ISO_SECTOR_SIZE];

    let (efi_lba, efi_size) = find_in_dir_flat(file, root_lba, root_size, b"EFI", &mut scratch)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "/EFI not found"))?;
    let (boot_lba, boot_size) =
        find_in_dir_flat(file, efi_lba, efi_size, b"BOOT", &mut scratch)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "/EFI/BOOT not found"))?;
    find_in_dir_flat(file, boot_lba, boot_size, b"BOOTX64.EFI", &mut scratch)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "/EFI/BOOT/BOOTX64.EFI not found"))
}

// ═══════════════════════════════════════════════════════════════════════════
//  Premount script builder (mirrors premount.rs::build_premount_script)
// ═══════════════════════════════════════════════════════════════════════════

fn hex_nibble(n: u8) -> u8 { if n < 10 { b'0' + n } else { b'A' + (n - 10) } }

fn format_decimal_u64(v: u64) -> [u8; 21] {
    let mut buf = [b'0'; 21];
    let mut val = v;
    let mut pos = 20;
    if val == 0 {
        buf[20] = b'0';
    } else {
        loop {
            buf[pos] = b'0' + (val % 10) as u8;
            val /= 10;
            if val == 0 { break; }
            pos -= 1;
        }
    }
    buf
}

fn build_premount_script_sim(offset_bytes: u64, needs_sr_mod: bool) -> Vec<u8> {
    let mut script = [0u8; 4096];

    let sr_mod_line: &[u8] = if needs_sr_mod { b"modprobe sr_mod 2>/dev/null\n" } else { b"" };

    let src_template = b"\
#!/bin/sh
echo 'choosable OFFSET start' >/tmp/choosable.log
modprobe loop 2>/dev/null
modprobe iso9660 2>/dev/null
SRMOD
mkdir -p /cdrom /lib/live/mount/medium 2>/dev/null
if ! command -v losetup >/dev/null 2>&1; then
  echo 'choosable: losetup not found' >>/tmp/choosable.log
fi
echo 'choosable: scanning partitions...' >/dev/console
while read -r major minor blocks name; do
  case \"$name\" in
    loop*|ram*|dm-*|sr*) continue ;;
    *[0-9]) ;;
    *) continue ;;
  esac
  dev=\"/dev/$name\"
  [ -b \"$dev\" ] || continue
  echo \"choosable: trying $name\" >/dev/console
  echo \"try $name\" >>/tmp/choosable.log
  LOOP=/dev/loop0
  if [ -b \"$LOOP\" ] && losetup \"$LOOP\" >/dev/null 2>&1; then
    LOOP=$(losetup -f 2>/dev/null) || continue
  fi
  [ -n \"$LOOP\" ] || continue
  losetup -o OFFSET \"$LOOP\" \"$dev\" 2>>/tmp/choosable.log || {
    if [ \"$LOOP\" = \"/dev/loop0\" ]; then
      LOOP=$(losetup -f 2>/dev/null) || continue
      losetup -o OFFSET \"$LOOP\" \"$dev\" 2>>/tmp/choosable.log || continue
    else
      continue
    fi
  }
  echo \"loopok $name $LOOP\" >>/tmp/choosable.log
  mount -t iso9660 -o ro \"$LOOP\" /cdrom 2>>/tmp/choosable.log || {
    echo \"choosable: mount iso9660 failed on $name\" >/dev/console
    losetup -d \"$LOOP\" 2>/dev/null
    continue
  }
  mount --make-rshared /cdrom 2>>/tmp/choosable.log
  echo \"mntok $name\" >>/tmp/choosable.log
  mount -o bind /cdrom /lib/live/mount/medium 2>>/tmp/choosable.log
  echo \"choosable: mounted ISO at /cdrom from $name\" >/dev/console
  FOUND=0
  for d in /cdrom; do
    if [ -f \"$d/casper/filesystem.squashfs\" ] || \
       [ -f \"$d/casper/filesystem.squashfs.gpg\" ] || \
       [ -f \"$d/live/filesystem.squashfs\" ] || \
       [ -f \"$d/LiveOS/squashfs.img\" ] || \
       [ -f \"$d/images/install.img\" ] || \
       [ -f \"$d/.disk/info\" ] || \
       [ -f \"$d/dists/stable/Release\" ]; then
      FOUND=1
    fi
  done
  if [ \"$FOUND\" = \"1\" ]; then
    echo \"choosable: FOUND content on $name\" >/dev/console
    echo \"found $name\" >>/tmp/choosable.log
    break
  fi
  echo \"choosable: no content found on $name\" >/dev/console
  echo \"notfound $name\" >>/tmp/choosable.log
  umount /lib/live/mount/medium 2>/dev/null
  umount /cdrom 2>/dev/null
  losetup -d \"$LOOP\" 2>/dev/null
done < /proc/partitions
echo 'choosable: gave up - no ISO found on any partition' >/dev/console
echo 'gaveup' >>/tmp/choosable.log
";

    let off_str = format_decimal_u64(offset_bytes);
    let mut off_start = 0;
    while off_start < 20 && off_str[off_start] == b'0' { off_start += 1; }

    let mut pos = 0usize;
    let bytes = src_template;
    let sr_mod_len = sr_mod_line.len();
    let mut i = 0;
    while i < bytes.len() {
        if i + 5 <= bytes.len()
            && bytes[i] == b'S' && bytes[i+1] == b'R' && bytes[i+2] == b'M'
            && bytes[i+3] == b'O' && bytes[i+4] == b'D'
        {
            for j in 0..sr_mod_len { if pos < 4095 { script[pos] = sr_mod_line[j]; pos += 1; } }
            i += 5;
        } else if i + 6 <= bytes.len()
            && bytes[i] == b'O' && bytes[i+1] == b'F' && bytes[i+2] == b'F'
            && bytes[i+3] == b'S' && bytes[i+4] == b'E' && bytes[i+5] == b'T'
        {
            for j in off_start..21 { if pos < 4095 { script[pos] = off_str[j]; pos += 1; } }
            i += 6;
        } else {
            if pos < 4095 { script[pos] = bytes[i]; pos += 1; }
            i += 1;
        }
    }

    script[..pos].to_vec()
}

fn cpio_newc_header_sim(buf: &mut [u8], name: &[u8], file_size: u32, mode: u32) -> usize {
    let name_len = name.len() as u32 + 1;
    let padded_name_len = ((110 + name_len as usize + 3) & !3) - 110;
    let header_fields: [u32; 13] = [1, mode, 0, 0, 1, 0, file_size, 0, 0, 0, 0, name_len, 0];
    let header_buf_len = 6 + 13 * 8;
    buf[..6].copy_from_slice(b"070701");
    let mut pos = 6usize;
    for &v in &header_fields {
        let s = [
            hex_nibble(((v >> 28) & 0xF) as u8), hex_nibble(((v >> 24) & 0xF) as u8),
            hex_nibble(((v >> 20) & 0xF) as u8), hex_nibble(((v >> 16) & 0xF) as u8),
            hex_nibble(((v >> 12) & 0xF) as u8), hex_nibble(((v >> 8) & 0xF) as u8),
            hex_nibble(((v >> 4) & 0xF) as u8), hex_nibble((v & 0xF) as u8),
        ];
        buf[pos..pos + 8].copy_from_slice(&s);
        pos += 8;
    }
    buf[pos..pos + name.len()].copy_from_slice(name);
    pos += name.len();
    buf[pos] = 0; pos += 1;
    while pos < header_buf_len + padded_name_len { buf[pos] = 0; pos += 1; }
    header_buf_len + padded_name_len
}

fn build_premount_cpio_entry_sim(blob: &mut [u8; 128], extent_lba: u32, file_size: u32) -> u32 {
    let name = b"PREMOUNT.CPIO";
    let record_len: u8 = 46;
    blob[0] = record_len;
    blob[1] = 0;
    blob[2..6].copy_from_slice(&extent_lba.to_le_bytes());
    blob[6..10].copy_from_slice(&extent_lba.to_be_bytes());
    blob[10..14].copy_from_slice(&file_size.to_le_bytes());
    blob[14..18].copy_from_slice(&file_size.to_be_bytes());
    blob[18..25].fill(0);
    blob[25] = 0;
    blob[26] = 0;
    blob[27] = 0;
    blob[28..30].copy_from_slice(&1u16.to_le_bytes());
    blob[30..32].copy_from_slice(&1u16.to_be_bytes());
    blob[32] = name.len() as u8;
    blob[33..33 + name.len()].copy_from_slice(name);
    blob[record_len as usize] = 0;
    (record_len as u32) + 1
}

fn find_first_overwritable_file_sim(
    file: &mut File,
    dir_lba: u32,
    dir_size: u32,
    scratch: &mut [u8; ISO_SECTOR_SIZE],
) -> Option<(u32, u32, [u8; 16], usize)> {
    let total_sectors = ((dir_size as u64 + 2047) / 2048) as u32;
    for s in 0..total_sectors {
        *scratch = read_file_iso_sector(file, (dir_lba + s) as u64).ok()?;
        let mut offset: usize = 0;
        while offset + 34 <= ISO_SECTOR_SIZE {
            let record_len = scratch[offset] as usize;
            if record_len == 0 { break; }
            if offset + record_len > ISO_SECTOR_SIZE { break; }
            let name_len = scratch[offset + 32] as usize;
            let name_offset = offset + 33;
            if name_offset + name_len > ISO_SECTOR_SIZE { break; }
            let flags = scratch[offset + 25];
            let is_dir = flags & 0x02 != 0;
            let is_dot = name_len == 1 && (scratch[name_offset] == 0 || scratch[name_offset] == 1);

            if !is_dot && !is_dir {
                let eff_len = if name_len >= 2 && scratch[name_offset + name_len - 2] == b';' { name_len - 2 } else { name_len };
                if eff_len > 15 { offset += record_len; continue; }
                let cl = eff_len.min(16);
                let mut upper = [0u8; 16];
                for i in 0..cl { upper[i] = scratch[name_offset + i].to_ascii_uppercase(); }
                let is_boot_cat = &upper[..cl] == b"BOOT.CATALOG" || &upper[..cl] == b"BOOT.CAT";
                let has_cfg = eff_len >= 4
                    && scratch[name_offset + eff_len - 4].to_ascii_uppercase() == b'.'
                    && scratch[name_offset + eff_len - 3].to_ascii_uppercase() == b'C'
                    && scratch[name_offset + eff_len - 2].to_ascii_uppercase() == b'F'
                    && scratch[name_offset + eff_len - 1].to_ascii_uppercase() == b'G';
                let is_efi = &upper[..cl] == b"BOOTX64.EFI" || &upper[..cl] == b"BOOTIA32.EFI";
                if !is_boot_cat && !has_cfg && !is_efi {
                    return Some((dir_lba + s, offset as u32, upper, eff_len));
                }
            }
            offset += record_len;
        }
    }
    None
}

fn find_eod_in_dir_sim(
    file: &mut File,
    dir_lba: u32,
    dir_size: u32,
    scratch: &mut [u8; ISO_SECTOR_SIZE],
    dir_size_out: &mut u32,
) -> Option<(u32, u32)> {
    let total_sectors = ((dir_size as u64 + 2047) / 2048) as u32;
    let mut walked = 0u32;
    for s in 0..total_sectors {
        *scratch = read_file_iso_sector(file, (dir_lba + s) as u64).ok()?;
        let mut off = 0usize;
        while off < ISO_SECTOR_SIZE && walked < dir_size {
            let record_len = scratch[off] as usize;
            if record_len == 0 {
                if s + 1 == total_sectors && off + 47 <= ISO_SECTOR_SIZE {
                    *dir_size_out = walked;
                    return Some((dir_lba + s, off as u32));
                }
                break;
            }
            if record_len < 34 || off + record_len > ISO_SECTOR_SIZE { break; }
            walked += record_len as u32;
            off += record_len;
        }
        if s + 1 == total_sectors && off + 47 <= ISO_SECTOR_SIZE {
            *dir_size_out = walked;
            return Some((dir_lba + s, off as u32));
        }
        walked = (s + 1) * ISO_SECTOR_SIZE as u32;
    }
    None
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════

fn make_test_iso_image(bootx64: std::path::PathBuf, kernel: std::path::PathBuf) -> IsoImage {
    IsoImage {
        volume_id: None,
        files: vec![
            IsoImageFile { source: bootx64.clone(), destination: "EFI/BOOT/BOOTX64.EFI".into() },
            IsoImageFile { source: kernel.clone(),  destination: "EFI/BOOT/KERNEL.EFI".into() },
        ],
        boot_info: BootInfo {
            bios_boot: None,
            uefi_boot: Some(UefiBootInfo {
                boot_image: bootx64,
                kernel_image: kernel,
                destination_in_iso: "EFI/BOOT/BOOTX64.EFI".into(),
                additional_efi_boot_files: vec![],
                grub_cfg_content: None,
            }),
        },
        layout_profile: IsoLayoutProfile::hardware(),
    }
}

#[test]
fn test_choosable_can_resolve_efi_boot_in_isohybrid_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let (bootx64_path, kernel_path, iso_path) = setup_integration_test_files(temp_dir.path())?;
    let image = make_test_iso_image(bootx64_path, kernel_path);
    let (_iso_path_buf, _temp_holder, _iso_file, _) = build_iso(&iso_path, &image, true)?;
    let mut file = File::open(&iso_path)?;
    let (efi_lba, efi_size) = resolve_efi_boot_flat(&mut file)?;
    assert!(efi_lba > 0);
    assert_eq!(efi_size, 64 * 1024);
    Ok(())
}

#[test]
fn test_choosable_can_resolve_efi_boot_in_flat_iso() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let (bootx64_path, kernel_path, iso_path) = setup_integration_test_files(temp_dir.path())?;
    let image = make_test_iso_image(bootx64_path, kernel_path);
    let (_iso_path_buf, _temp_holder, _iso_file, _) = build_iso(&iso_path, &image, false)?;
    let mut file = File::open(&iso_path)?;
    let (efi_lba, efi_size) = resolve_efi_boot_flat(&mut file)?;
    assert!(efi_lba > 0);
    assert_eq!(efi_size, 64 * 1024);
    Ok(())
}

#[test]
fn test_premount_script_with_sr_mod() {
    let script = build_premount_script_sim(0x12345678, true);
    let text = String::from_utf8_lossy(&script);
    assert!(text.contains("modprobe sr_mod 2>/dev/null\n"));
    assert!(text.contains("losetup -o 305419896 "));
    assert!(text.contains("#!/bin/sh"));
}

#[test]
fn test_premount_script_without_sr_mod() {
    let script = build_premount_script_sim(512, false);
    let text = String::from_utf8_lossy(&script);
    assert!(!text.contains("SRMOD"));
    assert!(!text.contains("modprobe sr_mod"));
    assert!(text.contains("losetup -o 512 "));
}

#[test]
fn test_premount_script_offset_zero() {
    let script = build_premount_script_sim(0, false);
    let text = String::from_utf8_lossy(&script);
    assert!(text.contains("losetup -o 0 "));
}

#[test]
fn test_cpio_newc_header() {
    let mut header = [0u8; 512];
    let name = b"scripts/live/00choosable";
    let hdr_len = cpio_newc_header_sim(&mut header, name, 1024, 0o100755);
    assert_eq!(&header[..6], b"070701");
    assert_eq!(&header[6..14], b"00000001");          // inode
    assert_eq!(&header[14..22], b"000081ED");         // mode 0o100755
    assert_eq!(&header[54..62], b"00000400");         // file_size 1024
    assert_eq!(&header[110..110 + name.len()], name);
    assert_eq!(header[110 + name.len()], 0);           // null terminator
    assert!(hdr_len > 110);
}

#[test]
fn test_build_premount_cpio_entry() {
    let mut blob = [0u8; 128];
    let written = build_premount_cpio_entry_sim(&mut blob, 42, 12345);
    assert_eq!(written, 47);
    assert_eq!(blob[0], 46);
    assert_eq!(u32::from_le_bytes(blob[2..6].try_into().unwrap()), 42);
    assert_eq!(u32::from_be_bytes(blob[6..10].try_into().unwrap()), 42);
    assert_eq!(u32::from_le_bytes(blob[10..14].try_into().unwrap()), 12345);
    assert_eq!(u32::from_be_bytes(blob[14..18].try_into().unwrap()), 12345);
    assert_eq!(blob[25], 0); // plain file
    assert_eq!(blob[32], 13);
    assert_eq!(&blob[33..46], b"PREMOUNT.CPIO");
    assert_eq!(blob[46], 0); // EOD
}

#[test]
fn test_find_eod_in_isohybrid_root_dir() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let (bootx64_path, kernel_path, iso_path) = setup_integration_test_files(temp_dir.path())?;
    let image = make_test_iso_image(bootx64_path, kernel_path);
    let (_iso_path_buf, _temp_holder, _iso_file, _) = build_iso(&iso_path, &image, true)?;
    let mut file = File::open(&iso_path)?;
    // isobemak writes the ISO9660 filesystem first, then overwrites
    // the file head with MBR+GPT via write_hybrid_structures.
    // Therefore PVD is always at LBA 16 from file start regardless
    // of isohybrid mode.  root_lba from PVD points directly to the
    // root directory sector within the file (no extra offset needed).
    let pvd = read_file_iso_sector(&mut file, 16)?;
    let root_lba = u32::from_le_bytes(pvd[158..162].try_into().unwrap());
    let root_size = u32::from_le_bytes(pvd[166..170].try_into().unwrap());
    let mut scratch = [0u8; ISO_SECTOR_SIZE];
    let mut new_root_size: u32 = 0;
    let eod = find_eod_in_dir_sim(&mut file, root_lba, root_size, &mut scratch, &mut new_root_size);
    assert!(eod.is_some(), "EOD marker must exist in root dir");
    assert!(new_root_size > 0);
    Ok(())
}

#[test]
fn test_pvd_must_be_locatable() -> io::Result<()> {
    let temp_dir = tempdir()?;
    let (bootx64_path, kernel_path, _) = setup_integration_test_files(temp_dir.path())?;
    let image = make_test_iso_image(bootx64_path, kernel_path);
    for isohybrid in [false, true] {
        let p = temp_dir.path().join(format!("pvd_{}.iso", isohybrid));
        let (_a, _b, _c, _d) = build_iso(&p, &image, isohybrid)?;
        let mut file = File::open(&p)?;
        let mut found = false;
        for n in 0..64 {
            if let Ok(s) = read_file_iso_sector(&mut file, 16 + n) {
                if s[0] == 1 && &s[1..6] == b"CD001" {
                    found = true;
                    let vs = u32::from_le_bytes(s[80..84].try_into().unwrap());
                    let rs = u32::from_le_bytes(s[166..170].try_into().unwrap());
                    assert!(vs > 0);
                    assert!(rs > 0);
                    break;
                }
            }
        }
        assert!(found, "PVD must be locatable (isohybrid={})", isohybrid);
    }
    Ok(())
}