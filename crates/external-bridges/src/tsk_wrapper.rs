//! Wrapper cho The Sleuth Kit (TSK).
//!
//! TSK cung cấp các công cụ điều tra số chuẩn quốc tế:
//! - `mmls`: Liệt kê layout phân vùng
//! - `fsstat`: Thông số chi tiết filesystem
//! - `fls`: Duyệt cây thư mục (bao gồm file đã xóa)
//! - `icat`: Trích xuất nội dung file theo MFT entry

use serde::Serialize;
use std::path::PathBuf;
use tokio::process::Command;

/// Đường dẫn mặc định tới các binary TSK
const DEFAULT_TSK_DIR: &str = "binaries/tsk";

/// Kết quả chạy lệnh TSK
#[derive(Debug, Clone, Serialize)]
pub struct TskResult {
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// Wrapper gọi các lệnh TSK
pub struct TskWrapper {
    /// Thư mục chứa binary TSK
    bin_dir: PathBuf,
}

impl TskWrapper {
    /// Tạo wrapper với thư mục binary mặc định.
    pub fn new() -> Self {
        Self {
            bin_dir: PathBuf::from(DEFAULT_TSK_DIR),
        }
    }

    /// Tạo wrapper với thư mục binary tuỳ chỉnh.
    pub fn with_bin_dir(bin_dir: impl Into<PathBuf>) -> Self {
        Self {
            bin_dir: bin_dir.into(),
        }
    }

    /// Kiểm tra xem TSK binary có tồn tại không
    pub fn is_available(&self) -> bool {
        self.bin_path("mmls").exists()
    }

    fn bin_path(&self, tool: &str) -> PathBuf {
        let ext = if cfg!(target_os = "windows") {
            ".exe"
        } else {
            ""
        };
        self.bin_dir.join(format!("{}{}", tool, ext))
    }

    /// Chạy `mmls` — liệt kê layout phân vùng.
    ///
    /// ```text
    /// mmls <image_path>
    /// ```
    /// Output: bảng các partition với offset, size, description.
    pub async fn mmls(&self, image_path: &str) -> anyhow::Result<TskResult> {
        self.run_tsk("mmls", &[image_path]).await
    }

    /// Chạy `fsstat` — đọc thông số chi tiết filesystem.
    ///
    /// ```text
    /// fsstat -o <partition_offset> <image_path>
    /// ```
    pub async fn fsstat(&self, image_path: &str, partition_offset: u64) -> anyhow::Result<TskResult> {
        self.run_tsk(
            "fsstat",
            &["-o", &partition_offset.to_string(), image_path],
        )
        .await
    }

    /// Chạy `fls` — duyệt cây thư mục (bao gồm file đã xóa).
    ///
    /// Options:
    /// - `-r`: Recursive
    /// - `-d`: Chỉ hiện file đã xóa
    /// - `-l`: Long format (với timestamps)
    pub async fn fls(
        &self,
        image_path: &str,
        partition_offset: u64,
        recursive: bool,
        deleted_only: bool,
    ) -> anyhow::Result<TskResult> {
        let mut args = vec!["-o", &partition_offset.to_string()];
        let offset_str = partition_offset.to_string();
        args = vec!["-o", &offset_str];

        if recursive {
            args.push("-r");
        }
        if deleted_only {
            args.push("-d");
        }
        args.push("-l"); // Long format
        args.push(image_path);

        self.run_tsk("fls", &args).await
    }

    /// Chạy `icat` — trích xuất nội dung file theo inode/MFT entry.
    ///
    /// Output được ghi vào `output_path`.
    pub async fn icat(
        &self,
        image_path: &str,
        partition_offset: u64,
        inode: u64,
        output_path: &str,
    ) -> anyhow::Result<TskResult> {
        let offset_str = partition_offset.to_string();
        let inode_str = inode.to_string();
        let args = vec!["-o", &offset_str, image_path, &inode_str];

        // icat ghi stdout ra file
        let bin = self.bin_path("icat");
        let output = Command::new(&bin)
            .args(&args)
            .output()
            .await?;

        // Ghi stdout vào file
        if output.status.success() {
            tokio::fs::write(output_path, &output.stdout).await?;
        }

        Ok(TskResult {
            command: format!("icat {}", args.join(" ")),
            exit_code: output.status.code().unwrap_or(-1),
            stdout: format!("Written {} bytes to {}", output.stdout.len(), output_path),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            success: output.status.success(),
        })
    }

    /// Chạy một lệnh TSK bất kỳ.
    async fn run_tsk(&self, tool: &str, args: &[&str]) -> anyhow::Result<TskResult> {
        let bin = self.bin_path(tool);

        tracing::info!(tool = tool, args = ?args, "Running TSK command");

        let output = Command::new(&bin).args(args).output().await?;

        let result = TskResult {
            command: format!("{} {}", tool, args.join(" ")),
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            success: output.status.success(),
        };

        if !result.success {
            tracing::warn!(
                tool = tool,
                exit_code = result.exit_code,
                stderr = %result.stderr,
                "TSK command failed"
            );
        }

        Ok(result)
    }
}

impl Default for TskWrapper {
    fn default() -> Self {
        Self::new()
    }
}
