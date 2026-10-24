---
name: mcp-disk-automation
description: Chuyên gia tự động hóa quy trình điều tra đĩa, điều phối tác vụ nền bất đồng bộ (Async Job Orchestration) và tối ưu hóa tài nguyên RAM Overlay.
---

# Kỹ Năng Tự Động Hóa Tác Vụ Đĩa & Điều Phối Bất Đồng Bộ (MCP Disk Automation)

Trong quá trình điều tra số trên các ổ đĩa dung lượng lớn (hàng trăm Gigabytes đến Terabytes), các tác vụ như quét chữ ký nhị phân toàn đĩa (`SIMD_MAGIC_CARVE`) hay nhân bản phân vùng hỏng (`DD_RESCUE_CLONE`) mất từ vài phút đến nhiều giờ.

Kỹ năng này hướng dẫn AI Agent cách:
1. **Khởi tạo và Điều phối Bất Đồng Bộ (Non-Blocking Orchestration):** Sử dụng cặp công cụ `start_async_disk_job` và `poll_async_job` mà không làm nghẽn hoặc tiêu hao vô ích Token của Context Window.
2. **Kiểm Soát Giới Hạn Bộ Nhớ Đệm RAM (Overlay Budget Management):** Giữ cho `VirtualDiskOverlay` luôn nằm trong hạn mức an toàn (< 512MB RAM), tự động xả hoặc tái sử dụng vùng đệm khi cần thiết.
3. **Phản Hồi Trạng Thái Thời Gian Thực Cho Người Dùng:** Trình bày tiến độ (`progress_percent`), tốc độ xử lý (`sectors_processed/s`) và số lượng mục phát hiện được (`items_found`) một cách ngắn gọn, súc tích.

---

## Danh Mục Playbooks
- [01_async_job_orchestration.md](file:///c:/Users/admin/MyProject/RecoveryAI/.agents/skills/mcp-disk-automation/playbooks/01_async_job_orchestration.md): Chiến lược thăm dò tiến độ không chặn, quản lý vòng đời tác vụ nền và ngắt kết nối an toàn.
