//! Chunked, cancellable exports executed only by background workers.
use crate::{handlers::RecoverySession, job_manager::JobManager};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

pub struct ExtractionProgress<'a> {
    pub manager: &'a JobManager,
    pub id: &'a str,
    pub cancel: &'a Arc<AtomicBool>,
    pub bytes: AtomicU64,
    pub total: Option<u64>,
}
impl ExtractionProgress<'_> {
    pub fn check(&self) -> Result<()> {
        ensure!(!self.cancel.load(Ordering::Acquire), "Job cancelled");
        Ok(())
    }
    pub fn written(&self, bytes: u64) {
        let done = self.bytes.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.manager.extraction_progress(self.id, done, self.total);
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Range {
    start_lba: u64,
    count: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Carve {
    ranges: Vec<Range>,
    filename: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ByteCarve {
    ranges: Vec<crate::byte_view::ByteRange>,
    filename: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tree {
    partition_start_lba: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractionRequest {
    pub mode: String,
    pub output_dir: String,
    pub params: Option<Value>,
}

pub fn run(
    session: &Arc<RecoverySession>,
    request: ExtractionRequest,
    progress: &ExtractionProgress<'_>,
) -> Result<Value> {
    progress.check()?;
    match request.mode.as_str() {
        "CARVE_BY_LBA_RANGES" => {
            let config: Carve =
                serde_json::from_value(request.params.context("Missing carving ranges")?)?;
            ensure!(
                !config.ranges.is_empty() && config.ranges.len() <= 4096,
                "Specify 1..4096 carving ranges"
            );
            let mut total = 0u64;
            for range in &config.ranges {
                ensure!(
                    range.count > 0
                        && range
                            .start_lba
                            .checked_add(range.count)
                            .is_some_and(|end| end <= session.total_sectors),
                    "Carving range exceeds source capacity"
                );
                total = total
                    .checked_add(
                        range
                            .count
                            .checked_mul(session.sector_size as u64)
                            .context("Range size overflow")?,
                    )
                    .context("Total extraction size overflow")?;
            }
            let (context, revision) = {
                let disk = session.lock_for_job(progress.cancel)?;
                (
                    crate::manifest::AuditContext::from_disk(&disk, &session.target_path),
                    disk.revision(),
                )
            };
            let mut output = crate::manifest::AuditedOutput::new(
                &context,
                Path::new(&request.output_dir),
                config.filename.as_deref().unwrap_or("recovered_file.bin"),
            )?;
            let mut provenance = Vec::new();
            let mut written = 0;
            let mut buffer = vec![0; 65536];
            for range in config.ranges {
                provenance.push(crate::manifest::SourceRange::new(
                    written,
                    range.start_lba * session.sector_size as u64,
                    range.count * session.sector_size as u64,
                    session.sector_size as usize,
                ));
                written += range.count * session.sector_size as u64;
                let end = range.start_lba + range.count;
                let mut lba = range.start_lba;
                while lba < end {
                    progress.check()?;
                    let count = (end - lba).min((65536 / session.sector_size) as u64) as usize;
                    let bytes = count * session.sector_size as usize;
                    {
                        let mut disk = session.lock_for_job(progress.cancel)?;
                        ensure!(
                            disk.revision() == revision,
                            "Overlay changed during extraction"
                        );
                        disk.operation(Arc::clone(progress.cancel)).read_sectors(
                            lba,
                            count,
                            &mut buffer[..bytes],
                        )?;
                    }
                    progress.check()?;
                    output.write(&buffer[..bytes])?;
                    progress.written(bytes as u64);
                    lba += count as u64;
                }
            }
            progress.check()?;
            output.update_bad_sectors(&session.lock_for_job(progress.cancel)?.bad_sector_ranges());
            let mut result = output.finish(provenance)?;
            result["status"] = json!("extracted");
            result["mode"] = json!("CARVE_BY_LBA_RANGES");
            result["total_bytes_recovered"] = json!(total);
            Ok(result)
        }
        "CARVE_BY_BYTE_RANGES" => {
            let config: ByteCarve =
                serde_json::from_value(request.params.context("Missing byte ranges")?)?;
            let capacity = session.total_sectors * session.sector_size as u64;
            let total = crate::byte_view::validate(&config.ranges, capacity)?;
            let (context, revision) = {
                let disk = session.lock_for_job(progress.cancel)?;
                (
                    crate::manifest::AuditContext::from_disk(&disk, &session.target_path),
                    disk.revision(),
                )
            };
            let mut output = crate::manifest::AuditedOutput::new(
                &context,
                Path::new(&request.output_dir),
                config.filename.as_deref().unwrap_or("recovered_file.bin"),
            )?;
            let mut provenance = Vec::new();
            let mut written = 0u64;
            let mut buffer = vec![0; 65536];
            for range in config.ranges {
                provenance.push(crate::manifest::SourceRange::new(
                    written,
                    range.source_byte_offset,
                    range.length_bytes,
                    session.sector_size as usize,
                ));
                let mut offset = range.source_byte_offset;
                let end = offset + range.length_bytes;
                while offset < end {
                    progress.check()?;
                    let count = (end - offset).min(buffer.len() as u64) as usize;
                    {
                        let mut disk = session.lock_for_job(progress.cancel)?;
                        ensure!(
                            disk.revision() == revision,
                            "Overlay changed during extraction"
                        );
                        let mut operation = disk.operation(Arc::clone(progress.cancel));
                        operation.seek(SeekFrom::Start(offset))?;
                        operation.read_exact(&mut buffer[..count])?;
                    }
                    progress.check()?;
                    output.write(&buffer[..count])?;
                    progress.written(count as u64);
                    offset += count as u64;
                }
                written += range.length_bytes;
            }
            progress.check()?;
            output.update_bad_sectors(&session.lock_for_job(progress.cancel)?.bad_sector_ranges());
            let mut result = output.finish(provenance)?;
            result["status"] = json!("extracted");
            result["mode"] = json!("CARVE_BY_BYTE_RANGES");
            result["total_bytes_recovered"] = json!(total);
            result["format_validated"] = json!(false);
            Ok(result)
        }
        "TSK_TREE_EXTRACT" => {
            let config: Tree = serde_json::from_value(
                request
                    .params
                    .context("Specify partition_start_lba for native tree extraction")?,
            )?;
            let probe = crate::safe_output::output_path(
                &session.target_path,
                Path::new(&request.output_dir),
                "resc-output-probe",
            )?;
            let directory = probe.parent().context("Invalid output parent")?;
            let mut disk = session.lock_for_job(progress.cancel)?;
            let context = crate::manifest::AuditContext::from_disk(&disk, &session.target_path);
            let mut operation = disk.operation(Arc::clone(progress.cancel));
            crate::native_fs::extract_tree(
                &mut operation,
                config.partition_start_lba,
                &session.target_path,
                directory,
                progress,
                &context,
            )
        }
        _ => anyhow::bail!("NotImplemented: extraction mode {}", request.mode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_reports_bytes_and_cancellation_is_observed_before_next_chunk() {
        let manager = JobManager::new();
        let (id, cancel) = manager.create_job("DATA_EXTRACTION").unwrap();
        let progress = ExtractionProgress {
            manager: &manager,
            id: &id,
            cancel: &cancel,
            bytes: AtomicU64::new(0),
            total: Some(131072),
        };
        progress.written(65536);
        let info = manager.get_job(&id).unwrap();
        assert!(matches!(
            info.status,
            crate::job_manager::JobStatus::Running {
                bytes_written: 65536,
                progress_percent: 50.0,
                ..
            }
        ));
        manager.cancel_job(&id).unwrap();
        assert!(progress.check().is_err());
    }
}
