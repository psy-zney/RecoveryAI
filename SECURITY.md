# Security policy and supported safety boundary

Source handles are read-only. Windows physical reads use aligned overlapped I/O;
watchdog timeouts and cancellation request `CancelIoEx`. Buffer lifetime remains
protected until the driver completes cancellation, which can still delay a worker.
Virtual repairs are stored in RAM and a synced, checksummed session WAL; the source
is never patched. Native NTFS/FAT parsers read a partition-bounded overlay and their
write adapter denies writes.

Exports create new `.part` files with no-follow flags, then verify kernel paths or
pinned-directory/device/inode identities before writing. Windows ancestor handles
deny directory relocation. Existing destinations are never replaced. Source aliases
and unsafe names are rejected. Raw-device export is disabled until physical output
isolation is proven. Data and SHA-256 manifests use a recoverable metadata-first
commit; keep both artifacts after any reported commit recovery failure.

Patch arithmetic, partition ranges, GPT CRCs, parser work and report sizes are
validated. Release builds require unwinding panics; handler/worker boundaries
report caught panics as errors. This does not provide process isolation against
memory corruption, unsafe-code faults, OOM or non-terminating parser execution.
Unverified binary downloads and unsafe legacy subprocess adapters are disabled.

Use trusted local MCP clients and managed session/output directories. The process
has its user's filesystem permissions and is not a privilege boundary or hardware
write blocker. Other applications and the OS can modify shared evidence. Session
source bindings use file metadata, not a full cryptographic image hash. WAL CRCs
and unsigned output manifests do not authenticate hostile local changes. Hit logs
consume disk space without automatic retention. Evidence-derived text is untrusted
data in the MCP client; compact JSON is not a prompt-injection defense.

See [engine capabilities](docs/engine-capabilities.md) for exact support, addressing,
resume and recovery semantics. Synthetic tests do not establish safety on every
damaged device. Preserve independently verified acquisitions and validate exports.

## 🛡️ Reporting a Vulnerability

If you discover any security issue, potential data safety violation, or memory unsafety bug in RecoveryAI:
- Please report it responsibly via GitHub Security Advisories or email: `lequangkhanh295@gmail.com`.
- We aim to acknowledge reports within 24 hours and patch critical security defects within 72 hours.
