// isobemak/src/iso/dir_record.rs

/// ISO9660 directory record structure
pub struct IsoDirEntry<'a> {
    pub lba: u32,
    pub size: u32,
    pub flags: u8,
    pub name: &'a str,
}

impl<'a> IsoDirEntry<'a> {
    /// Creates ISO9660 directory record bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let (file_id, file_id_len) = match self.name {
            "." => (vec![0x00], 1),
            ".." => (vec![0x01], 1),
            _ => {
                let name_str = if self.flags & 0x02 != 0 {
                    self.name.to_uppercase()
                } else {
                    format!("{};1", self.name.to_uppercase())
                };
                let bytes = name_str.into_bytes();
                let len = bytes.len();
                (bytes, len)
            }
        };

        let mut record_len = 33 + file_id_len;
        if record_len % 2 != 0 {
            record_len += 1;
        }
        assert!(
            record_len <= u8::MAX as usize,
            "Directory record length exceeds 255 bytes"
        );
        let mut record = vec![0u8; record_len];
        record[0] = record_len as u8;
        // record[1] is extended attribute record length, 0
        record[2..6].copy_from_slice(&self.lba.to_le_bytes());
        record[6..10].copy_from_slice(&self.lba.to_be_bytes());
        record[10..14].copy_from_slice(&self.size.to_le_bytes());
        record[14..18].copy_from_slice(&self.size.to_be_bytes());
        // bytes 18-24 are timestamp, leave as 0
        record[25] = self.flags;
        // record[26] is file unit size, 0
        // record[27] is interleave gap size, 0
        record[28..30].copy_from_slice(&1u16.to_le_bytes()); // Volume sequence number LE
        record[30..32].copy_from_slice(&1u16.to_be_bytes()); // Volume sequence number BE
        record[32] = file_id_len as u8;
        record[33..33 + file_id_len].copy_from_slice(&file_id);
        // The final byte is for padding if needed, and is already 0 from vec initialization.

        record
    }
}
