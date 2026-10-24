# Playbook 04: Cứu Dữ Liệu Phân Mảnh & Khám Nghiệm File Carving (Defragmentation & Raw Carving)

## 1. Bối cảnh sử dụng
- Khi toàn bộ File System Metadata (MFT, FAT, Extent Tree) đã bị xóa sạch hoặc format cấp thấp.
- Các công cụ thông thường chỉ cứu được file liên tục (contiguous), nhưng các file video dung lượng lớn (`.mp4`, `.mov`), file nén (`.zip`, `.docx`), hoặc database (`.sqlite`) thường bị phân mảnh thành nhiều đoạn nằm rải rác trên đĩa.

## 2. Kỹ thuật Ghép mảnh Hai đoạn (Bifragment Gap Carving)

Hầu hết các file bị phân mảnh trong thực tế là do ghi chèn vào các khoảng trống cluster tự do, dẫn đến mô hình 2 mảnh (Fragment A và Fragment B):
- **Fragment A (File Header):** Bắt đầu bằng Magic Bytes (ví dụ: `00 00 00 18 66 74 79 70` cho MP4, `50 4B 03 04` cho ZIP).
- **Gap (Khoảng trống bị chiếm):** Dữ liệu của file khác hoặc cluster trống.
- **Fragment B (File Tail / Continuance):** Đoạn thân hoặc đuôi file tiếp theo (ví dụ: `moov` atom hoặc `mdat` payload trong MP4, hoặc Central Directory trong ZIP).

## 3. Quy trình thực hiện với MCP Engine

### Bước 1: Quét Magic Bytes nhận diện Header
1. Khởi chạy tác vụ quét chữ ký bằng SIMD:
   ```json
   start_async_disk_job(
     job_type="SIMD_MAGIC_CARVE",
     params={"target_extensions": ["mp4", "zip", "jpg", "pdf", "sqlite"]}
   )
   ```
2. Lưu lại `header_lba` và `signature` tìm được.

### Bước 2: Đo độ liên tục Shannon Entropy và ranh giới
1. Phân tích 4KB cuối cùng của mảnh đầu (`Fragment A`):
   - Tính toán Shannon Entropy. Ví dụ: Dữ liệu video nén (H.264/H.265) có entropy ổn định từ `7.6 - 7.95 bits/byte`.
   - Nếu entropy đột ngột tụt xuống `0.0` (zero-fill) hoặc thay đổi đặc tính nhị phân, đó là điểm đứt đoạn (Cut-off LBA).

### Bước 3: Tiền lọc ứng viên nối mảnh (SIMD Fragment Prefilter)
Gọi tool tiền lọc của Native Engine để tìm cluster khớp biên:
```json
start_async_disk_job(
  job_type="FRAGMENT_PREFILTER",
  params={
    "head_last_lba": cut_off_lba,
    "search_window_clusters": 50000,
    "file_type": "mp4",
    "top_candidates": 5
  }
)
```
Native Engine sử dụng tập lệnh SIMD đo:
- **Boundary Coherence:** Khớp cú pháp cấu trúc container (ví dụ độ dài chunk, NAL unit header `00 00 00 01 xx`).
- **Entropy Continuity:** Chênh lệch entropy $|\Delta E| < 0.15$.

### Bước 4: Trích xuất file hoàn chỉnh
Sau khi xác định được `Fragment A: [start_lba_1, end_lba_1]` và `Fragment B: [start_lba_2, end_lba_2]`:
```json
extract_recovered_data(
  session_id="...",
  mode="CARVE_BY_LBA_RANGES",
  params={
    "ranges": [
      {"start_lba": start_lba_1, "count": count_1},
      {"start_lba": start_lba_2, "count": count_2}
    ],
    "output_filename": "recovered_video.mp4"
  },
  output_dir="C:/Recovery_Output"
)
```
Sau đó kiểm tra tính hợp lệ của file đã trích xuất bằng cách chạy parser/decoder kiểm tra header và đuôi file.
