# Changelog

All notable changes to **RecoveryAI** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [1.0.0] - 2026-10-25

### Added
- **Core Native Engine (Rust):**
  - Direct Win32 4KB DMA aligned physical drive reader (`crates/core-io`).
  - Copy-on-Write virtual RAM sandbox (`VirtualDiskOverlay`).
  - SCSI Watchdog and bad sector map tracking.
- **SIMD Forensic Parsers:**
  - Zero-copy MBR and GPT primary/backup decoders.
  - NTFS VBR parser, `$MFT` record analyzer, and Shannon entropy calculator.
  - FAT32/exFAT BPB and root directory cluster resolvers.
  - SIMD Aho-Corasick scanner (`ScannerEngine`) for rapid multi-signature file carving at GB/s.
  - Boundary Coherence mathematical matcher for fragmented media.
- **MCP Server & Protocol (JSON-RPC 2.0 stdio):**
  - Full implementation of 9 MCP tools + legacy aliases.
  - Background asynchronous task scheduler (`AsyncJobManager`) for non-blocking carving.
  - Direct physical disk enumeration tool (`list_physical_drives`).
- **Autonomous AI Skills (`.agents/skills/`):**
  - `disk-forensics` with 6 exhaustive playbooks.
  - `filesystem-repair` with NTFS, FAT32/exFAT, and EXT4 recipes.
  - `file-carving-and-defrag` with boundary coherence and atom reconstruction.
  - `mcp-disk-automation` with non-blocking polling and overlay budget management.
- **NPX Launcher & GitHub Distribution:**
  - Cross-platform `recoveryai-mcp` Node.js launcher with auto-download capability.
  - Automated GitHub Actions CI/CD workflows for Windows builds and releases.
  - Full end-to-end synthetic test suite (`tests/test_e2e_recovery.py`) passing 5/5 scenarios 100%.
