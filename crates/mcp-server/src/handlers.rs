//! Request handlers — xử lý MCP requests từ AI Agent.
//!
//! Mỗi tool call được route tới handler tương ứng.
//! Handlers đọc dữ liệu qua VirtualDiskOverlay (read-only + COW),
//! chạy parser/scanner, và trả về JSON gọn (< 5KB).

use crate::job_manager::JobManager;
use crate::tools_schema::*;
use anyhow::{bail, ensure, Context, Result};
use core_io::virtual_overlay::{RawStorageReader, VirtualDiskOverlay};
use disk_parsers::fat_parser::analyze_fat32;
use disk_parsers::mbr_gpt::GptHeader;
use disk_parsers::ntfs_parser::{analyze_ntfs_vbr, calculate_shannon_entropy};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub type DiskOverlay = VirtualDiskOverlay<Box<dyn RawStorageReader + Send>>;

/// Đại diện cho một phiên làm việc cứu dữ liệu đang mở
pub struct RecoverySession {
    pub session_id: String,
    pub target_path: String,
    pub overlay: Arc<Mutex<DiskOverlay>>,
    pub total_sectors: u64,
    #[allow(dead_code)]
    pub sector_size: u32,
    pub session_dir: std::path::PathBuf,
}

impl RecoverySession {
    /// Background jobs must remain cancellable while another traversal owns I/O.
    pub fn lock_for_job(
        &self,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<tokio::sync::MutexGuard<'_, DiskOverlay>> {
        loop {
            ensure!(
                !cancel.load(std::sync::atomic::Ordering::Acquire),
                "Job cancelled"
            );
            if let Ok(disk) = self.overlay.try_lock() {
                return Ok(disk);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
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
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        let mut input = BufReader::new(tokio::io::stdin());
        let mut output = tokio::io::stdout();
        loop {
            let mut line = Vec::new();
            let count = (&mut input)
                .take(262145)
                .read_until(b'\n', &mut line)
                .await?;
            if count == 0 {
                break;
            }
            ensure!(
                line.len() <= 262144,
                "MCP request exceeds 256 KiB input budget"
            );
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let response = match serde_json::from_slice::<Value>(&line) {
                Ok(request) => {
                    if request.get("id").is_none() {
                        continue;
                    }
                    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
                        || !request.get("id").is_some_and(|id| {
                            id.as_str().is_some_and(|s| s.len() <= 128)
                                || id.is_i64()
                                || id.is_u64()
                        })
                    {
                        json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid JSON-RPC request"}})
                    } else {
                        use futures_util::FutureExt;
                        std::panic::AssertUnwindSafe(self.handle_request(&request)).catch_unwind().await.unwrap_or_else(|_|json!({"jsonrpc":"2.0","id":request.get("id"),"error":{"code":-32603,"message":"Parser panicked; server remains available and durable overlay WAL is preserved"}}))
                    }
                }
                Err(_) => {
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid JSON"}})
                }
            };
            let mut encoded = serde_json::to_vec(&response)?;
            encoded.push(b'\n');
            output.write_all(&encoded).await?;
            output.flush().await?;
        }
        Ok(())
    }

    async fn handle_request(&self, request: &Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
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
        let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
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
            "read_scan_hits" => self.handle_read_scan_hits(&arguments).await,
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

        let result = result.and_then(|content| {
            ensure!(
                serde_json::to_vec(&content)?.len() <= 3800,
                "Response exceeds compact report budget; narrow the query"
            );
            Ok(content)
        });
        match result {
            Ok(content) => {
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{
                            "type": "text",
                            "text": content.to_string()
                        }]
                    }
                });
                if response.to_string().len() > 4900 {
                    json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"Error: report exceeds 5 KB; narrow the query"}],"isError":true}})
                } else {
                    response
                }
            }

            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{
                        "type": "text",
                        "text": format!("Error: {}", e.to_string().chars().take(512).collect::<String>())
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

                    let parsed: Value = serde_json::from_str(&stdout_str)?;
                    let raw_items = match parsed {
                        Value::Array(arr) => arr,
                        Value::Object(_) => vec![parsed],
                        _ => vec![],
                    };

                    let mut drives = Vec::new();
                    for item in raw_items {
                        let dev_id = item.get("DeviceID").and_then(|v| v.as_str()).unwrap_or("");
                        let model = item
                            .get("Model")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown Drive");
                        let iface = item
                            .get("InterfaceType")
                            .and_then(|v| v.as_str())
                            .unwrap_or("UNKNOWN");
                        let media = item.get("MediaType").and_then(|v| v.as_str()).unwrap_or("");
                        let size_bytes = item.get("Size").and_then(|v| v.as_u64()).unwrap_or(0);
                        let size_gb =
                            format!("{:.2} GB", size_bytes as f64 / (1024.0 * 1024.0 * 1024.0));

                        let drive_num = dev_id
                            .chars()
                            .filter(|c| c.is_ascii_digit())
                            .collect::<String>();
                        let is_external = iface.eq_ignore_ascii_case("USB")
                            || media.to_lowercase().contains("removable");
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
                        "drives": drives.iter().take(8).collect::<Vec<_>>(),
                        "truncated": drives.len()>8,
                        "connection_options_for_broken_laptop": {
                            "hardware_failure": "Nếu laptop bị hỏng nguồn/cháy main/hỏng màn hình: Tháo SSD/HDD ra cắm vào Box di động (M.2 NVMe hoặc SATA to USB) rồi cắm vào máy này -> Chọn ổ có is_external = true.",
                            "os_failure": "Nếu laptop vẫn lên nguồn nhưng màn hình xanh/hỏng Windows: Tạo USB WinPE cứu hộ, boot laptop bằng USB và chạy trực tiếp RecoveryAI trên laptop."
                        }
                    }))
                }
                _ => bail!("Physical drive enumeration failed: PowerShell/CIM returned an error"),
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            bail!("NotImplemented: physical drive enumeration on this OS")
        }
    }

    async fn handle_open_recovery_session(&self, args: &Value) -> Result<Value> {
        let params: OpenRecoverySessionParams = serde_json::from_value(args.clone())?;
        ensure!(
            self.sessions.lock().await.len() < 32,
            "Recovery session limit reached"
        );
        let path = params.target_path.trim().to_string();
        ensure!(
            !path.is_empty() && path.len() <= 32767 && !path.contains('\0'),
            "Invalid source path"
        );
        let result = tokio::task::spawn_blocking(move || -> Result<_> {
            let lower = path.to_ascii_lowercase();
            let physical_prefix = r"\\.\physicaldrive";
            let reader: Box<dyn RawStorageReader + Send>;
            let sector_size;
            let canonical;
            if lower.starts_with(physical_prefix) || path.chars().all(|c| c.is_ascii_digit()) {
                let digits = if lower.starts_with(physical_prefix) {
                    &path[physical_prefix.len()..]
                } else {
                    &path
                };
                ensure!(
                    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()),
                    "Invalid physical device index"
                );
                let number: u32 = digits.parse()?;
                let drive = core_io::PhysicalDrive::open(number)?;
                sector_size = drive.sector_size;
                canonical = format!(r"\\.\PhysicalDrive{number}");
                reader = Box::new(drive);
            } else if path.len() == 2
                && path.as_bytes()[0].is_ascii_alphabetic()
                && path.ends_with(':')
                || lower.starts_with(r"\\.\")
            {
                let volume = if lower.starts_with(r"\\.\") {
                    path.clone()
                } else {
                    format!(r"\\.\{path}")
                };
                let bytes = volume.as_bytes();
                ensure!(
                    bytes.len() == 6
                        && bytes[..4] == *br"\\.\"
                        && bytes[4].is_ascii_alphabetic()
                        && bytes[5] == b':',
                    "Unsupported raw device path; use PhysicalDrive<N> or a volume letter"
                );
                let drive = core_io::PhysicalDrive::open_path(&volume)?;
                sector_size = drive.sector_size;
                canonical = volume;
                reader = Box::new(drive);
            } else {
                let target = std::fs::canonicalize(&path)?;
                ensure!(
                    target.is_file(),
                    "NotImplemented: source must be a regular image file or a supported raw device"
                );
                sector_size = params.sector_size.unwrap_or(512);
                ensure!(
                    matches!(sector_size, 512 | 4096),
                    "sector_size must be 512 or 4096"
                );
                reader = Box::new(core_io::PhysicalDrive::open_image(
                    target.to_str().context("Source path is not UTF-8")?,
                )?);
                canonical = target
                    .to_str()
                    .context("Source path is not UTF-8")?
                    .to_string();
            }
            ensure!(
                params
                    .sector_size
                    .is_none_or(|requested| requested == sector_size),
                "Requested sector_size disagrees with device geometry"
            );
            let length = reader.len_bytes()?;
            let (session_id, session_dir, identity) = crate::session_store::open(
                &canonical,
                sector_size,
                length,
                params.resume_session_id.as_deref(),
            )?;
            let overlay = VirtualDiskOverlay::with_wal(
                reader,
                sector_size as usize,
                &crate::session_store::wal_path(&session_dir),
                &identity,
            )?;
            ensure!(
                overlay.len_bytes() % sector_size as u64 == 0,
                "Image size does not match sector geometry"
            );
            Ok((overlay, sector_size, canonical, session_id, session_dir))
        })
        .await??;
        let (overlay, sector_size, target_path, session_id, session_dir) = result;
        let total_sectors = overlay.len_bytes() / sector_size as u64;
        let session = Arc::new(RecoverySession {
            session_id: session_id.clone(),
            target_path: target_path.clone(),
            overlay: Arc::new(Mutex::new(overlay)),
            total_sectors,
            sector_size,
            session_dir: session_dir.clone(),
        });
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), session);
        *self.default_session_id.lock().await = Some(session_id.clone());
        Ok(
            json!({"session_id":session_id,"target_path":target_path,"total_sectors":total_sectors,"sector_size":sector_size,"lba_unit_bytes":sector_size,"status":"OPENED_READ_ONLY","length_bytes":total_sectors*sector_size as u64,"session_dir":session_dir}),
        )
    }

    async fn handle_scan_disk_topology(&self, args: &Value) -> Result<Value> {
        let params: ScanDiskTopologyParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;
        tokio::task::spawn_blocking(move || -> Result<Value> {
            let mut disk = session.overlay.try_lock().context("Session busy: poll or cancel the active job, then retry")?;
            let layout = crate::topology::read_layout(&mut *disk, session.sector_size)?;
            let mut first = vec![0;session.sector_size as usize];
            disk.read_sectors(0,1,&mut first)?;
            let mut common = json!({});
            if session.total_sectors > 2048 {
                let mut boot = vec![0;session.sector_size as usize]; disk.read_sectors(2048,1,&mut boot)?;
                common = json!({"lba_2048":{"is_valid_ntfs":analyze_ntfs_vbr(&boot).valid,"is_valid_fat32":analyze_fat32(&boot).valid}});
            }
            let last_lba = session.total_sectors - 1;
            let mut last = vec![0;session.sector_size as usize]; disk.read_sectors(last_lba,1,&mut last)?;
            Ok(json!({"session_id":session.session_id,"total_sectors":session.total_sectors,"lba_unit_bytes":session.sector_size,
                "lba_0_mbr":{"status":if layout.mbr_valid {"VALID_MBR"} else if first.iter().all(|&b|b==0) {"ZEROED"} else {"CORRUPTED"},"is_gpt_protective":layout.mbr.is_gpt_protective,"partitions_found":layout.partitions.len()},
                "lba_1_gpt_primary":{"status":if layout.primary_valid {"VALID_GPT"} else {"INVALID_OR_MISSING"}},
                "last_lba":{"lba":last_lba,"gpt_backup_valid":layout.backup_valid,"ntfs_backup_vbr_valid":analyze_ntfs_vbr(&last).valid},
                "common_offsets":common,"partitions":layout.partitions.iter().take(8).collect::<Vec<_>>(),"partitions_truncated":layout.partitions.len()>8,
                "anomalies":layout.anomalies.into_iter().map(|a|a.chars().take(160).collect::<String>()).take(8).collect::<Vec<_>>() }))
        }).await?
    }

    async fn handle_inspect_sector_structure(&self, args: &Value) -> Result<Value> {
        let params: InspectSectorStructureParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;
        let count = params.count.unwrap_or(1);
        ensure!((1..=8).contains(&count), "count must be in 1..8");
        tokio::task::spawn_blocking(move || -> Result<Value> {
            let mut disk = session.overlay.try_lock().context("Session busy: poll or cancel the active job, then retry")?;
            let mut buf = vec![0;count*session.sector_size as usize];
            disk.read_sectors(params.lba,count,&mut buf)?;
            let parse_as = params.parse_as.as_deref().unwrap_or("RAW_HEX");
            let parsed = match parse_as {
                "MBR" => json!(disk_parsers::mbr_gpt::validate_mbr(&buf,session.total_sectors).map_err(anyhow::Error::msg)?),
                "NTFS_VBR" => json!(analyze_ntfs_vbr(&buf[..512])),
                "FAT32_BPB" => json!(analyze_fat32(&buf[..512])),
                "GPT" => {
                    let header = GptHeader::from_sector(&buf).context("Invalid GPT header signature, size or CRC32")?;
                    header.validate_geometry(params.lba,session.total_sectors).map_err(anyhow::Error::msg)?;
                    json!({"header_crc_valid":true,"disk_guid":header.disk_guid_string(),"scope":"header_only"})
                }
                "RAW_HEX" => json!({"hex_preview":buf.iter().take(128).map(|b|format!("{b:02X}")).collect::<String>(),"preview_truncated":buf.len()>128,"is_zeroed":buf.iter().all(|&b|b==0)}),
                _ => bail!("NotImplemented: sector parser {parse_as}"),
            };
            Ok(json!({"session_id":session.session_id,"lba":params.lba,"count":count,"entropy":calculate_shannon_entropy(&buf),"parsed_structure":parsed}))
        }).await?
    }

    async fn handle_scan_filesystem_journals(&self, args: &Value) -> Result<Value> {
        let params: ScanFilesystemJournalsParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;
        ensure!(
            params.partition_start_lba < session.total_sectors,
            "Partition is outside source"
        );
        bail!("NotImplemented: deleted-record and journal scanning ($UsnJrnl/$LogFile/FAT allocation history)")
    }

    async fn handle_start_async_disk_job(&self, args: &Value) -> Result<Value> {
        let params: StartAsyncDiskJobParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;
        let arguments = params.params.unwrap_or_else(|| json!({}));
        // Validate before registering a job, so unknown jobs never appear successful.
        let check_session = Arc::clone(&session);
        let check_kind = params.job_type.clone();
        let check_arguments = arguments.clone();
        tokio::task::spawn_blocking(move || {
            crate::jobs::validate_params(&check_kind, &check_arguments, &check_session)
        })
        .await??;
        let (job_id, cancel) = self.job_manager.create_job(&params.job_type)?;
        let manager = Arc::clone(&self.job_manager);
        let id = job_id.clone();
        let kind = params.job_type.clone();
        tokio::task::spawn_blocking(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let revision = session.lock_for_job(&cancel)?.revision();
                crate::jobs::Worker {
                    session: &session,
                    manager: &manager,
                    id: &id,
                    cancel: &cancel,
                    revision,
                }
                .run(&kind, arguments)
            }))
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "Worker panicked while parsing untrusted source"
                ))
            });
            manager.finish(&id, result);
        });
        Ok(
            json!({"status":"job_started","job_id":job_id,"job_type":params.job_type,"session_id":params.session_id}),
        )
    }

    async fn handle_read_scan_hits(&self, args: &Value) -> Result<Value> {
        let params: ReadScanHitsParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;
        let id = uuid::Uuid::parse_str(&params.job_id)?.to_string();
        tokio::task::spawn_blocking(move || {
            crate::hit_store::page(
                &session.session_dir.join(format!("{id}_hits.jsonl")),
                params.cursor,
                params.limit.unwrap_or(20),
            )
        })
        .await?
    }
    async fn handle_poll_async_job(&self, args: &Value) -> Result<Value> {
        let params: PollAsyncJobParams = serde_json::from_value(args.clone())?;
        if params.cancel {
            self.job_manager.cancel_job(&params.job_id)?;
        }
        let mut result = json!(self.job_manager.get_job(&params.job_id)?);
        if params.cursor.is_some() || params.limit.is_some() {
            let path = self.job_manager.hits_path(&params.job_id)?;
            let page = tokio::task::spawn_blocking(move || {
                crate::hit_store::page(
                    &path,
                    params.cursor.unwrap_or(0),
                    params.limit.unwrap_or(20),
                )
            })
            .await??;
            // Avoid duplicating a completed summary when returning a full page.
            result["status"] = json!("HITS_PAGE");
            result["page"] = page;
        }
        Ok(result)
    }

    async fn handle_virtual_patch_and_verify(&self, args: &Value) -> Result<Value> {
        let params: VirtualPatchAndVerifyParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;
        ensure!(
            params.copy_from_lba.is_some() == params.to_lba.is_some(),
            "copy_from_lba and to_lba must be supplied together"
        );
        ensure!(
            params
                .partition_start_lba
                .is_none_or(|lba| lba < session.total_sectors),
            "Verification partition LBA outside source"
        );
        let patches = params.patches.unwrap_or_default();
        ensure!(patches.len() <= 256, "At most 256 patches per call");
        ensure!(
            params.description.as_ref().is_none_or(|s| s.len() <= 512),
            "Patch description exceeds 512 bytes"
        );
        ensure!(
            params.copy_from_lba.is_some() || !patches.is_empty(),
            "No patches supplied"
        );
        let mut decoded = Vec::new();
        for patch in patches {
            let clean: String = patch
                .hex_data
                .chars()
                .filter(|c| !c.is_ascii_whitespace())
                .collect();
            ensure!(
                !clean.is_empty()
                    && clean.len() <= session.sector_size as usize * 2
                    && clean.len().is_multiple_of(2)
                    && clean.bytes().all(|c| c.is_ascii_hexdigit()),
                "Invalid patch hex data"
            );
            let bytes = (0..clean.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&clean[i..i + 2], 16))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            ensure!(
                patch.offset < session.sector_size as usize
                    && patch
                        .offset
                        .checked_add(bytes.len())
                        .is_some_and(|end| end <= session.sector_size as usize),
                "Patch offset exceeds sector boundary"
            );
            ensure!(
                patch.lba < session.total_sectors,
                "Patch LBA outside source"
            );
            decoded.push((patch.lba, patch.offset, bytes));
        }
        tokio::task::spawn_blocking(move || -> Result<Value> {
            let mut disk = session.overlay.try_lock().context("Session busy: poll or cancel the active job, then retry")?;
            disk.set_cancel_token(None);
            let checkpoint = disk.history().len();
            let applied = (|| -> Result<()> {
                if let (Some(from),Some(to)) = (params.copy_from_lba,params.to_lba) {
                    disk.copy_virtual_sector(from,to,params.description.clone().unwrap_or_default())?;
                }
                for (lba,offset,bytes) in &decoded {
                    disk.apply_virtual_patch(*lba,*offset,bytes,params.description.clone().unwrap_or_default())?;
                }
                Ok(())
            })();
            if let Err(error) = applied {
                while disk.history().len()>checkpoint { disk.rollback_last_patch()?; }
                return Err(error);
            }
            let mut errors=Vec::new();
            let mut candidates = if let Some(lba)=params.partition_start_lba { vec![lba] } else {
                match crate::topology::read_layout(&mut *disk,session.sector_size) {
                    Ok(layout) if layout.partitions.len()<=16 => layout.partitions.iter().map(|p|p.start_lba).collect::<Vec<_>>(),
                    Ok(_) => { errors.push("Specify partition_start_lba when more than 16 partitions exist".to_string());Vec::new() }
                    Err(error) => { errors.push(error.to_string().chars().take(200).collect());Vec::new() }
                }
            };
            if params.partition_start_lba.is_none() {
                if let Some(to)=params.to_lba {candidates.push(to);}
                candidates.push(0);
            }
            candidates.sort_unstable(); candidates.dedup();
            let mut mount = None;
            for lba in candidates {
                match crate::native_fs::validate_mount(&mut *disk,lba) {
                    Ok(report) => {mount=Some(json!({"partition_start_lba":lba,"root":report}));break}
                    Err(error) => if errors.len()<4 {errors.push(error.to_string().chars().take(200).collect::<String>())},
                }
            }
            Ok(json!({"session_id":session.session_id,"patches_applied":disk.history().len()-checkpoint,"total_active_patches":disk.patch_count(),
                "mount_status":if mount.is_some(){"SUCCESS"}else{"UNVERIFIED"},"verification":mount,"verification_errors":errors,"source_modified":false}))
        }).await?
    }

    async fn handle_extract_recovered_data(&self, args: &Value) -> Result<Value> {
        let params: ExtractRecoveredDataParams = serde_json::from_value(args.clone())?;
        let session = self.get_session(&params.session_id).await?;
        ensure!(
            matches!(
                params.mode.as_str(),
                "CARVE_BY_LBA_RANGES" | "TSK_TREE_EXTRACT"
            ),
            "NotImplemented: extraction mode {}",
            params.mode
        );
        let (job_id, cancel) = self
            .job_manager
            .create_job(crate::job_manager::JobType::DataExtraction.as_str())?;
        let manager = Arc::clone(&self.job_manager);
        let id = job_id.clone();
        tokio::task::spawn_blocking(move || {
            let progress = crate::extraction::ExtractionProgress {
                manager: &manager,
                id: &id,
                cancel: &cancel,
                bytes: std::sync::atomic::AtomicU64::new(0),
                total: None,
            };
            let request = crate::extraction::ExtractionRequest {
                mode: params.mode,
                output_dir: params.output_dir,
                params: params.params,
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::extraction::run(&session, request, &progress)
            }))
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "Extraction parser panicked; current incomplete output was discarded"
                ))
            });
            manager.finish(&id, result);
        });
        Ok(
            json!({"status":"job_started","job_id":job_id,"job_type":"DATA_EXTRACTION","session_id":params.session_id}),
        )
    }

    async fn handle_legacy_scan_partitions(&self, args: &Value) -> Result<Value> {
        let params: ScanPartitionsParams = serde_json::from_value(args.clone())?;
        ensure!(
            !(params.image_path.is_some() && params.drive_number.is_some()),
            "Specify image_path or drive_number, not both"
        );
        ensure!(
            params.drive_number.is_none_or(|n| n >= 0),
            "Invalid physical drive number"
        );
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
            let sid = open_res
                .get("session_id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            self.handle_scan_disk_topology(&json!({ "session_id": sid }))
                .await
        } else {
            let sid = self
                .default_session_id
                .lock()
                .await
                .clone()
                .unwrap_or_default();
            self.handle_scan_disk_topology(&json!({ "session_id": sid }))
                .await
        }
    }

    async fn handle_legacy_read_boot_sector(&self, args: &Value) -> Result<Value> {
        let params: ReadBootSectorParams = serde_json::from_value(args.clone())?;
        let sid = self
            .default_session_id
            .lock()
            .await
            .clone()
            .unwrap_or_default();
        self.handle_inspect_sector_structure(&json!({
            "session_id": sid,
            "lba": params.partition_lba,
            "parse_as": "NTFS_VBR"
        }))
        .await
    }

    async fn handle_legacy_scan_signatures(&self, args: &Value) -> Result<Value> {
        let params: ScanSignaturesParams = serde_json::from_value(args.clone())?;
        ensure!(
            params
                .file_types
                .as_ref()
                .is_none_or(|types| types.is_empty()),
            "NotImplemented: legacy file-type filtering"
        );
        let sid = self
            .default_session_id
            .lock()
            .await
            .clone()
            .unwrap_or_default();
        self.handle_start_async_disk_job(&json!({
            "session_id": sid,
            "job_type": "SIMD_MAGIC_CARVE",
            "params": { "start_lba": params.start_lba, "count_sectors": params.count_sectors }
        }))
        .await
    }

    async fn handle_legacy_analyze_entropy(&self, args: &Value) -> Result<Value> {
        let params: AnalyzeEntropyParams = serde_json::from_value(args.clone())?;
        let sid = self
            .default_session_id
            .lock()
            .await
            .clone()
            .unwrap_or_default();
        self.handle_inspect_sector_structure(&json!({
            "session_id": sid,
            "lba": params.start_lba,
            "count": params.count_sectors.unwrap_or(1),
            "parse_as": "RAW_HEX"
        }))
        .await
    }

    async fn handle_legacy_apply_patch(&self, args: &Value) -> Result<Value> {
        let params: ApplyPatchParams = serde_json::from_value(args.clone())?;
        let sid = self
            .default_session_id
            .lock()
            .await
            .clone()
            .unwrap_or_default();
        self.handle_virtual_patch_and_verify(&json!({
            "session_id": sid,
            "patches": [{
                "lba": params.lba,
                "offset": params.offset,
                "hex_data": params.hex_data
            }],
            "description": params.description
        }))
        .await
    }

    async fn handle_legacy_rollback_patches(&self, args: &Value) -> Result<Value> {
        let params: RollbackPatchesParams = serde_json::from_value(args.clone())?;
        let session = self.get_session("default").await?;
        let mut disk = session
            .overlay
            .try_lock()
            .context("Session busy: poll or cancel the active job, then retry")?;
        let count = if params.rollback_all.unwrap_or(false) {
            let count = disk.history().len();
            disk.rollback_all_patches()?;
            count
        } else {
            usize::from(disk.rollback_last_patch()?.is_some())
        };
        Ok(
            json!({"status":"rolled_back","patches_removed":count,"active_sectors":disk.patch_count()}),
        )
    }

    async fn handle_legacy_extract_file(&self, _args: &Value) -> Result<Value> {
        bail!("NotImplemented: legacy inode export; use native TSK_TREE_EXTRACT with partition_start_lba")
    }

    async fn handle_legacy_run_carving(&self, _args: &Value) -> Result<Value> {
        bail!(
            "NotImplemented: PhotoRec export adapter; use SIMD_MAGIC_CARVE for signature discovery"
        )
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    use crate::job_manager::JobStatus;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    struct PanickingReader;
    impl RawStorageReader for PanickingReader {
        fn len_bytes(&self) -> std::io::Result<u64> {
            Ok(4096)
        }
        fn read_physical_sectors(&mut self, _: u64, _: usize, _: &mut [u8]) -> std::io::Result<()> {
            panic!("injected untrusted parser/read panic")
        }
    }
    fn session() -> Arc<RecoverySession> {
        Arc::new(RecoverySession {
            session_id: "held".into(),
            target_path: "unused.img".into(),
            overlay: Arc::new(Mutex::new(
                VirtualDiskOverlay::new(
                    Box::new(PanickingReader) as Box<dyn RawStorageReader + Send>
                )
                .unwrap(),
            )),
            total_sectors: 8,
            sector_size: 512,
            session_dir: std::env::temp_dir(),
        })
    }
    #[tokio::test]
    async fn inspection_returns_busy_instead_of_stalling_stdio_behind_extraction() {
        let server = DiskForensicsServer::new().await.unwrap();
        let session = session();
        server
            .sessions
            .lock()
            .await
            .insert("held".into(), Arc::clone(&session));
        let _held = session.overlay.lock().await;
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            server.handle_inspect_sector_structure(&json!({"session_id":"held","lba":0})),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(result.to_string().contains("Session busy"));
        let (id, _) = server.job_manager.create_job("DATA_EXTRACTION").unwrap();
        server
            .handle_poll_async_job(&json!({"job_id":id,"cancel":true}))
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn waiting_for_an_overlay_does_not_prevent_job_cancellation() {
        let session = session();
        let held = session.overlay.lock().await;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_session = Arc::clone(&session);
        let worker_cancel = Arc::clone(&cancel);
        let worker = tokio::task::spawn_blocking(move || {
            worker_session.lock_for_job(&worker_cancel).map(|_| ())
        });
        cancel.store(true, std::sync::atomic::Ordering::Release);
        let error = tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        drop(held);
    }
    #[tokio::test]
    async fn worker_panic_becomes_a_failed_job_and_polling_still_works() {
        let server = DiskForensicsServer::new().await.unwrap();
        let session = session();
        server.sessions.lock().await.insert("held".into(), session);
        let scan = server
            .handle_start_async_disk_job(&json!({
                "session_id":"held","job_type":"SIMD_VBR_HUNT"
            }))
            .await
            .unwrap();
        let id = scan["job_id"].as_str().unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match server.job_manager.get_job(id).unwrap().status {
                    JobStatus::Failed { error } => {
                        assert!(error.contains("panicked"));
                        break;
                    }
                    JobStatus::Running { .. } => tokio::task::yield_now().await,
                    other => panic!("unexpected panic job status: {other:?}"),
                }
            }
        })
        .await
        .unwrap();
        assert!(server
            .handle_poll_async_job(&json!({"job_id":id}))
            .await
            .is_ok());
        let _ = std::fs::remove_file(std::env::temp_dir().join(format!("{id}_hits.jsonl")));
    }
}
