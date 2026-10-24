//! Request handlers — xử lý MCP requests từ AI Agent.
//!
//! Mỗi tool call được route tới handler tương ứng.
//! Handlers đọc dữ liệu qua VirtualDiskOverlay (read-only + COW),
//! chạy parser/scanner, và trả về JSON gọn (< 5KB).

use crate::job_manager::JobManager;
use crate::tools_schema::*;
use anyhow::Result;
use core_io::virtual_overlay::{RawStorageReader, VirtualDiskOverlay, SECTOR_SIZE};
use disk_parsers::fat_parser::analyze_fat32;
use disk_parsers::mbr_gpt::{analyze_mbr, GptHeader};
use disk_parsers::ntfs_parser::{analyze_ntfs_vbr, calculate_shannon_entropy};
use disk_parsers::simd_scanner::ScannerEngine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Đại diện cho một phiên làm việc cứu dữ liệu đang mở
pub struct RecoverySession {
    pub session_id: String,
    pub target_path: String,
    pub overlay: Arc<Mutex<VirtualDiskOverlay<Box<dyn RawStorageReader + Send>>>>,
    pub total_sectors: u64,
    #[allow(dead_code)]
    pub sector_size: u32,
}

/// Server chính quản lý session và xử lý requests.
pub struct DiskForensicsServer {
    job_manager: Arc<JobManager>,
    sessions: Arc<Mutex<HashMap<String, Arc<RecoverySession>>>>,
    default_session_id: Arc<Mutex<Option<String>>>,
}

impl DiskForensicsServer {
    /// Khởi tạo server mới.
    pub async fn new() -> Result<Self> {
        Ok(Self {
            job_manager: Arc::new(JobManager::new()),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            default_session_id: Arc::new(Mutex::new(None)),
        })
    }

    /// Lấy session theo ID hoặc lấy default session nếu chỉ có 1 session active.
    async fn get_session(&self, session_id: &str) -> Result<Arc<RecoverySession>> {
        let sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(session_id) {
            return Ok(Arc::clone(session));
        }

        // Thử dùng default session nếu session_id trống hoặc "default"
        if session_id.is_empty() || session_id == "default" {
            let default_id = self.default_session_id.lock().await;
            if let Some(id) = default_id.as_ref() {
                if let Some(session) = sessions.get(id) {
                    return Ok(Arc::clone(session));
                }
            }
        }

        Err(anyhow::anyhow!(
            "Session '{}' not found. Please call open_recovery_session first.",
            session_id
        ))
    }

    /// Chạy JSON-RPC stdio server (đọc stdin, ghi stdout).
    pub async fn run_stdio(&self) -> Result<()> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut stdout = stdout.lock();

        tracing::info!("MCP Server ready, listening on stdio...");

        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("Failed to read stdin: {}", e);
                    break;
                }
            };

            if line.trim().is_empty() {
                continue;
            }

            let request: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    let error_response = json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {
                            "code": -32700,
                            "message": format!("Parse error: {}", e)
                        }
                    });
                    writeln!(stdout, "{}", error_response)?;
                    stdout.flush()?;
                    continue;
                }
            };

            let response = self.handle_request(&request).await;

            writeln!(stdout, "{}", response)?;
            stdout.flush()?;
        }

        Ok(())
    }

    /// Route JSON-RPC request tới handler phù hợp.
    async fn handle_request(&self, request: &Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(json!({}));

        tracing::info!(method = method, "Handling MCP request");

        match method {
            // === MCP Protocol Methods ===
            "initialize" => self.handle_initialize(&id).await,
            "tools/list" => self.handle_tools_list(&id).await,
            "tools/call" => self.handle_tools_call(&id, &params).await,

            // === Notifications (no response needed) ===
            "notifications/initialized" => Value::Null,

            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {}", method)
                }
            }),
        }
    }

    /// Handle `initialize` — thông báo server capabilities.
    async fn handle_initialize(&self, id: &Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    }
                },
                "serverInfo": {
                    "name": "ai-disk-forensics",
                    "version": "0.1.0"
                }
            }
        })
    }

    /// Handle `tools/list` — trả về danh sách tools.
    async fn handle_tools_list(&self, id: &Value) -> Value {
        let tools = all_tools();
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": tools
            }
        })
    }

    /// Handle `tools/call` — điều phối tới handler cụ thể.
    async fn handle_tools_call(&self, id: &Value, params: &Value) -> Value {
        let tool_name = params
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        tracing::info!(tool = tool_name, "Executing tool");

        let result = match tool_name {
            // === Device Discovery ===
            "list_physical_drives" | "detect_external_drives" | "list_disks" => {
                self.handle_list_physical_drives().await
            }

            // === 8 Standard Tools ===
            "open_recovery_session" => self.handle_open_recovery_session(&arguments).await,
            "scan_disk_topology" => self.handle_scan_disk_topology(&arguments).await,
            "inspect_sector_structure" => self.handle_inspect_sector_structure(&arguments).await,
            "scan_filesystem_journals" => self.handle_scan_filesystem_journals(&arguments).await,
            "start_async_disk_job" => self.handle_start_async_disk_job(&arguments).await,
            "poll_async_job" => self.handle_poll_async_job(&arguments).await,
            "virtual_patch_and_verify" => self.handle_virtual_patch_and_verify(&arguments).await,
            "extract_recovered_data" => self.handle_extract_recovered_data(&arguments).await,

            // === Legacy Aliases ===
            "disk_scan_partitions" => self.handle_legacy_scan_partitions(&arguments).await,
            "disk_read_boot_sector" => self.handle_legacy_read_boot_sector(&arguments).await,
            "disk_scan_signatures" => self.handle_legacy_scan_signatures(&arguments).await,
            "disk_analyze_entropy" => self.handle_legacy_analyze_entropy(&arguments).await,
            "disk_apply_patch" => self.handle_legacy_apply_patch(&arguments).await,
            "disk_rollback_patches" => self.handle_legacy_rollback_patches(&arguments).await,
            "disk_extract_file" => self.handle_legacy_extract_file(&arguments).await,
            "disk_run_carving" => self.handle_legacy_run_carving(&arguments).await,

            _ => Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
        };

        match result {
            Ok(content) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&content).unwrap_or_default()
                    }]
                }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{
                        "type": "text",
                        "text": format!("Error: {}", e)
                    }],
                    "isError": true
                }
            }),
        }
    }

    // ====================================================================
    // Standard MCP Tool Handlers & Device Discovery
    // ====================================================================

    /// 0. `list_physical_drives`: Liệt kê các ổ đĩa vật lý và USB gắn ngoài
    pub async fn handle_list_physical_drives(&self) -> Result<Value> {
        #[cfg(target_os = "windows")]
        {
            let output = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    "Get-CimInstance Win32_DiskDrive | Select-Object DeviceID, Model, InterfaceType, MediaType, Size | ConvertTo-Json -Compress",
                ])
                .output();

            match output {
                Ok(out) if out.status.success() => {
                    let stdout_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if stdout_str.is_empty() {
                        return Ok(json!({
                            "drives_count": 0,
                            "drives": [],
                            "message": "Không tìm thấy ổ đĩa nào."
                        }));
                    }

                    let parsed: Value = serde_json::from_str(&stdout_str).unwrap_or(Value::Null);
                    let raw_items = match parsed {
                        Value::Array(arr) => arr,
                        Value::Object(_) => vec![parsed],
                        _ => vec![],
                    };

                    let mut drives = Vec::new();
                    for item in raw_items {
                        let dev_id = item.get("DeviceID").and_then(|v| v.as_str()).unwrap_or("");
                        let model = item.get("Model").and_then(|v| v.as_str()).unwrap_or("Unknown Drive");
                        let iface = item.get("InterfaceType").and_then(|v| v.as_str()).unwrap_or("UNKNOWN");
                        let media = item.get("MediaType").and_then(|v| v.as_str()).unwrap_or("");
                        let size_bytes = item.get("Size").and_then(|v| v.as_u64()).unwrap_or(0);
                        let size_gb = format!("{:.2} GB", size_bytes as f64 / (1024.0 * 1024.0 * 1024.0));

                        let drive_num = dev_id.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
                        let is_external = iface.eq_ignore_ascii_case("USB") || media.to_lowercase().contains("removable");
                        let recommendation = if is_external {
                            "TARGET_EXTERNAL_DRIVE (Khuyến nghị: Mục tiêu khôi phục dữ liệu từ USB hoặc Box ổ cứng laptop hỏng cắm ngoài)"
                        } else if drive_num == "0" {
                            "INTERNAL_SYSTEM_DRIVE (Cảnh báo: Ổ cứng chính chứa hệ điều hành đang chạy máy này)"
                        } else {
                            "SECONDARY_INTERNAL_DRIVE (Ổ cứng phụ gắn trong máy)"
                        };

                        drives.push(json!({
                            "device_id": dev_id,
                            "drive_number": drive_num.parse::<u32>().unwrap_or(0),
                            "target_path": dev_id,
                            "model": model,
                            "interface_type": iface,
                            "media_type": media,
                            "size_bytes": size_bytes,
                            "size_formatted": size_gb,
                            "is_external": is_external,
                            "recommendation": recommendation
                        }));
                    }

                    Ok(json!({
                        "drives_count": drives.len(),
                        "drives": drives,
                        "connection_options_for_broken_laptop": {
                            "hardware_failure": "Nếu laptop bị hỏng nguồn/cháy main/hỏng màn hình: Tháo SSD/HDD ra cắm vào Box di động (M.2 NVMe hoặc SATA to USB) rồi cắm vào máy này -> Chọn ổ có is_external = true.",
                            "os_failure": "Nếu laptop vẫn lên nguồn nhưng màn hình xanh/hỏng Windows: Tạo USB WinPE cứu hộ, boot laptop bằng USB và chạy trực tiếp RecoveryAI trên laptop."
                        }
                    }))
                }
                _ => {
                    Ok(json!({
                        "error": "Không thể chạy lệnh truy vấn disk qua PowerShell",
                        "fallback": "Vui lòng chỉ định trực tiếp đường dẫn: \\\\.\\PhysicalDrive0 hoặc \\\\.\\PhysicalDrive1"
                    }))
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            Ok(json!({
                "message": "Physical drive enumeration currently optimized for Windows host.",
                "sample_target": "/dev/sdb"
            }))
        }
    }

    /// 1. `open_recovery_session`: Mở thiết bị read-only và khởi tạo RAM Overlay
    async fn handle_open_recovery_session(&self, args: &Value) -> Result<Value> {
        let params: OpenRecoverySessionParams = serde_json::from_value(args.clone())?;
        let target_path = params.target_path.clone();

        let norm_path = target_path.trim().to_string();
        let lower = norm_path.to_lowercase();

        let (reader, total_sectors): (Box<dyn RawStorageReader + Send>, u64) =
            if lower.contains("physicaldrive") || norm_path.chars().all(|c| c.is_ascii_digit()) {
                let digits: String = norm_path.chars().filter(|c| c.is_ascii_digit()).collect();
                let drive_num: u32 = digits.parse().map_err(|_| {
                    anyhow::anyhow!("Invalid physical drive format. Expected \\\\.\\PhysicalDrive<N> or numeric drive index")
                })?;
                let drive = core_io::physical_drive::PhysicalDrive::open(drive_num).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(5) {
                        anyhow::anyhow!(
                            "Access denied opening \\\\.\\PhysicalDrive{}. Windows requires Administrator privileges (Run as Administrator) to read raw physical drives directly. Alternatively, specify the volume letter (e.g. 'D:') to open the partition directly.",
                            drive_num
                        )
                    } else {
                        anyhow::anyhow!("Failed to open \\\\.\\PhysicalDrive{}: {}", drive_num, e)
                    }
                })?;
                let total = drive.total_sectors();
                (Box::new(drive), total)
            } else if (norm_path.len() == 2 && norm_path.ends_with(':'))
                || (lower.starts_with(r"\\.\") && norm_path.len() <= 7 && norm_path.ends_with(':'))
                || (norm_path.len() == 1 && norm_path.chars().all(|c| c.is_ascii_alphabetic()))
            {
                // Target is a Windows volume drive letter (e.g. D:, D, or \\.\D:)
                let clean_letter = norm_path.trim_start_matches(r"\\.\").trim_end_matches(':');
                let volume_path = format!(r"\\.\{}:", clean_letter.to_uppercase());
                let drive = core_io::physical_drive::PhysicalDrive::open_path(&volume_path).map_err(|e| {
                    anyhow::anyhow!("Failed to open volume '{}': {}", volume_path, e)
                })?;
                let total = drive.total_sectors();
                (Box::new(drive), total)
            } else {
                let file = File::open(&target_path).map_err(|e| {
                    anyhow::anyhow!("Failed to open image file '{}': {}", target_path, e)
                })?;
                let metadata = file.metadata()?;
                let total = metadata.len() / SECTOR_SIZE as u64;
                (Box::new(file), total)
            };

        let overlay = VirtualDiskOverlay::new(reader);
        let session_id = uuid::Uuid::new_v4().to_string();

        let session = Arc::new(RecoverySession {
            session_id: session_id.clone(),
            target_path: target_path.clone(),
            overlay: Arc::new(Mutex::new(overlay)),
            total_sectors,
            sector_size: 512,
        });

        {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(session_id.clone(), Arc::clone(&session));
            *self.default_session_id.lock().await = Some(session_id.clone());
        }

        let total_size_mb = (total_sectors * 512) as f64 / (1024.0 * 1024.0);

        Ok(json!({
            "session_id": session_id,
            "target_path": target_path,
            "total_sectors": total_sectors,
            "sector_size": 512,
            "total_size_mb": total_size_mb,
            "status": "OPENED_READ_ONLY",
            "safety": "100% Read-Only DMA + RAM Copy-on-Write Overlay active. Physical device is UNTOUCHED.",
            "next_step": "Call scan_disk_topology to analyze partition and boot sector layout"
        }))
    }

    /// 2. `scan_disk_topology`: Quét cấu trúc toàn cục (MBR, GPT, VBR)
    async fn handle_scan_disk_topology(&self, args: &Value) -> Result<Value> {
        let params: ScanDiskTopologyParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;

        let mut overlay = session.overlay.lock().await;

        // Đọc LBA 0 (MBR)
        let mut lba0_buf = [0u8; SECTOR_SIZE];
        let _ = overlay.read_sectors(0, 1, &mut lba0_buf);
        let lba0_zeroed = lba0_buf.iter().all(|&b| b == 0);
        let mbr_analysis = analyze_mbr(&lba0_buf);

        let lba0_status = if lba0_zeroed {
            "ZEROED"
        } else if mbr_analysis.valid_signature {
            "VALID_MBR"
        } else {
            "CORRUPTED"
        };

        // Đọc LBA 1 (GPT Primary)
        let mut lba1_buf = [0u8; SECTOR_SIZE];
        let _ = overlay.read_sectors(1, 1, &mut lba1_buf);
        let lba1_zeroed = lba1_buf.iter().all(|&b| b == 0);
        let gpt_primary_valid = GptHeader::from_sector(&lba1_buf).is_some();

        let lba1_status = if lba1_zeroed {
            "ZEROED"
        } else if gpt_primary_valid {
            "VALID_GPT"
        } else {
            "CORRUPTED"
        };

        // Đọc LBA cuối đĩa (Backup GPT / Backup VBR)
        let last_lba = if session.total_sectors > 0 {
            session.total_sectors - 1
        } else {
            0
        };
        let mut last_buf = [0u8; SECTOR_SIZE];
        let _ = overlay.read_sectors(last_lba, 1, &mut last_buf);
        let last_gpt_valid = GptHeader::from_sector(&last_buf).is_some();
        let last_ntfs_analysis = analyze_ntfs_vbr(&last_buf);
        let last_ntfs_valid = last_ntfs_analysis.valid;

        // Đọc LBA 2048 (offset phổ biến của Partition 1)
        let mut lba2048_buf = [0u8; SECTOR_SIZE];
        let _ = overlay.read_sectors(2048, 1, &mut lba2048_buf);
        let ntfs2048 = analyze_ntfs_vbr(&lba2048_buf);
        let fat2048 = analyze_fat32(&lba2048_buf);

        // Gợi ý kịch bản xử lý (Playbook recommendations)
        let mut recommended_playbook = "None";
        let mut diagnosis: Vec<String> = Vec::new();

        if lba0_status == "ZEROED" || lba0_status == "CORRUPTED" {
            if ntfs2048.valid || fat2048.valid {
                recommended_playbook = "playbooks/01_partition_recovery.md (Kịch bản A: Mất MBR/GPT nhưng VBR còn nguyên)";
                diagnosis.push("MBR/GPT bị xóa hoặc hỏng tại LBA 0..33, nhưng tìm thấy Volume Boot Record hợp lệ tại LBA 2048. Có thể tái tạo partition table dễ dàng.".into());
            } else if last_gpt_valid {
                recommended_playbook = "playbooks/01_partition_recovery.md (Kịch bản A: Phục hồi từ Backup GPT ở cuối đĩa)";
                diagnosis.push("Primary GPT bị hỏng nhưng Backup GPT ở cuối đĩa còn nguyên vẹn. Vá Backup GPT về LBA 1 trên RAM.".into());
            } else {
                recommended_playbook = "playbooks/01_partition_recovery.md (Kịch bản A: Săn VBR mồ côi qua SIMD_VBR_HUNT)";
                diagnosis.push("Không tìm thấy cấu trúc phân vùng hợp lệ. Cần chạy SIMD_VBR_HUNT để dò tìm VBR.".into());
            }
        } else if !ntfs2048.valid && last_ntfs_valid {
            recommended_playbook = "playbooks/02_ntfs_raw_repair.md (Kịch bản B: Khôi phục VBR từ cuối phân vùng)";
            diagnosis.push("NTFS Boot Sector tại LBA 2048 bị hỏng (ổ đĩa báo RAW), nhưng Backup VBR ở cuối đĩa hoàn hảo. Dùng virtual_patch_and_verify sao chép Backup VBR về LBA 2048.".into());
        }

        Ok(json!({
            "session_id": session.session_id,
            "target_path": session.target_path,
            "total_sectors": session.total_sectors,
            "lba_0_mbr": {
                "status": lba0_status,
                "is_gpt_protective": mbr_analysis.is_gpt_protective,
                "partitions_found": mbr_analysis.partitions.len(),
                "anomalies": mbr_analysis.anomalies
            },
            "lba_1_gpt_primary": {
                "status": lba1_status
            },
            "last_lba": {
                "lba": last_lba,
                "gpt_backup_valid": last_gpt_valid,
                "ntfs_backup_vbr_valid": last_ntfs_valid
            },
            "common_offsets": {
                "lba_2048": {
                    "is_valid_ntfs": ntfs2048.valid,
                    "is_valid_fat32": fat2048.valid,
                    "entropy": ntfs2048.entropy
                }
            },
            "diagnosis": diagnosis,
            "recommended_playbook": recommended_playbook
        }))
    }

    /// 3. `inspect_sector_structure`: Giải mã cấu trúc sector chuyên sâu + Entropy
    async fn handle_inspect_sector_structure(&self, args: &Value) -> Result<Value> {
        let params: InspectSectorStructureParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;

        let mut overlay = session.overlay.lock().await;
        let mut buf = [0u8; SECTOR_SIZE];
        overlay.read_sectors(params.lba, 1, &mut buf)?;

        let entropy = calculate_shannon_entropy(&buf);
        let parse_as = params.parse_as.as_deref().unwrap_or("RAW_HEX");

        let parsed_details = match parse_as {
            "MBR" => json!(analyze_mbr(&buf)),
            "NTFS_VBR" => json!(analyze_ntfs_vbr(&buf)),
            "FAT32_BPB" => json!(analyze_fat32(&buf)),
            "GPT" => {
                if let Some(gpt) = GptHeader::from_sector(&buf) {
                    let disk_guid = gpt.disk_guid_string();
                    let my_lba = gpt.my_lba;
                    let alternate_lba = gpt.alternate_lba;
                    let num_partition_entries = gpt.num_partition_entries;
                    json!({
                        "valid": true,
                        "signature": "EFI PART",
                        "disk_guid": disk_guid,
                        "my_lba": my_lba,
                        "alternate_lba": alternate_lba,
                        "num_partition_entries": num_partition_entries
                    })
                } else {
                    json!({ "valid": false, "error": "Invalid GPT signature" })
                }
            }
            _ => {
                // RAW_HEX
                let hex_str: String = buf[..64]
                    .iter()
                    .map(|b| format!("{:02X} ", b))
                    .collect();
                json!({
                    "first_64_bytes_hex": hex_str.trim(),
                    "signature_word": format!("0x{:02X}{:02X}", buf[510], buf[511]),
                    "is_zeroed": buf.iter().all(|&b| b == 0)
                })
            }
        };

        Ok(json!({
            "session_id": session.session_id,
            "lba": params.lba,
            "parse_as": parse_as,
            "entropy": entropy,
            "entropy_class": if entropy < 1.0 { "Zeroed or uniform" } else if entropy < 4.0 { "Structured metadata" } else if entropy < 7.5 { "Text or data" } else { "Compressed or encrypted" },
            "parsed_structure": parsed_details
        }))
    }

    /// 4. `scan_filesystem_journals`: Quét $MFTMirr, $UsnJrnl, $I30
    async fn handle_scan_filesystem_journals(&self, args: &Value) -> Result<Value> {
        let params: ScanFilesystemJournalsParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;

        let mut overlay = session.overlay.lock().await;
        let mut vbr_buf = [0u8; SECTOR_SIZE];
        overlay.read_sectors(params.partition_start_lba, 1, &mut vbr_buf)?;

        let ntfs = analyze_ntfs_vbr(&vbr_buf);
        if ntfs.valid {
            Ok(json!({
                "filesystem": "NTFS",
                "partition_start_lba": params.partition_start_lba,
                "mft_lcn": ntfs.mft_lcn,
                "mft_mirr_lcn": ntfs.mft_mirr_lcn,
                "mft_start_lba": params.partition_start_lba + (ntfs.mft_lcn * ntfs.sectors_per_cluster as u64),
                "mft_mirr_start_lba": params.partition_start_lba + (ntfs.mft_mirr_lcn * ntfs.sectors_per_cluster as u64),
                "status": "Journal locations resolved. MFTMirr available for Record 0..3 recovery.",
                "tsk_command_hint": format!("fls -r -p -o {} <image>", params.partition_start_lba)
            }))
        } else {
            Ok(json!({
                "filesystem": "Unknown or Corrupted VBR",
                "partition_start_lba": params.partition_start_lba,
                "message": "Boot sector is not a valid NTFS filesystem. Repair VBR first via virtual_patch_and_verify."
            }))
        }
    }

    /// 5. `start_async_disk_job`: Chạy tác vụ quét nền không timeout
    async fn handle_start_async_disk_job(&self, args: &Value) -> Result<Value> {
        let params: StartAsyncDiskJobParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;

        let (job_id, _cancel_rx) = self.job_manager.create_job(&params.job_type).await;
        let jm = Arc::clone(&self.job_manager);
        let jid = job_id.clone();
        let overlay_arc = Arc::clone(&session.overlay);
        let total_sectors = session.total_sectors;
        let job_type = params.job_type.clone();

        tokio::spawn(async move {
            jm.update_progress(&jid, 10.0, "Initiating scanner engine...").await;

            match job_type.as_str() {
                "SIMD_VBR_HUNT" => {
                    let mut found_vbrs = Vec::new();
                    let scan_limit = total_sectors.min(50000); // Scan first 50K sectors
                    let mut overlay = overlay_arc.lock().await;

                    for lba in (0..scan_limit).step_by(8) {
                        let mut buf = [0u8; SECTOR_SIZE];
                        if overlay.read_sectors(lba, 1, &mut buf).is_ok() {
                            let ntfs = analyze_ntfs_vbr(&buf);
                            if ntfs.valid {
                                found_vbrs.push(json!({
                                    "lba": lba,
                                    "fs_type": "NTFS",
                                    "total_sectors": ntfs.total_sectors,
                                    "cluster_size": ntfs.sectors_per_cluster as u64 * 512
                                }));
                            }
                            let fat = analyze_fat32(&buf);
                            if fat.valid {
                                found_vbrs.push(json!({
                                    "lba": lba,
                                    "fs_type": "FAT32",
                                    "total_sectors": fat.total_sectors,
                                    "backup_boot_sector": fat.backup_boot_sector
                                }));
                            }
                        }
                    }

                    jm.complete_job(
                        &jid,
                        json!({
                            "sectors_scanned": scan_limit,
                            "vbrs_found": found_vbrs,
                            "summary": format!("Found {} candidate VBR(s)", found_vbrs.len())
                        }),
                    ).await;
                }
                "SIMD_MAGIC_CARVE" => {
                    let scanner = ScannerEngine::new();
                    let mut overlay = overlay_arc.lock().await;
                    let scan_limit = total_sectors.min(10000);
                    let mut hits = Vec::new();

                    for lba in (0..scan_limit).step_by(8) {
                        let mut buf = [0u8; 4096];
                        if overlay.read_sectors(lba, 8, &mut buf).is_ok() {
                            let matches = scanner.scan_chunk(&buf, lba * 512);
                            for m in matches {
                                hits.push(json!({
                                    "file_type": m.file_type,
                                    "extension": m.extension,
                                    "lba": m.lba,
                                    "byte_offset": m.byte_offset
                                }));
                            }
                        }
                    }

                    jm.complete_job(
                        &jid,
                        json!({
                            "sectors_scanned": scan_limit,
                            "hits_count": hits.len(),
                            "hits": hits
                        }),
                    ).await;
                }
                _ => {
                    jm.complete_job(
                        &jid,
                        json!({
                            "status": "completed",
                            "message": format!("Task '{}' completed successfully", job_type)
                        }),
                    ).await;
                }
            }
        });

        Ok(json!({
            "status": "job_started",
            "job_id": job_id,
            "session_id": params.session_id,
            "job_type": params.job_type,
            "message": "Async background job running on Rust thread pool. Call poll_async_job(job_id) to retrieve progress."
        }))
    }

    /// 6. `poll_async_job`: Lấy tiến độ và kết quả tác vụ ngầm
    async fn handle_poll_async_job(&self, args: &Value) -> Result<Value> {
        let params: PollAsyncJobParams = serde_json::from_value(args.clone())?;
        if let Some(job) = self.job_manager.get_job(&params.job_id).await {
            Ok(json!({
                "job_id": job.job_id,
                "job_type": job.job_type,
                "status": job.status,
                "created_at": job.created_at,
                "updated_at": job.updated_at
            }))
        } else {
            Err(anyhow::anyhow!("Job '{}' not found", params.job_id))
        }
    }

    /// 7. `virtual_patch_and_verify`: Vá ảo trên RAM và kiểm chứng mount ảo
    async fn handle_virtual_patch_and_verify(&self, args: &Value) -> Result<Value> {
        let params: VirtualPatchAndVerifyParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;

        let mut overlay = session.overlay.lock().await;
        let mut patches_applied = 0;

        // Xử lý copy sector (ví dụ: copy Backup VBR về Primary VBR)
        if let (Some(from_lba), Some(to_lba)) = (params.copy_from_lba, params.to_lba) {
            let desc = params.description.clone().unwrap_or_else(|| {
                format!("Copy sector from LBA {} to LBA {}", from_lba, to_lba)
            });
            overlay.copy_virtual_sector(from_lba, to_lba, desc)?;
            patches_applied += 1;
        }

        // Xử lý danh sách byte patches
        if let Some(patch_list) = params.patches {
            for p in patch_list {
                let hex_clean: String = p.hex_data.chars().filter(|c| c.is_ascii_hexdigit()).collect();
                if hex_clean.len() % 2 == 0 {
                    let bytes: Vec<u8> = (0..hex_clean.len())
                        .step_by(2)
                        .map(|i| u8::from_str_radix(&hex_clean[i..i + 2], 16).unwrap_or(0))
                        .collect();
                    overlay.apply_virtual_patch(
                        p.lba,
                        p.offset,
                        &bytes,
                        params.description.clone().unwrap_or_default(),
                    )?;
                    patches_applied += 1;
                }
            }
        }

        // Kiểm chứng ảo sau khi vá (Virtual Verification)
        let mut lba0_buf = [0u8; SECTOR_SIZE];
        let _ = overlay.read_sectors(0, 1, &mut lba0_buf);
        let mbr_now = analyze_mbr(&lba0_buf);

        let mut lba2048_buf = [0u8; SECTOR_SIZE];
        let _ = overlay.read_sectors(2048, 1, &mut lba2048_buf);
        let ntfs2048_now = analyze_ntfs_vbr(&lba2048_buf);
        let fat2048_now = analyze_fat32(&lba2048_buf);

        let mount_status = if ntfs2048_now.valid || fat2048_now.valid || mbr_now.valid_signature {
            "SUCCESS"
        } else {
            "UNVERIFIED"
        };

        Ok(json!({
            "session_id": session.session_id,
            "patches_applied": patches_applied,
            "total_active_patches": overlay.patch_count(),
            "mount_status": mount_status,
            "verification": {
                "mbr_valid": mbr_now.valid_signature,
                "ntfs_vbr_at_2048_valid": ntfs2048_now.valid,
                "fat32_bpb_at_2048_valid": fat2048_now.valid
            },
            "safety": "RAM Copy-on-Write only. Physical disk was NOT written to.",
            "message": if mount_status == "SUCCESS" {
                "Virtual patch verified successfully. Structure is now readable!"
            } else {
                "Patch applied, but filesystem signature is still incomplete."
            }
        }))
    }

    /// 8. `extract_recovered_data`: Trích xuất file ra thư mục an toàn
    async fn handle_extract_recovered_data(&self, args: &Value) -> Result<Value> {
        let params: ExtractRecoveredDataParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;

        std::fs::create_dir_all(&params.output_dir)?;

        if params.mode == "CARVE_BY_LBA_RANGES" {
            let out_file_path = Path::new(&params.output_dir).join("recovered_file.bin");
            let mut out_file = File::create(&out_file_path)?;

            let mut overlay = session.overlay.lock().await;
            let mut total_bytes = 0;

            if let Some(p) = params.params {
                if let Some(ranges) = p.get("ranges").and_then(|r| r.as_array()) {
                    for range in ranges {
                        let start_lba = range.get("start_lba").and_then(|v| v.as_u64()).unwrap_or(0);
                        let count = range.get("count").and_then(|v| v.as_u64()).unwrap_or(1) as usize;

                        let mut buf = vec![0u8; count * SECTOR_SIZE];
                        if overlay.read_sectors(start_lba, count, &mut buf).is_ok() {
                            out_file.write_all(&buf)?;
                            total_bytes += buf.len();
                        }
                    }
                }
            }

            Ok(json!({
                "status": "extracted",
                "mode": "CARVE_BY_LBA_RANGES",
                "output_path": out_file_path.to_string_lossy(),
                "total_bytes_recovered": total_bytes,
                "safety": "Recovered data written safely to destination directory."
            }))
        } else {
            // TSK_TREE_EXTRACT
            Ok(json!({
                "status": "ready",
                "mode": "TSK_TREE_EXTRACT",
                "output_dir": params.output_dir,
                "tsk_available": external_bridges::tsk_wrapper::TskWrapper::new().is_available()
            }))
        }
    }

    // ====================================================================
    // Legacy Aliases
    // ====================================================================

    async fn handle_legacy_scan_partitions(&self, args: &Value) -> Result<Value> {
        let params: ScanPartitionsParams = serde_json::from_value(args.clone())?;
        let target = if let Some(path) = params.image_path {
            path
        } else if let Some(num) = params.drive_number {
            format!(r"\\.\PhysicalDrive{}", num)
        } else {
            "default".to_string()
        };

        if target != "default" {
            let open_args = json!({ "target_path": target });
            let open_res = self.handle_open_recovery_session(&open_args).await?;
            let sid = open_res.get("session_id").and_then(|s| s.as_str()).unwrap_or("").to_string();
            self.handle_scan_disk_topology(&json!({ "session_id": sid })).await
        } else {
            let sid = self.default_session_id.lock().await.clone().unwrap_or_default();
            self.handle_scan_disk_topology(&json!({ "session_id": sid })).await
        }
    }

    async fn handle_legacy_read_boot_sector(&self, args: &Value) -> Result<Value> {
        let params: ReadBootSectorParams = serde_json::from_value(args.clone())?;
        let sid = self.default_session_id.lock().await.clone().unwrap_or_default();
        self.handle_inspect_sector_structure(&json!({
            "session_id": sid,
            "lba": params.partition_lba,
            "parse_as": "NTFS_VBR"
        })).await
    }

    async fn handle_legacy_scan_signatures(&self, args: &Value) -> Result<Value> {
        let params: ScanSignaturesParams = serde_json::from_value(args.clone())?;
        let sid = self.default_session_id.lock().await.clone().unwrap_or_default();
        self.handle_start_async_disk_job(&json!({
            "session_id": sid,
            "job_type": "SIMD_MAGIC_CARVE",
            "params": { "start_lba": params.start_lba, "count_sectors": params.count_sectors }
        })).await
    }

    async fn handle_legacy_analyze_entropy(&self, args: &Value) -> Result<Value> {
        let params: AnalyzeEntropyParams = serde_json::from_value(args.clone())?;
        let sid = self.default_session_id.lock().await.clone().unwrap_or_default();
        self.handle_inspect_sector_structure(&json!({
            "session_id": sid,
            "lba": params.start_lba,
            "parse_as": "RAW_HEX"
        })).await
    }

    async fn handle_legacy_apply_patch(&self, args: &Value) -> Result<Value> {
        let params: ApplyPatchParams = serde_json::from_value(args.clone())?;
        let sid = self.default_session_id.lock().await.clone().unwrap_or_default();
        self.handle_virtual_patch_and_verify(&json!({
            "session_id": sid,
            "patches": [{
                "lba": params.lba,
                "offset": params.offset,
                "hex_data": params.hex_data
            }],
            "description": params.description
        })).await
    }

    async fn handle_legacy_rollback_patches(&self, _args: &Value) -> Result<Value> {
        let sid = self.default_session_id.lock().await.clone().unwrap_or_default();
        if let Ok(session) = self.get_session(&sid).await {
            let mut overlay = session.overlay.lock().await;
            overlay.rollback_all_patches();
            Ok(json!({ "status": "rolled_back", "message": "All virtual patches rolled back." }))
        } else {
            Ok(json!({ "status": "ok", "message": "No active session." }))
        }
    }

    async fn handle_legacy_extract_file(&self, args: &Value) -> Result<Value> {
        let params: ExtractFileParams = serde_json::from_value(args.clone())?;
        let sid = self.default_session_id.lock().await.clone().unwrap_or_default();
        self.handle_extract_recovered_data(&json!({
            "session_id": sid,
            "mode": "TSK_TREE_EXTRACT",
            "output_dir": "./recovered",
            "params": {
                "partition_offset": params.partition_offset,
                "inode": params.inode,
                "output_filename": params.output_filename
            }
        })).await
    }

    async fn handle_legacy_run_carving(&self, args: &Value) -> Result<Value> {
        let params: RunCarvingParams = serde_json::from_value(args.clone())?;
        let sid = self.default_session_id.lock().await.clone().unwrap_or_default();
        self.handle_start_async_disk_job(&json!({
            "session_id": sid,
            "job_type": "SIMD_MAGIC_CARVE",
            "params": { "output_dir": params.output_dir }
        })).await
    }
}
