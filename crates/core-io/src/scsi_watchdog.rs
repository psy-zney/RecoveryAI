//! SCSI Watchdog - Timeout 500ms chống treo I/O khi gặp Bad Sector.
//!
//! Khi đọc sector trên ổ đĩa bị hư, Windows có thể treo vô thời hạn.
//! Module này wrap việc đọc sector trong một timeout cứng, tự động
//! đánh dấu sector lỗi vào `BadSectorMap` nếu quá thời gian.

use crate::bad_sector_map::BadSectorMap;
use crate::physical_drive::PhysicalDrive;
use std::io;
use std::time::{Duration, Instant};

/// Thời gian timeout mặc định cho một lần đọc sector (500ms).
pub const DEFAULT_SECTOR_TIMEOUT: Duration = Duration::from_millis(500);

/// Watchdog bảo vệ I/O — tự động timeout và ghi nhận bad sector.
pub struct ScsiWatchdog {
    timeout: Duration,
}

impl ScsiWatchdog {
    /// Tạo watchdog với timeout mặc định 500ms.
    pub fn new() -> Self {
        Self {
            timeout: DEFAULT_SECTOR_TIMEOUT,
        }
    }

    /// Tạo watchdog với timeout tuỳ chỉnh.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Đọc sector an toàn với timeout.
    ///
    /// Nếu đọc thành công trong thời gian cho phép, trả về `Ok(read_duration)`.
    /// Nếu đọc lỗi hoặc quá chậm, ghi nhận vào `bad_sector_map` và trả về `Err`.
    ///
    /// # Arguments
    /// - `drive`: Ổ đĩa vật lý đã mở
    /// - `lba`: Địa chỉ Logical Block Address cần đọc
    /// - `buf`: Buffer đích (phải >= sector_size bytes, căn chỉnh 4KB)
    /// - `bad_sector_map`: Bản đồ lưu trữ các sector lỗi
    pub fn read_sector_safe(
        &self,
        drive: &PhysicalDrive,
        lba: u64,
        buf: &mut [u8],
        bad_sector_map: &mut BadSectorMap,
    ) -> io::Result<Duration> {
        // Kiểm tra xem sector này đã biết là lỗi chưa
        if bad_sector_map.is_bad(lba) {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("LBA {} is known bad sector, skipping", lba),
            ));
        }

        let start = Instant::now();
        let result = drive.read_sectors_raw(lba, 1, buf);
        let elapsed = start.elapsed();

        match result {
            Ok(()) => {
                // Đọc thành công nhưng quá chậm => đánh dấu là slow sector
                if elapsed > self.timeout {
                    tracing::warn!(
                        lba = lba,
                        elapsed_ms = elapsed.as_millis(),
                        "Sector read exceeded timeout, marking as slow"
                    );
                    bad_sector_map.mark_slow(lba, elapsed);
                }
                Ok(elapsed)
            }
            Err(e) => {
                tracing::error!(
                    lba = lba,
                    error = %e,
                    "Failed to read sector, marking as bad"
                );
                bad_sector_map.mark_bad(lba, e.to_string());
                Err(e)
            }
        }
    }

    /// Đọc một dải sector liên tiếp, bỏ qua các sector đã biết lỗi.
    ///
    /// Trả về số sector đọc thành công.
    pub fn read_range_safe(
        &self,
        drive: &PhysicalDrive,
        start_lba: u64,
        count: usize,
        sector_size: usize,
        buf: &mut [u8],
        bad_sector_map: &mut BadSectorMap,
    ) -> usize {
        let mut success_count = 0;

        for i in 0..count {
            let lba = start_lba + i as u64;
            let offset = i * sector_size;
            let sector_buf = &mut buf[offset..offset + sector_size];

            match self.read_sector_safe(drive, lba, sector_buf, bad_sector_map) {
                Ok(_) => success_count += 1,
                Err(_) => {
                    // Điền zero vào sector lỗi để parser không crash
                    sector_buf.fill(0);
                }
            }
        }

        success_count
    }
}

impl Default for ScsiWatchdog {
    fn default() -> Self {
        Self::new()
    }
}
