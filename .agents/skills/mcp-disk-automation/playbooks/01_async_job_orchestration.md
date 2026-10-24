# Playbook: Chiến Lược Thăm Dò Tiến Độ & Quản Lý Tác Vụ Nền Bất Đồng Bộ

## 1. Thiết Kế Bất Đồng Bộ Của Engine
Tất cả các tác vụ nặng được Rust Native Engine đẩy vào `tokio` thread pool nền để đảm bảo luồng JSON-RPC stdio luôn sẵn sàng đáp ứng lệnh mới (< 5ms).

```
[ AI Agent ]  -- start_async_disk_job --> [ MCP Server ] --> Spawn Worker Thread
[ AI Agent ]  <-- { job_id, status: RUNNING }
       |
       | (Periodic interval / Reactive wake-up)
       v
[ AI Agent ]  -- poll_async_job(job_id) --> [ MCP Server ]
[ AI Agent ]  <-- { progress_percent, items_found, status }
```

---

## 2. Quy Tắc Tiết Kiệm Token Context Window (Anti-Token Bloat)
> **QUY TẮC VÀNG:** Tuyệt đối KHÔNG gọi `poll_async_job` trong vòng lặp liên tục (busy-wait) trong cùng một lượt phản hồi. Việc gọi liên tục sẽ gây tràn token context window và tốn chi phí API không cần thiết.

### Chu Trình Khuyến Nghị Cho AI:
1. Gọi `start_async_disk_job`.
2. Kiểm tra `estimated_sectors` và `job_type`.
3. Báo cáo ngắn gọn cho người dùng:
   *"Tác vụ quét chữ ký nhị phân đã được khởi động ở chế độ nền (Job ID: `...`). Đang tiếp tục theo dõi tiến độ..."*
4. Gọi `poll_async_job` ở các mốc tiến độ hoặc dựa vào công cụ lập lịch (`schedule`).
5. Chỉ khi `status == "COMPLETED"`, tiến hành trích xuất danh sách kết quả `results[]`.

---

## 3. Quản Lý Ngân Sách RAM Overlay (`VirtualDiskOverlay`)
Mỗi sector vá trong RAM tiêu tốn 512 bytes đến 4096 bytes cộng với overhead của HashMap.
- Giới hạn khuyến nghị: `512 MB` (đủ để lưu trữ hơn `1,000,000` sector phân vùng và siêu dữ liệu).
- Luôn kiểm tra trường `overlay_memory_used_kb` trong kết quả của `virtual_patch_and_verify`.
- Nếu `overlay_memory_used_kb > 450,000 KB` (90% dung lượng), đóng và khởi tạo lại session mới để giải phóng RAM trước khi tiếp tục các bài kiểm tra tiếp theo.
