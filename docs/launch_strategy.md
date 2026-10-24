# 🚀 RecoveryAI — Comprehensive Launch Strategy & Distribution Playbook

This document details the exact launch strategy, submission drafts, copy text, and PR snippets to launch **RecoveryAI** across Hacker News, Reddit, and Global MCP Registries.

---

## 1. Hacker News (`Show HN`) Strategy

### Target Timing
- **Best Windows:** Tuesday or Wednesday between 07:00 AM – 09:00 AM EST (12:00 – 14:00 UTC).
- **Format:** Show HN text submission with direct GitHub repository link.

### Title Format (Direct, technical, non-clickbait)
```text
Show HN: RecoveryAI – An MCP server in Rust for low-level disk forensics and safe RAM repairs
```
*(Alternative:* `Show HN: RecoveryAI – Autonomous disk forensics in Rust with safe Copy-on-Write RAM overlays`*)*

### Submission Body Text
```markdown
Hi HN,

I built RecoveryAI, an open-source Model Context Protocol (MCP) server written in Rust that gives Claude, Cursor, and other AI agents the ability to perform low-level digital forensics and disk recovery on corrupted flash drives and hard disks.

GitHub: https://github.com/psy-zney/RecoveryAI

### The Problem
When a partition table disappears or a flash drive becomes "RAW" (corrupted VBR or unreadable $MFT), users generally have two options:
1. Proprietary GUI recovery suites ($70-$120+) that run opaque, slow brute-force carving and often stumble over fragmented media.
2. Forensic command-line utilities (TestDisk, The Sleuth Kit, ddrescue, hex editors). Extremely capable, but require calculating raw sector offsets, parsing filesystem geometry bytes by hand, and knowing how to safely patch boot records.

LLMs are exceptional at structural diagnosis and reverse-engineering file systems, but you obviously cannot feed a 500GB disk dump into an AI context window.

### Architecture: Hybrid Brain & Muscle
RecoveryAI separates raw byte-crunching from reasoning:
- The Muscle (Rust native engine): Performs raw 4KB-aligned unbuffered DMA reads (`FILE_FLAG_NO_BUFFERING` via Win32 `CreateFileW`), SIMD AVX2 Aho-Corasick magic header scanning, and Shannon entropy analysis (0.0 to 8.0 bits/byte). It compresses multi-gigabyte disk structures into concise < 5KB JSON Anomaly Reports.
- The Brain (AI Agent via MCP): Inspects the anomaly report, detects partition boundaries, locates backup boot records (e.g., Backup NTFS VBR at volume end), and simulates patches.
- Safe Virtual Overlay (Copy-on-Write): Physical drives are opened strictly with `GENERIC_READ`. All patch testing, partition rebuilding, and virtual filesystem mounts occur in a RAM overlay (`BTreeMap<u64, [u8; 512]>`). The physical disk is never touched with raw writes.

### Quick Test
You can attach it directly to Claude Desktop or Cursor without compiling Rust using npx:
```json
{
  "mcpServers": {
    "recoveryai": {
      "command": "npx",
      "args": ["-y", "recoveryai-mcp"]
    }
  }
}
```

Or run standalone:
```bash
cargo build --release
python tests/test_e2e_recovery.py
```

### Pre-empting Common Questions
- *Hallucination risk:* Because of the strict read-only handle + RAM Copy-on-Write overlay, even if an agent hallucinates a patch, it only exists in RAM. The physical drive remains bit-for-bit pristine.
- *Performance:* The SIMD signature scanner runs at multiple gigabytes per second on AVX2, and sector inspection takes <5ms.
- *Supported formats:* MBR, GPT (primary/backup), NTFS (VBR, $MFT, $LogFile), FAT32, exFAT, and raw SIMD carving for JPEG/PNG/PDF/ZIP/MP4/SQLITE.

Feedback, criticism, and code review from systems and forensics folks are very welcome!
```

---

## 2. Global MCP Registry Listings & Pull Requests

### A. Awesome MCP Servers (`punkpeye/awesome-mcp-servers`)
- **Repository:** https://github.com/punkpeye/awesome-mcp-servers
- **Section:** Under **"Operating System & File Systems"** or **"Developer Tools"**
- **Markdown PR entry:**
```markdown
- [RecoveryAI](https://github.com/psy-zney/RecoveryAI) - A SIMD-accelerated low-level disk forensics, file carving, and virtual RAM overlay recovery engine in Rust for safe physical drive inspection.
```

### B. Smithery.ai
- **URL:** https://smithery.ai/
- RecoveryAI includes `package.json` and a cross-platform launcher `bin/resc-mcp.js`.
- Register the repo directly at: `https://smithery.ai/new` with repository `psy-zney/RecoveryAI`.

### C. mcp.so & Glama
- Submit URL to:
  - https://mcp.so/submit
  - https://glama.ai/mcp/servers

---

## 3. Reddit Launch Strategy

### Subreddit: `r/rust`
- **Flair:** `Project`
- **Title:** `RecoveryAI: A low-level disk forensics & data recovery MCP server in Rust (AVX2 SIMD, 4KB DMA aligned, safe CoW RAM overlay)`
- **Core Talking Points:**
  - Zero-copy parsing using `bytemuck` and `zerocopy`.
  - Win32 Direct Memory Access without OS buffer caching (`FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH`).
  - Zero raw physical writes enforced at the type level (`Read` trait only on target handles).
  - Virtual Copy-on-Write overlay using `BTreeMap<u64, [u8; 512]>`.
  - Zero compiler warnings, 100% unit tests & E2E forensic tests passing.

### Subreddit: `r/netsec` or `r/cybersecurity`
- **Flair:** `Tool / Forensics`
- **Title:** `RecoveryAI: Open-source DFIR & automated partition triage engine via Model Context Protocol`
- **Core Talking Points:**
  - Automated detection of wiped MBRs, corrupted NTFS VBRs, and fragmented media.
  - Safe triage without risk of contamination (write-blocker equivalent in software).
  - Progressive disclosure payload model (<5KB per MCP roundtrip).

---

## 4. Key Launch Checklist

- [x] Zero Rust compiler warnings on `cargo check --workspace`
- [x] 100% Rust unit tests pass (`cargo test --workspace`)
- [x] 100% Real E2E scenarios pass (`python tests/test_e2e_recovery.py`)
- [x] Animated demo GIF (`assets/demo.gif`) embedded into `README.md`
- [x] Git repo cleaned of chat history files (`gemini-code-*.md` gitignore)
- [x] CI/CD workflows active on GitHub
- [ ] Submit Hacker News `Show HN` thread at optimal time window
- [ ] Open Pull Request to `punkpeye/awesome-mcp-servers`
- [ ] Submit to `smithery.ai` & `mcp.so`
