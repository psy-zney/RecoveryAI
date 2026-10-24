//! SIMD-accelerated file signature scanner.
//!
//! Sử dụng Aho-Corasick automaton (tận dụng SIMD: AVX2/SSE4.2)
//! để quét hàng chục Magic Bytes cùng lúc với tốc độ GB/s.
//!
//! Magic bytes phổ biến cho data recovery:
//! - JPEG: FF D8 FF E0/E1
//! - PNG: 89 50 4E 47
//! - PDF: 25 50 44 46
//! - ZIP/DOCX/XLSX: 50 4B 03 04
//! - NTFS MFT: 46 49 4C 45 ("FILE")
//! - MP4: ftyp (00 00 00 XX 66 74 79 70)
//! - SQLite: 53 51 4C 69

use aho_corasick::AhoCorasick;
use serde::Serialize;

/// Định nghĩa một chữ ký file (Magic Bytes).
#[derive(Debug, Clone)]
pub struct FileSignature {
    /// Tên loại file
    pub name: &'static str,
    /// Phần mở rộng file
    pub extension: &'static str,
    /// Chuỗi magic bytes (header)
    pub magic: &'static [u8],
    /// Offset từ đầu file (thường 0, MP4 có thể offset 4)
    pub offset: usize,
    /// Kích thước file tối thiểu (bytes)
    pub min_size: u64,
    /// Kích thước file tối đa điển hình (bytes), 0 = unlimited
    pub typical_max_size: u64,
}

/// Danh sách các chữ ký file phổ biến cho data recovery.
pub fn default_signatures() -> Vec<FileSignature> {
    vec![
        FileSignature {
            name: "JPEG Image",
            extension: "jpg",
            magic: &[0xFF, 0xD8, 0xFF, 0xE0],
            offset: 0,
            min_size: 1024,
            typical_max_size: 50 * 1024 * 1024,
        },
        FileSignature {
            name: "JPEG (EXIF)",
            extension: "jpg",
            magic: &[0xFF, 0xD8, 0xFF, 0xE1],
            offset: 0,
            min_size: 1024,
            typical_max_size: 50 * 1024 * 1024,
        },
        FileSignature {
            name: "PNG Image",
            extension: "png",
            magic: &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            offset: 0,
            min_size: 100,
            typical_max_size: 100 * 1024 * 1024,
        },
        FileSignature {
            name: "PDF Document",
            extension: "pdf",
            magic: b"%PDF",
            offset: 0,
            min_size: 256,
            typical_max_size: 500 * 1024 * 1024,
        },
        FileSignature {
            name: "ZIP Archive",
            extension: "zip",
            magic: &[0x50, 0x4B, 0x03, 0x04],
            offset: 0,
            min_size: 22,
            typical_max_size: 0,
        },
        FileSignature {
            name: "NTFS MFT Record",
            extension: "mft",
            magic: b"FILE",
            offset: 0,
            min_size: 1024,
            typical_max_size: 4096,
        },
        FileSignature {
            name: "SQLite Database",
            extension: "sqlite",
            magic: b"SQLite format 3\x00",
            offset: 0,
            min_size: 512,
            typical_max_size: 0,
        },
        FileSignature {
            name: "ELF Binary",
            extension: "elf",
            magic: &[0x7F, 0x45, 0x4C, 0x46],
            offset: 0,
            min_size: 64,
            typical_max_size: 0,
        },
        FileSignature {
            name: "Windows PE/EXE",
            extension: "exe",
            magic: b"MZ",
            offset: 0,
            min_size: 64,
            typical_max_size: 0,
        },
        FileSignature {
            name: "GIF Image",
            extension: "gif",
            magic: b"GIF89a",
            offset: 0,
            min_size: 64,
            typical_max_size: 50 * 1024 * 1024,
        },
        FileSignature {
            name: "BMP Image",
            extension: "bmp",
            magic: b"BM",
            offset: 0,
            min_size: 26,
            typical_max_size: 100 * 1024 * 1024,
        },
        FileSignature {
            name: "7-Zip Archive",
            extension: "7z",
            magic: &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C],
            offset: 0,
            min_size: 32,
            typical_max_size: 0,
        },
        FileSignature {
            name: "RAR Archive",
            extension: "rar",
            magic: b"Rar!\x1A\x07",
            offset: 0,
            min_size: 20,
            typical_max_size: 0,
        },
        FileSignature {
            name: "MP4 Video",
            extension: "mp4",
            magic: b"ftyp",
            offset: 4, // Bỏ qua 4 bytes box size ở đầu
            min_size: 1024,
            typical_max_size: 0,
        },
        FileSignature {
            name: "RIFF/AVI/WAV",
            extension: "avi",
            magic: b"RIFF",
            offset: 0,
            min_size: 44,
            typical_max_size: 0,
        },
    ]
}

/// Kết quả quét: vị trí tìm thấy một chữ ký file.
#[derive(Debug, Clone, Serialize)]
pub struct ScanHit {
    /// Byte offset trên đĩa (từ vùng quét)
    pub byte_offset: u64,
    /// LBA tương ứng (byte_offset / 512)
    pub lba: u64,
    /// Tên loại file
    pub file_type: String,
    /// Extension
    pub extension: String,
    /// Index của signature trong danh sách
    pub signature_index: usize,
}

/// Engine quét SIMD sử dụng Aho-Corasick.
///
/// Aho-Corasick xây dựng automaton DFA từ tất cả patterns,
/// cho phép quét tất cả magic bytes trong 1 lần duyệt qua data.
pub struct ScannerEngine {
    automaton: AhoCorasick,
    signatures: Vec<FileSignature>,
}

impl ScannerEngine {
    /// Tạo scanner với danh sách signatures mặc định.
    pub fn new() -> std::io::Result<Self> {
        Self::with_signatures(default_signatures())
    }

    /// Tạo scanner với danh sách signatures tuỳ chỉnh.
    pub fn with_signatures(signatures: Vec<FileSignature>) -> std::io::Result<Self> {
        if signatures.is_empty()
            || signatures.len() > 4096
            || signatures
                .iter()
                .any(|s| s.magic.is_empty() || s.magic.len() > 4096 || s.offset > 4096)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid scanner signature list",
            ));
        }
        let patterns: Vec<&[u8]> = signatures.iter().map(|s| s.magic).collect();
        let automaton = AhoCorasick::new(&patterns)
            .map_err(|error| std::io::Error::other(format!("Cannot build scanner: {error}")))?;
        Ok(Self {
            automaton,
            signatures,
        })
    }

    /// Quét một chunk dữ liệu và trả về tất cả các hit.
    ///
    /// `base_offset`: byte offset của chunk này trên đĩa (để tính vị trí tuyệt đối).
    pub fn scan_chunk(&self, data: &[u8], base_offset: u64) -> Vec<ScanHit> {
        let mut hits = Vec::new();

        for mat in self.automaton.find_overlapping_iter(data) {
            let sig_idx = mat.pattern().as_usize();
            let sig = &self.signatures[sig_idx];

            // Kiểm tra offset requirement
            let match_pos = mat.start();
            if sig.offset > 0 && match_pos < sig.offset {
                continue;
            }

            // Tính vị trí thực tế trên đĩa (trừ signature offset)
            let Some(actual_file_start) = base_offset
                .checked_add(match_pos as u64)
                .and_then(|position| position.checked_sub(sig.offset as u64))
            else {
                continue;
            };

            // Kiểm tra alignment (file thường bắt đầu ở ranh giới sector)
            let lba = actual_file_start / 512;

            hits.push(ScanHit {
                byte_offset: actual_file_start,
                lba,
                file_type: sig.name.to_string(),
                extension: sig.extension.to_string(),
                signature_index: sig_idx,
            });
        }

        hits
    }

    /// Trả về số lượng signatures đang được quét.
    pub fn signature_count(&self) -> usize {
        self.signatures.len()
    }
}

/// Kết quả quét tổng hợp cho AI (JSON gọn < 5KB)
#[derive(Debug, Serialize)]
pub struct ScanSummary {
    pub total_bytes_scanned: u64,
    pub total_hits: usize,
    pub hits_by_type: Vec<TypeCount>,
    /// Chỉ giữ tối đa 50 hits đầu tiên (tránh tràn context)
    pub sample_hits: Vec<ScanHit>,
}

#[derive(Debug, Serialize)]
pub struct TypeCount {
    pub file_type: String,
    pub count: usize,
}

/// Tạo báo cáo tóm tắt từ danh sách hits.
pub fn summarize_hits(hits: &[ScanHit], total_bytes: u64) -> ScanSummary {
    use std::collections::HashMap;

    let mut type_counts: HashMap<String, usize> = HashMap::new();
    for hit in hits {
        *type_counts.entry(hit.file_type.clone()).or_insert(0) += 1;
    }

    let mut hits_by_type: Vec<TypeCount> = type_counts
        .into_iter()
        .map(|(file_type, count)| TypeCount { file_type, count })
        .collect();
    hits_by_type.sort_by_key(|count| std::cmp::Reverse(count.count));

    ScanSummary {
        total_bytes_scanned: total_bytes,
        total_hits: hits.len(),
        hits_by_type,
        sample_hits: hits.iter().take(8).cloned().collect(),
    }
}

/// Carries at least 64 bytes across contiguous chunks and suppresses duplicate hits.
pub struct StreamingScanner {
    engine: ScannerEngine,
    tail: Vec<u8>,
    next_offset: Option<u64>,
    overlap: usize,
}
impl StreamingScanner {
    pub fn new() -> std::io::Result<Self> {
        let engine = ScannerEngine::new()?;
        let overlap = engine
            .signatures
            .iter()
            .map(|s| s.magic.len() + s.offset)
            .max()
            .unwrap_or(0)
            .max(64);
        Ok(Self {
            engine,
            tail: Vec::new(),
            next_offset: None,
            overlap,
        })
    }
    pub fn scan_next(&mut self, chunk: &[u8], offset: u64) -> std::io::Result<Vec<ScanHit>> {
        if self.next_offset.is_some_and(|expected| expected != offset) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Scanner chunks must be contiguous",
            ));
        }
        let end = offset.checked_add(chunk.len() as u64).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Scanner offset overflow")
        })?;
        let base = offset.checked_sub(self.tail.len() as u64).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Scanner offset underflow")
        })?;
        let mut data = std::mem::take(&mut self.tail);
        data.extend_from_slice(chunk);
        let hits = self
            .engine
            .scan_chunk(&data, base)
            .into_iter()
            .filter(|hit| {
                let signature = &self.engine.signatures[hit.signature_index];
                hit.byte_offset + signature.offset as u64 + signature.magic.len() as u64 > offset
            })
            .collect();
        self.tail = data[data.len().saturating_sub(self.overlap)..].to_vec();
        self.next_offset = Some(end);
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_custom_patterns_are_rejected() {
        assert!(ScannerEngine::with_signatures(Vec::new()).is_err());
    }
    #[test]
    fn split_signatures_are_found_once_at_every_boundary() {
        for magic in [
            b"%PDF".as_slice(),
            &[0xff, 0xd8, 0xff, 0xe0],
            &[0x50, 0x4b, 0x03, 0x04],
        ] {
            for split in 1..magic.len() {
                let mut data = vec![0; 512 * 4];
                let start = 512 - split;
                data[start..start + magic.len()].copy_from_slice(magic);
                let mut scanner = StreamingScanner::new().unwrap();
                let mut hits = Vec::new();
                for (i, chunk) in data.chunks(512).enumerate() {
                    hits.extend(scanner.scan_next(chunk, i as u64 * 512).unwrap());
                }
                assert_eq!(
                    hits.iter()
                        .filter(|h| h.byte_offset == start as u64)
                        .count(),
                    1
                );
            }
        }
    }
    #[test]
    fn test_detect_jpeg() {
        let scanner = ScannerEngine::new().unwrap();
        let mut data = vec![0u8; 1024];
        // Đặt JPEG header tại offset 0
        data[0] = 0xFF;
        data[1] = 0xD8;
        data[2] = 0xFF;
        data[3] = 0xE0;

        let hits = scanner.scan_chunk(&data, 0);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].extension, "jpg");
    }

    #[test]
    fn test_detect_pdf() {
        let scanner = ScannerEngine::new().unwrap();
        let mut data = vec![0u8; 512];
        data[..4].copy_from_slice(b"%PDF");

        let hits = scanner.scan_chunk(&data, 0);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].file_type, "PDF Document");
    }

    #[test]
    fn test_multiple_hits() {
        let scanner = ScannerEngine::new().unwrap();
        let mut data = vec![0u8; 2048];
        // JPEG tại offset 0
        data[0..4].copy_from_slice(&[0xFF, 0xD8, 0xFF, 0xE0]);
        // PDF tại offset 512
        data[512..516].copy_from_slice(b"%PDF");

        let hits = scanner.scan_chunk(&data, 0);
        assert!(hits.len() >= 2);
    }
}
