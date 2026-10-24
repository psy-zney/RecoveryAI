//! Honest MCP capabilities and parameter schemas. LBAs use the session sector size.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}
fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }
}
pub fn all_tools() -> Vec<ToolDefinition> {
    vec![
        tool("list_physical_drives","Enumerate Windows physical drives (read-only); up to eight samples, explicit errors if unavailable.",json!({}),&[]),
        tool("open_recovery_session","Open a source image or supported device read-only. Every LBA uses the session sector_size (512 or 4096). A durable WAL is restored with resume_session_id. Source-device exports require destination isolation and currently return NotImplemented.",json!({"target_path":{"type":"string"},"sector_size":{"type":"integer","enum":[512,4096]},"resume_session_id":{"type":"string","description":"Reopen an image session and replay its WAL"}}),&["target_path"]),
        tool("scan_disk_topology","Validate MBR partition bounds, both GPT header CRC32s, array CRC32s and partition bounds. Return compact partition samples.",json!({"session_id":{"type":"string"}}),&["session_id"]),
        tool("inspect_sector_structure","Read 1..8 sectors through the overlay. Return bounded hex preview or boot/header analysis; boot signatures do not establish mountability.",json!({"session_id":{"type":"string"},"lba":{"type":"integer","minimum":0},"count":{"type":"integer","minimum":1,"maximum":8},"parse_as":{"type":"string","enum":["RAW_HEX","MBR","GPT","NTFS_VBR","FAT32_BPB"]}}),&["session_id","lba"]),
        tool("scan_filesystem_journals","NotImplemented: deleted-record and journal recovery. This tool returns an explicit error.",json!({"session_id":{"type":"string"},"partition_start_lba":{"type":"integer","minimum":0}}),&["session_id","partition_start_lba"]),
        tool("start_async_disk_job","Run a cancellable background job. SIMD jobs accept start_lba/end_lba (exclusive) or count_sectors. DD_RESCUE_CLONE requires destination_image and optional mapfile; uses native block copying, no resume. FRAGMENT_PREFILTER requires first_fragment_end_lba; optional candidate_start_lba/candidate_end_lba/cluster_sectors. Fragment scores are heuristics, not format validation. Poll returns counts and bounded samples.",json!({"session_id":{"type":"string"},"job_type":{"type":"string","enum":["SIMD_MAGIC_CARVE","SIMD_VBR_HUNT","FRAGMENT_PREFILTER","DD_RESCUE_CLONE"]},"params":{"type":"object"}}),&["session_id","job_type"]),
        tool("poll_async_job","Poll compact progress/result. Set cancel=true to request cooperative cancellation; wait for Cancelled acknowledgement.",json!({"job_id":{"type":"string"},"cancel":{"type":"boolean","default":false},"cursor":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100}}),&["job_id"]),
        tool("read_scan_hits","Read persisted JSONL hits without rescanning. cursor is a byte offset returned by next_cursor. A page stops at the JSON budget even if limit is larger.",json!({"session_id":{"type":"string"},"job_id":{"type":"string"},"cursor":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100}}),&["session_id","job_id"]),
        tool("virtual_patch_and_verify","Apply checked virtual patches backed by the session WAL. SUCCESS requires native NTFS/FAT root-index traversal and a valid file entry. Specify partition_start_lba for an orphaned volume. UNVERIFIED means patches remain applied but filesystem validation failed.",json!({"session_id":{"type":"string"},"partition_start_lba":{"type":"integer","minimum":0},"copy_from_lba":{"type":"integer","minimum":0},"to_lba":{"type":"integer","minimum":0},"patches":{"type":"array","minItems":1,"maxItems":256,"items":{"type":"object","properties":{"lba":{"type":"integer","minimum":0},"offset":{"type":"integer","minimum":0,"maximum":4095},"hex_data":{"type":"string","maxLength":8192}},"required":["lba","offset","hex_data"],"additionalProperties":false}},"description":{"type":"string","maxLength":512}}),&["session_id"]),
        tool("extract_recovered_data","Starts a DATA_EXTRACTION job immediately; poll_async_job reports bytes_written and accepts cancellation. Each output has a SHA-256 provenance manifest, staged before data publication. Exclusive output creation, no overwrite. CARVE_BY_LBA_RANGES params: ranges [{start_lba,count}], optional filename. TSK_TREE_EXTRACT uses native parsers through RAM, params: {partition_start_lba}; exports allocated user files recursively, not deleted records or alternate streams. Tree quotas: 10000 files, 8 GiB, depth 64; errors preserve completed files, remove current incomplete file.",json!({"session_id":{"type":"string"},"mode":{"type":"string","enum":["CARVE_BY_LBA_RANGES","TSK_TREE_EXTRACT"]},"output_dir":{"type":"string"},"params":{"type":"object"}}),&["session_id","mode","output_dir"]),
        tool("disk_scan_partitions","Legacy alias: open an image/device and validate partition structures.",json!({"drive_number":{"type":"integer","minimum":0},"image_path":{"type":"string"}}),&[]),
        tool("disk_read_boot_sector","Legacy alias: inspect NTFS boot parameters at partition_lba.",json!({"partition_lba":{"type":"integer","minimum":0}}),&["partition_lba"]),
        tool("disk_apply_patch","Legacy alias for checked RAM byte patches.",json!({"lba":{"type":"integer","minimum":0},"offset":{"type":"integer","minimum":0,"maximum":4095},"hex_data":{"type":"string"},"description":{"type":"string"}}),&["lba","offset","hex_data","description"]),
        tool("disk_rollback_patches","Rollback all patches when rollback_all=true; otherwise undo only the last patch in the default session.",json!({"rollback_all":{"type":"boolean","default":false}}),&[]),
    ]
}

// ============================================================================
// Parameter Deserialization Structs
// ============================================================================

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenRecoverySessionParams {
    #[serde(alias = "path")]
    pub target_path: String,
    pub sector_size: Option<u32>,
    pub resume_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanDiskTopologyParams {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectSectorStructureParams {
    pub session_id: String,
    pub lba: u64,
    pub parse_as: Option<String>,
    pub count: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanFilesystemJournalsParams {
    pub session_id: String,
    pub partition_start_lba: u64,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartAsyncDiskJobParams {
    pub session_id: String,
    pub job_type: String,
    pub params: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PollAsyncJobParams {
    pub job_id: String,
    #[serde(default)]
    pub cancel: bool,
    #[serde(alias = "cursor_offset")]
    pub cursor: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SinglePatch {
    pub lba: u64,
    pub offset: usize,
    pub hex_data: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualPatchAndVerifyParams {
    pub partition_start_lba: Option<u64>,
    pub session_id: String,
    pub copy_from_lba: Option<u64>,
    pub to_lba: Option<u64>,
    pub patches: Option<Vec<SinglePatch>>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractRecoveredDataParams {
    pub session_id: String,
    #[serde(alias = "source_type")]
    pub mode: String,
    pub params: Option<Value>,
    #[serde(alias = "target_dir")]
    pub output_dir: String,
}

// Legacy structs
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanPartitionsParams {
    pub drive_number: Option<i32>,
    pub image_path: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadBootSectorParams {
    pub partition_lba: u64,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyPatchParams {
    pub lba: u64,
    pub offset: usize,
    pub hex_data: String,
    pub description: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackPatchesParams {
    pub rollback_all: Option<bool>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanSignaturesParams {
    pub start_lba: u64,
    pub count_sectors: Option<u64>,
    pub file_types: Option<Vec<String>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeEntropyParams {
    pub start_lba: u64,
    pub count_sectors: Option<u64>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractFileParams {
    pub partition_offset: u64,
    pub inode: u64,
    pub output_filename: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCarvingParams {
    pub file_types: Option<Vec<String>>,
    pub output_dir: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadScanHitsParams {
    pub session_id: String,
    pub job_id: String,
    #[serde(default, alias = "cursor_offset")]
    pub cursor: u64,
    pub limit: Option<usize>,
}
