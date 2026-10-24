#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    disk_parsers::mbr_gpt::analyze_mbr(data);
    disk_parsers::mbr_gpt::validate_mbr(data, 100000).ok();
    disk_parsers::mbr_gpt::GptHeader::from_sector(data);
    disk_parsers::ntfs_parser::analyze_ntfs_vbr(data);
    disk_parsers::fat_parser::analyze_fat32(data);
});
