//! Native filesystem access through the bounded, read-only RAM-overlay view.
use anyhow::{bail, Context, Result};
use core_io::{RawStorageReader, VirtualDiskOverlay};
use ntfs::Ntfs;
use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Partition-relative byte stream. FAT requires Write even for read operations;
/// this adapter deliberately denies every write, including library drop/flush.
pub struct VolumeReader<'a, R: RawStorageReader> {
    disk: &'a mut VirtualDiskOverlay<R>,
    start: u64,
    length: u64,
    position: u64,
    operations: u64,
    index_scan: bool,
    index_bytes: u64,
    seen_index_records: HashSet<u64>,
    trace: crate::manifest::SharedTrace,
    data_region: u64,
}

impl<'a, R: RawStorageReader> VolumeReader<'a, R> {
    pub fn new(disk: &'a mut VirtualDiskOverlay<R>, lba: u64, length: u64) -> Result<Self> {
        let start = lba
            .checked_mul(disk.sector_size() as u64)
            .context("Partition offset overflow")?;
        if length == 0
            || start
                .checked_add(length)
                .is_none_or(|end| end > disk.len_bytes())
        {
            bail!("Partition exceeds source capacity");
        }
        Ok(Self {
            disk,
            start,
            length,
            position: 0,
            operations: 0,
            index_scan: false,
            index_bytes: 0,
            seen_index_records: HashSet::new(),
            trace: Default::default(),
            data_region: 0,
        })
    }
}
impl<R: RawStorageReader> Read for VolumeReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.operations += 1;
        if self.operations > 4_000_000 {
            return Err(io::Error::other("Filesystem I/O operation budget exceeded"));
        }
        let count = (buf.len() as u64).min(self.length.saturating_sub(self.position)) as usize;
        if self.index_scan {
            self.index_bytes = self.index_bytes.saturating_add(count as u64);
            if self.index_bytes > 32 * 1024 * 1024 {
                return Err(io::Error::other("Directory index read budget exceeded"));
            }
        }
        self.disk
            .seek(SeekFrom::Start(self.start + self.position))?;
        let read = self.disk.read(&mut buf[..count]);
        if read.is_err() {
            self.trace
                .lock()
                .map_err(|_| io::Error::other("Trace lock poisoned"))?
                .bad_sectors = self.disk.bad_sector_ranges();
        }
        let count = read?;
        if self.index_scan && count >= 4 && &buf[..4] == b"INDX" {
            validate_index_node(&buf[..count], 24)?;
        }
        if self.index_scan
            && count >= 4
            && &buf[..4] == b"INDX"
            && !self.seen_index_records.insert(self.position)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Cyclic NTFS index allocation",
            ));
        }
        if self.position >= self.data_region {
            self.trace
                .lock()
                .map_err(|_| io::Error::other("Trace lock poisoned"))?
                .record(
                    self.start + self.position,
                    count as u64,
                    self.disk.sector_size(),
                )
                .map_err(io::Error::other)?;
        }
        self.position += count as u64;
        Ok(count)
    }
}
impl<R: RawStorageReader> Seek for VolumeReader<'_, R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::End(n) => self.length as i128 + n as i128,
            SeekFrom::Current(n) => self.position as i128 + n as i128,
        };
        if next < 0 || next > self.length as i128 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Seek outside partition",
            ));
        }
        self.position = next as u64;
        Ok(self.position)
    }
}
impl<R: RawStorageReader> Write for VolumeReader<'_, R> {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Filesystem parser is read-only",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn volume<R: RawStorageReader>(
    disk: &mut VirtualDiskOverlay<R>,
    lba: u64,
) -> Result<(&'static str, VolumeReader<'_, R>)> {
    let mut boot = vec![0; disk.sector_size()];
    disk.read_sectors(lba, 1, &mut boot)?;
    if boot[510..512] != [0x55, 0xaa] {
        bail!("Invalid filesystem boot signature");
    }
    let bps = u16::from_le_bytes([boot[11], boot[12]]) as u64;
    let spc = boot[13] as u64;
    if !matches!(bps, 512 | 1024 | 2048 | 4096) || !spc.is_power_of_two() || spc > 128 {
        bail!("Invalid filesystem sector/cluster geometry");
    }
    let mut data_region = 0;
    let (kind, sectors) = if &boot[3..11] == b"NTFS    " {
        let record = boot[64] as i8;
        let index = boot[68] as i8;
        for size in [record, index] {
            let bytes = if size > 0 {
                size as u64 * spc * bps
            } else {
                1u64.checked_shl(-(size as i16) as u32)
                    .context("Invalid NTFS record size")?
            };
            if !(512..=65536).contains(&bytes) || !bytes.is_power_of_two() {
                bail!("Invalid NTFS record size");
            }
        }
        ("NTFS", u64::from_le_bytes(boot[40..48].try_into()?))
    } else if &boot[3..11] == b"EXFAT   " {
        bail!("NotImplemented: native exFAT parser");
    } else {
        let reserved = u16::from_le_bytes(boot[14..16].try_into()?) as u64;
        let fats = boot[16] as u64;
        let fat16 = u16::from_le_bytes(boot[22..24].try_into()?) as u64;
        let fat_size = if fat16 != 0 {
            fat16
        } else {
            u32::from_le_bytes(boot[36..40].try_into()?) as u64
        };
        let short_total = u16::from_le_bytes(boot[19..21].try_into()?) as u64;
        let total = if short_total != 0 {
            short_total
        } else {
            u32::from_le_bytes(boot[32..36].try_into()?) as u64
        };
        let root_entries = u16::from_le_bytes(boot[17..19].try_into()?) as u64;
        let metadata = reserved + fats * fat_size + (root_entries * 32).div_ceil(bps);
        if reserved == 0 || !(1..=2).contains(&fats) || fat_size == 0 || metadata >= total {
            bail!("Invalid FAT geometry");
        }
        data_region = metadata * bps;
        ("FAT", total)
    };
    let length = sectors.checked_mul(bps).context("Volume length overflow")?;
    let mut reader = VolumeReader::new(disk, lba, length)?;
    reader.data_region = data_region;
    Ok((kind, reader))
}

fn validate_index_node(data: &[u8], base: usize) -> io::Result<()> {
    if data.len() < base + 16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Truncated NTFS index node",
        ));
    }
    let field = |offset| {
        u32::from_le_bytes(
            data[base + offset..base + offset + 4]
                .try_into()
                .unwrap_or([0; 4]),
        ) as u64
    };
    let (offset, used, allocated) = (field(0), field(4), field(8));
    if offset < 16
        || offset > used
        || used > allocated
        || allocated > (data.len() - base) as u64
        || data[base + 12] > 1
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid NTFS index node bounds",
        ));
    }
    Ok(())
}

fn validate_directory_index<R: RawStorageReader>(
    file: &ntfs::NtfsFile<'_>,
    reader: &mut VolumeReader<'_, R>,
) -> Result<()> {
    let mut found = false;
    for (number, attribute) in file.attributes_raw().take(4097).enumerate() {
        if number == 4096 {
            bail!("NTFS attribute budget exceeded");
        }
        let attribute = attribute?;
        if attribute.ty()? == ntfs::NtfsAttributeType::IndexRoot {
            if !attribute.is_resident() {
                bail!("NTFS index root must be resident");
            }
            let value = attribute.value(reader)?;
            let length = value.len();
            if !(32..=65536).contains(&length) {
                bail!("Invalid NTFS index root length");
            }
            let mut data = vec![0; length as usize];
            value.attach(reader).read_exact(&mut data)?;
            if data[..4] != 0x30u32.to_le_bytes() || data[4..8] != 1u32.to_le_bytes() {
                bail!("Invalid NTFS filename index type or collation rule");
            }
            validate_index_node(&data, 16)?;
            let root =
                attribute.resident_structured_value::<ntfs::structured_values::NtfsIndexRoot>()?;
            let size = root.index_record_size();
            if !(1024..=65536).contains(&size) || !size.is_power_of_two() {
                bail!("Invalid NTFS index allocation record size");
            }
            found = true;
        }
    }
    if !found {
        bail!("Directory has no resident index root");
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct MountReport {
    pub filesystem: String,
    pub root_entries: u64,
    pub valid_file_entries: u64,
    pub sample_names: Vec<String>,
}

pub fn validate_mount<R: RawStorageReader>(
    disk: &mut VirtualDiskOverlay<R>,
    lba: u64,
) -> Result<MountReport> {
    let (kind, mut reader) = volume(disk, lba)?;
    let mut report = MountReport {
        filesystem: kind.into(),
        root_entries: 0,
        valid_file_entries: 0,
        sample_names: Vec::new(),
    };
    if kind == "NTFS" {
        let ntfs = Ntfs::new(&mut reader)?;
        let root = ntfs.root_directory(&mut reader)?;
        validate_directory_index(&root, &mut reader)?;
        let index = root.directory_index(&mut reader)?;
        let mut entries = index.entries();
        loop {
            reader.index_scan = true;
            let entry = entries.next(&mut reader);
            reader.index_scan = false;
            let Some(entry) = entry else { break };
            let entry = entry?;
            let Some(name) = entry.key() else { continue };
            let name = name?;
            let file = entry.to_file(&ntfs, &mut reader)?;
            if file.sequence_number() != entry.file_reference().sequence_number() {
                bail!("Stale NTFS directory file reference");
            }
            if !file.flags().contains(ntfs::NtfsFileFlags::IN_USE) {
                bail!("Directory index references an unallocated MFT record");
            }
            count_entry(
                &mut report,
                &name.name().to_string_lossy(),
                !file.is_directory(),
            )?;
        }
    } else {
        let fs = fatfs::FileSystem::new(reader, fatfs::FsOptions::new())?;
        for entry in fs.root_dir().iter() {
            let entry = entry?;
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            // Opening and reading a nonempty file verifies its first data cluster.
            if entry.is_file() && entry.len() > 0 {
                let mut byte = [0];
                entry.to_file().read_exact(&mut byte)?;
            }
            count_entry(&mut report, &name, entry.is_file())?;
        }
    }
    if report.valid_file_entries == 0 {
        bail!("No valid file entry found in filesystem root");
    }
    Ok(report)
}

fn count_entry(report: &mut MountReport, name: &str, is_file: bool) -> Result<()> {
    report.root_entries += 1;
    if report.root_entries > 100_000 {
        bail!("Root directory entry budget exceeded");
    }
    if is_file {
        report.valid_file_entries += 1;
    }
    if report.sample_names.len() < 5 {
        report.sample_names.push(name.chars().take(80).collect());
    }
    Ok(())
}

/// Extracts allocated files recursively. Deleted records and alternate streams
/// require separate recovery logic and are deliberately not claimed here.
pub fn extract_tree<R: RawStorageReader>(
    disk: &mut VirtualDiskOverlay<R>,
    lba: u64,
    _source: &str,
    output: &Path,
    progress: &crate::extraction::ExtractionProgress<'_>,
    audit: &crate::manifest::AuditContext,
) -> Result<serde_json::Value> {
    validate_mount(disk, lba)?;
    let (kind, mut reader) = volume(disk, lba)?;
    let trace = reader.trace.clone();
    let mut files = 0u64;
    let mut total = 0u64;
    if kind == "NTFS" {
        let ntfs = Ntfs::new(&mut reader)?;
        let root = ntfs.root_directory(&mut reader)?;
        let mut pending = VecDeque::from([(root.file_record_number(), PathBuf::new(), 0usize)]);
        let mut visited = HashSet::new();
        while let Some((record, path, depth)) = pending.pop_front() {
            progress.check()?;
            if depth > 64 || !visited.insert(record) {
                bail!("Cyclic or excessively deep NTFS directory tree");
            }
            let dir = ntfs.file(&mut reader, record)?;
            validate_directory_index(&dir, &mut reader)?;
            reader.index_bytes = 0;
            reader.seen_index_records.clear();
            let index = dir.directory_index(&mut reader)?;
            let mut entries = index.entries();
            let mut names = HashSet::new();
            loop {
                reader.index_scan = true;
                let entry = entries.next(&mut reader);
                reader.index_scan = false;
                let Some(entry) = entry else { break };
                let entry = entry?;
                let Some(name) = entry.key() else { continue };
                let name = name?;
                if name.namespace() == ntfs::structured_values::NtfsFileNamespace::Dos {
                    continue;
                }
                let name = name.name().to_string_lossy();
                if name == "." {
                    continue;
                }
                if !names.insert(name.clone()) {
                    bail!("Duplicate NTFS directory filename");
                }
                crate::safe_output::validate_filename(&name)?;
                let file = entry.to_file(&ntfs, &mut reader)?;
                if file.sequence_number() != entry.file_reference().sequence_number() {
                    bail!("Stale NTFS directory file reference");
                }
                if file.file_record_number() < 16 {
                    continue;
                }
                if !file.flags().contains(ntfs::NtfsFileFlags::IN_USE) {
                    bail!("Directory index references an unallocated MFT record");
                }
                let parent = output.join(&path);
                if file.is_directory() {
                    if pending.len() + visited.len() > 10_000 {
                        bail!("Directory budget exceeded");
                    }
                    std::fs::create_dir(parent.join(&name))?;
                    pending.push_back((file.file_record_number(), path.join(&name), depth + 1));
                } else {
                    let item = file
                        .data(&mut reader, "")
                        .context("Missing unnamed NTFS data stream")??;
                    let attribute = item.to_attribute()?;
                    if !attribute.flags().is_empty() {
                        bail!(
                            "NotImplemented: compressed, encrypted or sparse NTFS attribute export"
                        );
                    }
                    let value = attribute.value(&mut reader)?;
                    let expected = value.len();
                    let direct = if matches!(
                        &value,
                        ntfs::attribute_value::NtfsAttributeValue::Resident(_)
                    ) {
                        let data_position = value
                            .data_position()
                            .value()
                            .context("Resident data has no source position")?
                            .get();
                        Some(resident_ranges(
                            &mut reader,
                            &file,
                            data_position,
                            expected,
                        )?)
                    } else {
                        if matches!(
                            &value,
                            ntfs::attribute_value::NtfsAttributeValue::AttributeListNonResident(_)
                        ) {
                            bail!(
                                "NotImplemented: exact provenance for NTFS attribute-list streams"
                            );
                        }
                        None
                    };
                    let stream = value.attach(&mut reader);
                    copy_file(
                        &parent,
                        &name,
                        stream,
                        expected,
                        &mut CopyContext {
                            files: &mut files,
                            total: &mut total,
                            progress,
                            audit,
                            trace: &trace,
                        },
                        direct,
                    )?;
                }
            }
        }
    } else {
        let fs = fatfs::FileSystem::new(reader, fatfs::FsOptions::new())?;
        let mut pending = VecDeque::from([(PathBuf::new(), 0usize)]);
        while let Some((path, depth)) = pending.pop_front() {
            progress.check()?;
            if depth > 64 {
                bail!("FAT directory depth budget exceeded");
            }
            let dir = if path.as_os_str().is_empty() {
                fs.root_dir()
            } else {
                fs.root_dir()
                    .open_dir(path.to_str().context("Invalid FAT path")?)?
            };
            for entry in dir.iter() {
                let entry = entry?;
                let name = entry.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                crate::safe_output::validate_filename(&name)?;
                let parent = output.join(&path);
                if entry.is_dir() {
                    if pending.len() + files as usize > 10_000 {
                        bail!("Directory budget exceeded");
                    }
                    std::fs::create_dir(parent.join(&name))?;
                    pending.push_back((path.join(&name), depth + 1));
                } else {
                    copy_file(
                        &parent,
                        &name,
                        entry.to_file(),
                        entry.len(),
                        &mut CopyContext {
                            files: &mut files,
                            total: &mut total,
                            progress,
                            audit,
                            trace: &trace,
                        },
                        None,
                    )?;
                }
            }
        }
    }
    if files == 0 {
        bail!("No user files extracted");
    }
    Ok(
        serde_json::json!({"status":"extracted", "mode":"TSK_TREE_EXTRACT", "backend":"native_overlay", "files_extracted":files, "total_bytes_recovered":total}),
    )
}

fn resident_ranges<R: RawStorageReader>(
    reader: &mut VolumeReader<'_, R>,
    file: &ntfs::NtfsFile<'_>,
    position: u64,
    length: u64,
) -> Result<Vec<crate::manifest::SourceRange>> {
    let record = file
        .position()
        .value()
        .context("Missing MFT record position")?
        .get();
    let saved = reader.stream_position()?;
    reader.seek(SeekFrom::Start(record))?;
    let record_size = file.ntfs().file_record_size() as usize;
    if record_size > 65536 {
        bail!("MFT record exceeds manifest mapping budget");
    }
    let mut record_bytes = vec![0; record_size];
    reader.read_exact(&mut record_bytes)?;
    reader.seek(SeekFrom::Start(saved))?;
    let usa_bytes = record_bytes
        .get(4..6)
        .context("Missing NTFS update sequence offset")?;
    let usa = u16::from_le_bytes(usa_bytes.try_into()?) as u64;
    for block in 0..record_size / 512 {
        let replacement = record_bytes
            .get(usa as usize + 2 * (block + 1)..usa as usize + 2 * (block + 2))
            .context("Invalid NTFS update sequence range")?
            .to_vec();
        record_bytes
            .get_mut(block * 512 + 510..block * 512 + 512)
            .context("Invalid NTFS sector trailer")?
            .copy_from_slice(&replacement);
    }
    let relative = position
        .checked_sub(record)
        .context("Resident attribute before MFT record")? as usize;
    let offset_bytes = record_bytes
        .get(relative + 20..relative + 22)
        .context("Missing resident value offset")?;
    let position = position
        .checked_add(u16::from_le_bytes(offset_bytes.try_into()?) as u64)
        .context("Resident value position overflow")?;
    let mut ranges = Vec::new();
    let mut offset = 0;
    while offset < length {
        let relative = position + offset - record;
        let within = relative % 512;
        let (source, amount) = if within >= 510 {
            (
                record + usa + 2 * (relative / 512 + 1) + (within - 510),
                (512 - within).min(length - offset),
            )
        } else {
            (position + offset, (510 - within).min(length - offset))
        };
        ranges.push(crate::manifest::SourceRange::new(
            offset,
            reader.start + source,
            amount,
            reader.disk.sector_size(),
        ));
        offset += amount;
    }
    Ok(ranges)
}

struct CopyContext<'a, 'b> {
    files: &'a mut u64,
    total: &'a mut u64,
    progress: &'a crate::extraction::ExtractionProgress<'b>,
    audit: &'a crate::manifest::AuditContext,
    trace: &'a crate::manifest::SharedTrace,
}
fn copy_file(
    parent: &Path,
    name: &str,
    mut input: impl Read,
    expected: u64,
    context: &mut CopyContext<'_, '_>,
    direct: Option<Vec<crate::manifest::SourceRange>>,
) -> Result<()> {
    let CopyContext {
        files,
        total,
        progress,
        audit,
        trace,
    } = context;
    if **files >= 10_000 || expected > (8u64 << 30).saturating_sub(**total) {
        bail!("Tree extraction file/byte budget exceeded");
    }
    let mut output = crate::manifest::AuditedOutput::new(audit, parent, name)?;
    trace
        .lock()
        .map_err(|_| anyhow::anyhow!("Trace lock poisoned"))?
        .begin();
    if direct.is_some() {
        trace
            .lock()
            .map_err(|_| anyhow::anyhow!("Trace lock poisoned"))?
            .enabled = false;
    }
    let mut copied = 0u64;
    let mut buffer = [0u8; 65536];
    while copied < expected {
        progress.check()?;
        let amount = (expected - copied).min(buffer.len() as u64) as usize;
        let count = input.read(&mut buffer[..amount])?;
        if count == 0 {
            break;
        }
        progress.check()?;
        output.write(&buffer[..count])?;
        copied += count as u64;
        progress.written(count as u64);
    }
    if copied != expected {
        bail!("Truncated filesystem file: expected {expected}, read {copied}");
    }
    let ranges = {
        let mut trace = trace
            .lock()
            .map_err(|_| anyhow::anyhow!("Trace lock poisoned"))?;
        trace.enabled = false;
        output.update_bad_sectors(&trace.bad_sectors);
        direct.unwrap_or_else(|| std::mem::take(&mut trace.ranges))
    };
    output.finish(ranges)?;
    **files += 1;
    **total += copied;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inverted_or_oversized_index_ranges_are_errors() {
        let mut bytes = [0u8; 64];
        bytes[16..20].copy_from_slice(&32u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&16u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&48u32.to_le_bytes());
        assert!(validate_index_node(&bytes, 16).is_err());
        bytes[16..20].copy_from_slice(&16u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&48u32.to_le_bytes());
        assert!(validate_index_node(&bytes, 16).is_ok());
        bytes[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(validate_index_node(&bytes, 16).is_err());
    }
}
