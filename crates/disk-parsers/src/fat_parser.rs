//! FAT32 / exFAT Boot Parameter Block parser.
//!
//! Giải mã cấu trúc BPB (BIOS Parameter Block) của FAT32 và exFAT.
//! Hỗ trợ đọc Backup Boot Sector từ Sector 6 (FAT32).

use serde::Serialize;
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

/// FAT32 Boot Sector / BPB (512 bytes)
#[repr(C, packed)]
#[derive(FromBytes, Immutable, KnownLayout, Unaligned, Debug, Clone, Copy)]
pub struct Fat32BootSector {
    /// Jump instruction
    pub jump: [u8; 3],
    /// OEM Name (8 bytes, thường "MSDOS5.0" hoặc "mkfs.fat")
    pub oem_name: [u8; 8],
    /// Bytes per sector (512, 1024, 2048, 4096)
    pub bytes_per_sector: u16,
    /// Sectors per cluster
    pub sectors_per_cluster: u8,
    /// Reserved sectors (thường 32 cho FAT32)
    pub reserved_sectors: u16,
    /// Số bản FAT (thường 2)
    pub num_fats: u8,
    /// Root entry count (0 cho FAT32)
    pub root_entry_count: u16,
    /// Total sectors 16-bit (0 cho FAT32)
    pub total_sectors_16: u16,
    /// Media descriptor
    pub media_descriptor: u8,
    /// FAT size 16-bit (0 cho FAT32)
    pub fat_size_16: u16,
    /// Sectors per track
    pub sectors_per_track: u16,
    /// Number of heads
    pub number_of_heads: u16,
    /// Hidden sectors
    pub hidden_sectors: u32,
    /// Total sectors 32-bit
    pub total_sectors_32: u32,
    // ---- FAT32 Extended BPB ----
    /// FAT size 32-bit (sectors per FAT)
    pub fat_size_32: u32,
    /// Extended flags
    pub ext_flags: u16,
    /// FS Version
    pub fs_version: u16,
    /// Root directory cluster number (thường 2)
    pub root_cluster: u32,
    /// FSInfo sector number (thường 1)
    pub fs_info_sector: u16,
    /// Backup boot sector location (thường 6)
    pub backup_boot_sector: u16,
    /// Reserved
    pub reserved_bytes: [u8; 12],
    /// Drive number
    pub drive_number: u8,
    /// Reserved1
    pub reserved1: u8,
    /// Boot signature (0x29 = extended)
    pub boot_signature: u8,
    /// Volume serial number
    pub volume_serial: u32,
    /// Volume label (11 bytes)
    pub volume_label: [u8; 11],
    /// FS Type ("FAT32   ")
    pub fs_type: [u8; 8],
}

impl Fat32BootSector {
    /// Kiểm tra xem đây có phải FAT32 hợp lệ không
    pub fn is_valid(&self) -> bool {
        self.bytes_per_sector >= 512
            && self.bytes_per_sector <= 4096
            && self.sectors_per_cluster > 0
            && self.num_fats > 0
            && self.fat_size_32 > 0
            && self.root_entry_count == 0 // FAT32 đặt root entries = 0
    }

    /// Kiểm tra OEM name / FS type
    pub fn is_fat32_fs_type(&self) -> bool {
        self.fs_type.starts_with(b"FAT32")
    }

    /// Parse từ sector data
    pub fn from_sector(data: &[u8]) -> Option<&Self> {
        if data.len() < std::mem::size_of::<Self>() {
            return None;
        }
        Self::ref_from_bytes(&data[..std::mem::size_of::<Self>()]).ok()
    }

    /// Tính tổng dung lượng volume (bytes)
    pub fn volume_size_bytes(&self) -> u64 {
        self.total_sectors_32 as u64 * self.bytes_per_sector as u64
    }

    /// Cluster size (bytes)
    pub fn cluster_size(&self) -> u32 {
        self.sectors_per_cluster as u32 * self.bytes_per_sector as u32
    }

    /// LBA bắt đầu vùng data (sau reserved + FAT tables)
    pub fn data_start_lba(&self) -> u32 {
        self.reserved_sectors as u32 + self.num_fats as u32 * self.fat_size_32
    }

    /// Volume label dạng string (trimmed)
    pub fn volume_label_string(&self) -> String {
        String::from_utf8_lossy(&self.volume_label)
            .trim()
            .to_string()
    }
}

/// Kết quả phân tích FAT32 VBR cho AI
#[derive(Debug, Serialize)]
pub struct Fat32Analysis {
    pub valid: bool,
    pub oem_name: String,
    pub fs_type: String,
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub cluster_size_bytes: u32,
    pub total_sectors: u32,
    pub volume_size_gb: f64,
    pub fat_count: u8,
    pub fat_size_sectors: u32,
    pub reserved_sectors: u16,
    pub root_cluster: u32,
    pub data_start_lba: u32,
    pub backup_boot_sector: u16,
    pub volume_label: String,
    pub volume_serial: String,
    pub anomalies: Vec<String>,
}

/// Phân tích FAT32 BPB và trả về báo cáo JSON gọn.
pub fn analyze_fat32(data: &[u8]) -> Fat32Analysis {
    let mut anomalies = Vec::new();

    let bpb = match Fat32BootSector::from_sector(data) {
        Some(b) => b,
        None => {
            return Fat32Analysis {
                valid: false,
                oem_name: String::new(),
                fs_type: String::new(),
                bytes_per_sector: 0,
                sectors_per_cluster: 0,
                cluster_size_bytes: 0,
                total_sectors: 0,
                volume_size_gb: 0.0,
                fat_count: 0,
                fat_size_sectors: 0,
                reserved_sectors: 0,
                root_cluster: 0,
                data_start_lba: 0,
                backup_boot_sector: 0,
                volume_label: String::new(),
                volume_serial: String::new(),
                anomalies: vec!["Cannot parse FAT32 BPB: insufficient data".into()],
            };
        }
    };

    if !bpb.is_fat32_fs_type() {
        let fs = String::from_utf8_lossy(&bpb.fs_type);
        anomalies.push(format!("FS Type is '{}', not 'FAT32   '", fs.trim()));
    }

    if bpb.backup_boot_sector != 6 {
        anomalies.push(format!(
            "Backup Boot Sector at sector {} (expected 6)",
            { bpb.backup_boot_sector }
        ));
    }

    if bpb.num_fats != 2 {
        anomalies.push(format!(
            "FAT count is {} (expected 2)",
            { bpb.num_fats }
        ));
    }

    if bpb.root_cluster != 2 {
        anomalies.push(format!(
            "Root cluster is {} (expected 2)",
            { bpb.root_cluster }
        ));
    }

    let volume_size_gb =
        bpb.total_sectors_32 as f64 * bpb.bytes_per_sector as f64 / (1024.0 * 1024.0 * 1024.0);

    Fat32Analysis {
        valid: bpb.is_valid(),
        oem_name: String::from_utf8_lossy(&bpb.oem_name).trim().to_string(),
        fs_type: String::from_utf8_lossy(&bpb.fs_type).trim().to_string(),
        bytes_per_sector: bpb.bytes_per_sector,
        sectors_per_cluster: bpb.sectors_per_cluster,
        cluster_size_bytes: bpb.cluster_size(),
        total_sectors: bpb.total_sectors_32,
        volume_size_gb,
        fat_count: bpb.num_fats,
        fat_size_sectors: bpb.fat_size_32,
        reserved_sectors: bpb.reserved_sectors,
        root_cluster: bpb.root_cluster,
        data_start_lba: bpb.data_start_lba(),
        backup_boot_sector: bpb.backup_boot_sector,
        volume_label: bpb.volume_label_string(),
        volume_serial: format!("{:08X}", { bpb.volume_serial }),
        anomalies,
    }
}
