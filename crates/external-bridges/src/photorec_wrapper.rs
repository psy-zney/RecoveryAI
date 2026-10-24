//! Wrapper cho PhotoRec / TestDisk.
//!
//! PhotoRec quét chữ ký file (File Carving) cho hơn 480 định dạng.
//! Chạy ở chế độ headless CLI: `photorec /cmd <image> search`

use serde::Serialize;
use std::path::PathBuf;
use tokio::process::Command;

const DEFAULT_PHOTOREC_BIN: &str = "binaries/photorec";

/// Kết quả chạy PhotoRec
#[derive(Debug, Clone, Serialize)]
pub struct PhotoRecResult {
    pub command: String,
    pub exit_code: i32,
    pub output_dir: String,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    /// Số file đã recover (parse từ output)
    pub files_recovered: Option<u64>,
}

/// Cấu hình chạy PhotoRec
#[derive(Debug, Clone)]
pub struct PhotoRecConfig {
    /// Đường dẫn image file
    pub image_path: String,
    /// Thư mục output cho file recovered
    pub output_dir: String,
    /// Chỉ quét các loại file cụ thể (empty = all)
    pub file_types: Vec<String>,
    /// Bỏ qua FAT/NTFS filesystem, quét raw
    pub whole_disk: bool,
    /// Partition number (0-based)
    pub partition: Option<u32>,
}

/// Wrapper gọi PhotoRec headless
pub struct PhotoRecWrapper {
    bin_path: PathBuf,
}

impl PhotoRecWrapper {
    pub fn new() -> Self {
        let ext = if cfg!(target_os = "windows") {
            ".exe"
        } else {
            ""
        };
        Self {
            bin_path: PathBuf::from(format!("{}{}", DEFAULT_PHOTOREC_BIN, ext)),
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

    /// Chạy PhotoRec ở chế độ headless.
    ///
    /// Lệnh: `photorec /cmd <image_path> partition_type,<partition>,search`
    pub async fn run_carving(&self, config: &PhotoRecConfig) -> anyhow::Result<PhotoRecResult> {
        // Tạo output directory
        tokio::fs::create_dir_all(&config.output_dir).await?;

        let mut args = vec!["/cmd".to_string(), config.image_path.clone()];

        // Cấu hình partition type
        if config.whole_disk {
            args.push("partition_type,None".into());
        }

        // Thêm output dir
        args.push(format!("fileopt,everything,disable"));

        // Enable specific file types
        for ft in &config.file_types {
            args.push(format!("fileopt,{},enable", ft));
        }

        args.push("search".into());

        tracing::info!(args = ?args, "Running PhotoRec");

        let output = Command::new(&self.bin_path)
            .args(&args)
            .current_dir(&config.output_dir)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Parse số file recovered từ output
        let files_recovered = parse_recovered_count(&stdout);

        Ok(PhotoRecResult {
            command: format!("photorec {}", args.join(" ")),
            exit_code: output.status.code().unwrap_or(-1),
            output_dir: config.output_dir.clone(),
            stdout,
            stderr,
            success: output.status.success(),
            files_recovered,
        })
    }

    /// Liệt kê các file đã recover trong output directory.
    pub async fn list_recovered_files(
        &self,
        output_dir: &str,
    ) -> anyhow::Result<Vec<RecoveredFile>> {
        let mut files = Vec::new();
        let mut entries = tokio::fs::read_dir(output_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let metadata = entry.metadata().await?;
            if metadata.is_file() {
                files.push(RecoveredFile {
                    name: entry.file_name().to_string_lossy().to_string(),
                    size_bytes: metadata.len(),
                    extension: entry
                        .path()
                        .extension()
                        .map(|e| e.to_string_lossy().to_string())
                        .unwrap_or_default(),
                });
            }
        }

        files.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        Ok(files)
    }
}

impl Default for PhotoRecWrapper {
    fn default() -> Self {
        Self::new()
    }
}

/// Thông tin file đã recover
#[derive(Debug, Serialize)]
pub struct RecoveredFile {
    pub name: String,
    pub size_bytes: u64,
    pub extension: String,
}

/// Parse số file recovered từ PhotoRec output
fn parse_recovered_count(output: &str) -> Option<u64> {
    // PhotoRec output dạng: "X files saved"
    for line in output.lines() {
        if line.contains("files saved") || line.contains("file(s) saved") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(num_str) = parts.first() {
                if let Ok(n) = num_str.parse() {
                    return Some(n);
                }
            }
        }
    }
    None
}
