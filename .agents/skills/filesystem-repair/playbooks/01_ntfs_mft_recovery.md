# Playbook: Phục Hồi Boot Sector & Bảng Chỉ Mục NTFS ($MFT)

## 1. Cấu Trúc Khối VBR NTFS (Volume Boot Record)
Một sector VBR NTFS hợp lệ (512 bytes) luôn tuân thủ các trường sau:
- Offset `0x00`: Nhảy lệnh x86 (`EB 52 90` hoặc `EB 5B 90`)
- Offset `0x03`: Chuỗi OEM ID `NTFS    ` (8 bytes)
- Offset `0x0B`: Bytes per Sector (thường là `0x0200` = 512 bytes)
- Offset `0x0D`: Sectors per Cluster (thường là `0x08` = 4KB cluster)
- Offset `0x28`: Tổng số sector của Volume (`total_sectors`, 8 bytes little-endian)
- Offset `0x30`: Cụm bắt đầu của `$MFT` (`mft_lcn`, 8 bytes)
- Offset `0x38`: Cụm bắt đầu của `$MFTMirr` (`mft_mirr_lcn`, 8 bytes)
- Offset `0x40`: Clusters per File Record Segment (thường là `0xF6` = 1024 bytes)
- Offset `0x1FE`: Magic signature `0x55AA`

## 2. Quy Trình Sửa Lỗi VBR NTFS Bằng Backup VBR
1. Xác định vị trí `Start LBA` và `End LBA` từ bảng phân vùng.
2. Kiểm tra Backup VBR tại `End LBA`:
   ```json
   {
     "jsonrpc": "2.0",
     "id": 20,
     "method": "tools/call",
     "params": {
       "name": "inspect_sector_structure",
       "arguments": {
         "session_id": "<SESSION_ID>",
         "lba": <END_LBA>,
         "count": 1
       }
     }
   }
   ```
3. Nếu Backup VBR có chữ ký `NTFS    ` và `0x55AA`, lấy chuỗi hex của sector này và vá đè lên `Start LBA` trong RAM Overlay:
   ```json
   {
     "jsonrpc": "2.0",
     "id": 21,
     "method": "tools/call",
     "params": {
       "name": "virtual_patch_and_verify",
       "arguments": {
         "session_id": "<SESSION_ID>",
         "lba": <START_LBA>,
         "patch_hex": "<BACKUP_VBR_HEX_STRING>"
       }
     }
   }
   ```

## 3. Khai Thác Bản Ghi Dự Phòng `$MFTMirr` Khi Record 0 Bị Lỗi
Khi `$MFT` chính bị hư hỏng ở những record đầu tiên (chữ ký `FILE0` bị ghi đè thành `0x00` hoặc rác):
1. Tính địa chỉ LBA của `$MFTMirr`:
   `mft_mirr_lba = Start LBA + (mft_mirr_lcn * sectors_per_cluster)`
2. `$MFTMirr` chứa chính xác 4 record File Record Segment (FRS 0, 1, 2, 3), mỗi record 1024 bytes (2 sector):
   - Record 0: `$MFT`
   - Record 1: `$MFTMirr`
   - Record 2: `$LogFile`
   - Record 3: `$Volume`
3. Đọc 4 record này từ `$MFTMirr` và vá ngược lại vùng bắt đầu của `$MFT` trên RAM Overlay.
4. Chạy `scan_filesystem_journals` để kiểm chứng cây thư mục đã có thể phân giải được.
