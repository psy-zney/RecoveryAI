# RecoveryAI Diagnostic Decision Tree for Autonomous AI Agents

This document dictates the optimal algorithmic path for an AI agent diagnosing any disk failure, data loss, or partition corruption.

---

## Phase 0: Session Initialization & Hardware Triage

```
Input: Path to drive or disk image (e.g. \\.\PhysicalDrive1 or disk.img)
Action: Call open_recovery_session(path)
```

1. **Check Hardware Health:**
   - Look at `bad_sectors_count` in session response.
   - If `bad_sectors_count > 0`:
     - **HALT DEEP SCANS IMMEDIATELY.**
     - Do not run intensive SIMD carvers directly on physical drive.
     - Advise user and trigger:
       `start_async_disk_job(job_type="DD_RESCUE_CLONE", params={"destination_image": "D:/clones/salvaged_disk.img"})`.
     - Continue recovery against the cloned `.img` file.
   - If `bad_sectors_count == 0`:
     - Proceed to **Phase 1: Partition Topology**.

---

## Phase 1: Partition Topology Analysis

```
Action: Call scan_disk_topology(session_id)
```

### Case 1.1: `mbr_valid: true` or `gpt_valid: true` with Partitions Found
- Check each partition in `partitions[]`:
  - If partition status is recognizable (`NTFS`, `FAT32`, `exFAT`, `EXT4`), inspect partition start LBA using `inspect_sector_structure(session_id, lba=start_lba)`.
  - If VBR is valid: Proceed to **Phase 2: Filesystem Analysis**.
  - If VBR is damaged / unrecognizable ("RAW"): Proceed to **Phase 3: Boot Sector & VBR Repair**.

### Case 1.2: `mbr_valid: false`, `gpt_valid: false`, No Partitions Found
- Check `backup_gpt_status`:
  - If `backup_gpt_status == "VALID"`:
    - Backup GPT is intact at the end of the disk (`Last LBA - 32`..`Last LBA`).
    - Use `inspect_sector_structure(session_id, lba=total_sectors - 33, count=33)` to inspect backup partition table.
    - Synthesize LBA 0..33 partition table and apply via `virtual_patch_and_verify`.
  - If `backup_gpt_status == "MISSING"` or `"CORRUPTED"`:
    - Partition table has been zeroed or wiped.
    - Trigger `start_async_disk_job(job_type="SIMD_VBR_HUNT")` to scan the disk for orphaned Volume Boot Records.
    - For each VBR discovered at `LBA_x`:
      - Extract `total_sectors` from VBR fields.
      - Calculate partition boundaries: `Start LBA = LBA_x`, `End LBA = LBA_x + total_sectors - 1`.
      - Construct MBR partition table entry or GPT entry.
      - Apply via `virtual_patch_and_verify(session_id, lba=0, patch_hex=...)`.

---

## Phase 2: Filesystem-Level Deleted File Recovery

```
Action: Call scan_filesystem_journals(session_id, partition_start_lba)
```

1. **NTFS Volumes:**
   - Evaluates `$MFT` records marked with `0x00` (Unallocated) in record flags.
   - Parses `$UsnJrnl` for recent file deletion timestamps.
   - Parses `$I30` directory index B-trees for orphaned file references.
   - If recoverable files are found:
     - Check `is_contiguous` and entropy of data runs.
     - Call `extract_recovered_data(session_id, source_type="TSK_TREE_EXTRACT", target_dir="D:/RecoveredData", file_ids_or_ranges=[file_ids])`.
2. **FAT32 / exFAT Volumes:**
   - Parses FAT Directory Tables for deleted file entries (leading byte `0xE5`).
   - Reconstructs file cluster chains from FAT1 / FAT2.
   - Call `extract_recovered_data(...)`.

---

## Phase 3: Boot Sector (VBR) Repair on RAM Overlay

```
Action: Target partition appears as "RAW" or "Unformatted"
```

1. **NTFS RAW Repair:**
   - NTFS stores a Backup VBR at the exact last sector of the partition (`End LBA`).
   - Call `inspect_sector_structure(session_id, lba=end_lba)`:
     - Verify signature `NTFS    ` at offset 3 and `0x55AA` at offset 510.
     - If Backup VBR is intact, extract its hex representation.
     - Call `virtual_patch_and_verify(session_id, lba=start_lba, patch_hex=backup_vbr_hex)`.
     - Check `verification_status`: if `"MOUNTABLE"`, proceed to **Phase 2**.
2. **FAT32 RAW Repair:**
   - FAT32 stores Backup Boot Sector at `Start LBA + 6`.
   - Inspect `Start LBA + 6`: verify `MSDOS5.0` or `FAT32   `.
   - Apply to `Start LBA` via `virtual_patch_and_verify`.
3. **exFAT RAW Repair:**
   - exFAT stores a 12-sector backup boot region at `Start LBA + 12`.
   - Verify 32-bit checksum across sectors 0..10 matching sector 11.
   - Apply backup boot region to `Start LBA` via `virtual_patch_and_verify`.

---

## Phase 4: Raw File Carving & Fragment Defragmentation

When filesystem structures (`$MFT` / `FAT`) are completely destroyed or overwritten:

```
Action: Call start_async_disk_job(job_type="SIMD_MAGIC_CARVE", params={"target_file_types": ["jpg", "png", "mp4", "zip", "pdf", "docx"]})
```

1. **Poll progress:** Call `poll_async_job(session_id, job_id)` until status is `"COMPLETED"`.
2. **Detect Fragmented Files:**
   - If a carved media file (e.g. `.mp4`, `.zip`, `.sqlite`) is truncated before its header declared size:
     - The file is fragmented across non-contiguous clusters.
     - Call `start_async_disk_job(job_type="FRAGMENT_PREFILTER", params={"first_fragment_end_lba": last_lba, "expected_file_type": "mp4"})`.
     - Poll for **Top 5 candidate fragments** ranked by Boundary Coherence Score.
     - Inspect candidate headers using `inspect_sector_structure`.
     - Stitch chosen cluster ranges using `extract_recovered_data(source_type="CARVE_BY_LBA_RANGES", file_ids_or_ranges=[[frag1_start, frag1_len], [frag2_start, frag2_len]])`.

---

## Phase 5: Reporting to User

After verification on RAM overlay and extraction, provide a structured report:
1. **Root Cause Analysis:** Physical, Partition Table, VBR, or File System Metadata.
2. **Virtual Verification Evidence:** Confirmed mountability and file tree preview from RAM overlay.
3. **Data Integrity Assurance:** Confirm original physical drive was never modified (100% read-only).
4. **Extraction Summary:** Total files salvaged and exact destination directory.
