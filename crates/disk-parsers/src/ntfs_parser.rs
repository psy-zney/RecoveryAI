//! NTFS Boot Sector parser và Shannon Entropy calculator.
//!
//! Giải mã zero-copy NTFS VBR (Volume Boot Record), đọc vị trí $MFT,
//! $MFTMirr, và tính Shannon Entropy để phát hiện dữ liệu bị mã hóa
//! hoặc bị ghi đè rác.

use serde::Serialize;
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

/// NTFS Boot Sector (512 bytes đầu của partition NTFS)
#[repr(C, packed)]
#[derive(FromBytes, Immutable, KnownLayout, Unaligned, Debug, Clone, Copy)]
pub struct NtfsBootSector {
    /// Jump instruction (EB 52 90)
    pub jump_instruction: [u8; 3],
    /// OEM ID — phải là "NTFS    " (8 bytes, padded with spaces)
    pub oem_id: [u8; 8],
    /// Bytes per sector (thường là 512 hoặc 4096)
    pub bytes_per_sector: u16,
    /// Sectors per cluster
    pub sectors_per_cluster: u8,
    /// Reserved sectors (luôn 0 cho NTFS)
    pub reserved_sectors: u16,
    /// Always zero
    pub always_zero_1: [u8; 3],
    /// Not used
    pub not_used_1: u16,
    /// Media descriptor (0xF8 cho ổ cứng)
    pub media_descriptor: u8,
    /// Always zero
    pub always_zero_2: u16,
    /// Sectors per track (CHS)
    pub sectors_per_track: u16,
    /// Number of heads (CHS)
    pub number_of_heads: u16,
    /// Hidden sectors (offset từ đầu đĩa tới partition này)
    pub hidden_sectors: u32,
    /// Not used
    pub not_used_2: u32,
    /// Not used (thường 0x800080)
    pub not_used_3: u32,
    /// Total sectors trong volume
    pub total_sectors: u64,
    /// Logical Cluster Number của $MFT
    pub mft_lcn: u64,
    /// Logical Cluster Number của $MFTMirr (bản sao backup)
    pub mft_mirr_lcn: u64,
    /// Clusters per MFT record (nếu âm: 2^|value| bytes)
    pub clusters_per_mft_record: i8,
    /// Not used
    pub not_used_4: [u8; 3],
    /// Clusters per index record
    pub clusters_per_index_record: i8,
    /// Not used
    pub not_used_5: [u8; 3],
    /// Volume serial number (64-bit)
    pub volume_serial_number: u64,
    /// Checksum
    pub checksum: u32,
    /// Bootstrap code
    pub bootstrap_code: [u8; 426],
    /// End of sector marker — phải là 0xAA55
    pub end_of_sector_marker: u16,
}

impl NtfsBootSector {
    /// Kiểm tra OEM ID hợp lệ ("NTFS    ")
    pub fn is_valid_oem_id(&self) -> bool {
        &self.oem_id == b"NTFS    "
    }

    /// Kiểm tra boot signature 0xAA55
    pub fn is_valid_signature(&self) -> bool {
        self.end_of_sector_marker == 0xAA55
    }

    /// Kiểm tra tổng thể VBR có hợp lệ không
    pub fn is_valid(&self) -> bool {
        self.is_valid_oem_id()
            && self.is_valid_signature()
            && (self.bytes_per_sector == 512 || self.bytes_per_sector == 4096)
            && self.sectors_per_cluster > 0
    }

    /// Tính kích thước MFT record (bytes).
    ///
    /// Nếu `clusters_per_mft_record` > 0: record size = clusters × cluster_size
    /// Nếu `clusters_per_mft_record` < 0: record size = 2^|value| bytes (thường 1024)
    pub fn mft_record_size(&self) -> u64 {
        if self.clusters_per_mft_record > 0 {
            self.clusters_per_mft_record as u64
                * self.sectors_per_cluster as u64
                * self.bytes_per_sector as u64
        } else {
            1u64 << ((-self.clusters_per_mft_record) as u64)
        }
    }

    /// Tính byte offset tuyệt đối của $MFT trên đĩa (từ đầu partition).
    pub fn mft_byte_offset(&self) -> u64 {
        self.mft_lcn * self.sectors_per_cluster as u64 * self.bytes_per_sector as u64
    }

    /// Tính byte offset tuyệt đối của $MFTMirr
    pub fn mft_mirr_byte_offset(&self) -> u64 {
        self.mft_mirr_lcn * self.sectors_per_cluster as u64 * self.bytes_per_sector as u64
    }

    /// Parse từ sector data
    pub fn from_sector(data: &[u8]) -> Option<&Self> {
        if data.len() < std::mem::size_of::<Self>() {
            return None;
        }
        Self::ref_from_bytes(&data[..std::mem::size_of::<Self>()]).ok()
    }
}

/// Kết quả phân tích NTFS VBR cho AI
#[derive(Debug, Serialize)]
pub struct NtfsVbrAnalysis {
    pub valid: bool,
    pub oem_id: String,
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub total_sectors: u64,
    pub volume_size_gb: f64,
    pub mft_lcn: u64,
    pub mft_mirr_lcn: u64,
    pub mft_record_size_bytes: u64,
    pub mft_byte_offset: u64,
    pub mft_mirr_byte_offset: u64,
    pub volume_serial: String,
    pub media_descriptor: String,
    pub anomalies: Vec<String>,
    pub entropy: f64,
}

/// Phân tích NTFS VBR và trả về báo cáo JSON gọn.
pub fn analyze_ntfs_vbr(data: &[u8]) -> NtfsVbrAnalysis {
    let mut anomalies = Vec::new();
    let entropy = calculate_shannon_entropy(data);

    let vbr = match NtfsBootSector::from_sector(data) {
        Some(v) => v,
        None => {
            return NtfsVbrAnalysis {
                valid: false,
                oem_id: String::new(),
                bytes_per_sector: 0,
                sectors_per_cluster: 0,
                total_sectors: 0,
                volume_size_gb: 0.0,
                mft_lcn: 0,
                mft_mirr_lcn: 0,
                mft_record_size_bytes: 0,
                mft_byte_offset: 0,
                mft_mirr_byte_offset: 0,
                volume_serial: String::new(),
                media_descriptor: String::new(),
                anomalies: vec!["Cannot parse NTFS VBR: insufficient data".into()],
                entropy,
            };
        }
    };

    // Kiểm tra anomalies
    if !vbr.is_valid_oem_id() {
        let oem = String::from_utf8_lossy(&vbr.oem_id);
        anomalies.push(format!("Invalid OEM ID: '{}' (expected 'NTFS    ')", oem));
    }

    if !vbr.is_valid_signature() {
        anomalies.push(format!(
            "Invalid boot signature: 0x{:04X} (expected 0xAA55)",
            { vbr.end_of_sector_marker }
        ));
    }

    if vbr.media_descriptor != 0xF8 {
        anomalies.push(format!(
            "Unusual media descriptor: 0x{:02X} (expected 0xF8 for HDD)",
            { vbr.media_descriptor }
        ));
    }

    if vbr.reserved_sectors != 0 {
        anomalies.push(format!(
            "Non-zero reserved sectors: {} (should be 0 for NTFS)",
            { vbr.reserved_sectors }
        ));
    }

    if entropy > 7.5 {
        anomalies.push(format!(
            "Very high entropy ({:.2}): VBR may be encrypted or corrupted",
            entropy
        ));
    } else if entropy < 1.0 && !data.iter().all(|&b| b == 0) {
        anomalies.push(format!(
            "Very low entropy ({:.2}): VBR may be partially zeroed",
            entropy
        ));
    }

    let volume_size_gb = vbr.total_sectors as f64 * vbr.bytes_per_sector as f64
        / (1024.0 * 1024.0 * 1024.0);

    NtfsVbrAnalysis {
        valid: vbr.is_valid(),
        oem_id: String::from_utf8_lossy(&vbr.oem_id).trim().to_string(),
        bytes_per_sector: vbr.bytes_per_sector,
        sectors_per_cluster: vbr.sectors_per_cluster,
        total_sectors: vbr.total_sectors,
        volume_size_gb,
        mft_lcn: vbr.mft_lcn,
        mft_mirr_lcn: vbr.mft_mirr_lcn,
        mft_record_size_bytes: vbr.mft_record_size(),
        mft_byte_offset: vbr.mft_byte_offset(),
        mft_mirr_byte_offset: vbr.mft_mirr_byte_offset(),
        volume_serial: format!("{:016X}", { vbr.volume_serial_number }),
        media_descriptor: format!("0x{:02X}", { vbr.media_descriptor }),
        anomalies,
        entropy,
    }
}

/// Tính Shannon Entropy chuẩn trong thang [0.0, 8.0] bits/byte.
///
/// - **0.0**: Dữ liệu hoàn toàn đồng nhất (tất cả bytes giống nhau)
/// - **~4.0-5.0**: Dữ liệu text hoặc code thông thường
/// - **~7.5-8.0**: Dữ liệu đã mã hóa hoặc nén (random-like)
///
/// AI sử dụng giá trị này để phân loại sector:
/// - Entropy thấp → có cấu trúc, có thể parse filesystem
/// - Entropy cao → có thể đã mã hóa BitLocker, hoặc dữ liệu nén
pub fn calculate_shannon_entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }

    let mut counts = [0usize; 256];
    for &b in data {
        counts[b as usize] += 1;
    }

    let len = data.len() as f64;
    let mut entropy = 0.0;
    for &c in &counts {
        if c > 0 {
            let p = (c as f64) / len;
            entropy -= p * p.log2();
        }
    }
    entropy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entropy_uniform() {
        let data = vec![0u8; 1024];
        assert_eq!(calculate_shannon_entropy(&data), 0.0);
    }

    #[test]
    fn test_entropy_two_values() {
        let mut data = vec![0u8; 512];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i % 2) as u8;
        }
        let entropy = calculate_shannon_entropy(&data);
        assert!((entropy - 1.0).abs() < 0.01); // ~1.0 bit
    }

    #[test]
    fn test_entropy_high() {
        // Dữ liệu "giả random" với phân bố đều 256 giá trị
        let data: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        let entropy = calculate_shannon_entropy(&data);
        assert!(entropy > 7.99); // Gần 8.0 bits
    }

    #[test]
    fn test_entropy_empty() {
        assert_eq!(calculate_shannon_entropy(&[]), 0.0);
    }
}
