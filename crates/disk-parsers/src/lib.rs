//! # disk-parsers
//!
//! Crate giải mã cấu trúc đĩa và quét SIMD tốc độ cao.
//!
//! Cung cấp:
//! - **mbr_gpt**: Zero-copy parsing cho MBR & GPT (Primary + Backup)
//! - **ntfs_parser**: Giải mã NTFS VBR, $MFT Record, entropy
//! - **fat_parser**: Giải mã FAT32 / exFAT BPB & chuỗi Cluster
//! - **simd_scanner**: Quét chữ ký file GB/s bằng Aho-Corasick + SIMD
//! - **fragment_matcher**: Đo độ khớp biên nối mảnh file bị phân mảnh

pub mod content_validation;
pub mod fat_parser;
pub mod fragment_matcher;
pub mod iso_bmff;
pub mod mbr_gpt;
pub mod ntfs_parser;
pub mod simd_scanner;

#[cfg(test)]
mod malformed_input_tests {
    #[test]
    fn generated_malformed_sectors_do_not_panic() {
        let mut state = 0x9e3779b97f4a7c15u64;
        let cases = std::env::var("RESC_FUZZ_CASES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(20000)
            .min(1000000);
        for iteration in 0..cases {
            let size = iteration % 4097;
            let mut bytes = vec![0u8; size];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            if iteration % 7 == 0 {
                bytes.fill(0);
            }
            // Preserve structural markers in part of the corpus so malformed
            // geometry reaches deeper validation, including zero divisors.
            if iteration % 3 == 0 {
                if let Some(magic) = bytes.get_mut(3..11) {
                    magic.copy_from_slice(b"NTFS    ");
                }
                if let Some(signature) = bytes.get_mut(510..512) {
                    signature.copy_from_slice(&[0x55, 0xaa]);
                }
            }
            if iteration % 5 == 0 {
                if let Some(magic) = bytes.get_mut(..8) {
                    magic.copy_from_slice(b"EFI PART");
                }
                if let Some(fs_type) = bytes.get_mut(82..90) {
                    fs_type.copy_from_slice(b"FAT32   ");
                }
            }
            let outcome = std::panic::catch_unwind(|| {
                super::mbr_gpt::analyze_mbr(&bytes);
                super::mbr_gpt::validate_mbr(&bytes, 100000).ok();
                super::mbr_gpt::GptHeader::from_sector(&bytes);
                super::ntfs_parser::analyze_ntfs_vbr(&bytes);
                super::fat_parser::analyze_fat32(&bytes);
                super::iso_bmff::analyze(&mut std::io::Cursor::new(&bytes), bytes.len() as u64)
                    .ok();
            });
            assert!(
                outcome.is_ok(),
                "parser panic for deterministic malformed case {iteration}, length {size}"
            );
        }
    }
}
