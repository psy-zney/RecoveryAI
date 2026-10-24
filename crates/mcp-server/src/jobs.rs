//! Blocking worker implementations. The overlay lock is released per 64 KiB chunk.
use crate::{handlers::RecoverySession, job_manager::JobManager, safe_output::PendingOutput};
use anyhow::{bail, ensure, Context, Result};
use disk_parsers::{fragment_matcher::evaluate_fragment_match, simd_scanner::StreamingScanner};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanParams {
    pub start_lba: Option<u64>,
    pub end_lba: Option<u64>,
    pub count_sectors: Option<u64>,
}
impl ScanParams {
    pub fn range(&self, total: u64) -> Result<(u64, u64)> {
        let start = self.start_lba.unwrap_or(0);
        ensure!(
            self.end_lba.is_none() || self.count_sectors.is_none(),
            "Specify end_lba or count_sectors, not both"
        );
        let end = match self.count_sectors {
            Some(count) => start.checked_add(count).context("Scan range overflow")?,
            None => self.end_lba.unwrap_or(total),
        };
        ensure!(
            start < end && end <= total,
            "Invalid scan range: use 0 <= start_lba < end_lba <= capacity (end exclusive)"
        );
        Ok((start, end))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FragmentParams {
    first_fragment_end_lba: u64,
    candidate_start_lba: Option<u64>,
    candidate_end_lba: Option<u64>,
    cluster_sectors: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloneParams {
    #[serde(alias = "output_path")]
    destination_image: String,
    mapfile: Option<String>,
}

pub fn validate_params(kind: &str, params: &Value, session: &RecoverySession) -> Result<()> {
    match kind {
        "SIMD_MAGIC_CARVE" | "SIMD_VBR_HUNT" => {
            serde_json::from_value::<ScanParams>(params.clone())?.range(session.total_sectors)?;
        }
        "FRAGMENT_PREFILTER" => {
            let p: FragmentParams = serde_json::from_value(params.clone())?;
            ensure!(
                p.first_fragment_end_lba < session.total_sectors,
                "Fragment tail outside source"
            );
            let size = p.cluster_sectors.unwrap_or(8);
            ensure!(
                size > 0 && size <= 128 && size.is_power_of_two(),
                "cluster_sectors must be a power of two in 1..128"
            );
            ScanParams {
                start_lba: p.candidate_start_lba,
                end_lba: p.candidate_end_lba,
                count_sectors: None,
            }
            .range(session.total_sectors)?;
        }
        "DD_RESCUE_CLONE" => {
            let p: CloneParams = serde_json::from_value(params.clone())?;
            let image = Path::new(&p.destination_image);
            let parent = image
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let filename = image
                .file_name()
                .and_then(|s| s.to_str())
                .context("Invalid clone filename")?;
            let target = crate::safe_output::output_path(&session.target_path, parent, filename)?;
            ensure!(!target.exists(), "Clone output already exists");
            let map = p
                .mapfile
                .unwrap_or_else(|| format!("{}.map", p.destination_image));
            let map = Path::new(&map);
            let parent = map
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let map = crate::safe_output::output_path(
                &session.target_path,
                parent,
                map.file_name()
                    .and_then(|s| s.to_str())
                    .context("Invalid mapfile filename")?,
            )?;
            ensure!(
                map != target && !map.exists(),
                "Mapfile must be new and distinct from clone image"
            );
        }
        _ => bail!("NotImplemented: unsupported job type {kind}"),
    }
    Ok(())
}

pub struct Worker<'a> {
    pub session: &'a Arc<RecoverySession>,
    pub manager: &'a JobManager,
    pub id: &'a str,
    pub cancel: &'a Arc<AtomicBool>,
    pub revision: u64,
}
impl Worker<'_> {
    fn check_cancel(&self) -> Result<()> {
        ensure!(!self.cancel.load(Ordering::Acquire), "Job cancelled");
        Ok(())
    }
    fn read(&self, lba: u64, count: usize, buffer: &mut [u8]) -> Result<()> {
        self.check_cancel()?;
        let mut disk = self.session.lock_for_job(self.cancel)?;
        ensure!(
            disk.revision() == self.revision,
            "RAM overlay changed during job; restart against a consistent view"
        );
        disk.operation(Arc::clone(self.cancel))
            .read_sectors(lba, count, buffer)?;
        Ok(())
    }
    pub fn run(&self, kind: &str, params: Value) -> Result<Value> {
        match kind {
            "SIMD_MAGIC_CARVE" | "SIMD_VBR_HUNT" => {
                self.scan(kind, serde_json::from_value(params)?)
            }
            "FRAGMENT_PREFILTER" => self.fragments(serde_json::from_value(params)?),
            "DD_RESCUE_CLONE" => self.clone_image(serde_json::from_value(params)?),
            _ => bail!("NotImplemented: unsupported job type {kind}"),
        }
    }
    fn hit_spool(&self) -> Result<crate::hit_store::HitWriter> {
        let path = self
            .session
            .session_dir
            .join(format!("{}_hits.jsonl", self.id));
        let spool = crate::hit_store::HitWriter::create(&path)?;
        self.manager.set_hits_path(self.id, path)?;
        Ok(spool)
    }
    fn scan(&self, kind: &str, p: ScanParams) -> Result<Value> {
        let (start, end) = p.range(self.session.total_sectors)?;
        let mut spool = self.hit_spool()?;
        let mut scanner = StreamingScanner::new()?;
        let mut hits = 0u64;
        let mut samples = Vec::new();
        let mut types: BTreeMap<String, u64> = BTreeMap::new();
        let mut lba = start;
        let mut buffer = vec![0; 65536];
        while lba < end {
            let count = (end - lba).min(65536 / self.session.sector_size as u64) as usize;
            let bytes = &mut buffer[..count * self.session.sector_size as usize];
            self.read(lba, count, bytes)?;
            if kind == "SIMD_MAGIC_CARVE" {
                for mut hit in scanner.scan_next(bytes, lba * self.session.sector_size as u64)? {
                    hit.lba = hit.byte_offset / self.session.sector_size as u64;
                    if hit.byte_offset < start * self.session.sector_size as u64 {
                        continue;
                    }
                    spool.append(json!({"lba":hit.lba,"byte_offset":hit.byte_offset,"extension":hit.extension,"signature":hit.file_type}))?;
                    hits += 1;
                    *types.entry(hit.extension.clone()).or_default() += 1;
                    if samples.len() < 8 {
                        samples.push(json!({"lba":hit.lba,"byte_offset":hit.byte_offset,"extension":hit.extension}));
                    }
                }
            } else {
                for (i, sector) in bytes
                    .chunks_exact(self.session.sector_size as usize)
                    .enumerate()
                {
                    self.check_cancel()?;
                    let ntfs = disk_parsers::ntfs_parser::analyze_ntfs_vbr(sector);
                    let fat = disk_parsers::fat_parser::analyze_fat32(sector);
                    if ntfs.valid || fat.valid {
                        hits += 1;
                        let kind = if ntfs.valid { "NTFS" } else { "FAT32" };
                        spool.append(json!({"lba":lba+i as u64,"fs_type":kind}))?;
                        *types.entry(kind.into()).or_default() += 1;
                        if samples.len() < 8 {
                            samples.push(json!({"lba":lba + i as u64,"fs_type":kind}));
                        }
                    }
                }
            }
            lba += count as u64;
            self.manager
                .progress(self.id, lba - start, end - start, hits);
        }
        self.check_cancel()?;
        spool.finish()?;
        Ok(
            json!({"start_lba":start,"end_lba":end,"sectors_scanned":end-start,"hits_count":hits,"counts_by_type":types,"sample_hits":samples,"samples_truncated":hits > samples.len() as u64}),
        )
    }
    fn fragments(&self, p: FragmentParams) -> Result<Value> {
        let mut spool = self.hit_spool()?;
        let size = p.cluster_sectors.unwrap_or(8);
        let (start, end) = ScanParams {
            start_lba: p.candidate_start_lba,
            end_lba: p.candidate_end_lba,
            count_sectors: None,
        }
        .range(self.session.total_sectors)?;
        let mut tail = vec![0; self.session.sector_size as usize];
        self.read(p.first_fragment_end_lba, 1, &mut tail)?;
        let mut head = vec![0; size * self.session.sector_size as usize];
        let mut top = Vec::new();
        let mut candidates = 0u64;
        let mut lba = start;
        while lba < end {
            let count = (end - lba).min(size as u64) as usize;
            if lba <= p.first_fragment_end_lba && p.first_fragment_end_lba < lba + count as u64 {
                lba += count as u64;
                continue;
            }
            self.read(
                lba,
                count,
                &mut head[..count * self.session.sector_size as usize],
            )?;
            let candidate = evaluate_fragment_match(
                &tail,
                &head[..(count * self.session.sector_size as usize)
                    .min(self.session.sector_size as usize)],
                p.first_fragment_end_lba,
                lba,
            );
            spool.append(serde_json::to_value(&candidate)?)?;
            top.push(candidate);
            top.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
            top.truncate(5);
            candidates += 1;
            lba += count as u64;
            self.manager
                .progress(self.id, lba - start, end - start, candidates);
        }
        self.check_cancel()?;
        spool.finish()?;
        Ok(
            json!({"candidates_scored":candidates,"top_candidates":top,"scoring":"entropy_and_byte_coherence_heuristic","format_validated":false}),
        )
    }
    fn clone_image(&self, p: CloneParams) -> Result<Value> {
        ensure!(
            self.session.lock_for_job(self.cancel)?.patch_count() == 0,
            "Clone requires an unpatched session"
        );
        let image = Path::new(&p.destination_image);
        let parent = image
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut output = PendingOutput::new(
            &self.session.target_path,
            parent,
            image
                .file_name()
                .and_then(|s| s.to_str())
                .context("Invalid clone filename")?,
        )?;
        let map_path = p
            .mapfile
            .unwrap_or_else(|| format!("{}.map", p.destination_image));
        let map_path = Path::new(&map_path);
        let parent = map_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut map = PendingOutput::new(
            &self.session.target_path,
            parent,
            map_path
                .file_name()
                .and_then(|s| s.to_str())
                .context("Invalid mapfile filename")?,
        )?;
        output.preserve_on_error();
        map.preserve_on_error();
        let map_budget = self
            .session
            .total_sectors
            .checked_mul(64)
            .and_then(|n| n.checked_add(1024))
            .context("Mapfile allocation size overflow")?;
        map.preallocate(map_budget)?;
        writeln!(map.file()?, "# resc block clone mapfile\n# current_pos current_status current_pass\n0x0 ? 1\n# pos size status")?;
        let mut buffer = vec![0; 65536];
        let mut lba = 0;
        let mut bad = 0u64;
        while lba < self.session.total_sectors {
            self.check_cancel()?;
            let count = (self.session.total_sectors - lba)
                .min(65536 / self.session.sector_size as u64) as usize;
            match self.read(
                lba,
                count,
                &mut buffer[..count * self.session.sector_size as usize],
            ) {
                Ok(()) => {
                    output
                        .file()?
                        .write_all(&buffer[..count * self.session.sector_size as usize])?;
                    writeln!(
                        map.file()?,
                        "0x{:x} 0x{:x} +",
                        lba * self.session.sector_size as u64,
                        count * self.session.sector_size as usize
                    )?;
                }
                Err(error) => {
                    if error.downcast_ref::<std::io::Error>().is_none() {
                        return Err(error);
                    }
                    for sector in lba..lba + count as u64 {
                        self.check_cancel()?;
                        let mut bytes = vec![0; self.session.sector_size as usize];
                        let status = match self.read(sector, 1, &mut bytes) {
                            Ok(()) => '+',
                            Err(error) if error.downcast_ref::<std::io::Error>().is_some() => {
                                bad += 1;
                                bytes.fill(0);
                                '-'
                            }
                            Err(error) => return Err(error),
                        };
                        output.file()?.write_all(&bytes)?;
                        writeln!(
                            map.file()?,
                            "0x{:x} 0x{:x} {}",
                            sector * self.session.sector_size as u64,
                            self.session.sector_size,
                            status
                        )?;
                    }
                }
            }
            lba += count as u64;
            self.manager
                .progress(self.id, lba, self.session.total_sectors, bad);
        }
        self.check_cancel()?;
        use std::io::Seek;
        let length = map.file()?.stream_position()?;
        map.file()?.set_len(length)?;
        let (image_path, map_path) = crate::safe_output::commit_pair(output, map)?;
        Ok(
            json!({"status":if bad==0 {"cloned"} else {"partial"},"bytes_written":lba*self.session.sector_size as u64,"bad_sectors":bad,"unreadable_sectors_zero_filled":bad,"image":image_path,"mapfile":map_path,"backend":"native_block_copy","resume_supported":false}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_io::{RawStorageReader, VirtualDiskOverlay};
    use std::io;
    struct FailingDisk;
    impl RawStorageReader for FailingDisk {
        fn len_bytes(&self) -> io::Result<u64> {
            Ok(4096)
        }
        fn read_physical_sectors(
            &mut self,
            lba: u64,
            count: usize,
            buf: &mut [u8],
        ) -> io::Result<()> {
            if lba <= 2 && lba + count as u64 > 2 {
                return Err(io::Error::other("injected bad sector"));
            }
            buf[..count * 512].fill(b'A');
            Ok(())
        }
    }
    #[test]
    fn clone_records_unreadable_sectors_instead_of_claiming_full_recovery() {
        let dir = std::env::temp_dir().join(format!("resc-clone-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let source = dir.join("source.img");
        std::fs::write(&source, vec![b'A'; 4096]).unwrap();
        let output = dir.join("clone.img");
        let map = dir.join("clone.map");
        let session = Arc::new(RecoverySession {
            session_id: "test".into(),
            target_path: source.to_str().unwrap().into(),
            overlay: Arc::new(tokio::sync::Mutex::new(
                VirtualDiskOverlay::new(Box::new(FailingDisk) as Box<dyn RawStorageReader + Send>)
                    .unwrap(),
            )),
            total_sectors: 8,
            sector_size: 512,
            session_dir: dir.clone(),
        });
        let manager = JobManager::new();
        let (id, cancel) = manager.create_job("clone").unwrap();
        let worker = Worker {
            session: &session,
            manager: &manager,
            id: &id,
            cancel: &cancel,
            revision: 0,
        };
        let result = worker
            .run(
                "DD_RESCUE_CLONE",
                json!({"destination_image":output,"mapfile":map}),
            )
            .unwrap();
        assert_eq!(result["status"], "partial");
        assert_eq!(result["bad_sectors"], 1);
        let bytes = std::fs::read(&output).unwrap();
        assert_eq!(&bytes[1024..1536], &[0; 512]);
        assert!(bytes[..1024].iter().all(|b| *b == b'A'));
        assert!(std::fs::read_to_string(&map)
            .unwrap()
            .contains("0x400 0x200 -"));
        assert_eq!(std::fs::read(&source).unwrap(), vec![b'A'; 4096]);
        for file in [output, map, source] {
            std::fs::remove_file(file).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn scan_ranges_reject_overflow_and_out_of_capacity_requests() {
        assert!(ScanParams {
            start_lba: Some(u64::MAX),
            count_sectors: Some(2),
            end_lba: None
        }
        .range(u64::MAX)
        .is_err());
        assert!(ScanParams {
            start_lba: Some(1),
            count_sectors: None,
            end_lba: Some(11)
        }
        .range(10)
        .is_err());
        assert_eq!(ScanParams::default().range(60000).unwrap(), (0, 60000));
    }
}
