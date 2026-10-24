# Playbook: Cứu Dữ Liệu SQLite Từ Các B-Tree Leaf Pages Mồ Côi

## 1. Cấu Trúc File Database SQLite (SQLite 3 Format)
Một tệp SQLite bắt đầu bằng 100 bytes Database Header:
- Offset `0x00`: Chuỗi ASCII `SQLite format 3\000` (16 bytes: `53 51 4C 69 74 65 20 66 6F 72 6D 61 74 20 33 00`)
- Offset `0x10`: Kích thước Page (`page_size`, 2 bytes big-endian: thường là 4096 = `0x1000` hoặc 1024, 8192).
- Offset `0x18`: Số lượng Reserved bytes ở cuối mỗi page (thường là 0).
- Offset `0x20`: First freelist trunk page.
- Offset `0x24`: Total number of freelist pages.

---

## 2. Các Loại B-Tree Page Trong SQLite
Mỗi page trong SQLite (ngoại trừ Page 1 có 100 byte header đầu) bắt đầu bằng một byte cờ `page_type`:
- `0x0D` (`13`): **Table B-Tree Leaf Page** (Chứa trực tiếp dữ liệu bản ghi hàng - Row Records!).
- `0x05` (`5`): Table B-Tree Interior Page (Chứa khóa và con trỏ trỏ tới page con).
- `0x0A` (`10`): Index B-Tree Leaf Page.
- `0x02` (`2`): Index B-Tree Interior Page.

### Cấu Trúc Header Của B-Tree Leaf Page (`0x0D`):
- Byte 0: `0x0D`
- Bytes 1..2: Vị trí bắt đầu của khối First Freeblock (`0x0000` nếu không có).
- Bytes 3..4: Số lượng Cell (bản ghi) trong page (`cell_count`).
- Bytes 5..6: Offset bắt đầu của vùng nội dung cell (`cell_content_area_start`).
- Byte 7: Số lượng fragmented free bytes.
- Bytes 8.. (`8 + 2 * cell_count`): Mảng con trỏ 2-byte chỉ vị trí từng cell trong page.

---

## 3. Quy Trình Khai Phá SQLite Khi Mất Header Page 1
Khi Page 1 bị xóa hoặc ghi đè, toàn bộ cấu trúc Schema (`sqlite_master`) bị mất. Tuy nhiên, hàng nghìn trang dữ liệu (`0x0D` leaf pages) vẫn còn nguyên vẹn rải rác trên các cluster:
1. Chạy `start_async_disk_job(job_type="SIMD_MAGIC_CARVE")` để tìm các page bắt đầu bằng byte `0x0D` tại các mốc bội số của 4096 bytes (4KB boundary aligned).
2. Kiểm tra tính toán học của Header Page:
   - `cell_count * 2 + 8 <= cell_content_area_start <= page_size`.
   - Tất cả con trỏ trong Cell Pointer Array phải trỏ tới vùng nằm giữa `cell_content_area_start` và `page_size`.
3. Giải mã Record Payload (Varint Parsing):
   - Mỗi cell trong Leaf Page bắt đầu bằng: `payload_size` (Varint) + `rowid` (Varint) + `record_header` (Serial Type Varints) + Dữ liệu các cột.
4. Trích xuất trực tiếp các chuỗi văn bản (Text/BLOB) và khôi phục lại bảng dữ liệu hoàn chỉnh mà không cần Page 1 ban đầu!
