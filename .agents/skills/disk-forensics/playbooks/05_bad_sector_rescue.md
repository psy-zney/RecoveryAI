# Playbook 05: Cách Ly Phần Cứng Hỏng & Nhân Bản An Toàn Với DDRescue

## 1. Dấu hiệu Nhận biết
- `bad_sectors_count > 0` trong kết quả của `open_recovery_session`.
- Thời gian đọc của một sector vượt quá `200ms` (Read Timeout).
- Hệ điều hành liên tục reset bus SATA/NVMe hoặc USB tự ngắt kết nối.

## 2. Nguyên tắc An toàn Cực hạn
> **CẢNH BÁO:** Không thực hiện bất kỳ lệnh quét sâu (`SIMD_MAGIC_CARVE`) hay phân tích Metadata lặp đi lặp lại trực tiếp trên ổ đĩa vật lý đang bị bad sector. Đầu từ đọc (Read/Write Head) có thể bị crash hoàn toàn, dẫn đến hư hỏng vĩnh viễn bề mặt đĩa từ.

## 3. Quy trình Clone 3 Giai đoạn (3-Phase Rescue Clone)

Gọi `start_async_disk_job` với kiểu `DD_RESCUE_CLONE`:

```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "tools/call",
  "params": {
    "name": "start_async_disk_job",
    "arguments": {
      "session_id": "<SESSION_ID>",
      "job_type": "DD_RESCUE_CLONE",
      "params": {
        "destination_image": "D:/RescueImages/disk_clone.img",
        "mapfile": "D:/RescueImages/disk_clone.map",
        "retry_passes": 1,
        "max_read_rate": "50M"
      }
    }
  }
}
```

### Các giai đoạn thực thi tự động của Engine:
1. **Giai đoạn 1 (Fast Copy - Non-Scraped):** Đọc tuần tự các block lớn (64KB - 1MB). Khi gặp bad block đầu tiên, nhảy qua một khoảng lớn (`cluster skip`) để cứu tối đa các vùng dữ liệu lành lặn với tốc độ cao nhất trước khi ổ đĩa bị suy giảm thêm.
2. **Giai đoạn 2 (Scraping):** Quay lại đọc các vùng biên tiếp giáp giữa khối dữ liệu tốt và khối lỗi để thu hẹp vùng chưa đọc.
3. **Giai đoạn 3 (Trimming / Retrying):** Thử đọc lại các sector nghi ngờ lỗi với kích thước 512 bytes và số lần thử (`retry_passes = 1`).

## 4. Polling và Chuyển Session
Định kỳ thăm dò tiến độ bằng `poll_async_job`:
```json
{
  "jsonrpc": "2.0",
  "id": 11,
  "method": "tools/call",
  "params": {
    "name": "poll_async_job",
    "arguments": {
      "session_id": "<SESSION_ID>",
      "job_id": "<JOB_ID>"
    }
  }
}
```

Khi `status == "COMPLETED"`:
1. Đóng phiên làm việc với ổ đĩa vật lý.
2. Mở phiên làm việc mới trỏ trực tiếp vào file ảnh đã nhân bản:
   `open_recovery_session(path="D:/RescueImages/disk_clone.img")`.
3. Tiến hành các bước phân tích phân vùng và khôi phục tệp trên file ảnh an toàn.
