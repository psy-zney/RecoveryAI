//! # core-io
//!
//! Crate cốt lõi cho truy cập đĩa thô (Raw Disk I/O) trên Windows.
//!
//! Cung cấp:
//! - **AlignedBuffer**: Bộ đệm RAM căn chỉnh 4KB cho DMA
//! - **PhysicalDrive**: Mở `\\.\PhysicalDriveX` chế độ read-only
//! - **ScsiWatchdog**: Timeout 500ms chống treo khi gặp bad sector
//! - **VirtualOverlay**: Copy-on-Write trên RAM, không ghi đĩa thật
//! - **BadSectorMap**: Bản đồ LBA lỗi / phản hồi chậm

pub mod aligned_buffer;
pub mod bad_sector_map;
pub mod physical_drive;
pub mod scsi_watchdog;
pub mod virtual_overlay;

pub use aligned_buffer::{AlignedBuffer, PAGE_ALIGNMENT};
pub use bad_sector_map::BadSectorMap;
pub use physical_drive::PhysicalDrive;
pub use scsi_watchdog::ScsiWatchdog;
pub use virtual_overlay::{RawStorageReader, VirtualDiskOverlay, SECTOR_SIZE};
