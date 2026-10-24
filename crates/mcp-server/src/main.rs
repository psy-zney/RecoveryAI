//! # MCP Server — AI-Driven Disk Forensics & Data Recovery
//!
//! Entry point cho JSON-RPC 2.0 server qua stdio.
//! Cung cấp 8 MCP Tools chia theo 4 nhóm:
//!
//! ## Nhóm 1: Khám phá & Chẩn đoán
//! - `disk_scan_partitions`: Quét layout phân vùng (MBR + GPT)
//! - `disk_read_boot_sector`: Đọc và phân tích VBR (NTFS/FAT32)
//!
//! ## Nhóm 2: Quét & Phân tích
//! - `disk_scan_signatures`: Quét chữ ký file (SIMD carving)
//! - `disk_analyze_entropy`: Đo Shannon Entropy của vùng đĩa
//!
//! ## Nhóm 3: Vá thử & Phục hồi (Copy-on-Write)
//! - `disk_apply_patch`: Áp dụng bản vá ảo trên RAM
//! - `disk_rollback_patches`: Rollback bản vá
//!
//! ## Nhóm 4: Trích xuất & Cứu dữ liệu
//! - `disk_extract_file`: Trích xuất file qua TSK icat
//! - `disk_run_carving`: Chạy PhotoRec file carving

mod handlers;
mod job_manager;
mod tools_schema;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = real_main().await {
        eprintln!("Fatal error in MCP Server: {:#}", e);
        std::process::exit(1);
    }
    Ok(())
}

async fn real_main() -> Result<()> {
    // Khởi tạo logging (ghi ra stderr để không can thiệp stdio MCP)
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init()
        .ok();

    tracing::info!("AI Disk Forensics MCP Server starting...");
    tracing::info!("Protocol: JSON-RPC 2.0 over stdio");
    tracing::info!("Safety: 100% Read-Only + Virtual Overlay (Copy-on-Write RAM)");

    // Khởi tạo MCP Server
    let server = handlers::DiskForensicsServer::new().await?;

    // Chạy JSON-RPC stdio server
    server.run_stdio().await?;

    Ok(())
}
