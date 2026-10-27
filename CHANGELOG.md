# Three-case integration (2026-10-26)

- Three independent agents researched and implemented partition/boot proposals,
  FAT32 deleted-root-entry evidence, and JPEG/PNG content boundaries/validation.
- Added analyze_partition_recovery, analyze_file_candidate and the asynchronous
  FAT32_DELETED_SCAN job; unified case IDs, sessions, byte plans and provenance.
- Extended CLI with analyze-partition, deleted-scan and analyze-file; full-image
  deep triage also scans detected FAT32 roots with explicit limits.
- Added independent research reports and cross-case synthetic regressions,
  including overlay boot repair -> deleted extent -> PNG validation -> exact export.
- Add a reproducible bounded-fragment/video counterexample study showing ambiguity,
  unsupported three-fragment search, and decode-success with wrong video content.

# Recovery workbench upgrade (2026-10-26)

- Add a Vietnamese installation/source-selection/recovery guide and portable MCP
  configuration generation; correct obsolete panic flags and capability claims.
- Add general read-only MCP triage, bounded MP4/MOV candidate inspection, streaming
  ISO BMFF box discovery and ordered exact-byte fragmented exports with provenance.
- Add a Python CLI for multi-route diagnostics, scans, tree extraction, image clone,
  reconstruction plans and independent optional FFmpeg/ffprobe verification.
- Add regressions for malicious plans, cross-chunk boxes, reordered unaligned
  fragments, CLI workflows and a real reconstructed video's full decode.
- Retain explicit limitations: no arbitrary fragment-order inference, missing-moov
  regeneration, deleted-record recovery or newly enabled external recovery adapters.

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
