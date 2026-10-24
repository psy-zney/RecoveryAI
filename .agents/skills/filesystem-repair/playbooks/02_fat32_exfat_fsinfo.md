# Playbook: Phục Hồi Cấu Trúc FAT32 FSInfo & Checksum exFAT

## 1. FAT32 Boot Sector & FSInfo Sector

### Cấu Trúc Khối Boot Sector FAT32 (LBA 0 của Phân Vùng):
- Offset `0x03`: OEM ID `MSDOS5.0` (8 bytes)
- Offset `0x0B`: Bytes per Sector (512)
- Offset `0x0D`: Sectors per Cluster (thường là 8, 16, 32, 64)
- Offset `0x0E`: Reserved Sector Count (thường là 32)
- Offset `0x10`: Number of FATs (thường là 2: FAT1 và FAT2)
- Offset `0x24`: Sectors per FAT (`fat_sz32`, 4 bytes)
- Offset `0x2C`: Root Directory Cluster (`root_cluster`, thường là 2)
- Offset `0x30`: FSInfo Sector Number (thường là 1)
- Offset `0x32`: Backup Boot Sector Number (thường là 6)
- Offset `0x1FE`: Chữ ký `0x55AA`

### Cấu Trúc FSInfo Sector (Thường ở LBA + 1):
- Chữ ký dẫn đầu tại offset 0: `0x41615252` (`RRaA`)
- Chữ ký cấu trúc tại offset 484 (`0x1E4`): `0x61417272` (`rrAa`)
- Chữ ký kết thúc tại offset 508 (`0x1FC`): `0xAA550000`
- Nếu hệ thống treo đột ngột, cờ free cluster count trong FSInfo có thể bị hỏng (`0xFFFFFFFF`), khiến Windows yêu cầu format lại. Thay thế sector này bằng bản sao dự phòng tại `Start LBA + 7`.

---

## 2. exFAT Boot Region & Checksum Verification

exFAT có cấu trúc Main Boot Region gồm 12 sector liên tục (Sectors 0..11):
- Sector 0: Main VBR (OEM ID `EXFAT   `)
- Sectors 1..8: Extended Boot Sectors (chứa mã boot loader x86)
- Sector 9: OEM Parameter Sector
- Sector 10: Reserved Sector
- Sector 11: Boot Checksum Sector (Lặp lại giá trị 32-bit checksum trên toàn bộ 512 bytes của sector này)

### Backup Boot Region:
- Nằm chính xác tại Sectors 12..23 của phân vùng.
- Luôn là một bản sao giống hệt 100% của Sectors 0..11.

### Giải Thuật Tính Checksum exFAT (C/Rust Algorithm):
```rust
pub fn compute_exfat_boot_checksum(boot_region_11_sectors: &[u8]) -> u32 {
    let mut checksum: u32 = 0;
    for (i, &byte) in boot_region_11_sectors.iter().enumerate() {
        // Bỏ qua trường VolumeFlags và PercentInUse ở sector 0 (offsets 106 và 107)
        if i == 106 || i == 107 {
            continue;
        }
        checksum = ((checksum << 31) | (checksum >> 1)).wrapping_add(byte as u32);
    }
    checksum
}
```

### Quy Trình Phục Hồi exFAT RAW:
1. Gọi `inspect_sector_structure(session_id, lba=start_lba + 12, count=12)` để đọc Backup Boot Region.
2. Kiểm tra tính toàn vẹn bằng cách đối chiếu checksum của 11 sector đầu với giá trị lưu ở sector 11 (Sector 23).
3. Nếu hợp lệ, sao chép toàn bộ 12 sector này vào RAM Overlay đè lên Sectors 0..11 thông qua `virtual_patch_and_verify`.
4. Windows và các công cụ forensic sẽ nhận diện lại toàn bộ bảng Allocation Bitmap và UpCase Table ngay lập tức.
