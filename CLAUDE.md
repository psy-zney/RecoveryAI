# CLAUDE.md — Claude Code Assistant Guidelines

## Commands
- **Build Native MCP Server:** `cargo build --release`
- **Run All Tests:** `cargo test` and `python tests/test_e2e_recovery.py`
- **Run Node Launcher / NPX:** `node bin/resc-mcp.js` or `npx .`
- **Inspect MCP Server:** `npx @modelcontextprotocol/inspector node bin/resc-mcp.js`

## Architecture & Code Map
- `crates/core-io`: Win32 read-only Direct I/O (`FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH`), 4KB DMA aligned memory buffers, and RAM Copy-on-Write (`VirtualDiskOverlay`).
- `crates/disk-parsers`: On-disk struct decoders (`MBR`, `GPT`, `NTFS VBR`, `FAT32/exFAT BPB`), Shannon Entropy calculator, and SIMD byte pattern scanner (`ScannerEngine`).
- `crates/external-bridges`: Subprocess bridges for Sleuth Kit (`fls`, `icat`), PhotoRec, and GNU ddrescue.
- `crates/mcp-server`: JSON-RPC 2.0 stdio server, tool dispatch router, and asynchronous job tracker (`AsyncJobManager`).

## Safety Invariant
Never attempt raw physical disk writes. All virtual patch trials must be executed inside `VirtualDiskOverlay` on RAM. Maintain 100% read-only integrity on physical drives (`\\.\PhysicalDriveX`).
