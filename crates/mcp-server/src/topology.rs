//! Bounded partition validation, including both GPT CRCs and entry arrays.
use anyhow::{bail, Context, Result};
use core_io::{RawStorageReader, VirtualDiskOverlay};
use disk_parsers::mbr_gpt::{analyze_mbr, validate_mbr, GptHeader, MbrAnalysis};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Partition {
    pub start_lba: u64,
    pub sector_count: u64,
}
pub struct Layout {
    pub mbr: MbrAnalysis,
    pub mbr_valid: bool,
    pub primary_valid: bool,
    pub backup_valid: bool,
    pub partitions: Vec<Partition>,
    pub anomalies: Vec<String>,
}

pub fn read_layout<R: RawStorageReader>(
    disk: &mut VirtualDiskOverlay<R>,
    sector_size: u32,
) -> Result<Layout> {
    let scale = 1u64;
    if !matches!(sector_size, 512 | 4096) {
        bail!("Unsupported logical sector size");
    }
    let capacity = disk.len_bytes() / sector_size as u64;
    let mut sector = vec![0; sector_size as usize];
    disk.read_sectors(0, scale as usize, &mut sector)?;
    let mbr = analyze_mbr(&sector);
    let mut anomalies = Vec::new();
    let mbr_valid = match validate_mbr(&sector, capacity) {
        Ok(_) => true,
        Err(error) => {
            anomalies.push(error);
            false
        }
    };
    let mut partitions = Vec::new();
    if mbr_valid && !mbr.is_gpt_protective {
        partitions.extend(mbr.partitions.iter().map(|p| Partition {
            start_lba: p.lba_start as u64 * scale,
            sector_count: p.sector_count as u64 * scale,
        }));
    }
    let mut headers = Vec::new();
    for (lba, expected_alternate) in [
        (1, capacity.saturating_sub(1)),
        (capacity.saturating_sub(1), 1),
    ] {
        let result = read_gpt(disk, lba, expected_alternate, sector_size);
        match result {
            Ok((header, entries)) => headers.push(Some((header, entries))),
            Err(error) => {
                if error.downcast_ref::<std::io::Error>().is_some() {
                    return Err(error);
                }
                anomalies.push(format!("GPT LBA {lba}: {error}"));
                headers.push(None);
            }
        }
    }
    let mut primary_valid = headers[0].is_some();
    let mut backup_valid = headers[1].is_some();
    if let (Some((a, _)), Some((b, _))) = (&headers[0], &headers[1]) {
        if a.disk_guid != b.disk_guid
            || a.first_usable_lba != b.first_usable_lba
            || a.last_usable_lba != b.last_usable_lba
            || a.partition_entries_crc32 != b.partition_entries_crc32
            || a.num_partition_entries != b.num_partition_entries
            || a.partition_entry_size != b.partition_entry_size
        {
            anomalies.push("Primary and backup GPT disagree".into());
            primary_valid = false;
            backup_valid = false;
        }
    }
    if mbr.is_gpt_protective {
        if primary_valid {
            partitions = headers[0]
                .as_ref()
                .context("Missing primary GPT")?
                .1
                .clone();
        } else if backup_valid {
            partitions = headers[1].as_ref().context("Missing backup GPT")?.1.clone();
        }
    }
    Ok(Layout {
        mbr,
        mbr_valid,
        primary_valid,
        backup_valid,
        partitions,
        anomalies,
    })
}

fn read_gpt<R: RawStorageReader>(
    disk: &mut VirtualDiskOverlay<R>,
    lba: u64,
    alternate: u64,
    sector_size: u32,
) -> Result<(GptHeader, Vec<Partition>)> {
    let scale = 1u64;
    let capacity = disk.len_bytes() / sector_size as u64;
    let mut data = vec![0; sector_size as usize];
    disk.read_sectors(lba * scale, scale as usize, &mut data)?;
    let header =
        *GptHeader::from_sector(&data).context("Invalid GPT header signature, size or CRC32")?;
    header
        .validate_geometry(lba, capacity)
        .map_err(anyhow::Error::msg)?;
    if header.alternate_lba != alternate {
        bail!("Invalid alternate GPT header address");
    }
    let count = header.num_partition_entries as usize;
    let size = header.partition_entry_size as usize;
    if count > 131072 || size > 4096 {
        bail!("GPT entry resource budget exceeded");
    }
    let bytes = count.checked_mul(size).context("GPT array size overflow")?;
    let blocks = bytes.div_ceil(sector_size as usize);
    let array_lba = header.partition_entry_start_lba;
    let array_end = array_lba
        .checked_add(blocks as u64)
        .context("GPT array overflow")?;
    if lba == 1 && (array_lba < 2 || array_end > header.first_usable_lba)
        || lba != 1 && (array_lba <= header.last_usable_lba || array_end > lba)
    {
        bail!("GPT entry array is not in the correct primary/backup metadata region");
    }
    if array_end > capacity
        || array_lba <= lba && lba < array_end
        || array_lba <= header.last_usable_lba && array_end > header.first_usable_lba
    {
        bail!("GPT array overlaps usable space or header");
    }
    let mut hasher = crc32fast::Hasher::new();
    let mut entries = Vec::new();
    // Entries are multiples of 128; bounded batches preserve complete entries.
    let entries_per_batch = (65536 / size).max(1);
    for first in (0..count).step_by(entries_per_batch) {
        let batch = (count - first).min(entries_per_batch);
        let offset = array_lba * sector_size as u64 + (first * size) as u64;
        use std::io::{Read, Seek, SeekFrom};
        disk.seek(SeekFrom::Start(offset))?;
        let mut buffer = vec![0; batch * size];
        disk.read_exact(&mut buffer)?;
        hasher.update(&buffer);
        for entry in buffer.chunks_exact(size) {
            if entry[..16].iter().all(|&b| b == 0) {
                continue;
            }
            let start = u64::from_le_bytes(entry[32..40].try_into()?);
            let end = u64::from_le_bytes(entry[40..48].try_into()?);
            if start < header.first_usable_lba || end > header.last_usable_lba || start > end {
                bail!("GPT partition exceeds usable disk capacity");
            }
            entries.push(Partition {
                start_lba: start * scale,
                sector_count: (end - start + 1) * scale,
            });
        }
    }
    if hasher.finalize() != header.partition_entries_crc32 {
        bail!("Invalid GPT partition array CRC32");
    }
    entries.sort_by_key(|p| p.start_lba);
    if entries
        .windows(2)
        .any(|p| p[0].start_lba + p[0].sector_count > p[1].start_lba)
    {
        bail!("Overlapping GPT partitions");
    }
    Ok((header, entries))
}
