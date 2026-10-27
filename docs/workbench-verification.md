# Recovery workbench verification — 2026-10-26

Validated on Linux x86_64, Rust stable 1.99.1, Node 24.19.0, Python 3.12.14 and
Debian FFmpeg/ffprobe 7.1.5. All functional evidence uses synthetic images/media.

| Check | Observed outcome |
| --- | --- |
| cargo fmt --all -- --check | Passed |
| cargo clippy --workspace --all-targets --locked -- -D warnings | Passed |
| cargo test --workspace --locked | 52 tests passed, 0 failed, 0 ignored; three doc-test targets contained 0 tests |
| cargo build --release --workspace --locked | Passed |
| python tests/test_e2e_recovery.py | All 16 existing E2E groups passed |
| python tests/test_recovery_workbench.py | 7 tests passed, 0 skipped in this environment |
| bash scripts/setup.sh | Passed with retained dependencies |
| Cloud installation script repeated | Passed; cached release build reused |
| Node launcher initialize / notifications / tools/list | Passed; 16 tools; generated JSON catalog matches live schema |
| git diff --check | Passed |

New functional checks cover non-sector-aligned fragments stored out of logical
order, cross-range parser reads, byte-exact output and SHA-256 provenance, rejected
overlap/out-of-capacity plans, existing-output protection, cross-chunk video box
signatures, general triage of a one-sector image, CLI deep multi-route scans,
native tree export and image cloning. Rust tests also check co64, extended box
sizes, nesting/table budgets and malformed-input parser behavior.

One FFmpeg-generated MPEG-4 video was split into two noncontiguous source ranges,
reconstructed through MCP, compared byte-for-byte using SHA-256, probed and fully
audio/video-decoded to null output without errors. The correct range ordering was
known from fixture construction. This does not demonstrate automatic discovery of
unknown fragment order, real-world damaged video recovery or missing-moov repair.

No physical devices, Windows execution, deleted-record recovery, exFAT/EXT4
recovery, RAID, encrypted-volume recovery or external PhotoRec/TSK/ddrescue
adapters were exercised. The optional FFmpeg test reports a skip on machines
where those executables are absent. Decode success is distinct from human review
of actual recovered content.

The existing locked binrw 0.11.3 dependency emits a nonfatal Cargo future-
incompatibility warning. Cargo.lock, dependency declarations and core I/O code
were unchanged. Setup instructions were saved to the cloud configuration draft;
publishing and restoration in a fresh task were not performed or claimed.


## Three-case integration validation (same environment)

Three independent agents added partition/boot analysis, FAT32 deleted-root-entry
scanning and JPEG/PNG structural validation. After integration:

| Check | Observed outcome |
| --- | --- |
| Workspace Clippy with -D warnings and formatting check | Passed |
| Workspace unit tests | 74 passed, 0 failed, 0 ignored; doc-test targets still contain 0 tests |
| Release build | Passed |
| Original forensic E2E suite | 16 groups passed |
| Workbench regression suite | 7 passed, 0 skipped, including real FFmpeg decode |
| FAT32 deleted-entry E2E suite | 3 passed |
| Three-case integration suite | 3 passed |
| Live MCP schema/catalog | 18 tools; FAT32_DELETED_SCAN job registered |

The combined fixture first repairs a damaged FAT32 boot in the overlay, discovers
a deleted one-cluster PNG candidate, validates PNG structure, exports exact bytes
and verifies manifest/output hashes while the original image hash stays unchanged.
Another fixture reconstructs primary GPT header addresses/CRC from backup evidence
and validates both copies in the overlay, with original bytes unchanged.

These are synthetic tests with known original data. They do not demonstrate
universal undelete, unknown fragment-order recovery or pixel/video content identity
on real damaged media. Independent sources and limits are recorded in each
research/case-*.md report. The earlier verification section documents the previous
upgrade stage; the current counts above supersede it for the integrated code.

## Controlled negative evidence (2026-10-27)

The reproducible synthetic study in
[`research/deep-evidence-study.vi.md`](research/deep-evidence-study.vi.md) exercised
six bounded PNG searches and five MP4/H.264 corruption cases through the release
MCP binary plus independent FFmpeg checks. It found one ambiguous PNG case with
two structurally valid contents and one cross-video packet splice that remained
structurally consistent, probed 12 frames and decoded without errors while its
decoded RGB hash differed from the original. All source-image hashes remained
unchanged. These are counterexamples and boundary checks, not recovery-rate data.

## Linux rescue/acquisition validation (2026-10-27)

`tests/test_rescue.py` adds 14 Linux tests. Real GNU ddrescue 1.29 was built outside
the checkout from Debian's upstream source archive with SHA-256
`35fcdca2acd98addbe9e620aeae4077ec1cf3e718cf6453e7eed021d31ea255d`
(matched against the HTTPS Debian `.dsc`; no independent signature verification
claimed). Its binary is not bundled or committed. Tests execute fast acquisition,
resume/retry/seal and upstream `--test-mode` fault injection against a synthetic
2 MiB image, proving one unreadable 512-byte region is reported PARTIAL and kept
as unresolved acquisition evidence rather than full recovery.

Other checks cover no-write planning, source identity drift, map gaps/overlap,
mutated sealed artifacts, same-backing/mounted-source/swap guards, case locking,
host argument validation, required strict-host-key SSH options, interrupted
transfer/resume, corrupt transfer rejection, toolkit hashes and triage binding
of image/map hashes. Block-device guards use mocked topology; no physical drive
was opened for acquisition. Local device inventory and setup readiness were run.
SSH process-protocol tests use an explicit local subprocess transport; no real
two-host connection, host-key authentication, Live USB boot, failing SSD or Windows
acquisition was exercised. CI installs gddrescue on Linux and runs this suite;
Linux-specific tests explicitly skip on other platforms.
