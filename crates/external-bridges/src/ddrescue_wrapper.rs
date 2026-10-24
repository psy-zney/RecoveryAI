//! Wrapper cho GNU ddrescue.
//!
//! ddrescue cứu dữ liệu từ ổ đĩa bị bad sector vật lý bằng cơ chế
//! đọc nhảy cóc (skip on error) và lưu trạng thái vào mapfile.
//!
//! Lệnh: `ddrescue [options] <input> <output> [mapfile]`

use serde::Serialize;
use std::path::PathBuf;
use tokio::process::Command;

const DEFAULT_DDRESCUE_BIN: &str = "binaries/ddrescue";

/// Trạng thái của ddrescue job
#[derive(Debug, Clone, Serialize)]
pub struct DdrescueStatus {
    /// Tổng bytes đã cứu được
    pub rescued_bytes: u64,
    /// Tổng bytes lỗi (không cứu được)
    pub error_bytes: u64,
    /// Phần trăm hoàn thành
    pub percent_complete: f64,
    /// Số pass hiện tại
    pub current_pass: u32,
    /// Tốc độ đọc hiện tại (bytes/sec)
    pub read_rate: u64,
}

/// Cấu hình chạy ddrescue
#[derive(Debug, Clone)]
pub struct DdrescueConfig {
    /// Source: đường dẫn ổ đĩa hoặc image file
    pub input_path: String,
    /// Destination: đường dẫn file output (.img)
    pub output_path: String,
    /// Mapfile: lưu tiến trình (cho retry)
    pub mapfile_path: String,
    /// Block size cho lần đọc đầu (mặc định 64K, tăng tốc pass 1)
    pub block_size: Option<u64>,
    /// Max retries cho bad sector (mặc định 3)
    pub max_retries: Option<u32>,
    /// Chỉ cứu dữ liệu trong vùng LBA chỉ định
    pub input_position: Option<u64>,
    /// Kích thước vùng cần cứu (bytes)
    pub size: Option<u64>,
    /// Skip size khi gặp lỗi (bytes, mặc định 64K)
    pub skip_size: Option<u64>,
    /// Timeout cho mỗi lần đọc (giây)
    pub timeout: Option<u32>,
}

/// Wrapper gọi GNU ddrescue
pub struct DdrescueWrapper {
    bin_path: PathBuf,
}

impl DdrescueWrapper {
    pub fn new() -> Self {
        let ext = if cfg!(target_os = "windows") {
            ".exe"
        } else {
            ""
        };
        Self {
            bin_path: PathBuf::from(format!("{}{}", DEFAULT_DDRESCUE_BIN, ext)),
        }
    }

    pub fn with_bin_path(bin_path: impl Into<PathBuf>) -> Self {
        Self {
            bin_path: bin_path.into(),
        }
    }

    pub fn is_available(&self) -> bool {
        self.bin_path.exists()
    }

    /// Chạy ddrescue pass 1 (fast rescue — bỏ qua vùng lỗi).
    ///
    /// Pass 1 dùng block size lớn, nhảy cóc khi gặp lỗi, để cứu
    /// nhanh nhất có thể các vùng tốt trước.
    pub async fn run_fast_rescue(
        &self,
        config: &DdrescueConfig,
    ) -> anyhow::Result<DdrescueResult> {
        let mut args = vec![
            "-n".to_string(), // No-split: chỉ chạy pass 1, bỏ qua slow areas
            "-v".to_string(), // Verbose
        ];

        if let Some(bs) = config.block_size {
            args.push("-b".into());
            args.push(bs.to_string());
        }

        if let Some(pos) = config.input_position {
            args.push("-i".into());
            args.push(pos.to_string());
        }

        if let Some(size) = config.size {
            args.push("-s".into());
            args.push(size.to_string());
        }

        if let Some(timeout) = config.timeout {
            args.push("-T".into());
            args.push(format!("{}s", timeout));
        }

        args.push(config.input_path.clone());
        args.push(config.output_path.clone());
        args.push(config.mapfile_path.clone());

        self.run_ddrescue(&args, &config).await
    }

    /// Chạy ddrescue pass 2+ (retry — thử lại các vùng lỗi với block nhỏ hơn).
    pub async fn run_retry_rescue(
        &self,
        config: &DdrescueConfig,
    ) -> anyhow::Result<DdrescueResult> {
        let mut args = vec![
            "-r".to_string(),
            config.max_retries.unwrap_or(3).to_string(),
            "-v".to_string(),
        ];

        args.push(config.input_path.clone());
        args.push(config.output_path.clone());
        args.push(config.mapfile_path.clone());

        self.run_ddrescue(&args, &config).await
    }

    /// Parse mapfile để lấy trạng thái hiện tại.
    pub async fn parse_mapfile(&self, mapfile_path: &str) -> anyhow::Result<DdrescueMapSummary> {
        let content = tokio::fs::read_to_string(mapfile_path).await?;

        let mut rescued = 0u64;
        let mut errors = 0u64;
        let mut non_tried = 0u64;

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                if let (Ok(_pos), Ok(size)) = (
                    parse_hex_or_dec(parts[0]),
                    parse_hex_or_dec(parts[1]),
                ) {
                    match parts[2] {
                        "+" => rescued += size,     // Finished
                        "-" => errors += size,      // Bad sector
                        "?" => non_tried += size,   // Not yet tried
                        "/" => non_tried += size,    // Non-trimmed
                        "*" => errors += size,      // Non-scraped
                        _ => {}
                    }
                }
            }
        }

        let total = rescued + errors + non_tried;
        let percent = if total > 0 {
            (rescued as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        Ok(DdrescueMapSummary {
            total_bytes: total,
            rescued_bytes: rescued,
            error_bytes: errors,
            non_tried_bytes: non_tried,
            percent_rescued: percent,
        })
    }

    async fn run_ddrescue(
        &self,
        args: &[String],
        config: &DdrescueConfig,
    ) -> anyhow::Result<DdrescueResult> {
        tracing::info!(args = ?args, "Running ddrescue");

        let output = Command::new(&self.bin_path)
            .args(args)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok(DdrescueResult {
            command: format!("ddrescue {}", args.join(" ")),
            exit_code: output.status.code().unwrap_or(-1),
            stdout,
            stderr,
            success: output.status.success(),
            output_path: config.output_path.clone(),
            mapfile_path: config.mapfile_path.clone(),
        })
    }
}

impl Default for DdrescueWrapper {
    fn default() -> Self {
        Self::new()
    }
}

/// Kết quả chạy ddrescue
#[derive(Debug, Serialize)]
pub struct DdrescueResult {
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub output_path: String,
    pub mapfile_path: String,
}

/// Tóm tắt trạng thái từ mapfile
#[derive(Debug, Serialize)]
pub struct DdrescueMapSummary {
    pub total_bytes: u64,
    pub rescued_bytes: u64,
    pub error_bytes: u64,
    pub non_tried_bytes: u64,
    pub percent_rescued: f64,
}

/// Parse hex (0x...) hoặc decimal string
fn parse_hex_or_dec(s: &str) -> Result<u64, std::num::ParseIntError> {
    if s.starts_with("0x") || s.starts_with("0X") {
        u64::from_str_radix(&s[2..], 16)
    } else {
        s.parse()
    }
}
