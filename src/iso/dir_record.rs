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
        let file_id_vec_option: Option<Vec<u8>>;
        let (file_id_bytes, file_id_len) = match self.name {
            "." => (b"\x00" as &[u8], 1),
            ".." => (b"\x01" as &[u8], 1),
            _ => {
                if self.flags & 0x02 != 0 {
                    (self.name.as_bytes(), self.name.len())
                } else {
                    let name_with_version = format!("{};1", self.name.to_uppercase());
                    file_id_vec_option = Some(name_with_version.into_bytes());
                    (
                        file_id_vec_option.as_ref().unwrap().as_slice(),
                        file_id_vec_option.as_ref().unwrap().len(),
                    )
                }
            }
        };

        let base_len = 33 + file_id_len;
        let record_len_usize = base_len + (base_len % 2);
        assert!(
            record_len_usize <= u8::MAX as usize,
            "Directory record length exceeds 255 bytes"
        );
        let record_len = record_len_usize as u8;

        let mut record = vec![0u8; record_len as usize];

        record[0] = record_len;
        record[1] = 0; // Extended attribute record length
        record[2..6].copy_from_slice(&self.lba.to_le_bytes());
        record[6..10].copy_from_slice(&self.lba.to_be_bytes());
        record[10..14].copy_from_slice(&self.size.to_le_bytes());
        record[14..18].copy_from_slice(&self.size.to_be_bytes());
        record[25] = self.flags;
        record[26] = 0; // File unit size
        record[27] = 0; // Interleave gap size
        record[28..30].copy_from_slice(&1u16.to_le_bytes()); // Volume sequence number LE
        record[30..32].copy_from_slice(&1u16.to_be_bytes()); // Volume sequence number BE
        record[32] = file_id_len as u8;
        record[33..33 + file_id_len].copy_from_slice(file_id_bytes);

        record
    }
}
