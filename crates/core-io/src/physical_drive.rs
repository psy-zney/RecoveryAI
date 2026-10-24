//! Truy cập ổ đĩa vật lý qua Win32 API (Read-Only).
//!
//! Mở `\\.\PhysicalDriveX` với các cờ:
//! - `GENERIC_READ`: Chỉ đọc, không ghi
//! - `FILE_FLAG_NO_BUFFERING`: Bỏ qua cache của OS, đọc trực tiếp từ đĩa (DMA)
//! - `FILE_SHARE_READ | FILE_SHARE_WRITE`: Cho phép chia sẻ với OS
//!
//! Tất cả đọc dùng `AlignedBuffer` để đảm bảo alignment 4096 bytes.

use crate::virtual_overlay::RawStorageReader;
use std::io;

#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
#[cfg(target_os = "windows")]
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, SetFilePointerEx, FILE_FLAG_NO_BUFFERING, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::GENERIC_READ;

/// Đại diện cho một ổ đĩa vật lý đã mở (read-only).
pub struct PhysicalDrive {
    #[cfg(target_os = "windows")]
    handle: HANDLE,
    #[cfg(not(target_os = "windows"))]
    _file: std::fs::File,
    /// Kích thước sector vật lý (512 hoặc 4096 bytes)
    pub sector_size: u32,
    /// Tổng dung lượng ổ đĩa tính bằng bytes
    pub disk_size_bytes: u64,
}

impl PhysicalDrive {
    /// Mở ổ đĩa vật lý theo số (0, 1, 2...).
    ///
    /// Yêu cầu quyền Administrator.
    ///
    /// # Errors
    /// - `io::Error` nếu không có quyền truy cập hoặc ổ đĩa không tồn tại
    #[cfg(target_os = "windows")]
    pub fn open(drive_number: u32) -> io::Result<Self> {
        let path = format!("\\\\.\\PhysicalDrive{}", drive_number);
        Self::open_path(&path)
    }

    /// Mở thiết bị từ Win32 device path bất kỳ (ví dụ `\\.\PhysicalDrive1` hoặc `\\.\D:`).
    #[cfg(target_os = "windows")]
    pub fn open_path(device_path: &str) -> io::Result<Self> {
        let wide_path: Vec<u16> = device_path.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING,
                std::ptr::null_mut() as _,
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        // Truy vấn geometry để lấy sector size và disk size
        let (sector_size, disk_size_bytes) = Self::query_disk_geometry(handle)?;

        Ok(Self {
            handle,
            sector_size,
            disk_size_bytes,
        })
    }

    /// Fallback cho non-Windows (development/testing)
    #[cfg(not(target_os = "windows"))]
    pub fn open(_drive_number: u32) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "PhysicalDrive only supported on Windows",
        ))
    }

    #[cfg(not(target_os = "windows"))]
    pub fn open_path(_device_path: &str) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "PhysicalDrive only supported on Windows",
        ))
    }

    /// Mở từ file image (.img, .dd) thay vì ổ đĩa thật.
    /// Hữu ích cho testing và phân tích offline.
    #[cfg(target_os = "windows")]
    pub fn open_image(path: &str) -> io::Result<Self> {
        let wide_path: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ,
                std::ptr::null(),
                OPEN_EXISTING,
                0, // Không cần NO_BUFFERING cho file image
                std::ptr::null_mut() as _,
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        // Lấy kích thước file
        let mut file_size: i64 = 0;
        let success =
            unsafe { SetFilePointerEx(handle, 0, &mut file_size, 2 /* FILE_END */) };
        if success == 0 {
            let err = io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(err);
        }
        // Reset về đầu file
        unsafe { SetFilePointerEx(handle, 0, std::ptr::null_mut(), 0 /* FILE_BEGIN */) };

        Ok(Self {
            handle,
            sector_size: 512,
            disk_size_bytes: file_size as u64,
        })
    }

    #[cfg(not(target_os = "windows"))]
    pub fn open_image(path: &str) -> io::Result<Self> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(path)?;
        let size = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(0))?;
        Ok(Self {
            _file: file,
            sector_size: 512,
            disk_size_bytes: size,
        })
    }

    /// Đọc `count` sector bắt đầu từ `start_lba` vào `buf`.
    ///
    /// `buf` phải có kích thước >= `count * sector_size` và phải căn chỉnh 4KB.
    #[cfg(target_os = "windows")]
    pub fn read_sectors_raw(&self, start_lba: u64, count: usize, buf: &mut [u8]) -> io::Result<()> {
        let byte_offset = start_lba * self.sector_size as u64;
        let bytes_to_read = count * self.sector_size as usize;

        if buf.len() < bytes_to_read {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Buffer too small: {} < {} (need {} sectors × {} bytes)",
                    buf.len(),
                    bytes_to_read,
                    count,
                    self.sector_size
                ),
            ));
        }

        // Seek tới vị trí LBA
        let success = unsafe {
            SetFilePointerEx(
                self.handle,
                byte_offset as i64,
                std::ptr::null_mut(),
                0, // FILE_BEGIN
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }

        // Đọc dữ liệu
        let mut bytes_read: u32 = 0;
        let success = unsafe {
            ReadFile(
                self.handle,
                buf.as_mut_ptr() as _,
                bytes_to_read as u32,
                &mut bytes_read,
                std::ptr::null_mut(),
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }

        if (bytes_read as usize) < bytes_to_read {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "Short read: got {} bytes, expected {}",
                    bytes_read, bytes_to_read
                ),
            ));
        }

        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    pub fn read_sectors_raw(&self, start_lba: u64, count: usize, buf: &mut [u8]) -> io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let byte_offset = start_lba * self.sector_size as u64;
        let bytes_to_read = count * self.sector_size as usize;
        let file = &self._file;
        // Non-Windows: use standard file I/O (for testing)
        let mut file_ref = unsafe { &*(&file as *const _ as *const std::fs::File) };
        // This is a simplified stub for non-Windows
        let _ = (byte_offset, bytes_to_read, buf);
        Err(io::Error::new(io::ErrorKind::Unsupported, "Not implemented for this OS"))
    }

    /// Truy vấn disk geometry qua IOCTL
    #[cfg(target_os = "windows")]
    fn query_disk_geometry(handle: HANDLE) -> io::Result<(u32, u64)> {
        use windows_sys::Win32::System::Ioctl::IOCTL_DISK_GET_DRIVE_GEOMETRY_EX;

        #[repr(C)]
        struct DiskGeometryEx {
            geometry: DiskGeometry,
            disk_size: i64,
            _data: [u8; 256],
        }

        #[repr(C)]
        struct DiskGeometry {
            cylinders: i64,
            media_type: u32,
            tracks_per_cylinder: u32,
            sectors_per_track: u32,
            bytes_per_sector: u32,
        }

        let mut geo = std::mem::MaybeUninit::<DiskGeometryEx>::zeroed();
        let mut bytes_returned: u32 = 0;

        let success = unsafe {
            windows_sys::Win32::System::IO::DeviceIoControl(
                handle,
                IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
                std::ptr::null(),
                0,
                geo.as_mut_ptr() as *mut _,
                std::mem::size_of::<DiskGeometryEx>() as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        if success == 0 {
            // Fallback nếu IOCTL thất bại
            tracing::warn!("IOCTL_DISK_GET_DRIVE_GEOMETRY_EX failed, using defaults");
            return Ok((512, 0));
        }

        let geo = unsafe { geo.assume_init() };
        Ok((geo.geometry.bytes_per_sector, geo.disk_size as u64))
    }

    /// Trả về tổng số sector trên đĩa
    pub fn total_sectors(&self) -> u64 {
        if self.sector_size > 0 {
            self.disk_size_bytes / self.sector_size as u64
        } else {
            0
        }
    }
}

impl RawStorageReader for PhysicalDrive {
    fn read_physical_sectors(
        &mut self,
        start_lba: u64,
        count: usize,
        buf: &mut [u8],
    ) -> io::Result<()> {
        self.read_sectors_raw(start_lba, count, buf)
    }
}

#[cfg(target_os = "windows")]
impl Drop for PhysicalDrive {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

unsafe impl Send for PhysicalDrive {}
