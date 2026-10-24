# Playbook: Cứu Video MP4 / MOV Bị Phân Mảnh Atom moov & mdat

## 1. Cấu Trúc Khối Atom của Chuẩn ISO Base Media (MP4/MOV)
Một file MP4 hợp lệ là một chuỗi các khối `box` (atom) liên tiếp:
- Mỗi atom gồm: `size` (4 bytes big-endian) + `type` (4 ký tự ASCII) + `data` (`size - 8` bytes).
- Nếu `size == 1`, atom sử dụng trường 64-bit `largesize` nằm ở 8 byte tiếp theo.

### 3 Atom Then Chốt Cần Nhớ:
1. `ftyp` (File Type Box): Thường nằm ở offset 0 (ví dụ: `isom`, `mp42`).
2. `mdat` (Media Data Box): Chứa toàn bộ dòng bit âm thanh / video nén (H.264/H.265/AAC). Thường chiếm 99% kích thước file.
3. `moov` (Movie Box): Chứa bảng chỉ mục khung hình, mẫu thời gian (`stts`, `stsz`, `stco`, `co64`).
   - Trên camera/điện thoại hiện đại, `moov` thường được ghi ở **cuối cùng** của file sau khi quá trình quay video dừng lại.
   - Nếu camera bị tắt nguồn đột ngột hoặc ổ đĩa bị format, atom `moov` thường bị tách rời khỏi `mdat`.

---

## 2. Dấu Hiệu Video MP4 Bị Đứt Đoạn
- Trích xuất file MP4 có atom `ftyp` và bắt đầu của `mdat`, nhưng video không thể mở được trên VLC/Media Player ("Cannot read index" hoặc "Muxer error").
- Kiểm tra hex bằng `inspect_sector_structure`: Cuối file không thấy atom `moov`.

---

## 3. Quy Trình Khôi Phục Atom moov Bị Lạc
1. Chạy lệnh săn tìm atom `moov` bằng SIMD:
   Gọi `start_async_disk_job(job_type="SIMD_MAGIC_CARVE", params={"target_patterns": ["6d6f6f76"]})`.
2. Đối với mỗi atom `moov` tìm được:
   - Đọc 4 byte trước chuỗi `moov` để lấy kích thước của atom.
   - Kiểm tra atom `mvhd` (Movie Header) bên trong `moov`:
     - Tần số xung nhịp (`timescale`).
     - Độ dài thời gian (`duration`).
   - Kiểm tra atom `stco` / `co64` (Chunk Offset Box): Đây là bảng chứa danh sách các offset byte trỏ vào dữ liệu video trong `mdat`.
3. Ghép nối:
   - Điều chỉnh lại các giá trị trong bảng `stco` (Chunk Offset) cho khớp với vị trí LBA bắt đầu của dữ liệu `mdat` trên đĩa.
   - Trích xuất kết hợp `[ftyp + mdat]` và `[moov]` thành tệp `.mp4` hoàn chỉnh.
