# Implemented engine capabilities

The live `tools/list` schema and this document describe the implemented engine.
Older architectural examples and forensic playbooks may describe unsupported work.

## Linux Live and two-computer acquisition CLI

`scripts/rescue.py` is a separate stdlib acquisition CLI, not an MCP endpoint.
It inventories Linux devices through lsblk, verifies source/destination backing
with sysfs (including partitions/dm/md), rejects mounted/swap source backing,
and uses external GNU ddrescue for fast/retry passes and explicit mapfile resume.
Block inputs are opened read-only; output is a new private case on a separate
known local backing disk. Unknown backing (including common network/overlay
filesystems), unsupported virtual devices and ambiguous geometry fail closed.
The output image starts as sparse zeros; unresolved bytes retain unknown status
in rescue.map. Source identity checks are metadata checks, not hardware attestation.

Sealing hashes acquired image/map and prevents subsequent acquisition into that
case. SSH control uses keys and strict host-key checking; resumable transfer of
sealed artifacts verifies full SHA-256 before publication. SSH remote acquisition
stages the image on a healthy disk attached to the source computer. Direct remote
block acquisition into the helper computer is not implemented. A bundle command
copies the toolkit for an existing Linux Live USB; it does not create a boot ISO.
Windows/macOS remain supported for image recovery, not this Linux acquisition CLI.
See [the connection/setup guide](two-computer-rescue.vi.md).

`recovery.py triage --acquisition-manifest sealed.json|received.json` verifies the
image/map hashes and includes acquisition status/unresolved bytes. This does not
import ddrescue map ranges into the native MCP bad-sector map. Preserve both
acquisition and recovery manifests. The historical Rust external adapters and
raw-device export guards described below remain unchanged.

## Addressing and read-only access

Every MCP LBA uses the session's logical `sector_size`: **512 or 4096 bytes**.
Specify the image geometry when opening a session. Physical-device geometry is
queried from Windows. The internal `RawStorageReader` alone uses 512-byte blocks;
the overlay translates between these units. Do not reuse old 512-byte LBA values
in a 4096-byte session.

Sources are opened read-only. Windows physical reads use `GENERIC_READ`,
`FILE_FLAG_NO_BUFFERING | FILE_FLAG_OVERLAPPED`, and 4096-aligned buffers. Image
reads also use overlapped I/O. A 500 ms watchdog or job cancellation requests
`CancelIoEx` on a pending read. The request is drained before freeing its buffer;
a driver that fails to complete cancellation can still delay the worker. Timeouts
and failed reads are recorded in the bad-sector map. Cancellation is not reported
as an unreadable sector. This is not a hardware write blocker: the OS or another
application can still change shared source media.

## Persistent virtual repairs

`VirtualDiskOverlay` implements `Read + Seek` with dynamic 512/4096-byte sectors.
It never writes the source. Checked byte patches, undo and clear operations are
written to a CRC32-framed WAL and synced before changing RAM. Invalid patch bounds
return errors. A session has at most 65,536 patch-history entries; each patch fits
one logical sector, and each request contains at most 256 patches.

Session metadata, `overlay.wal` and hit JSONL files live under
`RESC_SESSION_ROOT/<session_id>` (default: the OS temp directory's `resc-sessions`).
Keep that directory to preserve recovery state; OS temp cleanup can remove it.
Reopen an image with the same `target_path`, `sector_size`, and
`resume_session_id`. A file-identity/length/mtime binding rejects a different source
or geometry. The WAL is exclusively locked, replays complete verified records and
truncates a torn trailing record only after validating the source binding. CRC
corruption is an error. The WAL has a 256 MiB limit; capacity errors are explicit.

Raw-device session resume returns `NotImplemented` until immutable hardware
identity is checked. Active jobs themselves do not resume after process exit.
The source binding is metadata, not an independent cryptographic image hash; use
an immutable acquisition and retain its independently computed hash.

## Native validation and asynchronous extraction

Partition-bounded streams pass the overlay directly to `ntfs` and `fatfs`. FAT's
required `Write` adapter rejects every write. `mount_status: SUCCESS` requires
boot parsing, root-index traversal, and at least one valid file entry. `UNVERIFIED`
leaves virtual patches applied and returns bounded diagnostics. Empty filesystems
deliberately do not meet this success condition. Root traversal does not certify
every file or prove an OS can mount the entire volume.

`extract_recovered_data` immediately returns a `DATA_EXTRACTION` job ID. Poll that
ID for `bytes_written`, terminal results, or cancellation acknowledgement. Workers
check cancellation between 64 KiB chunks; the stdio request loop remains available.
Raw exports reject an overlay revision change between chunks. Native tree exports
pin the overlay for traversal, while polling/cancellation use an independent job
registry. Synchronous inspection/patch calls return a busy error while the overlay
is held; background jobs waiting for it remain cancellable.

| Extraction mode | Behavior |
| --- | --- |
| `TSK_TREE_EXTRACT` | Compatibility name for native NTFS/FAT allocated user-file extraction through the overlay; `params: {partition_start_lba}` |
| `CARVE_BY_LBA_RANGES` | Streams supplied ranges, `params: {ranges: [{start_lba, count}], filename?: string}`; validates every range against capacity |

Tree quotas are 10,000 files, 8 GiB total, depth 64, bounded parser I/O/index work
and at most 65,536 provenance extents per file. Exceeding a quota is an error.
Deleted entries, alternate streams, NTFS system files, compressed/encrypted/sparse
attributes and attribute-list streams are unsupported for extraction. Raw ranges
do not infer file lengths or certify format validity.

## Output guards, manifests and commit recovery

Image exports canonicalize the destination, create `.part` files exclusively, and
verify the opened handle before writing bytes. Windows uses
`FILE_FLAG_OPEN_REPARSE_POINT`, kernel final paths, source-file identity checks and
pinned ancestor directory handles that deny delete sharing. Unix uses a pinned
directory descriptor, `openat(O_EXCL | O_NOFOLLOW)`, and path/device/inode checks.
Commit never replaces an existing destination. Raw-device export/clone remains
`NotImplemented` until output physical-device isolation can be verified.

Every extracted file has `<filename>.manifest.json`, containing its SHA-256,
exact source byte extents with session LBA ranges, active virtual patches, and
encountered unreadable sectors intersecting those extents. Resident NTFS provenance
accounts for update-sequence fixup bytes. Hashes describe the exported overlay
view, so virtual patch bytes may differ from the original image. Manifests are
not signed and are not a complete chain-of-custody system.

Data and metadata are staged and synced. Metadata is published first, data second.
A failed commit attempts to restore both `.part` names, preserves artifacts and
returns their recovery paths. Never delete those artifacts until investigated.
This is a recoverable two-file protocol, not a single atomic transaction across
power loss. Successful files from a partially failed tree export remain published;
the current incomplete file is removed unless commit recovery requires preservation.

## Scan and clone jobs

`start_async_disk_job` dispatches blocking work to Tokio's blocking pool. Scan jobs
release the overlay between chunks and fail on a changed patch revision.

| Job | Actual behavior | Parameters |
| --- | --- | --- |
| `SIMD_MAGIC_CARVE` | Signature discovery with at least 64-byte chunk overlap and duplicate suppression | Optional `start_lba`, `end_lba` (exclusive), or `count_sectors` |
| `SIMD_VBR_HUNT` | Examines every sector in the requested range for NTFS/FAT boot candidates | Same range parameters |
| `FRAGMENT_PREFILTER` | Entropy/boundary scoring of real candidate blocks; reports five best candidates | Required `first_fragment_end_lba`; optional `candidate_start_lba`, `candidate_end_lba`, `cluster_sectors` (power of two, 1..128) |
| `DD_RESCUE_CLONE` | Native image block copy plus mapfile, with per-sector fallback after failed reads | Required `destination_image`; optional `mapfile` |

No artificial scan-prefix cap is applied. Every scan hit is written to the
session's `<job_id>_hits.jsonl`; polling keeps counts and eight samples. Retrieve
all persisted hits with `read_scan_hits({session_id, job_id, cursor, limit})` or
`poll_async_job({job_id, cursor, limit})`. `cursor` is the byte offset returned as
`next_cursor`. Limits are 1..100 rows, further reduced to fit the JSON budget.
A partial line from an active writer is retried without advancing the cursor.
Persisted hits can be read after reopening their session without rescanning.
Hit files consume disk space; there is no automatic retention or disk quota.

At most four jobs run concurrently; up to 128 job records are retained, evicting
finished records when needed. Up to 32 sessions can be opened per process.
Use `poll_async_job({job_id, cancel: true})` and await `Cancelled`. A successful
commit racing with cancellation stays `Completed` and reports the created output.

Cloning is not GNU ddrescue and has no resume or retry passes. Patched sessions
are rejected. Unreadable sectors become zeros and `-` map records; status is
`partial`, not byte-identical success. The mapfile is physically preallocated,
both files are synced, then the mapfile is committed before the clone image.
Copy errors and cancellation preserve clone/map `.part` files for recovery.
A failed second rename rolls back the map rename and reports recovery paths.

## Parser, protocol and panic boundaries

MBR validation checks the signature, status, sizes, disk bounds and overlap.
GPT checks both header CRC32s, table CRC32s, metadata placement, partition ranges
and available primary/backup consistency. Input-derived parser slices use checked
access, and geometry arithmetic guards zero divisors and overflow.

Release builds use `panic = "unwind"`. Request dispatch and worker boundaries
convert unwinding panics into explicit errors. Abort-configured builds are
rejected at startup. `catch_unwind` does not isolate memory corruption, OOM aborts,
unsafe-code faults or non-terminating parser execution; there is no subprocess
parser sandbox.

Tool responses, including the JSON-RPC envelope, remain under 5 KB. `tools/list`
is a capability catalog and can exceed this budget. Input lines are limited to
256 KiB; notifications receive no response and unknown arguments are rejected.
The launcher runs only an existing local binary; unverified automatic downloads
are disabled. Journal/NTFS-deleted-record scanning, exFAT/EXT4 recovery, old inode and
PhotoRec aliases, and unsafe legacy subprocess adapters return `NotImplemented`.
Fragment scores are heuristics, not calibrated probabilities or format validation.

## Verification

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
python tests/test_e2e_recovery.py
node --check bin/resc-mcp.js
```

Use `RESC_FUZZ_CASES=1000000` for the extended deterministic malformed-parser test.
`fuzz/` supplies separate cargo-fuzz targets for malformed boot structures and
overlay read/seek/patch operations; extended libFuzzer campaigns are separate work.
The E2E suite checks synthetic NTFS/FAT/GPT recovery, source hashes, manifest
hashes and extents, cross-chunk signatures, clone/map output, pagination, 4Kn WAL
replay after restart, and cancellation of a large extraction over stdio. Set
`RESC_MCP_BIN` to select a freshly built binary; `RECOVERYAI_MCP_BIN` remains a
fallback for existing configurations.

Synthetic tests do not certify failing real hardware or every corrupted image.
Validate acquisitions and exported formats independently before using evidence.


## General triage and MP4/MOV byte reconstruction

`diagnose_recovery` samples boot structures, identifies MBR/GPT and NTFS/FAT32
candidates, recognizes exFAT/BitLocker markers and proposes multiple recovery
routes. It does not patch anything, diagnose hardware or infer encryption/TRIM
from entropy. The sampled locations and partition truncation are explicit.

`ISO_BMFF_BOX_SCAN` is an asynchronous streaming search for plausible
ftyp/moov/mdat/moof headers. It carries cross-chunk signatures, reads 32/64-bit
sizes, rejects headers smaller than their fields and reports declared length and
whether it fits the scan range. False signatures and unrelated boxes remain
possible. Size 0 is an unknown-to-end candidate, not an inferred file length.

`analyze_video_candidate` reads an ordered, non-overlapping plan of byte ranges
without exporting it. It checks box bounds, bounded container nesting and
stco/co64 chunk starts against mdat payloads. It supports 32/64-bit box lengths
and size 0 within the supplied view. It skips large media payloads by seeking.
Budgets: 4096 boxes, depth 12 and 65536 offsets. `STRUCTURE_CONSISTENT` is not
sample-extent verification, codec decoding, fragment-content identity or proof of
complete recovery. Fragmented MP4 is explicitly deferred to an external demuxer.

`CARVE_BY_BYTE_RANGES` accepts params `{ranges: [{source_byte_offset,
length_bytes}], filename?}`. It copies 1..4096 non-overlapping source extents in
output order, without requiring sector alignment. Source bounds/overflow checks,
overlay revision checks, cancellation, output guards and provenance manifests
apply. It does not guess fragment order or validate the exported format.

The Python workbench integrates optional local FFmpeg/ffprobe for MOV/MP4 demux
and full audio/video decode checks of exported candidates. This is CLI-level
verification, not an MCP subprocess tool or automatic codec repair. FFmpeg is
not bundled. See [the guide](recovery-guide.vi.md) and [licenses/provenance](open-source-engines.md).


## Independently researched three-case integration

The shared case IDs are PARTITION_BOOT, DELETED_METADATA and FRAGMENTS_CONTENT.
`diagnose_recovery` reports all three without forcing one failure hypothesis.
`analyze_partition_recovery` returns primary/backup GPT and NTFS/FAT32 evidence,
geometry/identity caveats and overlay-only proposals. GPT repair regenerates
location-dependent header fields and CRC; it never copies a backup header blindly.
Analysis does not apply repairs. See the case-1 research report for NTFS backup
location nuances and bounded hypotheses when volume boundaries are unknown.

`FAT32_DELETED_SCAN` accepts partition_start_lba and optional max_clusters (1..4096,
default 256). It scans deleted short entries in the FAT32 root directory, checks
FAT geometry, cycles, mirrors/active FAT and live root references, and spools hits
for pagination. Only one-cluster FAT-free candidates from a complete root scan
may include a byte export plan. Names are incomplete and content is unverified;
allocated, conflicting and unknown multi-cluster extents are not guessed. The
job holds the overlay while scanning; polling/cancellation remains independent.
Other inspection/patch calls can return busy. It does not implement journals,
NTFS undelete, subdirectory/LFN reconstruction or general deleted-chain recovery.

`analyze_file_candidate` validates ordered byte views for JPEG, PNG or MP4/MOV.
PNG validates chunk CRC/order and an IEND terminator; JPEG validates marker/segment
layout, table references and scan markers through EOI. Structure consistency is
not pixel decode or proof of correct fragment content. JPEG/PNG budgets are
64 MiB read, 16384 PNG chunks and 65536 JPEG markers. Unknown features, truncation
and budgets are distinct outcomes. MP4/MOV retains the ISO BMFF analyzer limits.

See [three-case guide](three-cases.vi.md) and independent reports under research/.
