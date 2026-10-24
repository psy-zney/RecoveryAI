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

pub mod fat_parser;
pub mod fragment_matcher;
pub mod mbr_gpt;
pub mod ntfs_parser;
pub mod simd_scanner;
