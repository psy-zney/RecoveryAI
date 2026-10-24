//! MCP Tools Schema — Định nghĩa 8 tools chuẩn theo Section 5 và SKILL.md.
//!
//! Mỗi tool có:
//! - `name`: Tên duy nhất
//! - `description`: Mô tả cho AI hiểu khi nào cần dùng
//! - `inputSchema`: JSON Schema mô tả parameters

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Định nghĩa một MCP Tool
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Trả về danh sách tất cả MCP Tools (bao gồm 8 tools chuẩn và legacy aliases).
pub fn all_tools() -> Vec<ToolDefinition> {
    vec![
        // ===== Nhóm 1: Khởi tạo & Chẩn đoán Tổng thể (Triage & Overview) =====
        ToolDefinition {
            name: "list_physical_drives".into(),
            description: "Liệt kê toàn bộ các ổ đĩa vật lý và thiết bị USB/ổ cứng ngoài kết nối với máy tính (bao gồm ổ SSD/HDD laptop tháo rời cắm qua Box USB, USB Flash drive, ổ cứng nội bộ). Trả về device_id (\\\\.\\PhysicalDriveX), tên Model, BusType (USB/NVMe/SATA), dung lượng GB và khuyến nghị mục tiêu cứu dữ liệu.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "open_recovery_session".into(),
            description: "Mở thiết bị đĩa vật lý (\\\\.\\PhysicalDriveX) hoặc file ảnh (.img) ở chế độ 100% Read-Only DMA. Khởi tạo Virtual Disk Overlay trên RAM (Copy-on-Write) và trả về session_id, dung lượng vật lý, số sector.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "target_path": {
                        "type": "string",
                        "description": "Đường dẫn đĩa vật lý (ví dụ: \\\\.\\PhysicalDrive1) hoặc file ảnh .img (ví dụ: C:/path/to/disk.img)"
                    }
                },
                "required": ["target_path"]
            }),
        },
        ToolDefinition {
            name: "scan_disk_topology".into(),
            description: "Quét nhanh cấu trúc toàn cục: LBA 0 (MBR), LBA 1..33 (GPT Primary), LBA cuối đĩa (Backup GPT/VBR) và các offset phân vùng chuẩn (LBA 63, 2048). Trả về bản đồ trạng thái VALID, ZEROED, CORRUPTED kèm khuyến nghị kịch bản xử lý.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID nhận được từ open_recovery_session"
                    }
                },
                "required": ["session_id"]
            }),
        },

        // ===== Nhóm 2: Phân tích Chuyên sâu & Khám nghiệm Cấu trúc (Deep Inspection) =====
        ToolDefinition {
            name: "inspect_sector_structure".into(),
            description: "Giải mã cấu trúc sector tại LBA chỉ định theo kiểu dữ liệu chọn trước (RAW_HEX, MBR, GPT, NTFS_VBR, FAT32_BPB, MFT_ENTRY). Đo Shannon Entropy (0.0 - 8.0) và chỉ ra các trường bị sai lệch (anomalies).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID"
                    },
                    "lba": {
                        "type": "integer",
                        "description": "Địa chỉ LBA của sector cần kiểm tra"
                    },
                    "parse_as": {
                        "type": "string",
                        "enum": ["RAW_HEX", "MBR", "GPT", "NTFS_VBR", "FAT32_BPB", "MFT_ENTRY"],
                        "description": "Định dạng cấu trúc muốn giải mã"
                    }
                },
                "required": ["session_id", "lba"]
            }),
        },
        ToolDefinition {
            name: "scan_filesystem_journals".into(),
            description: "Quét vùng $MFTMirr, $UsnJrnl, chỉ mục thư mục $I30 (với NTFS) hoặc bảng FAT thứ 2 (với FAT32) để tìm danh sách tệp tin vừa bị xóa hoặc bị ghi đè bảng mục lục chính.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID"
                    },
                    "partition_start_lba": {
                        "type": "integer",
                        "description": "LBA bắt đầu của phân vùng"
                    }
                },
                "required": ["session_id", "partition_start_lba"]
            }),
        },

        // ===== Nhóm 3: Tác vụ Quét Ngầm Bất đồng bộ (Async SIMD Jobs) =====
        ToolDefinition {
            name: "start_async_disk_job".into(),
            description: "Khởi chạy tác vụ quét đĩa nặng trên Rust thread pool chống timeout MCP: DD_RESCUE_CLONE (clone đĩa lỗi), SIMD_VBR_HUNT (săn VBR mồ côi), SIMD_MAGIC_CARVE (quét chữ ký file GB/s), FRAGMENT_PREFILTER (lọc ứng viên nối mảnh file). Trả về ngay lập tức job_id.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID"
                    },
                    "job_type": {
                        "type": "string",
                        "enum": ["DD_RESCUE_CLONE", "SIMD_VBR_HUNT", "SIMD_MAGIC_CARVE", "FRAGMENT_PREFILTER"],
                        "description": "Loại tác vụ ngầm cần chạy"
                    },
                    "params": {
                        "type": "object",
                        "description": "Các tham số bổ sung cho tác vụ"
                    }
                },
                "required": ["session_id", "job_type"]
            }),
        },
        ToolDefinition {
            name: "poll_async_job".into(),
            description: "Kiểm tra tiến độ tác vụ quét ngầm theo job_id. Trả về progress_pct (%), tốc độ MB/s, bad sectors gặp phải và danh sách kết quả tìm thấy hiện tại.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "job_id": {
                        "type": "string",
                        "description": "Mã job_id nhận từ start_async_disk_job"
                    }
                },
                "required": ["job_id"]
            }),
        },

        // ===== Nhóm 4: Vá lỗi Ảo trên RAM & Trích xuất Dữ liệu (Virtual Patching & Extraction) =====
        ToolDefinition {
            name: "virtual_patch_and_verify".into(),
            description: "Áp dụng bản vá ảo lên RAM (Copy-on-Write) hoặc sao chép sector dự phòng (copy_from_lba -> to_lba). Tự động chạy bộ thẩm định ảo kiểm tra VBR/MBR. Trả về mount_status và danh sách tệp tin đọc được nếu vá đúng.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID"
                    },
                    "copy_from_lba": {
                        "type": "integer",
                        "description": "LBA sector nguồn để sao chép dự phòng (ví dụ Backup VBR)"
                    },
                    "to_lba": {
                        "type": "integer",
                        "description": "LBA sector đích cần vá đè trên RAM"
                    },
                    "patches": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "lba": { "type": "integer" },
                                "offset": { "type": "integer" },
                                "hex_data": { "type": "string" }
                            },
                            "required": ["lba", "offset", "hex_data"]
                        },
                        "description": "Danh sách các byte vá trực tiếp"
                    },
                    "description": {
                        "type": "string",
                        "description": "Lý do áp dụng bản vá"
                    }
                },
                "required": ["session_id"]
            }),
        },
        ToolDefinition {
            name: "extract_recovered_data".into(),
            description: "Trích xuất dữ liệu đã phục hồi từ phân vùng ảo đã vá hoặc chuỗi LBA ghép mảnh ra thư mục an toàn ngoài đĩa.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Session ID"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["TSK_TREE_EXTRACT", "CARVE_BY_LBA_RANGES"],
                        "description": "Chế độ trích xuất"
                    },
                    "params": {
                        "type": "object",
                        "description": "Thông số bổ sung (như danh sách LBA ranges, tên file đích, inode)"
                    },
                    "output_dir": {
                        "type": "string",
                        "description": "Thư mục đích lưu file đã cứu"
                    }
                },
                "required": ["session_id", "mode", "output_dir"]
            }),
        },

        // ===== Legacy Aliases (Để tương thích ngược) =====
        ToolDefinition {
            name: "disk_scan_partitions".into(),
            description: "[Alias] Quét layout phân vùng đĩa (MBR + GPT).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "drive_number": { "type": "integer" },
                    "image_path": { "type": "string" }
                }
            }),
        },
        ToolDefinition {
            name: "disk_read_boot_sector".into(),
            description: "[Alias] Đọc và phân tích Boot Sector (VBR).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "partition_lba": { "type": "integer" }
                },
                "required": ["partition_lba"]
            }),
        },
        ToolDefinition {
            name: "disk_apply_patch".into(),
            description: "[Alias] Áp dụng bản vá ảo lên RAM.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "lba": { "type": "integer" },
                    "offset": { "type": "integer" },
                    "hex_data": { "type": "string" },
                    "description": { "type": "string" }
                },
                "required": ["lba", "offset", "hex_data", "description"]
            }),
        },
        ToolDefinition {
            name: "disk_rollback_patches".into(),
            description: "[Alias] Rollback tất cả bản vá ảo trên RAM.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "rollback_all": { "type": "boolean" }
                }
            }),
        },
    ]
}

// ============================================================================
// Parameter Deserialization Structs
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct OpenRecoverySessionParams {
    pub target_path: String,
}

#[derive(Debug, Deserialize)]
pub struct ScanDiskTopologyParams {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
pub struct InspectSectorStructureParams {
    pub session_id: String,
    pub lba: u64,
    pub parse_as: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ScanFilesystemJournalsParams {
    pub session_id: String,
    pub partition_start_lba: u64,
}

#[derive(Debug, Deserialize)]
pub struct StartAsyncDiskJobParams {
    pub session_id: String,
    pub job_type: String,
    pub params: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct PollAsyncJobParams {
    pub job_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SinglePatch {
    pub lba: u64,
    pub offset: usize,
    pub hex_data: String,
}

#[derive(Debug, Deserialize)]
pub struct VirtualPatchAndVerifyParams {
    pub session_id: String,
    pub copy_from_lba: Option<u64>,
    pub to_lba: Option<u64>,
    pub patches: Option<Vec<SinglePatch>>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExtractRecoveredDataParams {
    pub session_id: String,
    pub mode: String,
    pub params: Option<Value>,
    pub output_dir: String,
}

// Legacy structs
#[derive(Debug, Deserialize)]
pub struct ScanPartitionsParams {
    pub drive_number: Option<i32>,
    pub image_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReadBootSectorParams {
    pub partition_lba: u64,
}

#[derive(Debug, Deserialize)]
pub struct ApplyPatchParams {
    pub lba: u64,
    pub offset: usize,
    pub hex_data: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct RollbackPatchesParams {
    pub rollback_all: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ScanSignaturesParams {
    pub start_lba: u64,
    pub count_sectors: Option<u64>,
    pub file_types: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct AnalyzeEntropyParams {
    pub start_lba: u64,
    pub count_sectors: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ExtractFileParams {
    pub partition_offset: u64,
    pub inode: u64,
    pub output_filename: String,
}

#[derive(Debug, Deserialize)]
pub struct RunCarvingParams {
    pub file_types: Option<Vec<String>>,
    pub output_dir: String,
}
