//! Virtual Disk Overlay — Copy-on-Write trên RAM.
//!
//! Cho phép AI thử nghiệm vá MBR, GPT, Boot Sector... mà **không bao giờ
//! ghi một bit nào xuống đĩa thật**.
//!
//! Mọi bản vá được lưu trong `BTreeMap<LBA, [u8; 512]>` trên RAM.
//! Khi đọc, overlay tự động merge dữ liệu thật từ đĩa với các patch đã áp dụng.

use std::collections::BTreeMap;
use std::io;

/// Kích thước sector chuẩn (bytes)
pub const SECTOR_SIZE: usize = 512;

/// Trait trừu tượng hoá nguồn dữ liệu đĩa vật lý.
///
/// Cho phép `VirtualDiskOverlay` hoạt động với cả ổ đĩa thật
/// lẫn file image (.img, .dd).
pub trait RawStorageReader {
    /// Đọc `count` sector vật lý bắt đầu từ `start_lba` vào `buf`.
    fn read_physical_sectors(
        &mut self,
        start_lba: u64,
        count: usize,
        buf: &mut [u8],
    ) -> io::Result<()>;
}

impl RawStorageReader for std::fs::File {
    fn read_physical_sectors(
        &mut self,
        start_lba: u64,
        count: usize,
        buf: &mut [u8],
    ) -> io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let offset = start_lba * SECTOR_SIZE as u64;
        self.seek(SeekFrom::Start(offset))?;
        let bytes_to_read = count * SECTOR_SIZE;
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
    fn read_physical_sectors(
        &mut self,
        start_lba: u64,
        count: usize,
        buf: &mut [u8],
    ) -> io::Result<()> {
        (**self).read_physical_sectors(start_lba, count, buf)
    }
}

/// Thông tin về một bản vá đã áp dụng trên RAM.
#[derive(Debug, Clone)]
pub struct PatchInfo {
    /// Số thứ tự bản vá (để rollback từng cái)
    pub patch_id: u64,
    /// Mô tả bản vá (AI ghi chú lý do)
    pub description: String,
    /// Dữ liệu sector gốc trước khi patch (cho undo)
    pub original_data: [u8; SECTOR_SIZE],
}

/// Lớp overlay ảo trên RAM — nền tảng Copy-on-Write.
///
/// # An toàn
/// - Không bao giờ ghi dữ liệu xuống ổ đĩa vật lý
/// - Mọi thay đổi chỉ tồn tại trên RAM
/// - Hỗ trợ rollback toàn bộ hoặc từng bản vá
pub struct VirtualDiskOverlay<R: RawStorageReader> {
    physical_reader: R,
    /// Các sector đã được AI "vá" thử nghiệm trên RAM
    ram_patches: BTreeMap<u64, [u8; SECTOR_SIZE]>,
    /// Lịch sử bản vá (cho undo và audit trail)
    patch_history: Vec<(u64, PatchInfo)>,
    /// Bộ đếm patch ID
    next_patch_id: u64,
}

impl<R: RawStorageReader> VirtualDiskOverlay<R> {
    /// Tạo overlay mới trên một nguồn đĩa vật lý.
    pub fn new(reader: R) -> Self {
        Self {
            physical_reader: reader,
            ram_patches: BTreeMap::new(),
            patch_history: Vec::new(),
            next_patch_id: 1,
        }
    }

    /// AI gọi hàm này qua MCP để áp dụng bản vá ảo lên RAM.
    ///
    /// # Arguments
    /// - `lba`: Địa chỉ Logical Block Address của sector cần vá
    /// - `offset_in_sector`: Vị trí bắt đầu vá trong sector (0-511)
    /// - `patch_bytes`: Dữ liệu vá
    /// - `description`: Mô tả lý do vá (AI ghi chú)
    pub fn apply_virtual_patch(
        &mut self,
        lba: u64,
        offset_in_sector: usize,
        patch_bytes: &[u8],
        description: String,
    ) -> io::Result<u64> {
        // Đọc sector hiện tại (từ patch hoặc từ đĩa thật)
        let mut sector_buf = [0u8; SECTOR_SIZE];
        if let Some(existing) = self.ram_patches.get(&lba) {
            sector_buf.copy_from_slice(existing);
        } else {
            self.physical_reader
                .read_physical_sectors(lba, 1, &mut sector_buf)?;
        }

        // Lưu dữ liệu gốc trước khi patch
        let original_data = sector_buf;

        // Áp dụng patch
        let end = (offset_in_sector + patch_bytes.len()).min(SECTOR_SIZE);
        sector_buf[offset_in_sector..end]
            .copy_from_slice(&patch_bytes[..(end - offset_in_sector)]);

        let patch_id = self.next_patch_id;
        self.next_patch_id += 1;

        // Lưu lịch sử
        self.patch_history.push((
            lba,
            PatchInfo {
                patch_id,
                description,
                original_data,
            },
        ));

        // Áp dụng vào overlay
        self.ram_patches.insert(lba, sector_buf);

        tracing::info!(
            patch_id = patch_id,
            lba = lba,
            offset = offset_in_sector,
            len = patch_bytes.len(),
            "Virtual patch applied"
        );

        Ok(patch_id)
    }

    /// Sao chép một sector từ LBA nguồn sang LBA đích trên RAM Overlay.
    /// Dùng cho kịch bản phục hồi Boot Sector từ Backup VBR (NTFS cuối đĩa, FAT32 sector 6).
    pub fn copy_virtual_sector(
        &mut self,
        from_lba: u64,
        to_lba: u64,
        description: String,
    ) -> io::Result<u64> {
        let mut buf = [0u8; SECTOR_SIZE];
        self.read_sectors(from_lba, 1, &mut buf)?;
        self.apply_virtual_patch(to_lba, 0, &buf, description)
    }

    /// Mọi trình Parser (MBR, NTFS, FAT32) đều đọc qua hàm này.
    ///
    /// Tự động merge dữ liệu đĩa thật với các patch đã áp dụng.
    pub fn read_sectors(
        &mut self,
        start_lba: u64,
        count: usize,
        out_buf: &mut [u8],
    ) -> io::Result<()> {
        // 1. Đọc dữ liệu gốc từ đĩa vật lý
        self.physical_reader
            .read_physical_sectors(start_lba, count, out_buf)?;

        // 2. Ghi đè các sector đã được AI vá trên RAM
        for i in 0..count {
            let current_lba = start_lba + i as u64;
            if let Some(patched_sector) = self.ram_patches.get(&current_lba) {
                let offset = i * SECTOR_SIZE;
                out_buf[offset..offset + SECTOR_SIZE].copy_from_slice(patched_sector);
            }
        }

        Ok(())
    }

    /// Xóa toàn bộ bản vá — reset về trạng thái đĩa gốc.
    pub fn rollback_all_patches(&mut self) {
        let count = self.ram_patches.len();
        self.ram_patches.clear();
        self.patch_history.clear();
        self.next_patch_id = 1;
        tracing::info!(patches_cleared = count, "All virtual patches rolled back");
    }

    /// Rollback bản vá cuối cùng.
    pub fn rollback_last_patch(&mut self) -> Option<u64> {
        if let Some((lba, info)) = self.patch_history.pop() {
            // Khôi phục sector gốc
            // Kiểm tra xem còn patch nào khác trên LBA này không
            let has_other_patches = self.patch_history.iter().any(|(l, _)| *l == lba);
            if has_other_patches {
                // Replay tất cả patch trên LBA này từ đầu
                let mut sector = info.original_data;
                for (l, p) in &self.patch_history {
                    if *l == lba {
                        sector = p.original_data; // Simplified: sử dụng patch cuối
                    }
                }
                self.ram_patches.insert(lba, sector);
            } else {
                self.ram_patches.remove(&lba);
            }
            tracing::info!(
                patch_id = info.patch_id,
                lba = lba,
                "Virtual patch rolled back"
            );
            Some(info.patch_id)
        } else {
            None
        }
    }

    /// Trả về số lượng bản vá đang active.
    pub fn patch_count(&self) -> usize {
        self.ram_patches.len()
    }

    /// Liệt kê tất cả LBA đã được patch.
    pub fn patched_lbas(&self) -> Vec<u64> {
        self.ram_patches.keys().copied().collect()
    }

    /// Trả về lịch sử bản vá.
    pub fn history(&self) -> &[(u64, PatchInfo)] {
        &self.patch_history
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_read_without_patches() {
        let disk = MockDisk::new(10);
        let mut overlay = VirtualDiskOverlay::new(disk);
        let mut buf = [0u8; SECTOR_SIZE];
        overlay.read_sectors(0, 1, &mut buf).unwrap();
        assert!(buf.iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn test_apply_and_read_patch() {
        let disk = MockDisk::new(10);
        let mut overlay = VirtualDiskOverlay::new(disk);

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
        let mut overlay = VirtualDiskOverlay::new(disk);
        overlay
            .apply_virtual_patch(0, 0, &[0xFF], "Test".into())
            .unwrap();
        assert_eq!(overlay.patch_count(), 1);
        overlay.rollback_all_patches();
        assert_eq!(overlay.patch_count(), 0);

        let mut buf = [0u8; SECTOR_SIZE];
        overlay.read_sectors(0, 1, &mut buf).unwrap();
        assert_eq!(buf[0], 0xAA); // Trở về giá trị gốc
    }
}
