# P1/P2 hardening verification

Local verification: Windows x86_64-pc-windows-gnu, 2026-10-25.
These results apply to synthetic fixtures and injected failures, not failing hardware.

| Requested fix | Implementation | Regression evidence |
| --- | --- | --- |
| Asynchronous extraction | DATA_EXTRACTION workers, 64 KiB cancellation checks and bytes_written; busy errors for synchronous overlay operations, cancellable background lock acquisition | Large 2 GiB extraction returns a job immediately; polling/cancellation stays live; incomplete output removed; lock contention and caught worker panic tests |
| Clone/map commit | Exclusive .part staging, map preallocation, sync_all on both, metadata first, data second, rollback and artifact recovery paths | Injected StorageFull during data/map writes and before image publication; map rollback preserves staged bytes; real clone and map agree with source |
| TOCTOU guards | Windows no-follow and final kernel paths with pinned ancestors; Unix openat/fstat identity checks | Existing source/output preserved; pinned handle identity checked; Unix parent-symlink swap regression included |
| Interruptible reads | Windows overlapped I/O, 500 ms watchdog, CancelIoEx and lifetime-safe completion drain | Named-pipe pending IRP is cancelled on timeout and on an AtomicBool signal; timeout enters bad-sector map |
| Panic/overlay durability | panic=unwind, handler/worker catches, checked parser access, native 512/4096 sectors and CRC-framed WAL | 4Kn boundary validation, torn-tail replay, durable undo and source/geometry mismatch rejection; 1,000,000 deterministic malformed-parser cases; worker panic does not stop polling |
| Persistent hits/manifests | JSONL spooling for scan and fragment candidates; byte-cursor pagination; SHA-256 and exact source extents with patches/bad-sector context | All 2,000 hits retrieved under 5 KB/page and reopened after restart; maximum row/partial tail tests; reconstructed NTFS/FAT/raw file bytes and manifest SHA-256 match |

Successful checks:

- `cargo test --workspace --offline`: **46 tests passed** (18 core I/O, 12 parsers,
  1 external bridge, 15 MCP server). Linux has an additional Unix-specific test.
- `RESC_FUZZ_CASES=1000000 cargo test --workspace --offline generated_malformed_sectors_do_not_panic`:
  passed; deterministic random/zero/truncated/marked boot structures, 64.68 seconds.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: passed.
- Workspace and standalone fuzz harness formatting: passed.
- Debug workspace build: passed.
- `tests/test_e2e_recovery.py` against `target/debug/mcp-server.exe`: **16 groups passed**.
- Node launcher syntax and Git whitespace checks (excluding pre-existing README
  trailing whitespace): passed.

Verification limits:

- Release compilation was attempted with unwinding enabled, but Windows Application
  Control blocked a dependency build script (`memoffset`, OS error 4551). Dependency
  proc-macro loading also failed in that attempt. No security policy was changed.
  A release build and E2E run against that release binary still need validation in
  an environment that permits the compiler's build artifacts to execute.
- Linux/Unix branches were not executed locally; CI includes Windows and Ubuntu.
- cargo-fuzz harnesses were added and formatted, but extended libFuzzer campaigns
  were not run. The million-case check above is a deterministic regression test.
- CancelIoEx requests cancellation; a broken driver can delay its completion.
  catch_unwind is not a subprocess sandbox. Two-file commit is recoverable but not
  one atomic power-failure transaction. Source bindings use file metadata rather
  than a full immutable image hash. See engine-capabilities.md for full limits.
