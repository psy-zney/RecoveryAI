//! Zero-copy parsing cho MBR (Master Boot Record) và GPT (GUID Partition Table).
//!
//! Ép kiểu trực tiếp 512 bytes sector thành struct C mà không cần cấp phát bộ nhớ.
//! Hỗ trợ:
//! - MBR: Đọc 4 partition entries, kiểm tra signature 0xAA55
//! - GPT: Đọc Primary Header (LBA 1) và Backup Header (LBA cuối)

use core_io::SECTOR_SIZE;
use serde::Serialize;
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

// ============================================================================
// MBR Structures
// ============================================================================

/// MBR Partition Entry (16 bytes mỗi entry, tối đa 4 entries)
#[repr(C, packed)]
#[derive(FromBytes, Immutable, KnownLayout, Unaligned, Debug, Clone, Copy)]
pub struct MbrPartitionEntry {
    /// 0x80 = Active/Bootable, 0x00 = Inactive
    pub status: u8,
    /// CHS address đầu tiên
    pub chs_first: [u8; 3],
    /// Loại partition (0x07=NTFS, 0x0C=FAT32 LBA, 0xEE=GPT Protective)
    pub partition_type: u8,
    /// CHS address cuối cùng
    pub chs_last: [u8; 3],
    /// LBA bắt đầu của partition
    pub lba_start: u32,
    /// Số sector trong partition
    pub sector_count: u32,
}

/// Master Boot Record (512 bytes — Sector 0)
#[repr(C, packed)]
#[derive(FromBytes, Immutable, KnownLayout, Unaligned, Debug, Clone, Copy)]
pub struct MasterBootRecord {
    /// Bootstrap code (446 bytes)
    pub bootstrap_code: [u8; 446],
    /// 4 partition entries (64 bytes tổng)
    pub partitions: [MbrPartitionEntry; 4],
    /// Boot signature — phải là 0xAA55 (little-endian)
    pub signature: u16,
}

impl MasterBootRecord {
    /// Kiểm tra MBR signature hợp lệ (0xAA55)
    pub fn is_valid(&self) -> bool {
        self.signature == 0xAA55
    }

    /// Kiểm tra xem đây có phải GPT Protective MBR không
    pub fn is_gpt_protective(&self) -> bool {
        self.partitions[0].partition_type == 0xEE
    }

    /// Parse MBR từ 512 bytes sector data
    pub fn from_sector(data: &[u8]) -> Option<&Self> {
        if data.len() < SECTOR_SIZE {
            return None;
        }
        Self::ref_from_bytes(data.get(..std::mem::size_of::<Self>())?).ok()
    }
}

/// Kết quả phân tích MBR dạng JSON gọn cho AI.
#[derive(Debug, Serialize)]
pub struct MbrAnalysis {
    pub valid_signature: bool,
    pub is_gpt_protective: bool,
    pub partitions: Vec<MbrPartitionInfo>,
    pub anomalies: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MbrPartitionInfo {
    pub index: u8,
    pub status: String,
    pub partition_type: String,
    pub partition_type_id: u8,
    pub lba_start: u32,
    pub sector_count: u32,
    pub size_mb: f64,
}

/// Phân tích MBR và trả về báo cáo JSON gọn.
pub fn analyze_mbr(data: &[u8]) -> MbrAnalysis {
    let mut anomalies = Vec::new();

    let mbr = match MasterBootRecord::from_sector(data) {
        Some(m) => m,
        None => {
            return MbrAnalysis {
                valid_signature: false,
                is_gpt_protective: false,
                partitions: vec![],
                anomalies: vec!["Cannot parse MBR: insufficient data".into()],
            };
        }
    };

    if !mbr.is_valid() {
        anomalies.push(format!(
            "Invalid MBR signature: 0x{:04X} (expected 0xAA55)",
            { mbr.signature }
        ));
    }

    let mut partitions = Vec::new();
    for (i, entry) in mbr.partitions.iter().enumerate() {
        if entry.partition_type == 0x00 {
            continue; // Bỏ qua entry trống
        }

        let type_name = match entry.partition_type {
            0x01 => "FAT12",
            0x04 | 0x06 => "FAT16",
            0x07 => "NTFS/exFAT",
            0x0B => "FAT32 CHS",
            0x0C => "FAT32 LBA",
            0x0E => "FAT16 LBA",
            0x0F => "Extended LBA",
            0x82 => "Linux swap",
            0x83 => "Linux ext",
            0xEE => "GPT Protective",
            0xEF => "EFI System",
            _ => "Unknown",
        };

        let size_mb = entry.sector_count as f64 * 512.0 / (1024.0 * 1024.0);

        // Kiểm tra anomalies
        if entry.lba_start == 0 && entry.partition_type != 0xEE {
            anomalies.push(format!("Partition {} starts at LBA 0 (suspicious)", i));
        }
        if entry.sector_count == 0 {
            anomalies.push(format!(
                "Partition {} has 0 sectors but non-zero type 0x{:02X}",
                i, entry.partition_type
            ));
        }

        partitions.push(MbrPartitionInfo {
            index: i as u8,
            status: if entry.status == 0x80 {
                "Active".into()
            } else {
                "Inactive".into()
            },
            partition_type: type_name.into(),
            partition_type_id: entry.partition_type,
            lba_start: entry.lba_start,
            sector_count: entry.sector_count,
            size_mb,
        });
    }

    MbrAnalysis {
        valid_signature: mbr.is_valid(),
        is_gpt_protective: mbr.is_gpt_protective(),
        partitions,
        anomalies,
    }
}

/// Validates the signature and every nonempty partition against disk capacity.
pub fn validate_mbr(data: &[u8], total_sectors: u64) -> Result<MbrAnalysis, String> {
    let report = analyze_mbr(data);
    if !report.valid_signature {
        return Err("Invalid MBR boot signature".into());
    }
    let mut ranges = Vec::new();
    for p in &report.partitions {
        let end = p.lba_start as u64 + p.sector_count as u64;
        if p.lba_start == 0 || p.sector_count == 0 || end > total_sectors {
            return Err(format!(
                "MBR partition {} exceeds disk capacity or has invalid range",
                p.index
            ));
        }
        if !matches!(data.get(446 + p.index as usize * 16), Some(0 | 0x80)) {
            return Err("Invalid partition status".into());
        }
        ranges.push((p.lba_start as u64, end));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|r| r[0].1 > r[1].0) {
        return Err("Overlapping MBR partition ranges".into());
    }
    Ok(report)
}

// ============================================================================
// GPT Structures
// ============================================================================

/// GPT Header (92 bytes tại LBA 1 và LBA cuối)
#[repr(C, packed)]
#[derive(FromBytes, Immutable, KnownLayout, Unaligned, Debug, Clone, Copy)]
pub struct GptHeader {
    /// Signature "EFI PART" (8 bytes)
    pub signature: [u8; 8],
    /// Revision (thường là 0x00010000)
    pub revision: u32,
    /// Header size (thường là 92)
    pub header_size: u32,
    /// CRC32 của header
    pub header_crc32: u32,
    /// Reserved (phải là 0)
    pub reserved: u32,
    /// LBA của header này
    pub my_lba: u64,
    /// LBA của header dự phòng
    pub alternate_lba: u64,
    /// LBA bắt đầu dùng được cho partition
    pub first_usable_lba: u64,
    /// LBA cuối cùng dùng được
    pub last_usable_lba: u64,
    /// Disk GUID
    pub disk_guid: [u8; 16],
    /// LBA bắt đầu của partition entry array
    pub partition_entry_start_lba: u64,
    /// Số partition entries
    pub num_partition_entries: u32,
    /// Kích thước mỗi partition entry (thường 128 bytes)
    pub partition_entry_size: u32,
    /// CRC32 của partition entry array
    pub partition_entries_crc32: u32,
}

impl GptHeader {
    /// Kiểm tra signature "EFI PART" hợp lệ
    pub fn is_valid(&self) -> bool {
        &self.signature == b"EFI PART"
    }

    /// Parse GPT Header từ sector data
    pub fn from_sector(data: &[u8]) -> Option<&Self> {
        if data.len() < std::mem::size_of::<Self>() {
            return None;
        }
        let header = Self::ref_from_bytes(data.get(..std::mem::size_of::<Self>())?).ok()?;
        let size = header.header_size as usize;
        if !header.is_valid() || !(92..=data.len()).contains(&size) {
            return None;
        }
        let mut crc_data = data.get(..size)?.to_vec();
        crc_data.get_mut(16..20)?.fill(0);
        if crc32fast::hash(&crc_data) == header.header_crc32 {
            Some(header)
        } else {
            None
        }
    }

    pub fn validate_geometry(&self, lba: u64, total_sectors: u64) -> Result<(), String> {
        if self.my_lba != lba
            || self.alternate_lba >= total_sectors
            || self.alternate_lba == lba
            || self.first_usable_lba > self.last_usable_lba
            || self.last_usable_lba >= total_sectors
            || self.partition_entry_size < 128
            || !self.partition_entry_size.is_multiple_of(128)
            || self.num_partition_entries == 0
            || self.revision != 0x00010000
            || self.reserved != 0
        {
            return Err("Invalid GPT header geometry".into());
        }
        Ok(())
    }

    /// Format GUID thành chuỗi chuẩn
    pub fn disk_guid_string(&self) -> String {
        format_guid(&self.disk_guid)
    }
}

/// GPT Partition Entry (128 bytes)
#[repr(C, packed)]
#[derive(FromBytes, Immutable, KnownLayout, Unaligned, Debug, Clone, Copy)]
pub struct GptPartitionEntry {
    /// Partition type GUID
    pub type_guid: [u8; 16],
    /// Unique partition GUID
    pub unique_guid: [u8; 16],
    /// Starting LBA
    pub starting_lba: u64,
    /// Ending LBA (inclusive)
    pub ending_lba: u64,
    /// Attributes
    pub attributes: u64,
    /// Name (UTF-16LE, 36 characters max)
    pub name: [u8; 72],
}

impl GptPartitionEntry {
    /// Kiểm tra xem entry có trống không (all zeros GUID)
    pub fn is_empty(&self) -> bool {
        self.type_guid.iter().all(|&b| b == 0)
    }

    /// Trả về tên partition (UTF-16LE → String)
    pub fn name_string(&self) -> String {
        let u16_chars: Vec<u16> = self
            .name
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&c| c != 0)
            .collect();
        String::from_utf16_lossy(&u16_chars)
    }

    /// Kích thước partition tính bằng sectors
    pub fn size_sectors(&self) -> u64 {
        if self.ending_lba >= self.starting_lba {
            self.ending_lba
                .checked_sub(self.starting_lba)
                .and_then(|n| n.checked_add(1))
                .unwrap_or(0)
        } else {
            0
        }
    }
}

/// Kết quả phân tích GPT dạng JSON gọn cho AI.
#[derive(Debug, Serialize)]
pub struct GptAnalysis {
    pub primary_valid: bool,
    pub backup_valid: bool,
    pub disk_guid: String,
    pub partitions: Vec<GptPartitionInfo>,
    pub anomalies: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GptPartitionInfo {
    pub index: u32,
    pub name: String,
    pub type_guid: String,
    pub starting_lba: u64,
    pub ending_lba: u64,
    pub size_mb: f64,
}

/// Format 16-byte GUID thành chuỗi chuẩn.
fn format_guid(bytes: &[u8; 16]) -> String {
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        bytes[3], bytes[2], bytes[1], bytes[0],
        bytes[5], bytes[4],
        bytes[7], bytes[6],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}
