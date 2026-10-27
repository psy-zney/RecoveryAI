# Nghiên cứu chuyên ngành: carving file phân mảnh và xác minh ứng viên

Ngày đối chiếu nguồn: 26/10/2026. Tài liệu này là nghiên cứu và đề xuất thiết kế,
**không phải mô tả tính năng đã triển khai thêm** trong lượt nghiên cứu này.

## Chất lượng và giới hạn nguồn

Đã tải bản PDF đầy đủ từ repository chính thức
[DFRWS papers](https://github.com/dfrws/papers), rồi đọc trang metadata/abstract,
các phần thuật toán, đánh giá và giới hạn liên quan. Tên bài, tác giả, năm,
journal/pages và DOI dưới đây được đọc **trực tiếp trong PDF**, không gán từ trí
nhớ hoặc trích dẫn thứ cấp. Ba bản PDF đã được đối chiếu byte-for-byte với commit
archive `112bba7230d82a1a3d446b248ec4dcbe2487817c`.

Đây là các công trình nền tảng 2007–2009, không phải bằng chứng rằng phương pháp
nào đang là tốt nhất vào năm 2026. Truy cập trực tiếp `dfrws.org` bị proxy trả 403;
không đổi allowlist, không bỏ proxy hoặc tắt TLS. Repository DFRWS trên GitHub là
nguồn lưu trữ chính thức có thể truy cập bằng policy hiện tại. DOI đã xác minh
trong PDF; không tuyên bố truy cập thành công Crossref/publisher qua DOI.

## 1. Garfinkel 2007: validators nhiều tầng và bifragment gap carving

**Simson L. Garfinkel.** “Carving contiguous and fragmented files with fast object
validation.” *Digital Investigation* **4S** (2007), S2–S12.
DOI: [10.1016/j.diin.2007.06.017](https://doi.org/10.1016/j.diin.2007.06.017).

[PDF đầy đủ, DFRWS archive](https://raw.githubusercontent.com/dfrws/papers/112bba7230d82a1a3d446b248ec4dcbe2487817c/2007%20USA/2007_USA_paper-carving_contiguous_and_fragmented_files_with_object_validation.pdf).
Mức đọc: full text các mục 4.1, 4.2, 5.1–5.3 và kết luận; không chỉ abstract.

**Điểm đã xác minh trong bài:**

- Mục 4.1 xếp kiểm tra header/footer rẻ trước container validation và giải nén.
  Header/footer phù hợp dùng để loại nhanh ứng viên; không xác nhận phần nằm giữa.
- Mục 4.2 không giới hạn validator ở true/false: có `V_OK`, `V_ERR`, `V_EOF`,
  chiều dài suy ra và các hàm nhận diện cấu trúc/offset. Điều này giúp phân biệt
  ứng viên sai với ứng viên chưa đủ byte để kết luận.
- Mục 5.2.1 dựng thử hai đoạn nằm hai bên một gap, rồi đưa view ghép vào validator.
  Bài chỉ ra chi phí O(n²) cho một object có header/footer trong mô hình đó;
  không phải lời giải nhanh cho số mảnh bất kỳ trên toàn ảnh lớn.
- Mục 4.1.3/Fig. 1 cho thấy decoder có thể đi qua các sector sai rồi mới báo lỗi;
  byte/sector nơi decoder dừng không mặc nhiên là điểm phân mảnh.
- Mục 4.1.5 và kết quả MSOLE vẫn ghi nhận ứng viên mở được trong ứng dụng nhưng
  nội dung sai. Cần phân biệt cấu trúc, decoder và nội dung người dùng xác minh.

**Ý nghĩa cụ thể cho resc:** framework validators và `ByteView` hiện có là cơ sở
phù hợp cho một job tìm gap **có budget**. Cần thêm kết quả “thiếu dữ liệu” và
evidence theo vị trí, thay vì biến mọi prefix chưa hoàn chỉnh thành “hỏng file”.
Không lấy entropy hoặc decoder exit code làm bằng chứng độc lập đủ để tự chọn mảnh.

## 2. Pal, Sencar & Memon 2008: phát hiện điểm phân mảnh và tìm đường ghép

**Anandabrata Pal, Husrev T. Sencar, Nasir Memon.** “Detecting file fragmentation
point using sequential hypothesis testing.” *Digital Investigation* **5** (2008),
S2–S13. DOI: [10.1016/j.diin.2008.05.015](https://doi.org/10.1016/j.diin.2008.05.015).

[PDF đầy đủ, DFRWS archive](https://raw.githubusercontent.com/dfrws/papers/112bba7230d82a1a3d446b248ec4dcbe2487817c/2008%20USA/2008_USA_paper-detecting_file_fragmentation_point_using_sequential_hypothesis_testing.pdf).
Mức đọc: full text mục 3, 4.3, 5, 6, 7 và kết luận.

**Điểm đã xác minh trong bài:**

- Bài tách ba vấn đề: tìm đầu file, tìm các block thuộc file và xếp đúng thứ tự.
  Bước phát hiện chữ ký chỉ giải quyết phần đầu của vấn đề.
- Mục 3.2 mô tả graph-theoretic carving/PUP từ công trình Pal & Memon trước đó:
  matching metric phải phụ thuộc loại nội dung, không có một khoảng cách byte
  hay entropy chung đủ tin cậy cho mọi format. Chỉ đọc mô tả này như nguồn thứ cấp
  của công trình trước, không nhận đã đọc bài Pal & Memon 2006.
- Mục 5 dùng sequential hypothesis testing, với quyết định tiếp tục/loại/chưa đủ
  bằng chứng, thay cho dừng ngay tại một điểm codec báo lỗi. Mục 6 kết hợp SHT với
  Parallel Unique Path và tìm cục bộ ứng viên trước khi mở rộng phạm vi.
- Mục 3/Fig. 2 đưa ví dụ JPEG **decode được nhưng ghép sai**. Đây là phản ví dụ
  trực tiếp cho giả định “decoder không báo lỗi ⇒ đúng toàn bộ mảnh”.
- Thực nghiệm ở mục 7 dùng JPEG thuộc DFRWS 2006/2007. Bài ghi rõ không tự sửa
  decoding errors hay missing fragments trong các test đó. Không lấy kết quả
  này để cam kết hiệu quả với MP4, ảnh SSD hiện đại hay dữ liệu bị overwrite.
- Nguy cơ false addition được xem nặng hơn false elimination: mảnh đúng tạm bỏ
  có thể được xét lại, trong khi ghép thêm block sai có thể làm lệch cả đường ghép.

**Ý nghĩa cụ thể cho resc:** bước tiếp theo nên là bộ tìm ứng viên nhiều đường có
giới hạn và kết quả mơ hồ rõ ràng. Graph chỉ nên dựng trên tập extents đã lọc theo
loại file/metadata, không nối tất cả sector với nhau. Giữ nhiều giả thuyết và lưu
lý do chấm điểm; không đổi tên score heuristic hiện tại thành xác suất thống kê.

Một thiết kế beam search hoặc best-first search là **đề xuất riêng cho dự án**,
không phải tuyên bố đã cài đặt đúng thuật toán SHT–PUP trong bài.

## 3. Sencar & Memon 2009: JPEG thiếu mảnh/header cần kiến thức codec

**Husrev T. Sencar, Nasir Memon.** “Identification and recovery of JPEG files with
missing fragments.” *Digital Investigation* **6** (2009), S88–S98.
DOI: [10.1016/j.diin.2009.06.007](https://doi.org/10.1016/j.diin.2009.06.007).

[PDF đầy đủ, DFRWS archive](https://raw.githubusercontent.com/dfrws/papers/112bba7230d82a1a3d446b248ec4dcbe2487817c/2009%20USA/2009_USA_paper-identification_and_recovery_of_jpeg_files_with_missing_fragments.pdf).
Mức đọc: full text mục 4, 5.2 và kết luận, cùng abstract/metadata.

**Điểm đã xác minh trong bài:**

- Tác giả dùng mẫu bit liên quan Huffman tables để thu hẹp tập mảnh JPEG có thể
  tương thích. Đây là bằng chứng tương thích encoding, không phải danh tính file.
- Mục 4.2 lưu ý nhiều ảnh dùng cùng Huffman tables thì phương pháp không tách
  riêng được từng ảnh. Không thể kết luận “cùng bảng ⇒ cùng file”.
- Mục 5.2 xem xét pseudo-header từ các ảnh liên quan và vai trò của kích thước,
  quantization tables, cấu hình MCU/chroma subsampling và Huffman tables.
  MCU/Huffman phù hợp là yêu cầu quan trọng; chọn header tùy ý không bảo đảm cứu được.
- Các ví dụ pseudo-header có thể tạo ảnh lệch vị trí, giảm chất lượng hoặc mất
  đồng bộ; kết luận còn yêu cầu dữ liệu đa dạng hơn để đánh giá tính tổng quát.

**Ý nghĩa cụ thể cho resc:** một nhánh JPEG chuyên sâu có thể lập “header/table
bank” từ các file nguyên vẹn cùng nguồn và dùng nó để xếp hạng ứng viên. Sản phẩm
có header dựng lại phải đánh dấu là **derived/partial reconstruction**, tách byte
nguồn với byte được tạo; không xuất manifest làm người dùng hiểu là file gốc nguyên
vẹn. Thuật toán JPEG trong bài không được suy rộng trực tiếp thành cách dựng lại
`moov` của MP4/MOV.

## Đề xuất lộ trình cho resc, theo lợi ích và độ kiểm chứng

| Thứ tự | Hướng làm | Hợp đồng cần có trước khi công bố |
| --- | --- | --- |
| 1 | Validator nhiều tầng, phân biệt reject/incomplete/unsupported/consistent | Ghi vị trí/bằng chứng, không biến prefix hay budget-exhaustion thành kết luận hỏng |
| 2 | Job bifragment search giới hạn trên `ByteView` | Giới hạn số plan/gap/byte/time, cancel; lưu mọi extent và score; dừng mà không tự chọn khi có nhiều ứng viên ngang bằng |
| 3 | Candidate graph theo format và metadata | Không trùng byte nguồn, giới hạn độ sâu/top-k; codec state và cấu trúc mạnh hơn entropy; cache các phép kiểm đã làm |
| 4 | JPEG-specific fragment grouping/header-bank thử nghiệm | Kiểm tra DQT/DHT/MCU, lưu donor/derived bytes, ghi partial reconstruction; benchmark false joins |
| 5 | Corpus đánh giá ground truth và dữ liệu âm | Đo đúng byte/extent và false positive; có file cùng codec/tables nhưng khác nội dung, fragments đảo thứ tự/thiếu/ghi đè |

Trong lộ trình này, benchmark phải phân biệt ít nhất: signature hit, cấu trúc hợp
lệ, decode pass, khớp bytes ground truth và người dùng xác nhận nội dung. Không
gộp thành một cờ “recovered”. Mỗi ứng viên được thử phải giữ nguồn chỉ đọc và
provenance có thể kiểm lại; không ghi file thật cho mọi nhánh search nếu view ảo
đã đủ để loại sớm.

DFRWS 2006/2007 là bộ test có thật, được mô tả và dùng trong cả hai bài đầu.
Trong lượt nghiên cứu này **chưa tải/chạy corpus đó** và không coi chúng là đại
diện đầy đủ cho mọi thiết bị, filesystem, format hay mức phân mảnh hiện nay.

## Dấu vết tải nguồn

Các PDF không được chép vào repository dự án; chỉ lưu link và checksum để người
đọc có thể tải lại từ archive chính thức. SHA-256 bản đã đọc:

| Bài | SHA-256 |
| --- | --- |
| Garfinkel 2007 | `01be6e8318197e3d9364e24801d398fee984461ba04a313213ac0f695c3206c3` |
| Pal/Sencar/Memon 2008 | `e8a64607b9fc1631aa6a9f3e877fa1c9fdf07435447b13702c9e0ccd7eb9195e` |
| Sencar/Memon 2009 | `d467965e5b1187b2e8a1967995447aeaac487683a33823b802fcf8298d0f801c` |

Không thay đổi application code, dependency, tests, cấu hình network hay công bố
tính năng mới trong công việc nghiên cứu này.
