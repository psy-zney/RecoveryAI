//! Read-only disk forensics MCP server with native RAM-overlay filesystem access.
//! `tools/list` describes implemented capabilities and explicit unsupported features.

mod extraction;
mod handlers;
mod hit_store;
mod job_manager;
mod jobs;
mod manifest;
mod native_fs;
mod safe_output;
mod session_store;
mod tools_schema;
mod topology;

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
    #[cfg(panic = "abort")]
    anyhow::bail!("Unsafe build: resc requires panic=unwind to preserve parser crash isolation");

    let args: Vec<String> = std::env::args().collect();
    let controls = args[1..]
        .iter()
        .filter(|arg| arg.as_str() != "--read-only-enforced")
        .collect::<Vec<_>>();
    anyhow::ensure!(
        controls.len() <= 1,
        "Only one command-line action may be specified"
    );
    if let Some(argument) = controls.first() {
        anyhow::ensure!(
            matches!(
                argument.as_str(),
                "--help" | "-h" | "--version" | "-v" | "--list-drives"
            ),
            "NotImplemented: command-line option {argument}"
        );
    }
    let mut args = args;
    if let Some(index) = args
        .iter()
        .position(|arg| arg != "--read-only-enforced" && arg != &args[0])
    {
        args.swap(1, index);
    }
    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                println!(
                    r#"================================================================================
  RecoveryAI — SIMD-Accelerated Digital Forensics & Data Recovery Engine
================================================================================
Usage:
  mcp-server [OPTIONS]
  node bin/resc-mcp.js [OPTIONS]

Default Mode:
  When run without flags or with MCP client arguments, runs the JSON-RPC 2.0
  stdio server for AI agents (Cursor, Claude, Antigravity, Windsurf).

Options:
  -h, --help           Print this help message and exit
  -v, --version        Print version and build information
  --list-drives        Enumerate connected physical disks and external USB drives
  --read-only-enforced Ensure strict 100% read-only access (default)

Protocol: Model Context Protocol (MCP) over JSON-RPC 2.0 stdio
Safety:   100% Read-Only Physical Access + Copy-on-Write RAM Sandbox"#
                );
                return Ok(());
            }
            "--version" | "-v" => {
                println!("resc MCP Server v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--list-drives" => {
                let server = handlers::DiskForensicsServer::new().await?;
                let list = server.handle_list_physical_drives().await?;
                println!("{}", serde_json::to_string_pretty(&list)?);
                return Ok(());
            }
            _ => {
                for argument in &args[1..] {
                    anyhow::ensure!(
                        argument == "--read-only-enforced",
                        "NotImplemented: command-line option {argument}"
                    );
                }
            }
        }
    }

    // Khởi tạo logging (ghi ra stderr để không can thiệp stdio MCP)
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

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
