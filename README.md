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
[![Safety](https://img.shields.io/badge/Physical_I/O-100%25_Read--Only-brightgreen)](#safety-architecture-in-memory-virtual-overlay)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](./LICENSE)

*Turn Claude, Cursor, or Gemini into a Digital Forensics & Data Recovery Specialist — without ever writing a single byte to your damaged drive.*

[Architecture](#hybrid-brain--muscle-architecture) • [Safety Guarantee](#safety-architecture-in-memory-virtual-overlay) • [Quick Start](#quick-start) • [MCP Tools](#mcp-tools-catalog) • [Forensic Playbooks](#built-in-forensic-skills--playbooks)

</div>

---

<!-- TODO: Replace with your 45-second terminal + Claude/Cursor split-screen GIF -->
<!-- ![resc Demo](./assets/demo.gif) -->

## Setup and recovery workbench

Read the **[Vietnamese step-by-step recovery guide](docs/recovery-guide.vi.md)**
for installation, selecting raw source images, diagnosing multiple failure types,
MP4/MOV box analysis, exact byte-range reconstruction and FFmpeg verification.
The workbench does not require choosing a failure hypothesis in advance.
For a computer that cannot boot its OS, start with the
[two-computer / Linux Live rescue guide](docs/two-computer-rescue.vi.md).
`scripts/rescue.py` adds Linux device inventory, GNU ddrescue acquisition with
resume/retry, an existing-Live-USB toolkit bundle, authenticated SSH control and
resumable hash-verified transfer of sealed acquisitions. SSH acquisition stages
the image on a healthy destination disk at the source computer before transfer;
it does not make a powered-off computer accessible through an ordinary USB cable.
Read the [professional literature review and research roadmap](docs/research/literature-review-and-roadmap.vi.md)
for 14 examined works and the distinction between implemented capabilities and
proposed metadata-, codec- and validation-guided reconstruction.
The [controlled deep-evidence study](docs/research/deep-evidence-study.vi.md)
records reproducible negative cases: two structurally valid PNG candidates can
conflict, and an MP4 can probe/decode all frames while containing wrong video data.
See the [three-case integration guide](docs/three-cases.vi.md) and independently
researched implementations for partition/boot damage, deleted FAT32 metadata and
fragmented content. Added tools include `analyze_partition_recovery` and
`analyze_file_candidate`; FAT32 deleted-entry discovery is an asynchronous job.

```bash
bash scripts/setup.sh
python scripts/recovery.py doctor
python scripts/recovery.py triage /path/to/source.img --deep
python scripts/recovery.py mcp-config --session-root /path/to/persistent-state
```

New MCP tools: `diagnose_recovery`, `analyze_video_candidate`. New scan job:
`ISO_BMFF_BOX_SCAN`. New export mode: `CARVE_BY_BYTE_RANGES`. These add evidence-led
triage and exact extent reconstruction; they do not infer arbitrary fragment order
or regenerate missing video metadata. See [algorithm provenance](docs/open-source-engines.md).

## Why `resc`?

When a USB drive or hard disk turns `RAW`, loses its partition table, or suffers from corrupted `$MFT` / `FAT` headers, users face a painful choice:
1. **Commercial GUI tools ($70–$100+):** Expensive, black-box scanners that often fail on complex structural corruption or fragmented media (`moov` atom separation in MP4s, broken SQLite B-trees).
2. **Open-source CLI forensics (`TestDisk`, `The Sleuth Kit`, `ddrescue`, Hex Editors):** Extremely powerful, but require deep knowledge of LBA offsets, Boot Sector byte structures, and manual hex patching.

**`resc` bridges this gap.** However, you cannot simply feed a 500GB raw disk image into an LLM's context window. 

`resc` implements a **Hybrid Brain & Muscle Architecture**:
* **The Rust engine:** Provides read-only image I/O, Windows physical I/O, streaming signature discovery, entropy measurements, MBR/GPT validation, native NTFS/FAT traversal and bounded MP4/MOV structure inspection. Actual throughput depends on the source and hardware.
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
│  • Virtual Disk Overlay (BTreeMap<LBA, 512] [u8;> In-Memory Patching)   │
└───────────────────────────────────▲─────────────────────────────────────┘
                                    │ Rust Internal API / Zero-Copy Structs
┌───────────────────────────────────▼─────────────────────────────────────┐
│                 NATIVE ENGINE (core-io / disk-parsers)                  │
│  • Layer 0 (Hardware I/O): 4KB-Aligned DMA, 500ms SCSI Watchdog         │
│  • Layer 1 (SIMD Scanner): AVX2 Magic Bytes & Shannon Entropy Map       │
│  • Layer 2 (FS Parsers): MBR, GPT, NTFS ($MFT, $I30, $UsnJrnl), FAT32   │
│  • Layer 3 (Bridges): Bifragment Coherence Matcher + TSK / ddrescue     │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Safety Architecture: In-Memory Virtual Overlay

Allowing an AI Agent to directly modify sectors on a damaged physical drive is dangerous. **`resc` enforces a strict hardware-level Read-Only policy:**

1. **Read-only source access:** Image files and supported Windows physical devices are opened read-only. Linux development uses regular raw images; raw-device export/isolation is not implemented. Output files and WALs are separate writable artifacts.
2. **Copy-on-Write (CoW) RAM Patching:** When the AI Agent wants to rebuild a destroyed MBR or restore a corrupted NTFS Volume Boot Record (VBR) from its end-of-volume backup, it calls `virtual_patch_and_verify`.
3. **Native verification:** Patches use the session logical sector size (512/4096), are journaled to a separate WAL and read through the overlay. A successful verification requires native NTFS/FAT root traversal and a valid file entry; it does not certify the whole volume or every file.

---

## Quick Start

### 1. Zero-Install via `npx` (MCP Client Configuration)

Add `resc` to your `claude_desktop_config.json` or `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "resc": {
      "command": "npx",
      "args": ["-y", "resc-mcp", "--read-only-enforced"]
    }
  }
}
```

The npm package is named `resc-mcp`; its `resc`, `resc-mcp` and legacy
`recoveryai-mcp` commands all use `bin/resc-mcp.js`. To select the `resc` alias
explicitly, use `npx --package resc-mcp resc`. Bare `npx resc` resolves the npm
package named `resc` and requires that package to be published separately.

### 2. Build from Source (Rust Workspace)

Prerequisites: **current Rust stable** (Cargo), **Node >=18**, and **Python >=3.11**. For physical drive access on Windows, run your terminal/client as **Administrator**.

```bash
git clone https://github.com/psy-zney/resc.git
cd resc

# External legacy forensic adapters are currently disabled; no companion downloads are needed.

# Build optimized release binary with AVX2/SIMD enabled
cargo build --release

# Run disposable synthetic corruption / reconstruction tests
python ./tests/test_e2e_recovery.py
python ./tests/test_recovery_workbench.py
python ./tests/test_deleted_recovery.py
python ./tests/test_three_cases.py
```

---

## MCP Tools Catalog

`tools/list` exposes **18 tools** including four legacy aliases. The live schema
and [AGENTS.md tool matrix](AGENTS.md#mcp-tools) are the parameter reference.

The main workflow is open → diagnose → inspect/scan → validate an evidence-based
repair or reconstruction → export asynchronously → verify provenance/content.
`scan_filesystem_journals` remains an explicit NotImplemented endpoint. New video
box scanning and byte-range exports supplement native NTFS/FAT allocated-file
extraction. They do not automatically reconstruct arbitrary fragmented files.

---

## Built-in Forensic Skills & Playbooks

`resc` ships with an AI-native `.agents/skills/` knowledge base (inspired by [REA](https://github.com/morluto/rea)) that equips AI coding assistants and agents with structured forensic SOPs:

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

Licensed under the [Apache License 2.0](./LICENSE). Workspace/package metadata and historical documents may contain older license labels; LICENSE is the project license document.
