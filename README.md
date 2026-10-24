# RecoveryAI 🛡️⚡

> **High-Performance, SIMD-Accelerated Digital Forensics & Data Recovery Engine for Autonomous AI Agents via the Model Context Protocol (MCP).**

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Rust](https://img.shields.io/badge/Rust-2021_Edition-orange.svg)](https://www.rust-lang.org/)
[![Protocol](https://img.shields.io/badge/MCP-JSON--RPC_2.0-brightgreen.svg)](https://modelcontextprotocol.io/)
[![Node](https://img.shields.io/badge/Node->=_18.0.0-green.svg)](https://nodejs.org/)
[![Safety](https://img.shields.io/badge/Safety-100%25_Read--Only-red.svg)](AGENTS.md)
[![SIMD](https://img.shields.io/badge/SIMD-AVX2%2FSSE4.2-purple.svg)](crates/disk-parsers/)

---

## 🌟 Overview

**RecoveryAI** is an industrial-grade forensics and data recovery engine built in native **Rust** with **SIMD acceleration**, specifically designed to empower AI Agents (**Cursor**, **Claude Desktop**, **Google Antigravity**, **Windsurf**, and **VS Code Copilot**) with the ability to safely diagnose, repair, and recover lost data from damaged disks, corrupted file systems, formatted USB drives, and broken laptops.

Unlike traditional recovery software with opaque GUIs, RecoveryAI exposes fine-grained, evidence-based recovery tools over **JSON-RPC 2.0 stdio (Model Context Protocol)**. AI models can reason across sector boundaries, calculate partition geometry, verify repairs in an in-memory virtual sandbox, and stitch fragmented files without touching the physical media.

---

## 🔒 The 3 Golden Safety Invariants

1. **Strict 100% Read-Only Physical Access:**
   - Physical drives (`\\.\PhysicalDriveX`), volumes (`\\.\D:`), and raw disk images are opened exclusively with Win32 `GENERIC_READ` and `FILE_SHARE_READ | FILE_SHARE_WRITE`.
   - The engine contains **ZERO** code paths that issue raw physical write commands to target disks.
2. **Virtual Sandbox Isolation (Copy-on-Write RAM Overlay):**
   - All partition table rebuilds, boot sector fixes, and cluster pointer repairs are applied exclusively to an in-memory `VirtualDiskOverlay`.
   - AI models can safely simulate and verify fixes on RAM without any risk of irreversible damage.
3. **Progressive Disclosure (< 5KB Payloads):**
   - High-level tools return concise summaries to preserve LLM token context windows.
   - Heavy operations (e.g. SIMD magic carving over millions of sectors) run asynchronously in the background.

---

## 🚀 Quickstart & NPX Setup

### Option 1: Direct Execution via NPX (No installation required)

Run the server immediately using `npx`:

```bash
# Launch directly via NPX
npx recoveryai-mcp

# Or inspect interactively via the official MCP Inspector:
npx @modelcontextprotocol/inspector node bin/recoveryai-mcp.js
```

### Option 2: Add to Claude Desktop (`claude_desktop_config.json`)

On Windows, edit `%APPDATA%\Claude\claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "recoveryai": {
      "command": "node",
      "args": ["C:/Users/admin/MyProject/RecoveryAI/bin/recoveryai-mcp.js", "--read-only-enforced"],
      "env": {
        "RUST_LOG": "info"
      }
    }
  }
}
```

### Option 3: Add to Cursor IDE (`.cursor/mcp.json`)

```json
{
  "mcpServers": {
    "recoveryai": {
      "command": "node",
      "args": ["C:/Users/admin/MyProject/RecoveryAI/bin/recoveryai-mcp.js"]
    }
  }
}
```

### Option 4: Add to Antigravity IDE (`mcp_config.json`)

```json
{
  "mcpServers": {
    "recoveryai-native": {
      "command": "C:/Users/admin/MyProject/RecoveryAI/target/release/mcp-server.exe",
      "args": ["--read-only-enforced", "--max-ram-overlay-mb", "512"]
    }
  }
}
```

---

## 🛠️ MCP Tool Reference Matrix

The engine provides 9 standard tools designed specifically for AI reasoning:

| Tool Name | Latency | Purpose | Key Arguments |
|:---|:---:|:---|:---|
| `list_physical_drives` | Fast (<300ms) | Enumerates all connected disks, external USBs & enclosures with recovery hints | None |
| `open_recovery_session` | Fast (<50ms) | Validates target disk path, measures sector geometry & initializes RAM overlay | `target_path: string` (e.g. `\\.\PhysicalDrive1`, `D:`, or `case.img`) |
| `scan_disk_topology` | Fast (<100ms) | Inspects MBR, GPT primary/backup, and standard partition boundary offsets | `session_id: string` |
| `inspect_sector_structure` | Fast (<50ms) | Hex & ASCII inspection + Shannon entropy calculation of sector ranges | `session_id: string`, `lba: int`, `parse_as: string` |
| `scan_filesystem_journals` | Medium (<1s) | Scans NTFS `$MFT` unallocated records, `$UsnJrnl`, and FAT root clusters | `session_id: string`, `partition_start_lba: int` |
| `start_async_disk_job` | Immediate (<10ms)| Dispatches heavy background jobs (`SIMD_MAGIC_CARVE`, `FRAGMENT_PREFILTER`, `DD_RESCUE_CLONE`) | `session_id: string`, `job_type: string`, `params: object` |
| `poll_async_job` | Fast (<5ms) | Non-blocking telemetry for active background tasks | `session_id: string`, `job_id: string` |
| `virtual_patch_and_verify` | Fast (<50ms) | Tests MBR/VBR repairs in RAM Copy-on-Write overlay without physical write | `session_id: string`, `lba: int`, `patch_hex: string` |
| `extract_recovered_data` | Disk-bound | Exports verified files/partitions to safe destination folder | `session_id: string`, `source_type: string`, `target_dir: string` |

---

## 💻 Hardware & Storage Support

- **Storage Interfaces:** NVMe M.2 PCIe Gen 3/4/5, SATA SSD, 3.5" Desktop HDD, 2.5" Laptop HDD, USB Flash Drives, SD/microSD cards, and USB External Enclosures.
- **Sector Sizes:** 512-byte native (512n), 512-byte emulated (512e / Advanced Format), and 4096-byte native (4Kn) with automatic 4KB DMA alignment.
- **File Systems:** NTFS, FAT12, FAT16, FAT32, exFAT, Linux EXT2/3/4.
- **Image Formats:** Raw disk images (`.img`, `.dd`, `.raw`, `.bin`) and mounted VHD/VHDX volumes.

---

## 🧠 Autonomous AI Agent Skills (`.agents/skills/`)

The repository includes standardized domain skills with playbooks and exact byte layouts:

- [disk-forensics](.agents/skills/disk-forensics/SKILL.md): MBR/GPT reconstruction, backup GPT restoration, and 3-phase hardware rescue with GNU ddrescue.
- [filesystem-repair](.agents/skills/filesystem-repair/SKILL.md): In-memory virtual patch recipes for NTFS `$MFTMirr`, FAT32 FSInfo, and exFAT 12-sector boot checksums.
- [file-carving-and-defrag](.agents/skills/file-carving-and-defrag/SKILL.md): Boundary Coherence scoring algorithm, MP4 atom (`moov`/`mdat`) reassembly, and orphaned SQLite B-tree leaf page salvage.
- [mcp-disk-automation](.agents/skills/mcp-disk-automation/SKILL.md): Non-blocking asynchronous job orchestration and RAM overlay budget management.

---

## 🏗️ Monorepo Architecture

```
RecoveryAI/
├── bin/
│   └── recoveryai-mcp.js          # Cross-platform Node.js / npx launcher bridge
├── crates/
│   ├── core-io/                   # Low-level 4KB DMA aligned read-only Win32 I/O & RAM Copy-on-Write overlay
│   ├── disk-parsers/              # SIMD Aho-Corasick scanner, Shannon entropy, MBR/GPT, NTFS/FAT/exFAT
│   ├── external-bridges/          # Async subprocess runners for Sleuth Kit (TSK), PhotoRec, GNU ddrescue
│   └── mcp-server/                # JSON-RPC 2.0 stdio server implementing 9 MCP tools
├── target/release/mcp-server.exe  # Production native binary (panic=abort, stripped)
├── .agents/skills/                # Specialized AI agent skills and playbooks
├── ai-context/                    # JSON tool catalog schemas and diagnostic decision trees
├── tests/                         # E2E forensic validation suite
├── mcp_config.json                # Standard MCP client configuration
└── package.json                   # NPM package definition for npx execution
```

---

## 🧪 Building & Testing

### Prerequisites
- [Rust toolchain](https://rustup.rs/) (stable `x86_64-pc-windows-gnu` or `x86_64-pc-windows-msvc`)
- [Node.js](https://nodejs.org/) (>= 18.0.0)
- Python 3.x (for end-to-end test suite)

### Build Native Engine
```bash
# Build native release binary (with panic=abort for MinGW safety)
RUSTFLAGS="-C panic=abort" cargo build --release
```

### Run Unit Tests
```bash
cargo test
```

### Run End-to-End Forensic Test Suite
```bash
# Generates 4 synthetic corrupted disk images and validates all 5 recovery scenarios:
python tests/test_e2e_recovery.py
```

---

## 📄 License

Licensed under the **Apache License, Version 2.0**. See [LICENSE](LICENSE) for details.
