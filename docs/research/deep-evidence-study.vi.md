# Thí nghiệm sâu: giới hạn của tìm mảnh và kiểm định nội dung

Ngày chạy: **27-10-2026**. Đây là một thí nghiệm đối chứng nhỏ trên dữ liệu tổng
hợp, dùng chính MCP server bản release của `resc`. Mục tiêu là tìm các phản ví dụ
cho kết luận quá mạnh như “cấu trúc hợp lệ nghĩa là file đúng” hoặc “decoder chạy
được nghĩa là đã phục hồi đúng”. Kết quả này **không phải tỷ lệ recovery** và
không đại diện cho ổ đĩa thực tế.

Mã tái lập nằm tại [`experiments/recovery_evidence_study.py`](../../experiments/recovery_evidence_study.py),
kết quả máy đọc được nằm tại [`evidence-study-results.json`](evidence-study-results.json).
SHA-256 của script tại thời điểm chạy là
`974c1e84521180a89f29708742484542e35c5c6622d923b0f4244d40de3961fc`.

## 1. Câu hỏi và cách đo

Thí nghiệm PNG đặt sáu ảnh nguồn 8 KiB. Search chỉ biết chữ ký PNG, IEND, lưới
256 byte và giới hạn tối đa hai extent; nó không được biết offset thật, điểm cắt,
thứ tự hoặc tên case. Mỗi giả thuyết được gửi qua tool production
`analyze_file_candidate`. Hash ground truth chỉ được dùng sau khi toàn bộ search
kết thúc để chấm `exact`, không dùng để xếp hạng hoặc dừng sớm.

Thí nghiệm MP4 tạo hai video H.264 64x64, 6 fps, 2 giây bằng FFmpeg/libx264. Cấu
hình `ultrafast`, `yuv420p`, QP 20, `keyint=1:scenecut=0`, không B-frame và một
thread làm 12 packet của hai video có cùng chuỗi kích thước
`592, 35, 35, ..., 35`. Sáu packet cuối của video đỏ được thay đúng độ dài bằng
packet từ video xanh; container và sample table được giữ nguyên. Mỗi case đi qua:

1. `analyze_video_candidate` của `resc` để kiểm tra cấu trúc;
2. `ffprobe -count_frames` để demux/probe độc lập;
3. `ffmpeg -xerror` để giải mã RGB đầy đủ;
4. SHA-256 của RGB giải mã để so với video gốc.

Mọi ảnh nguồn được hash trước và sau. Tất cả case trả `source_unchanged=true`.

## 2. Kết quả tìm mảnh PNG

| Case | Lần validate | Candidate hợp cấu trúc | Đúng byte | Sai byte | Kết luận |
| --- | ---: | ---: | ---: | ---: | --- |
| Liền mạch | 2 | 2 plan / 1 nội dung | 2 | 0 | Tìm được; hai plan quy về cùng bytes |
| Hai mảnh xuôi | 43 | 1 | 1 | 0 | Tìm được trong giả định lưới |
| Hai mảnh đảo vị trí vật lý | 12 | 1 | 1 | 0 | Search hai chiều là cần thiết |
| Hai đuôi PNG đều hợp lệ | 118 | 2 | 1 | 1 | Validator không tự giải ambiguity |
| Đuôi bị ghi đè một byte | 43 | 0 | 0 | 0 | Từ chối thay vì công bố sai |
| Ba mảnh | 87 | 0 | 0 | 0 | Ngoài mô hình tối đa hai extent |

Case hai đuôi là phản ví dụ quan trọng: cả hai candidate đều có cấu trúc PNG và
CRC hợp lệ, nhưng chỉ một candidate khớp file gốc. Vì search không có metadata
hoặc ngữ nghĩa hình ảnh, lựa chọn tự động một trong hai sẽ là kết luận không có
đủ bằng chứng.

## 3. Kết quả kiểm định MP4/H.264

| Case | resc structure | ffprobe | Decode | 12 frame | RGB giống gốc |
| --- | --- | ---: | ---: | ---: | ---: |
| Nguyên vẹn | `STRUCTURE_CONSISTENT` | 0 | 0 | Có | Có |
| Đổi atom `moov` thành `free` | `MISSING_MOOV` | 0 | 0 | Có | Có |
| Cắt 10 byte cuối `mdat` | `INVALID_OR_BUDGET_EXCEEDED` | 1 | 183 | Không | Không đo được |
| Giữ box, zero toàn media payload | `STRUCTURE_CONSISTENT` | 0 | 183 | Không | Không đo được |
| Ghép sáu packet từ video khác | `STRUCTURE_CONSISTENT` | 0 | 0 | Có | **Không** |

FFmpeg vẫn dò và giải mã được elementary H.264 khi atom `moov` bị đổi tên trong
fixture này. Điều đó không có nghĩa metadata/timeline MP4 đã được phục hồi. Case
zero payload cho thấy parser box có thể chấp nhận container trong khi codec hỏng.
Case ghép chéo mạnh hơn: container hợp lệ, probe đủ 12 frame và decoder không báo
lỗi, nhưng hash RGB khác bản gốc. Decoder thành công chỉ là một mức bằng chứng;
nó không chứng minh danh tính nội dung.

Lần chạy đầu đã dừng trước khi ghi kết quả vì encoder mặc định tạo packet đỏ và
xanh lệch một byte. Guard equal-size đã ngăn một phép thay payload làm hỏng sample
table bị diễn giải sai thành phản ví dụ hợp lệ. Cấu hình encoder cố định phía trên
được chọn sau khi kiểm tra chuỗi kích thước và guard vẫn được giữ trong script.

## 4. Cách tái lập

```bash
cargo build --release --workspace --locked
python experiments/recovery_evidence_study.py \
  --output /tmp/resc-evidence-results.json
```

Output dùng chế độ tạo mới và từ chối ghi đè. Media tạm được xóa khi hoàn tất;
JSON giữ phiên bản Python/FFmpeg, tham số fixture, packet sizes, kết quả từng case
và hash nội dung giải mã.

## 5. Phạm vi kết luận

Thí nghiệm không tái lập SHT/PUP hay thuật toán của các paper, không dùng corpus
ổ đĩa độc lập, không đo dữ liệu camera/điện thoại, audio, B-frame, fMP4, H.265/AV1,
NTFS compressed/sparse hoặc media bị overwrite thực tế. Search PNG được lợi từ
lưới 256 byte và giới hạn hai extent; thất bại ba mảnh là kết quả dự kiến của mô
hình, không phải bằng chứng rằng ba mảnh không thể phục hồi.

Untrunc được khảo sát ở mức mã nguồn/README tại commit
`9d86ec9ef2ffed1bf8131abe80742c0574db52b6`. Phiên này thiếu FFmpeg development
headers cần để build, nên không chạy thí nghiệm reference-video repair, không
chép mã GPL vào dự án và không tuyên bố kết quả Untrunc.

Kết quả hỗ trợ ba quyết định thiết kế: giữ nhiều candidate thay vì chọn âm thầm;
tách container, codec, content identity thành các mức bằng chứng; và chỉ công bố
tỷ lệ phục hồi sau benchmark có corpus/ground truth độc lập. Roadmap cho NTFS,
MP4 sample map, candidate graph và corpus nằm trong
[`literature-review-and-roadmap.vi.md`](literature-review-and-roadmap.vi.md).
