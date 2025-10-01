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
            "." => (vec![0u8], 1),
            ".." => (vec![1u8], 1),
            _ => {
                let name_str = if self.flags & 0x02 != 0 {
                    self.name.to_uppercase()
                } else {
                    format!("{};1", self.name.to_uppercase())
                };
                let file_id = name_str.as_bytes().to_vec();
                let len = file_id.len();
                (file_id, len)
            }
        };

        let base_len = 32;
        let total_len = base_len + file_id_len;
        let pad_len = if total_len % 2 == 0 { 0usize } else { 1usize };
        let record_len = (total_len + pad_len) as u8;

        let mut record = vec![0u8; record_len as usize];

        record[0] = record_len;
        record[1] = 0; // Extended attribute length
        record[2..6].copy_from_slice(&self.lba.to_le_bytes()); // LBA
        record[6..10].copy_from_slice(&self.size.to_le_bytes()); // Data length
        // Bytes 10-25: recording timestamp (zeroed)
        record[26] = self.flags;
        record[27] = 0; // File unit size
        record[28] = 0; // Interleave unit size
        record[29..31].copy_from_slice(&1u16.to_le_bytes()); // Volume sequence number LE
        record[31] = file_id_len as u8; // File identifier length
        let file_id_start = 32;
        record[file_id_start..file_id_start + file_id_len].copy_from_slice(&file_id);
        if pad_len == 1 {
            record[file_id_start + file_id_len] = 0;
        }

        record
    }
}
