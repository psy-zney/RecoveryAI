# Playbook 01: Phục hồi Bảng Phân Vùng (MBR & GPT Recovery)

## 1. Dấu hiệu nhận biết
- Ổ đĩa hoặc file `.img` hiển thị trạng thái `Unallocated Space` trong Disk Management.
- Khi gọi `scan_disk_topology`, sector LBA 0 (MBR) hoặc LBA 1..33 (GPT) có trạng thái `ZEROED` hoặc `CORRUPTED`.
- Không thể mở ổ đĩa bằng Explorer, Windows báo "You need to format the disk in drive X: before you can use it."

## 2. Quy trình điều tra cấp thấp (Forensic Investigation)

### Bước 1: Kiểm tra Backup GPT (Secondary GPT Header)
GPT luôn lưu một bản sao của Partition Table ở cuối ổ đĩa:
- Địa chỉ: `Last LBA` của ổ đĩa (ví dụ: `total_sectors - 1`).
- Partition Entry Array của Backup GPT nằm tại `Last LBA - 32` đến `Last LBA - 1`.

Thao tác với MCP:
1. Gọi `inspect_sector_structure(session_id, lba=last_lba, parse_as="GPT")`.
2. Nếu `gpt_header.signature == "EFI PART"` và `crc32` hợp lệ:
   - Bản sao dự phòng còn nguyên vẹn.
   - Sử dụng `virtual_patch_and_verify` để sao chép thông tin phân vùng từ Backup GPT tái tạo Primary GPT ở LBA 1..33 trên RAM.

### Bước 2: Săn lùng Volume Boot Record mồ côi (SIMD VBR Hunt)
Nếu cả Primary GPT và Backup GPT đều bị phá hủy (ví dụ: bị lệnh `clean all` xóa sạch 100MB đầu và cuối):
1. Khởi chạy tác vụ quét ngầm:
   ```json
   start_async_disk_job(
     job_type="SIMD_VBR_HUNT",
     params={"scan_step_sectors": 8, "common_alignments_only": false}
   )
   ```
2. Gọi `poll_async_job(job_id)` định kỳ để nhận danh sách VBR tìm thấy.
3. Nhận diện các chữ ký VBR:
   - **NTFS:** LBA chứa signature `NTFS    ` tại offset +3, marker `0x55AA` tại offset +510.
   - **FAT32:** LBA chứa signature `MSDOS5.0` tại offset +3, string `FAT32   ` tại offset +82.
   - **exFAT:** LBA chứa signature `EXFAT   ` tại offset +3.

### Bước 3: Tái tạo Partition Entry trên RAM
Từ LBA tìm thấy VBR:
- `Start LBA` = VBR LBA (thông thường là bội số của 2048 = 1MB alignment).
- `Total Sectors` = Đọc từ trường `total_sectors` (offset 0x28 trong NTFS VBR).
- `End LBA` = `Start LBA + Total Sectors - 1`.

Áp dụng bản vá:
```json
virtual_patch_and_verify(
  patches=[{
    "lba": 0,
    "offset": 446,
    "hex_data": "<constructed_mbr_partition_entry_16_bytes>"
  }]
)
```
Sau đó kiểm tra `mount_status` trả về danh sách file.
