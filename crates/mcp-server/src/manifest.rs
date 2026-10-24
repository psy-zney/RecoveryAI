//! Exact byte provenance and SHA-256 sidecars, committed before extracted data.
use anyhow::{ensure, Context, Result};
use core_io::virtual_overlay::PatchAudit;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
};
#[derive(Debug, Clone, Serialize)]
pub struct SourceRange {
    pub output_byte_offset: u64,
    pub source_byte_offset: u64,
    pub length_bytes: u64,
    pub start_lba: u64,
    pub end_lba_exclusive: u64,
}
impl SourceRange {
    pub fn new(output: u64, source: u64, length: u64, sector: usize) -> Self {
        Self {
            output_byte_offset: output,
            source_byte_offset: source,
            length_bytes: length,
            start_lba: source / sector as u64,
            end_lba_exclusive: (source + length).div_ceil(sector as u64),
        }
    }
}
#[derive(Default)]
pub struct ReadTrace {
    pub enabled: bool,
    pub ranges: Vec<SourceRange>,
    pub bytes: u64,
    pub bad_sectors: Vec<(u64, u64)>,
}
impl ReadTrace {
    pub fn begin(&mut self) {
        self.enabled = true;
        self.ranges.clear();
        self.bytes = 0;
    }
    pub fn record(&mut self, source: u64, length: u64, sector: usize) -> Result<()> {
        if !self.enabled || length == 0 {
            return Ok(());
        }
        if let Some(previous) = self.ranges.last_mut() {
            if previous.source_byte_offset + previous.length_bytes == source {
                previous.length_bytes += length;
                previous.end_lba_exclusive = (source + length).div_ceil(sector as u64);
                self.bytes += length;
                return Ok(());
            }
        }
        ensure!(
            self.ranges.len() < 65536,
            "File extent manifest exceeds 65536 ranges"
        );
        self.ranges
            .push(SourceRange::new(self.bytes, source, length, sector));
        self.bytes += length;
        Ok(())
    }
}
pub type SharedTrace = Arc<Mutex<ReadTrace>>;
#[derive(Clone)]
pub struct AuditContext {
    pub source: String,
    pub sector_size: usize,
    pub patches: Vec<PatchAudit>,
    pub bad_sectors: Vec<(u64, u64)>,
}
impl AuditContext {
    pub fn from_disk<R: core_io::RawStorageReader>(
        disk: &core_io::VirtualDiskOverlay<R>,
        source: &str,
    ) -> Self {
        Self {
            source: source.into(),
            sector_size: disk.sector_size(),
            patches: disk.audit_patches(),
            bad_sectors: disk.bad_sector_ranges(),
        }
    }
}
pub struct AuditedOutput {
    data: crate::safe_output::PendingOutput,
    manifest: crate::safe_output::PendingOutput,
    hash: Sha256,
    size: u64,
    context: AuditContext,
}
impl AuditedOutput {
    pub fn new(context: &AuditContext, parent: &Path, name: &str) -> Result<Self> {
        let data = crate::safe_output::PendingOutput::new(&context.source, parent, name)?;
        let manifest = crate::safe_output::PendingOutput::new(
            &context.source,
            parent,
            &format!("{name}.manifest.json"),
        )?;
        Ok(Self {
            data,
            manifest,
            hash: Sha256::new(),
            size: 0,
            context: context.clone(),
        })
    }
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.data.file()?.write_all(bytes)?;
        self.hash.update(bytes);
        self.size += bytes.len() as u64;
        Ok(())
    }
    pub fn update_bad_sectors(&mut self, encountered: &[(u64, u64)]) {
        self.context.bad_sectors.extend_from_slice(encountered);
        self.context.bad_sectors.sort_unstable();
        self.context.bad_sectors.dedup();
    }
    pub fn finish(mut self, ranges: Vec<SourceRange>) -> Result<serde_json::Value> {
        ensure!(
            ranges
                .iter()
                .try_fold(0u64, |n, r| n.checked_add(r.length_bytes))
                == Some(self.size),
            "NotImplemented: exact provenance unavailable for this file's stream layout"
        );
        let sha = format!("{:x}", self.hash.finalize());
        let bad = self
            .context
            .bad_sectors
            .iter()
            .filter(|(lba, count)| {
                ranges
                    .iter()
                    .any(|r| *lba < r.end_lba_exclusive && lba.saturating_add(*count) > r.start_lba)
            })
            .copied()
            .collect::<Vec<_>>();
        let manifest = serde_json::json!({"version":1,"source_image":self.context.source,"output_file":self.data.final_path(),"size_bytes":self.size,"sha256_hash":sha,"sector_size":self.context.sector_size,"source_lba_ranges":ranges,"applied_ram_patches":self.context.patches,"unreadable_bad_sectors_in_range":bad});
        serde_json::to_writer_pretty(self.manifest.file()?, &manifest)
            .context("Cannot write forensic manifest")?;
        let (path, sidecar) = crate::safe_output::commit_pair(self.data, self.manifest)?;
        Ok(
            serde_json::json!({"output_path":path,"manifest_path":sidecar,"sha256_hash":sha,"size_bytes":self.size}),
        )
    }
}
