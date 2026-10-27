//! Bounded FAT32 deleted short-entry discovery, never speculative undelete.
//!
//! A deleted directory entry retains a first cluster and byte size, but normally
//! loses its FAT chain. Only a one-cluster free extent is offered as a candidate
//! export plan, and even that does not prove the original content survives.
use anyhow::{bail, ensure, Context, Result};
use core_io::{RawStorageReader, VirtualDiskOverlay};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_DIRECTORY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: u64 = 65536;
const MAX_CANDIDATES: usize = 4096;

fn le16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}
fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Acquire), "Job cancelled");
    Ok(())
}

struct Geometry {
    start: u64,
    sector: usize,
    sectors_per_cluster: u64,
    cluster_bytes: u64,
    reserved: u64,
    fat_sectors: u64,
    fat_copies: u64,
    mirrored: bool,
    active_fat: u64,
    data_start: u64,
    max_cluster: u32,
    root: u32,
}
impl Geometry {
    fn read<R: RawStorageReader>(
        disk: &mut VirtualDiskOverlay<R>,
        start: u64,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        check_cancel(cancel)?;
        let sector = disk.sector_size();
        ensure!(
            start < disk.len_bytes() / sector as u64,
            "Partition start outside source capacity"
        );
        let mut boot = vec![0; sector];
        disk.read_sectors(start, 1, &mut boot)?;
        check_cancel(cancel)?;
        ensure!(
            boot.get(510..512) == Some(&[0x55, 0xaa])
                && le16(&boot[11..13]) as usize == sector
                && le16(&boot[17..19]) == 0
                && le16(&boot[19..21]) == 0
                && le16(&boot[22..24]) == 0
                && le16(&boot[42..44]) == 0,
            "NotImplemented: scan requires valid FAT32 BPB/version and matching session geometry"
        );
        let sectors_per_cluster = boot[13] as u64;
        let reserved = le16(&boot[14..16]) as u64;
        let fat_copies = boot[16] as u64;
        let total = le32(&boot[32..36]) as u64;
        let fat_sectors = le32(&boot[36..40]) as u64;
        let flags = le16(&boot[40..42]);
        ensure!(
            sectors_per_cluster.is_power_of_two()
                && sectors_per_cluster <= 128
                && reserved > 0
                && (1..=2).contains(&fat_copies)
                && fat_sectors > 0,
            "Invalid FAT32 cluster/FAT geometry"
        );
        let data_start = reserved
            .checked_add(
                fat_copies
                    .checked_mul(fat_sectors)
                    .context("FAT size overflow")?,
            )
            .context("FAT region overflow")?;
        ensure!(total > data_start, "FAT regions exceed declared volume");
        ensure!(
            start
                .checked_add(total)
                .is_some_and(|end| end <= disk.len_bytes() / sector as u64),
            "FAT32 volume exceeds source capacity"
        );
        let cluster_count = (total - data_start) / sectors_per_cluster;
        ensure!(
            (65525..=0x0fff_fff4).contains(&cluster_count),
            "NotImplemented: cluster count is not valid FAT32 (FAT12/FAT16 are unsupported)"
        );
        ensure!(
            (cluster_count + 2) * 4 <= fat_sectors * sector as u64,
            "FAT table cannot address all declared data clusters"
        );
        let root = le32(&boot[44..48]);
        ensure!(
            root >= 2 && root as u64 <= cluster_count + 1,
            "Root cluster exceeds FAT32 data capacity"
        );
        let mirrored = flags & 0x80 == 0;
        let active_fat = if mirrored { 0 } else { (flags & 0xf) as u64 };
        ensure!(
            active_fat < fat_copies,
            "Active FAT index exceeds FAT copy count"
        );
        // If a valid partition table declares this volume, honor its stricter
        // extent. An explicitly selected orphan volume remains inspectable.
        let layout = crate::topology::read_layout(disk, sector as u32)?;
        if let Some(partition) = layout.partitions.iter().find(|p| p.start_lba == start) {
            ensure!(
                total <= partition.sector_count,
                "FAT32 volume exceeds enclosing partition bounds"
            );
        }
        Ok(Self {
            start,
            sector,
            sectors_per_cluster,
            cluster_bytes: sectors_per_cluster * sector as u64,
            reserved,
            fat_sectors,
            fat_copies,
            mirrored,
            active_fat,
            data_start,
            max_cluster: (cluster_count + 1) as u32,
            root,
        })
    }
    fn cluster_lba(&self, cluster: u32) -> Result<u64> {
        ensure!(
            (2..=self.max_cluster).contains(&cluster),
            "Cluster outside FAT32 data region"
        );
        Ok(self.start + self.data_start + (cluster as u64 - 2) * self.sectors_per_cluster)
    }
}

struct Reader<'a, R: RawStorageReader> {
    disk: &'a mut VirtualDiskOverlay<R>,
    geometry: Geometry,
    cancel: &'a AtomicBool,
    fat_cache: BTreeMap<(u64, u64), Vec<u8>>,
}
impl<R: RawStorageReader> Reader<'_, R> {
    fn read(&mut self, lba: u64, count: usize, output: &mut [u8]) -> Result<()> {
        check_cancel(self.cancel)?;
        self.disk.read_sectors(lba, count, output)?;
        check_cancel(self.cancel)?;
        Ok(())
    }
    fn fat_values(&mut self, cluster: u32) -> Result<Vec<u32>> {
        ensure!(
            cluster <= self.geometry.max_cluster,
            "FAT lookup outside declared cluster capacity"
        );
        let offset = cluster as u64 * 4;
        let relative = offset / self.geometry.sector as u64;
        let within = (offset % self.geometry.sector as u64) as usize;
        ensure!(
            relative < self.geometry.fat_sectors,
            "FAT lookup outside FAT bounds"
        );
        let copies = if self.geometry.mirrored {
            self.geometry.fat_copies
        } else {
            1
        };
        let mut values = Vec::with_capacity(copies as usize);
        for index in 0..copies {
            check_cancel(self.cancel)?;
            let fat = if self.geometry.mirrored {
                index
            } else {
                self.geometry.active_fat
            };
            let key = (fat, relative);
            if !self.fat_cache.contains_key(&key) {
                if self.fat_cache.len() >= 128 {
                    self.fat_cache.clear();
                }
                let lba = self.geometry.start
                    + self.geometry.reserved
                    + fat * self.geometry.fat_sectors
                    + relative;
                let mut sector = vec![0; self.geometry.sector];
                self.read(lba, 1, &mut sector)?;
                self.fat_cache.insert(key, sector);
            }
            let bytes = &self.fat_cache[&key];
            values.push(le32(&bytes[within..within + 4]) & 0x0fff_ffff);
        }
        Ok(values)
    }
}

struct Candidate {
    entry_byte_offset: u64,
    name_hint: String,
    first_cluster: u32,
    size: u32,
    metadata_valid: bool,
}
fn safe_name_hint(entry: &[u8]) -> String {
    let display = |bytes: &[u8]| -> String {
        bytes
            .iter()
            .map(|b| {
                if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b' ') {
                    *b as char
                } else {
                    '?'
                }
            })
            .collect::<String>()
            .trim_end()
            .to_string()
    };
    let base = format!("?{}", display(&entry[1..8]));
    let extension = display(&entry[8..11]);
    if extension.is_empty() {
        base
    } else {
        format!("{base}.{extension}")
    }
}

pub fn scan<R: RawStorageReader, F: FnMut(Value) -> Result<()>>(
    disk: &mut VirtualDiskOverlay<R>,
    partition_start_lba: u64,
    max_clusters: u64,
    cancel: &AtomicBool,
    mut emit: F,
) -> Result<Value> {
    ensure!(
        (1..=4096).contains(&max_clusters),
        "max_clusters must be in 1..4096"
    );
    let geometry = Geometry::read(disk, partition_start_lba, cancel)?;
    let mut cluster = geometry.root;
    let mut reader = Reader {
        disk,
        geometry,
        cancel,
        fat_cache: BTreeMap::new(),
    };
    let mut visited = HashSet::new();
    let mut live_starts = HashSet::new();
    let mut candidates = Vec::new();
    let mut entries = 0u64;
    let mut bytes_read = 0u64;
    let mut truncated = false;
    let mut end_marker = false;
    let mut root_chain_ended = false;
    loop {
        check_cancel(cancel)?;
        if visited.len() as u64 >= max_clusters
            || bytes_read + reader.geometry.cluster_bytes > MAX_DIRECTORY_BYTES
            || entries >= MAX_DIRECTORY_ENTRIES
            || candidates.len() >= MAX_CANDIDATES
        {
            truncated = true;
            break;
        }
        ensure!(
            visited.insert(cluster),
            "Cycle in FAT32 root directory chain"
        );
        let next_values = reader.fat_values(cluster)?;
        ensure!(
            next_values.iter().all(|v| *v == next_values[0]),
            "FAT copies disagree on root directory chain"
        );
        let next = next_values[0];
        ensure!(
            (2..=reader.geometry.max_cluster).contains(&next) || next >= 0x0fff_fff8,
            "Invalid, free, reserved or bad cluster in root directory chain"
        );
        let lba = reader.geometry.cluster_lba(cluster)?;
        let mut data = vec![0; reader.geometry.cluster_bytes as usize];
        reader.read(lba, reader.geometry.sectors_per_cluster as usize, &mut data)?;
        bytes_read += data.len() as u64;
        for (slot, entry) in data.as_chunks::<32>().0.iter().enumerate() {
            check_cancel(cancel)?;
            if entries >= MAX_DIRECTORY_ENTRIES || candidates.len() >= MAX_CANDIDATES {
                truncated = true;
                break;
            }
            entries += 1;
            if entry[0] == 0 {
                end_marker = true;
                break;
            }
            let attributes = entry[11];
            if attributes == 0x0f {
                continue;
            }
            let raw_first = ((le16(&entry[20..22]) as u32) << 16) | le16(&entry[26..28]) as u32;
            if entry[0] != 0xe5 {
                if raw_first >= 2 {
                    live_starts.insert(raw_first);
                }
                continue;
            }
            if attributes & 0x18 != 0 {
                continue;
            }
            candidates.push(Candidate {
                entry_byte_offset: lba * reader.geometry.sector as u64 + (slot * 32) as u64,
                name_hint: safe_name_hint(entry),
                first_cluster: raw_first,
                size: le32(&entry[28..32]),
                metadata_valid: attributes & 0xc0 == 0
                    && entry[12] & !0x18 == 0
                    && raw_first & 0xf000_0000 == 0,
            });
        }
        if end_marker || truncated {
            break;
        }
        match next {
            0x0fff_fff8..=0x0fff_ffff => {
                root_chain_ended = true;
                break;
            }
            n if (2..=reader.geometry.max_cluster).contains(&n) => cluster = n,
            _ => bail!("Invalid, free, reserved or bad cluster in root directory chain"),
        }
    }
    let mut allocation_free = 0;
    let mut plans = 0;
    let mut states: BTreeMap<&str, u64> = BTreeMap::new();
    for candidate in &candidates {
        check_cancel(cancel)?;
        let mut hit = json!({
            "kind":"FAT32_DELETED_SHORT_ENTRY", "entry_byte_offset":candidate.entry_byte_offset,
            "name_hint":candidate.name_hint, "name_complete":false,
            "first_cluster":candidate.first_cluster, "size_bytes":candidate.size,
            "content_verified":false, "fragment_order_inferred":false,
        });
        let state = if !candidate.metadata_valid {
            "INVALID_DIRECTORY_METADATA"
        } else if candidate.size == 0 {
            "EMPTY_FILE_NO_DATA_EXTENT"
        } else if !(2..=reader.geometry.max_cluster).contains(&candidate.first_cluster) {
            "INVALID_FIRST_CLUSTER"
        } else if live_starts.contains(&candidate.first_cluster)
            || visited.contains(&candidate.first_cluster)
        {
            "REFERENCED_BY_LIVE_ROOT_METADATA"
        } else {
            let values = reader.fat_values(candidate.first_cluster)?;
            hit["fat_values"] = json!(values);
            if !values.iter().all(|v| *v == values[0]) {
                "FAT_COPIES_DISAGREE"
            } else if values[0] != 0 {
                "CURRENTLY_ALLOCATED_OR_RESERVED"
            } else {
                allocation_free += 1;
                if candidate.size as u64 <= reader.geometry.cluster_bytes && !truncated {
                    let source = reader.geometry.cluster_lba(candidate.first_cluster)?
                        * reader.geometry.sector as u64;
                    hit["candidate_export_plan"] = json!({"ranges":[{"source_byte_offset":source,"length_bytes":candidate.size}]});
                    plans += 1;
                    "SINGLE_CLUSTER_EXTENT_CONTENT_UNVERIFIED"
                } else if truncated {
                    "ROOT_SCAN_INCOMPLETE_NO_PLAN"
                } else {
                    "FREE_FIRST_CLUSTER_LOST_CHAIN_NO_PLAN"
                }
            }
        };
        hit["allocation_state"] = json!(state);
        *states.entry(state).or_default() += 1;
        ensure!(
            serde_json::to_vec(&hit)?.len() <= 1000,
            "Deleted entry exceeds persisted hit budget"
        );
        emit(hit)?;
    }
    Ok(json!({
        "scope":"FAT32_ROOT_DIRECTORY_SHORT_ENTRIES_ONLY", "partition_start_lba":partition_start_lba,
        "sector_size":reader.geometry.sector,"cluster_size_bytes":reader.geometry.cluster_bytes,
        "fat_mode":if reader.geometry.mirrored {"MIRROR_COMPARISON"} else {"ACTIVE_FAT_ONLY"},
        "root_clusters_scanned":visited.len(),"directory_entries_examined":entries,"directory_bytes_read":bytes_read,
        "deleted_entries_found":candidates.len(),"allocation_free_first_clusters":allocation_free,
        "candidate_export_plans":plans,"counts_by_state":states,
        "directory_end_marker_found":end_marker,"root_chain_ended":root_chain_ended,"scan_truncated":truncated,
        "limits":{"max_clusters":max_clusters,"max_directory_bytes":MAX_DIRECTORY_BYTES,"max_entries":MAX_DIRECTORY_ENTRIES,"max_candidates":MAX_CANDIDATES},
        "limitations":["Root only: no subdirectories, LFN reconstruction, NTFS deleted records or journals",
        "Directory deletion markers and FAT-free state do not prove original bytes survive; allocation can change or metadata can be stale",
        "Only one-cluster, unallocated, non-root-referenced extents receive candidate plans; multi-cluster deleted chains are never guessed",
        "No source patches or output writes were performed; independently validate any exported content"]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io, sync::Arc};

    #[derive(Clone)]
    struct SparseDisk {
        length: u64,
        writes: Arc<BTreeMap<u64, Vec<u8>>>,
    }
    impl RawStorageReader for SparseDisk {
        fn len_bytes(&self) -> io::Result<u64> {
            Ok(self.length)
        }
        fn read_physical_sectors(
            &mut self,
            lba: u64,
            count: usize,
            output: &mut [u8],
        ) -> io::Result<()> {
            let start = lba * 512;
            let size = count * 512;
            output[..size].fill(0);
            for (offset, bytes) in self.writes.iter() {
                let a = start.max(*offset);
                let b = (start + size as u64).min(offset + bytes.len() as u64);
                if a < b {
                    output[(a - start) as usize..(b - start) as usize]
                        .copy_from_slice(&bytes[(a - offset) as usize..(b - offset) as usize]);
                }
            }
            Ok(())
        }
    }
    fn fixture(
        sector: usize,
        candidate_size: u32,
        allocation: u32,
    ) -> (VirtualDiskOverlay<SparseDisk>, u64) {
        let mut boot = vec![0; sector];
        boot[11..13].copy_from_slice(&(sector as u16).to_le_bytes());
        boot[13] = 1;
        boot[14..16].copy_from_slice(&32u16.to_le_bytes());
        boot[16] = 2;
        boot[32..36].copy_from_slice(&131072u32.to_le_bytes());
        boot[36..40].copy_from_slice(&1024u32.to_le_bytes());
        boot[44..48].copy_from_slice(&2u32.to_le_bytes());
        boot[82..90].copy_from_slice(b"FAT32   ");
        boot[510..512].copy_from_slice(&[0x55, 0xaa]);
        let mut root = vec![0; sector];
        root[..11].copy_from_slice(b"\xe5ELETED TXT");
        root[11] = 0x20;
        root[26..28].copy_from_slice(&3u16.to_le_bytes());
        root[28..32].copy_from_slice(&candidate_size.to_le_bytes());
        let mut fat = vec![0; sector];
        fat[8..12].copy_from_slice(&0x0fff_ffffu32.to_le_bytes());
        fat[12..16].copy_from_slice(&allocation.to_le_bytes());
        let data_lba = 32 + 2 * 1024;
        let mut writes = BTreeMap::new();
        writes.insert(0, boot);
        writes.insert(32 * sector as u64, fat.clone());
        writes.insert((32 + 1024) * sector as u64, fat);
        writes.insert(data_lba * sector as u64, root);
        writes.insert((data_lba + 1) * sector as u64, b"deleted evidence".to_vec());
        let source = SparseDisk {
            length: 131072 * sector as u64,
            writes: Arc::new(writes),
        };
        (
            VirtualDiskOverlay::with_sector_size(source, sector).unwrap(),
            (data_lba + 1) * sector as u64,
        )
    }
    fn collect(disk: &mut VirtualDiskOverlay<SparseDisk>) -> (Value, Vec<Value>) {
        let mut hits = Vec::new();
        let result = scan(disk, 0, 256, &AtomicBool::new(false), |hit| {
            hits.push(hit);
            Ok(())
        })
        .unwrap();
        (result, hits)
    }
    #[test]
    fn free_single_cluster_candidate_has_exact_plan_for_512_and_4096_geometry() {
        for sector in [512, 4096] {
            let (mut disk, offset) = fixture(sector, 16, 0);
            let (report, hits) = collect(&mut disk);
            assert_eq!(report["candidate_export_plans"], 1);
            assert_eq!(
                hits[0]["candidate_export_plan"]["ranges"][0],
                json!({"source_byte_offset":offset,"length_bytes":16})
            );
            assert_eq!(hits[0]["content_verified"], false);
            assert_eq!(disk.patch_count(), 0);
        }
    }
    #[test]
    fn allocated_and_lost_multi_cluster_entries_never_offer_export_plans() {
        for (size, fat, state) in [
            (16, 0x0fff_ffff, "CURRENTLY_ALLOCATED_OR_RESERVED"),
            (513, 0, "FREE_FIRST_CLUSTER_LOST_CHAIN_NO_PLAN"),
        ] {
            let (mut disk, _) = fixture(512, size, fat);
            let (_, hits) = collect(&mut disk);
            assert_eq!(hits[0]["allocation_state"], state);
            assert!(hits[0].get("candidate_export_plan").is_none());
        }
    }
    #[test]
    fn invalid_geometry_and_precancelled_jobs_fail_before_emitting_hits() {
        let (mut disk, _) = fixture(512, 16, 0);
        assert!(scan(&mut disk, 0, 0, &AtomicBool::new(false), |_| panic!(
            "unexpected hit"
        ))
        .is_err());
        assert!(scan(&mut disk, 0, 1, &AtomicBool::new(true), |_| panic!(
            "unexpected hit"
        ))
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
        assert!(
            scan(&mut disk, 131072, 1, &AtomicBool::new(false), |_| panic!(
                "unexpected hit"
            ))
            .is_err()
        );
    }
    #[test]
    fn mirror_disagreement_and_live_root_references_suppress_candidate_plans() {
        let (mut disk, _) = fixture(512, 16, 0);
        disk.apply_virtual_patch(
            32 + 1024,
            12,
            &0x0fff_ffffu32.to_le_bytes(),
            "test FAT disagreement".into(),
        )
        .unwrap();
        let (_, hits) = collect(&mut disk);
        assert_eq!(hits[0]["allocation_state"], "FAT_COPIES_DISAGREE");
        assert!(hits[0].get("candidate_export_plan").is_none());
        let (mut disk, _) = fixture(512, 16, 0);
        let mut live = [0u8; 32];
        live[..11].copy_from_slice(b"LIVE    TXT");
        live[11] = 0x20;
        live[26..28].copy_from_slice(&3u16.to_le_bytes());
        live[28..32].copy_from_slice(&16u32.to_le_bytes());
        disk.apply_virtual_patch(32 + 2 * 1024, 32, &live, "test live root reference".into())
            .unwrap();
        let patches = disk.patch_count();
        let (_, hits) = collect(&mut disk);
        assert_eq!(
            hits[0]["allocation_state"],
            "REFERENCED_BY_LIVE_ROOT_METADATA"
        );
        assert!(hits[0].get("candidate_export_plan").is_none());
        assert_eq!(
            disk.patch_count(),
            patches,
            "scan itself must never create a patch"
        );
    }
    #[test]
    fn disabled_mirroring_obeys_selected_active_fat() {
        let (mut disk, _) = fixture(512, 16, 0);
        disk.apply_virtual_patch(0, 40, &0x0081u16.to_le_bytes(), "test FAT 1 active".into())
            .unwrap();
        disk.apply_virtual_patch(
            32,
            12,
            &0x0fff_ffffu32.to_le_bytes(),
            "test inactive stale FAT".into(),
        )
        .unwrap();
        let (report, hits) = collect(&mut disk);
        assert_eq!(report["fat_mode"], "ACTIVE_FAT_ONLY");
        assert!(hits[0].get("candidate_export_plan").is_some());
    }
    #[test]
    fn free_or_cyclic_root_chains_fail_without_guessed_entries() {
        let (mut disk, _) = fixture(512, 16, 0);
        for lba in [32, 32 + 1024] {
            disk.apply_virtual_patch(lba, 8, &[0; 4], "test root incorrectly free".into())
                .unwrap();
        }
        assert!(scan(&mut disk, 0, 256, &AtomicBool::new(false), |_| panic!(
            "unexpected hit"
        ))
        .is_err());
        let (mut disk, _) = fixture(512, 16, 0);
        disk.apply_virtual_patch(
            32 + 2 * 1024,
            0,
            &[0xff; 512],
            "test full root cluster".into(),
        )
        .unwrap();
        for lba in [32, 32 + 1024] {
            disk.apply_virtual_patch(lba, 8, &2u32.to_le_bytes(), "test cyclic root chain".into())
                .unwrap();
        }
        let error = scan(&mut disk, 0, 2, &AtomicBool::new(false), |_| Ok(())).unwrap_err();
        assert!(error.to_string().contains("Cycle"));
    }

    #[test]
    fn incomplete_root_scan_never_generates_a_candidate_export_plan() {
        let (mut disk, _) = fixture(512, 16, 0);
        let mut root = [0xff; 512];
        root[..32].fill(0);
        root[..11].copy_from_slice(b"\xe5ELETED TXT");
        root[11] = 0x20;
        root[26..28].copy_from_slice(&3u16.to_le_bytes());
        root[28..32].copy_from_slice(&16u32.to_le_bytes());
        disk.apply_virtual_patch(
            32 + 2 * 1024,
            0,
            &root,
            "test full first directory cluster".into(),
        )
        .unwrap();
        for lba in [32, 32 + 1024] {
            disk.apply_virtual_patch(
                lba,
                8,
                &4u32.to_le_bytes(),
                "test directory continuation".into(),
            )
            .unwrap();
        }
        let mut hits = Vec::new();
        let report = scan(&mut disk, 0, 1, &AtomicBool::new(false), |hit| {
            hits.push(hit);
            Ok(())
        })
        .unwrap();
        assert_eq!(report["scan_truncated"], true);
        assert_eq!(report["candidate_export_plans"], 0);
        assert_eq!(hits[0]["allocation_state"], "ROOT_SCAN_INCOMPLETE_NO_PLAN");
        assert!(hits[0].get("candidate_export_plan").is_none());
    }
}
