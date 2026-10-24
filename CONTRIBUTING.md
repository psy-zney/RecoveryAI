# Contributing to RecoveryAI 🛡️

We welcome contributions from digital forensics specialists, systems engineers, and AI developers!

---

## 🔒 The 3 Golden Rules (Non-Negotiable)

1. **Strict 100% Read-Only Physical Access:**
   - Never write to physical drives (`\\.\PhysicalDriveX`). All device handles must specify `GENERIC_READ`.
2. **Virtual Sandbox Isolation (Copy-on-Write):**
   - All sector modifications, partition reconstructions, and cluster adjustments must occur in `VirtualDiskOverlay` on RAM.
3. **Progressive Disclosure (< 5KB Payloads):**
   - High-level tools must return lightweight summaries to avoid exhausting LLM token context windows.

---

## 🛠️ Development Workflow

### 1. Prerequisites
- Rust (stable toolchain: `x86_64-pc-windows-gnu` or `x86_64-pc-windows-msvc`)
- Node.js (>= 18.0.0)
- Python 3.10+ (for synthetic corrupted image tests)

### 2. Building
```bash
# Build native binary
cargo build --release

# Note: On GNU MinGW toolchain, ensure panic=abort is set:
$env:RUSTFLAGS="-C panic=abort"; cargo build --release
```

### 3. Running Tests
```bash
# Run unit tests across workspace crates
cargo test

# Run End-to-End Forensic Test Suite
python tests/test_e2e_recovery.py
```

### 4. Code Style & Rules
- **No Unaligned Packed References (E0793):** Never borrow references to fields in `#[repr(C, packed)]` structs. Always evaluate by value `{ field }` or copy into a local variable before formatting.
- **DMA Alignment:** Always use `DmaAlignedBuffer` (4096-byte page aligned) for Direct I/O.
- **Error Handling:** Use `anyhow::Result` in applications and `thiserror` in core libraries.
