> **Implementation status:** See [implemented capabilities and limitations](docs/engine-capabilities.md)
> for the hardened engine's actual behavior: 512/4096-byte sectors, durable overlay WAL,
> asynchronous extraction, paginated hits and SHA-256 provenance manifests. Native NTFS/FAT reads use the overlay;
> unsupported features return explicit errors. Historical feature claims below are
> not a substitute for this capability reference or the live MCP tool schema.

<div align="center">

# 🛡️ resc
**Low-Level Disk Forensics & Data Recovery MCP Engine in Rust**

[![Rust](https://img.shields.io/badge/Built_with-Rust_&_SIMD-orange?logo=rust)](https://www.rust-lang.org/)
[![MCP](https://img.shields.io/badge/Protocol-MCP_(JSON--RPC)-blue)](https://modelcontextprotocol.io/)
[![Safety](https://img.shields.io/badge/Source_I/O-read--only-blue)](#safety-architecture-in-memory-virtual-overlay)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](./LICENSE)

*Use an MCP agent to inspect disk images and test recovery patches in a virtual overlay.*

[Architecture](#hybrid-brain--muscle-architecture) • [Safety Guarantee](#safety-architecture-in-memory-virtual-overlay) • [Quick Start](#quick-start) • [MCP Tools](#mcp-tools-catalog) • [Forensic Playbooks](#built-in-forensic-skills--playbooks)

</div>

---

![Illustrated resc recovery workflow](./assets/demo.gif)

This animation is a simulated illustration, not a recorded recovery session. Consult the capability reference for implemented operations.

## Why `resc`?

When a USB drive or hard disk turns `RAW`, loses its partition table, or suffers from corrupted `$MFT` / `FAT` headers, users face a painful choice:
1. **Commercial GUI tools ($70–$100+):** Expensive, black-box scanners that often fail on complex structural corruption or fragmented media (`moov` atom separation in MP4s, broken SQLite B-trees).
2. **Open-source CLI forensics (`TestDisk`, `The Sleuth Kit`, `ddrescue`, Hex Editors):** Extremely powerful, but require deep knowledge of LBA offsets, Boot Sector byte structures, and manual hex patching.

**`resc` bridges this gap.** However, you cannot simply feed a 500GB raw disk image into an LLM's context window. 

`resc` implements a **Hybrid Brain & Muscle Architecture**:
* **The Muscle (Rust):** Reads sources, parses disk structures, scans signatures, and computes entropy. Support varies by format and platform; see the capability reference.
* **The Brain (AI Agent via MCP):** Receives ultra-compact **< 5KB JSON Anomaly Reports**, diagnoses structural corruption, tests hex patches safely in **RAM (Copy-on-Write Overlay)**, and orchestrates file extraction.

---

## Hybrid "Brain & Muscle" Architecture

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                    AI AGENT (Claude / Cursor / Gemini)                  │
│  • Reads Forensic Playbooks (.agents/skills/)                           │
│  • Analyzes <5KB JSON Anomaly Maps & formulates recovery strategies     │
└───────────────────────────────────▲─────────────────────────────────────┘
                                    │ Model Context Protocol (JSON-RPC stdio)
┌───────────────────────────────────▼─────────────────────────────────────┐
│                        resc MCP SERVER (crates/mcp-server)              │
│  • Async Job Queue (Prevents MCP timeouts on multi-terabyte scans)      │
│  • Virtual Disk Overlay (512/4096-byte sectors and durable WAL)        │
└───────────────────────────────────▲─────────────────────────────────────┘
                                    │ Rust Internal API / Zero-Copy Structs
┌───────────────────────────────────▼─────────────────────────────────────┐
│                 NATIVE ENGINE (core-io / disk-parsers)                  │
│  • Layer 0: Read-only image and Windows physical-device I/O            │
│  • Layer 1: Signature scanning and entropy analysis                    │
│  • Layer 2: MBR/GPT and native NTFS/FAT validation                    │
│  • Layer 3: Background image extraction and cloning                    │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Safety Architecture: In-Memory Virtual Overlay

Source media are opened read-only by resc. Virtual patches are applied to an in-memory overlay and recorded in a session WAL; they are never written to the source by resc. This is **not a hardware write blocker**: the OS or another application may still modify shared media. Native NTFS/FAT validation checks root traversal and a file entry; it does not certify the whole filesystem. See [capabilities](docs/engine-capabilities.md) and [SECURITY.md](SECURITY.md) for the boundaries.

---

## Quick Start

### 1. MCP Client Configuration

Copy [mcp_config.example.json](mcp_config.example.json) into your MCP client configuration and replace its placeholders with absolute paths. The launcher requires a locally built or otherwise trusted native binary; it does not download one. It uses `RESC_MCP_BIN`, or the legacy `RECOVERYAI_MCP_BIN` fallback, when set.

If `resc-mcp` is available on npm, the equivalent package-based configuration is:

```json
{
  "mcpServers": {
    "resc": {
      "command": "npx",
      "args": ["-y", "resc-mcp"],
      "env": { "RESC_MCP_BIN": "<absolute-path-to-trusted-mcp-server-binary>" }
    }
  }
}
```

The npm package is named `resc-mcp`; its `resc`, `resc-mcp` and legacy
`recoveryai-mcp` commands all use `bin/resc-mcp.js`. To select the `resc` alias
explicitly, use `npx --package resc-mcp resc`. Bare `npx resc` resolves the npm
package named `resc` and requires that package to be published separately. Package publication and release availability must be verified separately; the npm package does not include the native binary.

### 2. Build from Source (Rust Workspace)

Prerequisites: **Rust 1.80+** (Cargo). For physical drive access on Windows, run your terminal/client as **Administrator**.

```bash
git clone https://github.com/psy-zney/resc.git
cd resc

# Build the release binary
cargo build --release --locked

# Run End-to-End synthetic corruption tests (Safe 1GB virtual images)
python ./tests/test_e2e_recovery.py
```

Windows supports image files and physical-drive inspection. CI also exercises image workflows on Ubuntu; raw devices on non-Windows platforms return `NotImplemented`. macOS is not covered by CI. Raw-device extraction and cloning are currently unavailable even on Windows. See [capabilities](docs/engine-capabilities.md).

---

## MCP Tools Catalog

The server advertises **10 primary tools** and four legacy aliases. `scan_filesystem_journals` returns `NotImplemented`. Other legacy calls may be accepted but are not advertised. The server reports MCP protocol version `2024-11-05` at initialization; consult the live schema for exact arguments.

| Tool Name | Layer | Description |
| :--- | :--- | :--- |
| `list_physical_drives` | Layer 0 | Enumerates Windows physical drives. |
| `open_recovery_session` | Layer 0 | Opens a supported physical drive or image read-only and initializes the overlay. |
| `scan_disk_topology` | Layer 1/2 | Rapidly inspects LBA 0 (MBR), LBA 1..33 (GPT), Backup GPT, and common VBR offsets. Returns a `<2KB` JSON anomaly map. |
| `inspect_sector_structure` | Layer 2 | Inspects up to eight sectors through the overlay and provides a bounded preview or header analysis. |
| `scan_filesystem_journals` | Layer 2 | Placeholder; journal and deleted-record scanning is not implemented. |
| `start_async_disk_job` | Layer 1/3 | Launches non-blocking background threads for `DD_RESCUE_CLONE`, `SIMD_VBR_HUNT`, `SIMD_MAGIC_CARVE`, or `FRAGMENT_PREFILTER`. |
| `poll_async_job` | Bridge | Streams real-time progress (`MB/s`, `bad_sectors_count`, discovered LBA hits) without timing out the MCP connection. |
| `read_scan_hits` | Bridge | Pages through persisted scan hits. |
| `virtual_patch_and_verify` | Layer 0/2 | Applies virtual patches and checks NTFS/FAT root traversal. |
| `extract_recovered_data` | Layer 3 | Starts an image extraction job with provenance manifests. |

---

## Built-in Forensic Skills & Playbooks

`resc` ships with an AI-native `.agents/skills/` knowledge base (inspired by [REA](https://github.com/morluto/rea)). Some playbooks describe aspirational operations; check the [capability reference](docs/engine-capabilities.md) before use:

* **`disk-forensics/`**: Partition table reconstruction (`01_partition_recovery.md`), RAW NTFS repair (`02_ntfs_raw_repair.md`), FAT32/exFAT backup boot sector restoration (`03_fat_exfat_repair.md`), hardware bad sector triage (`05_bad_sector_rescue.md`), and laptop recovery scenarios (`06_laptop_recovery_scenarios.md`).
* **`file-carving-and-defrag/`**: Statistical boundary coherence matching (`01_boundary_coherence.md`), fragmented MP4/MOV `moov`/`mdat` atom reconstruction (`02_mp4_mov_atom_repair.md`), and corrupted SQLite B-tree page salvage (`03_sqlite_database_salvage.md`).
* **`filesystem-repair/`**: Deep `$MFT` record surgical repair (`01_ntfs_mft_recovery.md`), FAT32 `FSInfo` reconstruction (`02_fat32_exfat_fsinfo.md`), and Linux `ext4` backup Superblock recovery (`03_ext4_superblock_recovery.md`).
* **`mcp-disk-automation/`**: Asynchronous job orchestration patterns (`01_async_job_orchestration.md`).

---

## Project Structure

```text
resc/
├── .agents/skills/          # 4 Specialized AI Forensic Skill Modules & 13 Playbooks
├── ai-context/              # Decision trees & tool catalogs for LLM context injection
├── bin/                     # Node.js NPX wrapper (resc-mcp.js) for instant MCP integration
├── crates/
│   ├── core-io/             # 4KB-aligned DMA buffers, SCSI watchdog, RAM Virtual Overlay
│   ├── disk-parsers/        # Zero-copy MBR/GPT/NTFS/FAT structs, SIMD carver, Defrag matcher
│   ├── external-bridges/    # Headless wrappers for The Sleuth Kit, PhotoRec, GNU ddrescue
│   └── mcp-server/          # Tokio JSON-RPC stdio server & Async Job Manager
└── tests/                   # Synthetic disk corruption generator & E2E verification suite
```

## Reliability, Verification & OS Boundaries

`resc` applies defensive checks when handling untrusted, corrupted disk media:

* **Verified test results:** Local Windows validation passed **46 unit tests**, **16 synthetic E2E groups**, and **1,000,000 deterministic malformed-input cases** across MBR, GPT, NTFS boot-sector and FAT32 parsers with no observed panics. This regression corpus is not a completed libFuzzer campaign or proof of safety for every input.
* **Persistent CoW overlay:** Patches for **512-byte and 4096-byte logical sectors** are synced to a session Write-Ahead Log (`.wal`) before changing RAM. Retained image sessions can replay verified records after restart when source identity and geometry match. Preserve `RESC_SESSION_ROOT`; interrupted jobs do not resume automatically.
* **TOCTOU and overwrite protection:** Background extraction uses no-follow flags and platform handle identity checks before writes. Exclusive `.part` files, `sync_all`, metadata-first publication and rollback protect clone/mapfile and file/manifest pairs. This is a recoverable protocol, not one atomic transaction across power loss. Completed extractions include SHA-256 provenance manifests (`.manifest.json`).
* **Kernel I/O boundaries:** Windows overlapped reads request `CancelIoEx` after a 500 ms timeout or job cancellation. Buffers remain alive until kernel completion; an unresponsive driver can still delay cancellation. There is no guaranteed hard deadline when the driver does not complete its request.
* **Panic boundaries:** Release builds require `panic = "unwind"`; handler and worker boundaries report caught panics as errors. `catch_unwind` does not isolate memory corruption, OOM aborts or non-terminating parser execution. Safe Rust and passing tests reduce risk without eliminating these boundaries.

[CI](.github/workflows/ci.yml) runs on Windows and Ubuntu. For commit [`19cb436`](https://github.com/psy-zney/resc/commit/19cb436651b660b7616be20aeb0c4d6e0d0be484), [Ubuntu CI](https://github.com/psy-zney/resc/actions/runs/40133458988/job/120989912337) passed release build, unit tests, Clippy and E2E. Local Windows release compilation was blocked by Application Control (OS error 4551); the local E2E results above used the debug binary. See the [verification report](docs/hardening-verification.md) and [implemented capabilities](docs/engine-capabilities.md) for evidence and limits.

## Disclaimer & Safety Notice

Data recovery on failing hardware carries inherent physical risks. If a drive is making mechanical clicking noises, **unplug it immediately** and seek a cleanroom lab. `resc` is strictly read-only at the software layer, but scanning a dying NAND flash chip or degraded platter can accelerate hardware failure—always use the `DD_RESCUE_CLONE` workflow first when `bad_sectors_count > 0`.

## License

Licensed under the [Apache License 2.0](./LICENSE).
