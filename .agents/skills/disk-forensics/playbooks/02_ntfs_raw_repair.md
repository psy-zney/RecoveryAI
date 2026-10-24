# Playbook 02: Khôi phục Phân Vùng NTFS bị lỗi RAW (NTFS Raw Repair)

## 1. Dấu hiệu nhận biết
- Ổ đĩa hiển thị phân vùng NTFS là `RAW` trong Windows Explorer hoặc Disk Management.
- Hệ điều hành thông báo: *"The volume does not contain a recognized file system."* hoặc *"The parameter is incorrect."*
- LBA 2048 (hoặc Start LBA) bị hỏng (corrupted OEM ID hoặc signature `0x55AA` bị mất).

## 2. Cấu trúc NTFS Boot Sector (VBR - Volume Boot Record)
- Offset 0x03..0x0A: Chữ ký `NTFS    ` (8 bytes)
- Offset 0x0B..0x0C: Bytes per sector (thường là 512 hoặc 4096)
- Offset 0x0D: Sectors per cluster (1, 2, 4, 8... thường là 8 -> 4096 bytes/cluster)
- Offset 0x28..0x2F: Total Sectors (kích thước phân vùng tính theo sector)
- Offset 0x30..0x37: Logical Cluster Number của `$MFT`
- Offset 0x38..0x3F: Logical Cluster Number của `$MFTMirr`
- Offset 0x1FE..0x1FF: End of Sector Marker (`0x55AA`)

## 3. Quy trình khắc phục bằng Virtual Overlay

### Bước 1: Kiểm tra tính toàn vẹn của Primary VBR
1. Gọi `inspect_sector_structure(session_id, lba=start_lba, parse_as="NTFS_VBR")`.
2. Kiểm tra các trường:
   - `oem_id` có đúng `"NTFS    "` không?
   - `bytes_per_sector` có nằm trong {512, 1024, 2048, 4096} không?
   - `end_of_sector_marker` có phải `0xAA55` không?
   - Nếu bất kỳ trường nào sai, Primary VBR đã bị hỏng.

### Bước 2: Tìm kiếm Backup VBR
NTFS luôn tự động ghi một bản sao của VBR ở **sector cuối cùng của phân vùng**:
- `Backup_VBR_LBA = Start_LBA + Total_Sectors` (hoặc `Start_LBA + Total_Sectors - 1`).
- Gọi `inspect_sector_structure(session_id, lba=backup_vbr_lba, parse_as="NTFS_VBR")`.
- Nếu Backup VBR hợp lệ (OEM ID `"NTFS    "` và marker `0x55AA` đầy đủ), ta tiến hành vá.

### Bước 3: Áp dụng bản vá ảo (Virtual Copy-on-Write Patch)
Sao chép sector từ Backup VBR đè lên Primary VBR trên RAM:
```json
virtual_patch_and_verify(
  session_id="...",
  copy_from_lba=backup_vbr_lba,
  to_lba=primary_vbr_lba
)
```
Nếu `mount_status` trả về `"SUCCESS"`, cấu trúc NTFS đã có thể đọc được.

### Bước 4: Kiểm tra và phục hồi MFT Record 0..3 từ $MFTMirr
Nếu VBR tốt nhưng file system vẫn không mount được:
1. Đọc trường `mft_lcn` từ VBR. Tính `MFT_Start_LBA = Start_LBA + (mft_lcn * sectors_per_cluster)`.
2. Kiểm tra 1024 bytes đầu tiên của `$MFT` qua `inspect_sector_structure(lba=MFT_Start_LBA, parse_as="MFT_ENTRY")`.
3. Nếu 4 record đầu tiên bị hỏng (Record 0: `$MFT`, Record 1: `$MFTMirr`, Record 2: `$LogFile`, Record 3: `$Volume`), đọc `mft_mirr_lcn` để sao chép 4 record dự phòng từ `$MFTMirr` đè lên `$MFT` trên RAM Overlay.
