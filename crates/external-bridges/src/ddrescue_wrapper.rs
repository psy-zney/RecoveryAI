//! Wrapper cho GNU ddrescue.
//!
//! ddrescue cứu dữ liệu từ ổ đĩa bị bad sector vật lý bằng cơ chế
//! đọc nhảy cóc (skip on error) và lưu trạng thái vào mapfile.
//!
//! Lệnh: `ddrescue [options] <input> <output> [mapfile]`

use serde::Serialize;
use std::path::PathBuf;

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
        _config: &DdrescueConfig,
    ) -> anyhow::Result<DdrescueResult> {
        anyhow::bail!("NotImplemented: legacy subprocess adapter is disabled; use the native RAM-overlay MCP engine")
    }

    /// Chạy ddrescue pass 2+ (retry — thử lại các vùng lỗi với block nhỏ hơn).
    pub async fn run_retry_rescue(
        &self,
        _config: &DdrescueConfig,
    ) -> anyhow::Result<DdrescueResult> {
        anyhow::bail!("NotImplemented: legacy subprocess adapter is disabled; use the native RAM-overlay MCP engine")
    }

    /// Parse mapfile để lấy trạng thái hiện tại.
    pub async fn parse_mapfile(&self, mapfile_path: &str) -> anyhow::Result<DdrescueMapSummary> {
        use tokio::io::AsyncReadExt;
        let mut content = String::new();
        tokio::fs::File::open(mapfile_path)
            .await?
            .take(16 * 1024 * 1024 + 1)
            .read_to_string(&mut content)
            .await?;
        anyhow::ensure!(
            content.len() <= 16 * 1024 * 1024,
            "Mapfile exceeds 16 MiB parsing budget"
        );
        parse_map_summary(&content)
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
fn parse_map_summary(content: &str) -> anyhow::Result<DdrescueMapSummary> {
    let mut rescued = 0u64;
    let mut errors = 0u64;
    let mut pending = 0u64;
    let mut previous_end = None;
    for line in content
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        if (2..=3).contains(&fields.len()) && matches!(fields[1], "?" | "+" | "-" | "/" | "*") {
            parse_hex_or_dec(fields[0])?;
            if fields.len() == 3 {
                fields[2].parse::<u32>()?;
            }
            continue;
        }
        anyhow::ensure!(fields.len() == 3, "Malformed ddrescue mapfile row");
        let position = parse_hex_or_dec(fields[0])?;
        let size = parse_hex_or_dec(fields[1])?;
        let end = position
            .checked_add(size)
            .ok_or_else(|| anyhow::anyhow!("Mapfile position overflow"))?;
        anyhow::ensure!(
            size > 0 && previous_end.is_none_or(|previous| position >= previous),
            "Overlapping or empty mapfile range"
        );
        previous_end = Some(end);
        let counter = match fields[2] {
            "+" => &mut rescued,
            "-" => &mut errors,
            "?" | "/" | "*" => &mut pending,
            _ => anyhow::bail!("Invalid mapfile sector status"),
        };
        *counter = counter
            .checked_add(size)
            .ok_or_else(|| anyhow::anyhow!("Mapfile size overflow"))?;
    }
    anyhow::ensure!(previous_end.is_some(), "Mapfile has no data ranges");
    let total = rescued
        .checked_add(errors)
        .and_then(|n| n.checked_add(pending))
        .ok_or_else(|| anyhow::anyhow!("Mapfile total overflow"))?;
    Ok(DdrescueMapSummary {
        total_bytes: total,
        rescued_bytes: rescued,
        error_bytes: errors,
        non_tried_bytes: pending,
        percent_rescued: 100.0 * rescued as f64 / total as f64,
    })
}

fn parse_hex_or_dec(s: &str) -> Result<u64, std::num::ParseIntError> {
    if s.starts_with("0x") || s.starts_with("0X") {
        u64::from_str_radix(&s[2..], 16)
    } else {
        s.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapfile_reports_verified_counts_and_rejects_malformed_ranges() {
        let report =
            parse_map_summary("# header\n0x0 ? 1\n0x0 0x200 +\n0x200 0x200 -\n0x400 0x200 *\n")
                .unwrap();
        assert_eq!(
            (
                report.rescued_bytes,
                report.error_bytes,
                report.non_tried_bytes
            ),
            (512, 512, 512)
        );
        assert!(parse_map_summary("0 0x400 +\n0x200 0x400 -").is_err());
        assert!(parse_map_summary("0xffffffffffffffff 2 +").is_err());
        assert!(parse_map_summary("garbage").is_err());
    }
}
