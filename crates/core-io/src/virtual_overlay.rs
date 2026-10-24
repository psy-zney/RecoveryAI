//! Virtual Disk Overlay — Copy-on-Write trên RAM.
//!
//! Cho phép AI thử nghiệm vá MBR, GPT, Boot Sector... mà **không bao giờ
//! ghi một bit nào xuống đĩa thật**.
//!
//! Bản vá dùng sector 512/4096 byte trong RAM, có WAL bền vững khi mở session.
//! Khi đọc, overlay tự động merge dữ liệu thật từ đĩa với các patch đã áp dụng.

use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Kích thước sector chuẩn (bytes)
pub const SECTOR_SIZE: usize = 512;

/// Trait trừu tượng hoá nguồn dữ liệu đĩa vật lý.
///
/// Cho phép `VirtualDiskOverlay` hoạt động với cả ổ đĩa thật
/// lẫn file image (.img, .dd).
pub trait RawStorageReader {
    /// Byte length. All LBAs in this trait use 512-byte blocks.
    fn len_bytes(&self) -> io::Result<u64>;
    /// Đọc `count` sector vật lý bắt đầu từ `start_lba` vào `buf`.
    fn read_physical_sectors(
        &mut self,
        start_lba: u64,
        count: usize,
        buf: &mut [u8],
    ) -> io::Result<()>;
    fn read_cancellable(
        &mut self,
        lba: u64,
        count: usize,
        buf: &mut [u8],
        cancel: Option<&AtomicBool>,
    ) -> io::Result<()> {
        if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Job cancelled"));
        }
        self.read_physical_sectors(lba, count, buf)
    }
    fn bad_sector_ranges(&self) -> Vec<(u64, u64)> {
        Vec::new()
    }
}

impl RawStorageReader for std::fs::File {
    fn len_bytes(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }
    fn read_physical_sectors(
        &mut self,
        start_lba: u64,
        count: usize,
        buf: &mut [u8],
    ) -> io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let offset = start_lba
            .checked_mul(SECTOR_SIZE as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "LBA overflow"))?;
        self.seek(SeekFrom::Start(offset))?;
        let bytes_to_read = count
            .checked_mul(SECTOR_SIZE)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Read size overflow"))?;
        if buf.len() < bytes_to_read {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Buffer size smaller than requested sectors",
            ));
        }
        self.read_exact(&mut buf[..bytes_to_read])?;
        Ok(())
    }
}

impl<T: RawStorageReader + ?Sized> RawStorageReader for Box<T> {
    fn len_bytes(&self) -> io::Result<u64> {
        (**self).len_bytes()
    }
    fn read_cancellable(
        &mut self,
        lba: u64,
        count: usize,
        buf: &mut [u8],
        cancel: Option<&AtomicBool>,
    ) -> io::Result<()> {
        (**self).read_cancellable(lba, count, buf, cancel)
    }
    fn bad_sector_ranges(&self) -> Vec<(u64, u64)> {
        (**self).bad_sector_ranges()
    }
    fn read_physical_sectors(
        &mut self,
        start_lba: u64,
        count: usize,
        buf: &mut [u8],
    ) -> io::Result<()> {
        (**self).read_physical_sectors(start_lba, count, buf)
    }
}

/// A durable patch entry, with data for exact undo and forensic manifests.
#[derive(Debug, Clone)]
pub struct PatchInfo {
    pub patch_id: u64,
    pub description: String,
    pub original_data: Vec<u8>,
    pub patched_data: Vec<u8>,
    pub offset: usize,
    pub size: usize,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct PatchAudit {
    pub patch_id: u64,
    pub lba: u64,
    pub offset: usize,
    pub original_hex: String,
    pub patched_hex: String,
    pub description: String,
}
pub struct VirtualDiskOverlay<R: RawStorageReader> {
    physical_reader: R,
    length: u64,
    position: u64,
    sector_size: usize,
    revision: u64,
    max_patches: usize,
    ram_patches: BTreeMap<u64, Vec<u8>>,
    patch_history: Vec<(u64, PatchInfo)>,
    next_patch_id: u64,
    cancel: Option<Arc<AtomicBool>>,
    wal: Option<crate::overlay_wal::OverlayWal>,
}
impl<R: RawStorageReader> VirtualDiskOverlay<R> {
    pub fn new(reader: R) -> io::Result<Self> {
        Self::with_sector_size(reader, 512)
    }
    pub fn with_sector_size(reader: R, sector_size: usize) -> io::Result<Self> {
        let length = reader.len_bytes()?;
        if !matches!(sector_size, 512 | 4096)
            || length == 0
            || !length.is_multiple_of(sector_size as u64)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Source length must match 512/4096-byte sector geometry",
            ));
        }
        Ok(Self {
            physical_reader: reader,
            length,
            position: 0,
            sector_size,
            revision: 0,
            max_patches: 65536,
            ram_patches: BTreeMap::new(),
            patch_history: Vec::new(),
            next_patch_id: 1,
            cancel: None,
            wal: None,
        })
    }
    pub fn with_wal(
        reader: R,
        sector_size: usize,
        path: &std::path::Path,
        source_identity: &str,
    ) -> io::Result<Self> {
        let mut disk = Self::with_sector_size(reader, sector_size)?;
        let (wal, records) =
            crate::overlay_wal::OverlayWal::open(path, sector_size, disk.length, source_identity)?;
        for record in records {
            disk.replay(record)?;
        }
        disk.wal = Some(wal);
        Ok(disk)
    }
    fn replay(&mut self, record: crate::overlay_wal::WalRecord) -> io::Result<()> {
        use crate::overlay_wal::WalRecord;
        match record {
            WalRecord::Patch {
                lba,
                offset,
                size,
                before,
                after,
                description,
                id,
            } => {
                self.check_range(lba, 1, self.sector_size)?;
                if before.len() != self.sector_size
                    || after.len() != self.sector_size
                    || offset >= self.sector_size
                    || id != self.next_patch_id
                    || self.patch_history.len() >= self.max_patches
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid WAL patch",
                    ));
                }
                if size == 0
                    || offset
                        .checked_add(size)
                        .is_none_or(|end| end > self.sector_size)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid WAL patch span",
                    ));
                }
                self.ram_patches.insert(lba, after.clone());
                self.patch_history.push((
                    lba,
                    PatchInfo {
                        patch_id: id,
                        description,
                        original_data: before,
                        patched_data: after,
                        offset,
                        size,
                    },
                ));
                self.next_patch_id += 1;
                self.revision += 1;
            }
            WalRecord::Undo { id } => {
                if self
                    .patch_history
                    .last()
                    .is_none_or(|(_, p)| p.patch_id != id)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid WAL undo order",
                    ));
                }
                self.undo_memory();
            }
            WalRecord::Clear => {
                self.ram_patches.clear();
                self.patch_history.clear();
                self.revision += 1;
                self.next_patch_id = 1;
            }
            WalRecord::Header { .. } => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unexpected WAL header",
                ))
            }
        }
        Ok(())
    }
    fn journal(&mut self, record: &crate::overlay_wal::WalRecord) -> io::Result<()> {
        if let Some(wal) = &mut self.wal {
            wal.append(record)?;
        }
        Ok(())
    }
    pub fn sector_size(&self) -> usize {
        self.sector_size
    }
    pub fn len_bytes(&self) -> u64 {
        self.length
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn set_cancel_token(&mut self, cancel: Option<Arc<AtomicBool>>) {
        self.cancel = cancel;
    }
    pub fn bad_sector_ranges(&self) -> Vec<(u64, u64)> {
        let scale = self.sector_size as u64 / 512;
        self.physical_reader
            .bad_sector_ranges()
            .into_iter()
            .map(|(start, count)| (start / scale, (start % scale + count).div_ceil(scale)))
            .collect()
    }
    pub fn operation(&mut self, cancel: Arc<AtomicBool>) -> OverlayOperation<'_, R> {
        let previous = self.cancel.replace(cancel);
        OverlayOperation {
            disk: self,
            previous,
        }
    }
    fn check_range(&self, lba: u64, count: usize, buffer: usize) -> io::Result<()> {
        let bytes = count
            .checked_mul(self.sector_size)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Read size overflow"))?;
        let end = lba
            .checked_mul(self.sector_size as u64)
            .and_then(|n| n.checked_add(bytes as u64));
        if buffer < bytes || end.is_none_or(|end| end > self.length) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Read exceeds source or buffer boundary",
            ));
        }
        Ok(())
    }
    pub fn apply_virtual_patch(
        &mut self,
        lba: u64,
        offset: usize,
        bytes: &[u8],
        description: String,
    ) -> io::Result<u64> {
        let end = offset
            .checked_add(bytes.len())
            .filter(|n| *n <= self.sector_size);
        let end = match end {
            Some(end) if offset < self.sector_size && !bytes.is_empty() => end,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Patch offset exceeds sector boundary",
                ))
            }
        };
        self.check_range(lba, 1, self.sector_size)?;
        if self.patch_history.len() >= self.max_patches || description.len() > 512 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "RAM patch budget exceeded",
            ));
        }
        let mut before = vec![0; self.sector_size];
        self.read_sectors(lba, 1, &mut before)?;
        let mut after = before.clone();
        after
            .get_mut(offset..end)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Patch offset exceeds sector boundary",
                )
            })?
            .copy_from_slice(bytes);
        let id = self.next_patch_id;
        self.journal(&crate::overlay_wal::WalRecord::Patch {
            lba,
            offset,
            size: bytes.len(),
            before: before.clone(),
            after: after.clone(),
            description: description.clone(),
            id,
        })?;
        self.ram_patches.insert(lba, after.clone());
        self.patch_history.push((
            lba,
            PatchInfo {
                patch_id: id,
                description,
                original_data: before,
                patched_data: after,
                offset,
                size: bytes.len(),
            },
        ));
        self.next_patch_id += 1;
        self.revision += 1;
        Ok(id)
    }
    pub fn copy_virtual_sector(
        &mut self,
        from: u64,
        to: u64,
        description: String,
    ) -> io::Result<u64> {
        let mut bytes = vec![0; self.sector_size];
        self.read_sectors(from, 1, &mut bytes)?;
        self.apply_virtual_patch(to, 0, &bytes, description)
    }
    pub fn read_sectors(&mut self, start: u64, count: usize, output: &mut [u8]) -> io::Result<()> {
        self.check_range(start, count, output.len())?;
        if self
            .cancel
            .as_ref()
            .is_some_and(|f| f.load(Ordering::Acquire))
        {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Job cancelled"));
        }
        let scale = self.sector_size / 512;
        if self
            .ram_patches
            .range(start..start + count as u64)
            .next()
            .is_none()
        {
            return self.physical_reader.read_cancellable(
                start * scale as u64,
                count * scale,
                output,
                self.cancel.as_deref(),
            );
        }
        for i in 0..count {
            let lba = start + i as u64;
            let sector = output
                .get_mut(i * self.sector_size..(i + 1) * self.sector_size)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Output too short"))?;
            if let Some(patch) = self.ram_patches.get(&lba) {
                sector.copy_from_slice(patch)
            } else {
                self.physical_reader.read_cancellable(
                    lba * scale as u64,
                    scale,
                    sector,
                    self.cancel.as_deref(),
                )?;
            }
        }
        Ok(())
    }
    pub fn rollback_all_patches(&mut self) -> io::Result<()> {
        self.journal(&crate::overlay_wal::WalRecord::Clear)?;
        self.ram_patches.clear();
        self.patch_history.clear();
        self.next_patch_id = 1;
        self.revision += 1;
        Ok(())
    }
    fn undo_memory(&mut self) -> Option<u64> {
        let (lba, info) = self.patch_history.pop()?;
        if self.patch_history.iter().any(|(other, _)| *other == lba) {
            self.ram_patches.insert(lba, info.original_data);
        } else {
            self.ram_patches.remove(&lba);
        }
        self.revision += 1;
        Some(info.patch_id)
    }
    pub fn rollback_last_patch(&mut self) -> io::Result<Option<u64>> {
        if let Some((_, info)) = self.patch_history.last() {
            self.journal(&crate::overlay_wal::WalRecord::Undo { id: info.patch_id })?;
        }
        Ok(self.undo_memory())
    }
    pub fn patch_count(&self) -> usize {
        self.ram_patches.len()
    }
    pub fn patched_lbas(&self) -> Vec<u64> {
        self.ram_patches.keys().copied().collect()
    }
    pub fn history(&self) -> &[(u64, PatchInfo)] {
        &self.patch_history
    }
    pub fn audit_patches(&self) -> Vec<PatchAudit> {
        self.patch_history
            .iter()
            .map(|(lba, p)| PatchAudit {
                patch_id: p.patch_id,
                lba: *lba,
                offset: p.offset,
                original_hex: p
                    .original_data
                    .iter()
                    .skip(p.offset)
                    .take(p.size)
                    .map(|b| format!("{b:02x}"))
                    .collect(),
                patched_hex: p
                    .patched_data
                    .iter()
                    .skip(p.offset)
                    .take(p.size)
                    .map(|b| format!("{b:02x}"))
                    .collect(),
                description: p.description.clone(),
            })
            .collect()
    }
}
pub struct OverlayOperation<'a, R: RawStorageReader> {
    disk: &'a mut VirtualDiskOverlay<R>,
    previous: Option<Arc<AtomicBool>>,
}
impl<R: RawStorageReader> std::ops::Deref for OverlayOperation<'_, R> {
    type Target = VirtualDiskOverlay<R>;
    fn deref(&self) -> &Self::Target {
        self.disk
    }
}
impl<R: RawStorageReader> std::ops::DerefMut for OverlayOperation<'_, R> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.disk
    }
}
impl<R: RawStorageReader> Drop for OverlayOperation<'_, R> {
    fn drop(&mut self) {
        self.disk.cancel = self.previous.take();
    }
}
impl<R: RawStorageReader> Read for VirtualDiskOverlay<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let amount = (output.len() as u64).min(self.length.saturating_sub(self.position)) as usize;
        let mut done = 0;
        while done < amount {
            let lba = self.position / self.sector_size as u64;
            let offset = (self.position % self.sector_size as u64) as usize;
            if offset == 0 && amount - done >= self.sector_size {
                let count = ((amount - done) / self.sector_size).min(65536 / self.sector_size);
                let bytes = count * self.sector_size;
                self.read_sectors(lba, count, &mut output[done..done + bytes])?;
                done += bytes;
                self.position += bytes as u64;
            } else {
                let mut sector = vec![0; self.sector_size];
                self.read_sectors(lba, 1, &mut sector)?;
                let bytes = (self.sector_size - offset).min(amount - done);
                output[done..done + bytes].copy_from_slice(&sector[offset..offset + bytes]);
                done += bytes;
                self.position += bytes as u64;
            }
        }
        Ok(done)
    }
}
impl<R: RawStorageReader> Seek for VirtualDiskOverlay<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let n = match from {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.position as i128 + n as i128,
            SeekFrom::End(n) => self.length as i128 + n as i128,
        };
        if n < 0 || n > u64::MAX as i128 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid seek"));
        }
        self.position = n as u64;
        Ok(self.position)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_kn_overlay_wal_survives_reopen_and_torn_tail() {
        let root = std::env::temp_dir().join(format!("resc-wal-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("overlay.wal");
        let _ = std::fs::remove_file(&path);
        {
            let mut disk =
                VirtualDiskOverlay::with_wal(MockDisk::new(32), 4096, &path, "source-A").unwrap();
            disk.apply_virtual_patch(1, 4094, &[1, 2], "4Kn boundary".into())
                .unwrap();
            let mut bytes = vec![0; 4096];
            disk.read_sectors(1, 1, &mut bytes).unwrap();
            assert_eq!(&bytes[4094..], &[1, 2]);
            assert!(disk
                .apply_virtual_patch(1, 4095, &[1, 2], String::new())
                .is_err());
        }
        {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(&[0x12, 0x34])
                .unwrap();
        }
        {
            let mut disk =
                VirtualDiskOverlay::with_wal(MockDisk::new(32), 4096, &path, "source-A").unwrap();
            disk.seek(SeekFrom::Start(8190)).unwrap();
            let mut bytes = [0; 2];
            disk.read_exact(&mut bytes).unwrap();
            assert_eq!(bytes, [1, 2]);
            disk.rollback_last_patch().unwrap();
        }
        {
            let disk =
                VirtualDiskOverlay::with_wal(MockDisk::new(32), 4096, &path, "source-A").unwrap();
            assert_eq!(disk.patch_count(), 0);
        }
        assert!(VirtualDiskOverlay::with_wal(MockDisk::new(32), 512, &path, "source-A").is_err());
        assert!(VirtualDiskOverlay::with_wal(MockDisk::new(32), 4096, &path, "source-B").is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn invalid_patches_are_rejected_without_mutation() {
        let mut overlay = VirtualDiskOverlay::new(MockDisk::new(2)).unwrap();
        for (offset, bytes) in [
            (513, vec![1]),
            (511, vec![1, 2]),
            (usize::MAX, vec![1]),
            (0, vec![]),
        ] {
            let err = overlay
                .apply_virtual_patch(0, offset, &bytes, String::new())
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
            assert_eq!(err.to_string(), "Patch offset exceeds sector boundary");
        }
        assert_eq!(overlay.patch_count(), 0);
    }

    #[test]
    fn rollback_preserves_previous_patch_on_same_sector() {
        let mut overlay = VirtualDiskOverlay::new(MockDisk::new(2)).unwrap();
        overlay
            .apply_virtual_patch(0, 0, &[1], String::new())
            .unwrap();
        overlay
            .apply_virtual_patch(0, 1, &[2], String::new())
            .unwrap();
        overlay.rollback_last_patch().unwrap();
        let mut buf = [0; SECTOR_SIZE];
        overlay.read_sectors(0, 1, &mut buf).unwrap();
        assert_eq!(&buf[..2], &[1, 0xaa]);
    }

    struct MockDisk {
        data: Vec<u8>,
    }

    impl MockDisk {
        fn new(sectors: usize) -> Self {
            Self {
                data: vec![0xAA; sectors * SECTOR_SIZE],
            }
        }
    }

    impl RawStorageReader for MockDisk {
        fn len_bytes(&self) -> io::Result<u64> {
            Ok(self.data.len() as u64)
        }
        fn read_physical_sectors(
            &mut self,
            start_lba: u64,
            count: usize,
            buf: &mut [u8],
        ) -> io::Result<()> {
            let start = start_lba as usize * SECTOR_SIZE;
            let end = start + count * SECTOR_SIZE;
            if end > self.data.len() {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Past end"));
            }
            buf[..count * SECTOR_SIZE].copy_from_slice(&self.data[start..end]);
            Ok(())
        }
    }

    #[test]
    fn read_seek_crosses_patched_sector_and_handles_eof() {
        let mut overlay = VirtualDiskOverlay::new(MockDisk::new(2)).unwrap();
        overlay
            .apply_virtual_patch(1, 0, &[1, 2], String::new())
            .unwrap();
        overlay.seek(SeekFrom::Start(510)).unwrap();
        let mut bytes = [0; 4];
        overlay.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, [0xaa, 0xaa, 1, 2]);
        overlay.seek(SeekFrom::End(-1)).unwrap();
        assert_eq!(overlay.read(&mut bytes).unwrap(), 1);
        assert_eq!(overlay.read(&mut bytes).unwrap(), 0);
        assert!(overlay.seek(SeekFrom::Current(-2048)).is_err());
        assert!(overlay.read_sectors(u64::MAX, 2, &mut bytes).is_err());
    }

    #[test]
    fn test_read_without_patches() {
        let disk = MockDisk::new(10);
        let mut overlay = VirtualDiskOverlay::new(disk).unwrap();
        let mut buf = [0u8; SECTOR_SIZE];
        overlay.read_sectors(0, 1, &mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn test_apply_and_read_patch() {
        let disk = MockDisk::new(10);
        let mut overlay = VirtualDiskOverlay::new(disk).unwrap();

        // Áp dụng patch MBR signature tại sector 0
        overlay
            .apply_virtual_patch(0, 510, &[0x55, 0xAA], "Fix MBR signature".into())
            .unwrap();

        let mut buf = [0u8; SECTOR_SIZE];
        overlay.read_sectors(0, 1, &mut buf).unwrap();
        assert_eq!(buf[510], 0x55);
        assert_eq!(buf[511], 0xAA);
        // Các byte khác vẫn giữ nguyên 0xAA (từ đĩa gốc)
        assert_eq!(buf[0], 0xAA);
    }

    #[test]
    fn test_rollback_all() {
        let disk = MockDisk::new(10);
        let mut overlay = VirtualDiskOverlay::new(disk).unwrap();
        overlay
            .apply_virtual_patch(0, 0, &[0xFF], "Test".into())
            .unwrap();
        assert_eq!(overlay.patch_count(), 1);
        overlay.rollback_all_patches().unwrap();
        assert_eq!(overlay.patch_count(), 0);

        let mut buf = [0u8; SECTOR_SIZE];
        overlay.read_sectors(0, 1, &mut buf).unwrap();
        assert_eq!(buf[0], 0xAA); // Trở về giá trị gốc
    }
}
