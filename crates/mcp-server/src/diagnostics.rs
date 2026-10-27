//! Non-mutating triage and bounded video candidate inspection.
use crate::{
    byte_view::{ByteRange, ByteView},
    handlers::RecoverySession,
};
use anyhow::{Context, Result};
use disk_parsers::{
    fat_parser::analyze_fat32,
    ntfs_parser::{analyze_ntfs_vbr, calculate_shannon_entropy},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoParams {
    pub session_id: String,
    pub ranges: Vec<ByteRange>,
}

pub async fn video(session: Arc<RecoverySession>, ranges: Vec<ByteRange>) -> Result<Value> {
    tokio::task::spawn_blocking(move || {
        let mut disk = session.overlay.try_lock().context("Session busy: poll or cancel the active job, then retry")?;
        let capacity = disk.len_bytes();
        let length = crate::byte_view::validate(&ranges, capacity)?;
        let mut view = ByteView::new(&mut *disk, &ranges, capacity)?;
        let report = disk_parsers::iso_bmff::analyze(&mut view, length)?;
        Ok(json!({"session_id":session.session_id,"view":"ordered_byte_ranges","range_count":ranges.len(),"report":report,"next_step":"Validate exported candidate with FFmpeg/ffprobe; structural consistency does not prove correct fragment order or decoded integrity"}))
    }).await?
}

pub async fn triage(session: Arc<RecoverySession>) -> Result<Value> {
    tokio::task::spawn_blocking(move || {
        let mut disk = session.overlay.try_lock().context("Session busy: poll or cancel the active job, then retry")?;
        let layout = crate::topology::read_layout(&mut *disk, session.sector_size)?;
        let mut locations = vec![0];
        locations.extend(layout.partitions.iter().take(8).map(|p| p.start_lba));
        if session.total_sectors > 2048 { locations.push(2048); }
        locations.sort_unstable(); locations.dedup();
        let mut volumes = Vec::new();
        let mut sector = vec![0; session.sector_size as usize];
        let mut first_zero = false;
        for lba in locations {
            disk.read_sectors(lba, 1, &mut sector)?;
            if lba == 0 { first_zero = sector.iter().all(|b| *b == 0); }
            let ntfs = analyze_ntfs_vbr(&sector);
            let fat = analyze_fat32(&sector);
            let ntfs_geometry = ntfs.valid && ntfs.bytes_per_sector as u32 == session.sector_size
                && ntfs.total_sectors <= session.total_sectors - lba;
            let fat_geometry = fat.valid && fat.bytes_per_sector as u32 == session.sector_size
                && fat.total_sectors as u64 <= session.total_sectors - lba;
            let (kind, valid) = if ntfs_geometry { ("NTFS", true) } else if fat_geometry { ("FAT32", true) }
                else if sector.get(3..11) == Some(b"EXFAT   ".as_slice()) { ("exFAT_UNSUPPORTED", false) }
                else if sector.get(3..11) == Some(b"-FVE-FS-".as_slice()) { ("BITLOCKER_SIGNATURE", false) }
                else { ("UNKNOWN_OR_DAMAGED", false) };
            volumes.push(json!({"start_lba":lba,"classification":kind,"boot_geometry_valid":valid,"entropy_bits_per_byte":calculate_shannon_entropy(&sector)}));
        }
        let has_volume = volumes.iter().any(|v| v["boot_geometry_valid"] == true);
        let partition_scheme = if layout.primary_valid || layout.backup_valid { "GPT" }
            else if layout.mbr_valid && !layout.mbr.is_gpt_protective && !layout.partitions.is_empty() { "MBR" }
            else { "UNVERIFIED_OR_ABSENT" };
        let mut routes = vec![json!({"action":"SIMD_MAGIC_CARVE","reason":"Discover candidate file headers across all supported signatures; lengths and recovery are unverified"}), json!({"action":"ISO_BMFF_BOX_SCAN","reason":"Locate MP4/MOV ftyp/moov/mdat/moof candidates, including metadata separated from media"})];
        if has_volume { routes.insert(0, json!({"action":"TSK_TREE_EXTRACT","reason":"Try native allocated NTFS/FAT file extents first; filesystem metadata is stronger evidence than guessed fragment adjacency"})); }
        if partition_scheme == "UNVERIFIED_OR_ABSENT" || !has_volume { routes.push(json!({"action":"SIMD_VBR_HUNT","reason":"Search orphaned boot records; missing partition metadata does not prove files were erased"})); }
        let cases = json!([
            {"id":"PARTITION_BOOT","evidence":if partition_scheme=="UNVERIFIED_OR_ABSENT" {"PARTITION_METADATA_UNVERIFIED"} else {"PARTITION_METADATA_RECOGNIZED"},"next_tool":"analyze_partition_recovery"},
            {"id":"DELETED_METADATA","evidence":"DELETED_RECORDS_NOT_SCANNED","next_job":"FAT32_DELETED_SCAN","scope":"FAT32 root deleted SFN only; NTFS undelete unsupported"},
            {"id":"FRAGMENTS_CONTENT","evidence":"CONTENT_NOT_YET_VALIDATED","next_tool":"analyze_file_candidate","scope":"JPEG/PNG structure and MP4/MOV boxes; caller supplies fragment order"}
        ]);
        Ok(json!({"cases":cases,"session_id":session.session_id,"read_only":true,"sector_size":session.sector_size,"partition_scheme":partition_scheme,"first_sector_zeroed":first_zero,"volumes":volumes,"partitions_truncated":layout.partitions.len()>8,"routes":routes,"limitations":["Sampled boot locations only; not a full filesystem or hardware diagnosis","Entropy does not prove encryption, overwrite, TRIM or recoverability","NTFS deleted records/journals, RAID/LVM, encrypted volumes, exFAT/EXT4 recovery and unknown fragment ordering require other engines or evidence","No repair patches or extraction were performed"]}))
    }).await?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartitionParams {
    pub session_id: String,
    pub partition_start_lba: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentParams {
    pub session_id: String,
    pub ranges: Vec<ByteRange>,
    pub format: Option<String>,
}

pub async fn partition(session: Arc<RecoverySession>, lba: Option<u64>) -> Result<Value> {
    tokio::task::spawn_blocking(move || {
        let mut disk = session
            .overlay
            .try_lock()
            .context("Session busy: poll or cancel the active job, then retry")?;
        let mut report = crate::partition_recovery::analyze(&mut *disk, session.sector_size, lba)?;
        report["case_id"] = json!("PARTITION_BOOT");
        Ok(report)
    })
    .await?
}

pub async fn content(
    session: Arc<RecoverySession>,
    ranges: Vec<ByteRange>,
    format: String,
) -> Result<Value> {
    tokio::task::spawn_blocking(move || {
        let mut disk = session.overlay.try_lock().context("Session busy: poll or cancel the active job, then retry")?;
        let capacity = disk.len_bytes();
        let length = crate::byte_view::validate(&ranges, capacity)?;
        let mut view = ByteView::new(&mut *disk, &ranges, capacity)?;
        let report = disk_parsers::content_validation::analyze(&mut view, length, &format)?;
        Ok(json!({"case_id":"FRAGMENTS_CONTENT","session_id":session.session_id,"view":"ordered_byte_ranges","range_count":ranges.len(),"report":report}))
    }).await?
}
