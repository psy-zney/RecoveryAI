# Playbook: Thuật Toán Toán Học Khớp Biên & Ghép Nối Mảnh Tệp (Boundary Coherence)

## 1. Bản Chất của Sự Phân Mảnh (Fragmentation Problem)
Khi hệ điều hành ghi một tệp tin lớn lên ổ đĩa nhưng không đủ các block liên tục, tệp tin bị chia làm nhiều phần:
- **Mảnh 1 (Fragment A):** Bắt đầu tại LBA `A_start`, chứa Header chuẩn của tệp tin. Độ dài `A_len` sectors.
- **Mảnh 2 (Fragment B):** Bắt đầu tại LBA `B_start` ở một vị trí xa xôi trên đĩa, không có Header chuẩn, kết thúc bằng Footer hoặc tiếp tục phân mảnh.

Nếu dùng bộ Carving thông thường chỉ lấy các sector liên tục từ `A_start` với kích thước ghi trong Header, nửa sau của file sẽ là rác của file khác, và tệp cứu ra sẽ bị hỏng (corrupted).

---

## 2. Giải Thuật Đánh Giá Điểm Tiếp Giáp (Boundary Coherence Scoring)

Khi phát hiện Mảnh A bị đứt tại sector `LBA_cut` (`A_start + A_len - 1`):
1. **Lọc sơ bộ (SIMD Prefilter):**
   Gọi `start_async_disk_job(job_type="FRAGMENT_PREFILTER")` để Native Engine quét các cluster chưa cấp phát (Unallocated Clusters) thỏa mãn:
   - Entropy nằm trong khoảng dung sai của tệp tin mục tiêu ($|H_{cluster} - H_{fragmentA}| < 0.35$).
   - Không chứa Header của một loại tệp tin khác.
2. **Tính toán hàm Coherence Score $S(B)$:**
   $$S(B) = w_1 \cdot \text{EntropyContinuity}(A, B) + w_2 \cdot \text{ByteFrequencyParity}(A, B) + w_3 \cdot \text{FormatSpecificValidator}(A, B)$$
   Trong đó:
   - $\text{EntropyContinuity} = 1.0 - \min(1.0, |H_{\text{tail\_A}} - H_{\text{head\_B}}|)$
   - $\text{ByteFrequencyParity}$: Độ tương đồng cosine giữa biểu đồ tần suất byte (Byte Frequency Distribution) của 4KB đuôi mảnh A và 4KB đầu mảnh B.
   - $\text{FormatSpecificValidator}$: Kiểm tra tính hợp lệ về mặt ngữ pháp nhị phân của định dạng tệp tin cụ thể (ví dụ: chuỗi bit H.264 NALU, bit Huffman của JPEG, kích thước atom MP4).

---

## 3. Quy Trình AI Agent Điều Khiển Ghép Mảnh
1. Lấy danh sách Top 5 ứng viên từ `poll_async_job`.
2. Kiểm tra phần đầu của từng ứng viên bằng `inspect_sector_structure`.
3. Khi xác định ứng viên chính xác:
   Gọi `extract_recovered_data` với chế độ `CARVE_BY_LBA_RANGES`:
   ```json
   {
     "jsonrpc": "2.0",
     "id": 30,
     "method": "tools/call",
     "params": {
       "name": "extract_recovered_data",
       "arguments": {
         "session_id": "<SESSION_ID>",
         "source_type": "CARVE_BY_LBA_RANGES",
         "target_dir": "D:/RecoveredData/DefragmentedMedia",
         "file_ids_or_ranges": [
           {
             "output_filename": "restored_video.mp4",
             "ranges": [
               { "start_lba": 1048576, "count": 20480 },
               { "start_lba": 5242880, "count": 40960 }
             ]
           }
         ]
       }
     }
   }
   ```
