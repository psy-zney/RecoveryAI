//! Bản đồ Bad Sector — theo dõi các LBA lỗi hoặc phản hồi chậm.
//!
//! Khi SCSI Watchdog phát hiện sector lỗi, thông tin được ghi vào đây
//! để tránh đọc lại sector đó (giảm nguy cơ treo đĩa).

use std::collections::BTreeMap;
use std::time::Duration;

/// Trạng thái của một sector có vấn đề.
#[derive(Debug, Clone)]
pub enum SectorStatus {
    /// Sector bị lỗi hoàn toàn, không đọc được
    Bad {
        error_message: String,
        /// Số lần thử đọc
        attempt_count: u32,
    },
    /// Sector đọc được nhưng quá chậm (vượt timeout)
    Slow {
        read_duration: Duration,
        /// Số lần phát hiện chậm
        slow_count: u32,
    },
}

/// Bản đồ LBA → trạng thái lỗi.
///
/// Dữ liệu này được AI sử dụng để:
/// 1. Tránh đọc lại sector lỗi (giảm thời gian quét)
/// 2. Xác định vùng đĩa bị hư vật lý
/// 3. Quyết định chiến thuật khôi phục (ddrescue cho vùng lỗi, trực tiếp cho vùng tốt)
pub struct BadSectorMap {
    sectors: BTreeMap<u64, SectorStatus>,
}

impl BadSectorMap {
    /// Tạo bản đồ trống.
    pub fn new() -> Self {
        Self {
            sectors: BTreeMap::new(),
        }
    }

    /// Đánh dấu sector là lỗi hoàn toàn.
    pub fn mark_bad(&mut self, lba: u64, error_message: String) {
        let entry = self.sectors.entry(lba).or_insert(SectorStatus::Bad {
            error_message: error_message.clone(),
            attempt_count: 0,
        });
        match entry {
            SectorStatus::Bad { attempt_count, .. } => {
                *attempt_count += 1;
            }
            _ => {
                *entry = SectorStatus::Bad {
                    error_message,
                    attempt_count: 1,
                };
            }
        }
    }

    /// Đánh dấu sector là chậm (đọc được nhưng vượt timeout).
    pub fn mark_slow(&mut self, lba: u64, duration: Duration) {
        let entry = self.sectors.entry(lba).or_insert(SectorStatus::Slow {
            read_duration: duration,
            slow_count: 0,
        });
        // Never downgrade a Bad sector to Slow.
        if let SectorStatus::Slow {
            read_duration,
            slow_count,
        } = entry
        {
            *read_duration = duration;
            *slow_count += 1;
        }
    }

    /// Kiểm tra xem sector có bị lỗi không.
    pub fn is_bad(&self, lba: u64) -> bool {
        matches!(self.sectors.get(&lba), Some(SectorStatus::Bad { .. }))
    }

    /// Kiểm tra xem sector có vấn đề (lỗi hoặc chậm).
    pub fn has_issue(&self, lba: u64) -> bool {
        self.sectors.contains_key(&lba)
    }

    /// Tổng số sector có vấn đề.
    pub fn total_issues(&self) -> usize {
        self.sectors.len()
    }

    /// Số sector lỗi hoàn toàn.
    pub fn bad_count(&self) -> usize {
        self.sectors
            .values()
            .filter(|s| matches!(s, SectorStatus::Bad { .. }))
            .count()
    }

    /// Số sector chậm.
    pub fn slow_count(&self) -> usize {
        self.sectors
            .values()
            .filter(|s| matches!(s, SectorStatus::Slow { .. }))
            .count()
    }

    /// Liệt kê các dải sector lỗi liên tiếp (cho AI phân tích).
    ///
    /// Trả về `Vec<(start_lba, count)>` — các vùng lỗi liền nhau.
    pub fn contiguous_bad_ranges(&self) -> Vec<(u64, u64)> {
        let bad_lbas: Vec<u64> = self
            .sectors
            .iter()
            .filter(|(_, s)| matches!(s, SectorStatus::Bad { .. }))
            .map(|(lba, _)| *lba)
            .collect();

        if bad_lbas.is_empty() {
            return vec![];
        }

        let mut ranges = vec![];
        let mut range_start = bad_lbas[0];
        let mut range_end = bad_lbas[0];

        for &lba in &bad_lbas[1..] {
            if lba == range_end + 1 {
                range_end = lba;
            } else {
                ranges.push((range_start, range_end - range_start + 1));
                range_start = lba;
                range_end = lba;
            }
        }
        ranges.push((range_start, range_end - range_start + 1));

        ranges
    }

    /// Xuất báo cáo JSON gọn cho AI (< 5KB).
    pub fn to_summary_json(&self) -> serde_json::Value {
        // Chỉ serialize khi serde_json được import ở mcp-server level
        // Tại đây trả về thông tin cơ bản
        let ranges = self.contiguous_bad_ranges();
        let summary = format!(
            "{{\"bad_sectors\":{},\"slow_sectors\":{},\"bad_ranges\":{},\"total_issues\":{}}}",
            self.bad_count(),
            self.slow_count(),
            ranges.len(),
            self.total_issues()
        );
        // Parse lại thành serde_json::Value nếu cần
        serde_json::from_str(&summary).unwrap_or(serde_json::Value::Null)
    }

    /// Trả về iterator trên tất cả sector có vấn đề.
    pub fn iter(&self) -> impl Iterator<Item = (&u64, &SectorStatus)> {
        self.sectors.iter()
    }
}

impl Default for BadSectorMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mark_and_check() {
        let mut map = BadSectorMap::new();
        map.mark_bad(100, "Read error".into());
        assert!(map.is_bad(100));
        assert!(!map.is_bad(101));
        assert_eq!(map.bad_count(), 1);
    }

    #[test]
    fn test_contiguous_ranges() {
        let mut map = BadSectorMap::new();
        for lba in 10..=15 {
            map.mark_bad(lba, "bad".into());
        }
        map.mark_bad(20, "bad".into());
        map.mark_bad(21, "bad".into());

        let ranges = map.contiguous_bad_ranges();
        assert_eq!(ranges, vec![(10, 6), (20, 2)]);
    }

    #[test]
    fn test_slow_does_not_downgrade_bad() {
        let mut map = BadSectorMap::new();
        map.mark_bad(50, "error".into());
        map.mark_slow(50, Duration::from_millis(600));
        assert!(map.is_bad(50)); // Vẫn là bad
    }
}
