pub mod boot_catalog;
pub mod builder;
pub mod dir_record;
pub mod gpt;
pub mod mbr;
pub mod volume_descriptor;

pub const ESP_START_LBA: u32 = 34;
