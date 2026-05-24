#[derive(Debug, Clone)]
pub struct Partition {
    pub start_lba_512: u64,
    pub size_lba_512: u64,
}
#[derive(Debug, Clone)]
pub struct IsoRegion {
    pub data_start_lba: u32,
    pub total_sectors: u32,
}
#[derive(Debug, Clone)]
pub struct DiskLayout {
    pub partitions: Vec<Partition>,
    pub iso_region: IsoRegion,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UefiBootStrategy {
    ElToritoDirectEfi,
    EspPartition,
}

impl DiskLayout {
    pub fn from_partition_params(esp_align: u32, esp_size: Option<u32>, iso_data_lba: u32) -> Self {
        let mut parts = Vec::new();
        if let Some(sz) = esp_size {
            if sz > 0 {
                parts.push(Partition {
                    start_lba_512: esp_align as u64,
                    size_lba_512: sz as u64,
                });
            }
        }
        Self {
            partitions: parts,
            iso_region: IsoRegion {
                data_start_lba: iso_data_lba,
                total_sectors: 0,
            },
        }
    }
    pub fn esp_partition(&self) -> Option<&Partition> {
        self.partitions.first()
    }
    pub fn has_esp(&self) -> bool {
        self.esp_partition().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_with_esp() {
        let l = DiskLayout::from_partition_params(2048, Some(32768), 24);
        assert!(l.has_esp());
        let e = l.esp_partition().unwrap();
        assert_eq!(e.start_lba_512, 2048);
        assert_eq!(e.size_lba_512, 32768);
        assert_eq!(l.iso_region.data_start_lba, 24);
    }
    #[test]
    fn test_no_esp() {
        assert!(!DiskLayout::from_partition_params(0, None, 20).has_esp());
    }
    #[test]
    fn test_zero_esp() {
        assert!(!DiskLayout::from_partition_params(2048, Some(0), 24).has_esp());
    }
}
