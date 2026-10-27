# Recovery decision tree for the implemented engine

Consult `tools/list` and [capabilities](../docs/engine-capabilities.md) first. All MCP
LBAs use the session sector_size (512 or 4096). Unsupported features return errors; never infer that a
recovery step occurred merely because it appears in an older playbook.

1. Open a regular image with `open_recovery_session(target_path, sector_size?)`.
   A zero-length, misaligned or unreadable source must fail. Raw device exports are
   currently unsupported because destination-device isolation is not implemented.
2. Start with `diagnose_recovery(session_id)` for multiple evidence-led routes. Use `scan_disk_topology(session_id)` for MBR capacity and GPT CRC/range checks.
   Inspect a partition with `inspect_sector_structure`, using `count` between 1 and 8.
3. If a boot record is damaged, inspect its backup at an evidence-derived address.
   Apply `virtual_patch_and_verify` with a sector copy or checked byte patches and
   the actual `partition_start_lba`. No physical repair writes are allowed.
4. Only `SUCCESS` means a native NTFS/FAT parser traversed the root index and found
   a valid file entry. `UNVERIFIED` leaves the patch in RAM and supplies diagnostic
   errors; it does not establish mountability or recovered data integrity.
5. Export allocated user files with `extract_recovered_data(mode="TSK_TREE_EXTRACT",
   params={"partition_start_lba":...}, output_dir=...)`. This uses native overlay
   parsing. Export known byte runs with `CARVE_BY_LBA_RANGES` and
   `params={"ranges":[{"start_lba":...,"count":...}]}`. Use a new trusted directory.
   Poll the DATA_EXTRACTION job for bytes_written and completion; verify its manifest.
6. For signature discovery, dispatch `SIMD_MAGIC_CARVE`; for orphan boot-sector
   candidates use `SIMD_VBR_HUNT`. Set optional `start_lba`/`end_lba` (exclusive) to
   narrow a scan. Poll reports counts and eight samples. Read all persisted hits using
   `read_scan_hits(session_id, job_id, cursor, limit)` and each returned `next_cursor`.
7. `FRAGMENT_PREFILTER` scores candidate blocks statistically. Its confidence field
   is a heuristic, not a probability or proof of a correct file. Independently
   validate reconstructed formats before reporting success.
8. `DD_RESCUE_CLONE` uses native image block copying and a mapfile. It has no resume
   or ddrescue retry passes. `partial` means unreadable sectors were zero-filled;
   preserve and report the mapfile. Do not describe such a clone as byte-identical.
9. Cancel a job with `poll_async_job(job_id, cancel=true)` and wait for acknowledgement.
   Windows pending reads request CancelIoEx after a 500 ms timeout or cancellation;
   a driver that does not complete cancellation can still delay the worker.

Journal/NTFS-deleted-record scanning, exFAT/EXT4 recovery, PhotoRec execution and legacy
single-inode export return `NotImplemented`. A tool error must stop any dependent
workflow. State partial output, read failures and verification limitations explicitly.

All MCP LBAs use the session sector_size (512 or 4096). Preserve RESC_SESSION_ROOT
for WAL and hit logs. An image can be reopened with resume_session_id; source
identity and geometry must match. Never build with panic=abort. Preserve staged
clone/map or file/manifest artifacts when a commit returns recovery paths.

For separated MP4/MOV metadata, run ISO_BMFF_BOX_SCAN, then validate an evidence-
based ordered byte plan with analyze_video_candidate. Exact unaligned extents can
be exported using CARVE_BY_BYTE_RANGES. Do not infer fragment ownership from
nearby box signatures or treat STRUCTURE_CONSISTENT as decoded video integrity.
Use the recovery-triage skill and optional FFmpeg verification described in the
Vietnamese guide. These steps supplement rather than replace filesystem extents.

The three-case workflow now adds analyze_partition_recovery for bounded backup
repair evidence, FAT32_DELETED_SCAN for root deleted SFN allocation evidence, and
analyze_file_candidate for JPEG/PNG/MP4/MOV content structure. Follow
[the integration guide](../docs/three-cases.vi.md); these capabilities share one
overlay and byte-plan format. A candidate from FAT32 deletion is not verified
content, and no unknown multi-cluster chain or fragment order is inferred.
