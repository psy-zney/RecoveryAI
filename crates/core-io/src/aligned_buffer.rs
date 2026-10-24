//! Bộ đệm RAM căn chỉnh 4096 bytes (Page-aligned) cho Direct Memory Access.
//!
//! Khi mở `\\.\PhysicalDriveX` với cờ `FILE_FLAG_NO_BUFFERING`, Windows yêu cầu:
//! - Địa chỉ bộ nhớ đọc vào phải chia hết cho kích thước sector vật lý
//! - Số byte đọc cũng phải chia hết cho sector size
//!
//! `AlignedBuffer` đảm bảo hai điều kiện trên bằng cách sử dụng
//! `std::alloc::alloc_zeroed` với alignment = 4096.

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::ops::{Deref, DerefMut};

/// Alignment yêu cầu cho Direct I/O trên Windows (4KB = page size)
pub const PAGE_ALIGNMENT: usize = 4096;

/// Vùng bộ nhớ được cấp phát với alignment 4096 bytes.
///
/// Kích thước thực tế luôn được làm tròn lên bội số gần nhất của 4096.
/// Buffer sẽ tự động giải phóng khi đi ra khỏi scope (RAII).
pub struct AlignedBuffer {
    ptr: *mut u8,
    layout: Layout,
    len: usize,
}

// SAFETY: AlignedBuffer sở hữu bộ nhớ riêng, không chia sẻ giữa các thread.
// Các thao tác trên con trỏ được bọc trong Deref/DerefMut an toàn.
unsafe impl Send for AlignedBuffer {}

impl AlignedBuffer {
    /// Tạo buffer mới với kích thước tối thiểu `size` bytes.
    ///
    /// Kích thước thực tế sẽ được làm tròn lên bội số 4096 gần nhất.
    /// Buffer được khởi tạo với giá trị 0 (zeroed).
    ///
    /// # Panics
    /// Panic nếu hệ thống không đủ RAM để cấp phát.
    pub fn new(size: usize) -> Self {
        // Làm tròn kích thước lên bội số gần nhất của 4096 bytes
        let aligned_size = (size + PAGE_ALIGNMENT - 1) & !(PAGE_ALIGNMENT - 1);
        let layout = Layout::from_size_align(aligned_size, PAGE_ALIGNMENT)
            .expect("Invalid memory alignment layout");
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        Self {
            ptr,
            layout,
            len: aligned_size,
        }
    }

    /// Trả về kích thước thực tế đã cấp phát (đã căn chỉnh).
    pub fn capacity(&self) -> usize {
        self.len
    }

    /// Trả về con trỏ raw tới đầu buffer (dùng cho Win32 API).
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    /// Trả về con trỏ raw read-only tới đầu buffer.
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }
}

impl Deref for AlignedBuffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl DerefMut for AlignedBuffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr, self.layout) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alignment() {
        let buf = AlignedBuffer::new(1);
        assert_eq!(buf.capacity(), PAGE_ALIGNMENT);
        assert_eq!(buf.as_ptr() as usize % PAGE_ALIGNMENT, 0);
    }

    #[test]
    fn test_exact_page_size() {
        let buf = AlignedBuffer::new(4096);
        assert_eq!(buf.capacity(), 4096);
    }

    #[test]
    fn test_round_up() {
        let buf = AlignedBuffer::new(5000);
        assert_eq!(buf.capacity(), 8192); // 2 * 4096
    }

    #[test]
    fn test_zeroed() {
        let buf = AlignedBuffer::new(4096);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_write_and_read() {
        let mut buf = AlignedBuffer::new(512);
        buf[0] = 0xEB;
        buf[1] = 0x52;
        buf[510] = 0x55;
        buf[511] = 0xAA;
        assert_eq!(buf[0], 0xEB);
        assert_eq!(buf[510], 0x55);
        assert_eq!(buf[511], 0xAA);
    }
}
