# Case 3 — Mất metadata file, raw carving và nội dung bị phân mảnh

## Nghiên cứu độc lập

Mục tiêu của case này là phân biệt **tìm thấy header**, **xác định ranh giới file**,
**cấu trúc hợp lệ**, **decode được** và **đúng nội dung người dùng cần**. Chúng là
những mức bằng chứng khác nhau. Khi metadata filesystem còn usable, extents của
filesystem vẫn là bằng chứng mạnh hơn suy đoán mảnh kế tiếp bằng khoảng cách hoặc
entropy.

Đã đọc trực tiếp nguồn upstream chính thức qua HTTPS, giữ TLS/proxy, ngày
2026-10-26:

| Nguồn | Nội dung đối chiếu |
| --- | --- |
| [libpng pngrutil.c](https://github.com/pnggroup/libpng/blob/libpng16/pngrutil.c) | Kiểm tra CRC, IHDR, thứ tự đọc chunk, IDAT và IEND |
| [libjpeg-turbo jdmarker.c](https://github.com/libjpeg-turbo/libjpeg-turbo/blob/main/src/jdmarker.c) | SOI/SOF/SOS/EOI, length của segment, component selectors, restart markers và các frame profile được decoder thực hỗ trợ |
| [PNG specification](https://www.w3.org/TR/png/) | Tài liệu tiêu chuẩn để tra cứu tiếp; phiên này không tải nội dung từ w3.org |
| [JPEG ITU-T T.81](https://www.itu.int/rec/T-REC-T.81) | Tài liệu chuẩn nền; phiên này không tải bản PDF từ itu.int |

Bản nguồn libpng đã đọc có SHA-256
`a1d62bc61c118b225c60549ea8de3065ae84642af56dd3776bf4692fb0f6aa5e`;
bản libjpeg-turbo có SHA-256
`f1ea7316e9f05c99929bfc70afebecc505b76fa14836c425d60ad33179771033`.
Hai URL branch có thể thay đổi; hash ghi lại đúng nội dung đã khảo sát. Không copy
code từ các nguồn này vào dự án và không thêm thư viện/lockfile/dependency mới.
`crc32fast` đã là dependency hiện có. Bộ phân tích dưới đây là implementation mới
với phạm vi nhỏ hơn decoder upstream.

## Đã thêm vào mã nguồn

Module `crates/disk-parsers/src/content_validation.rs` dùng `Read + Seek`, có thể
đọc cùng một byte-view dựng từ nhiều mảnh theo thứ tự do caller cung cấp. API:

```rust
analyze(reader, length_bytes, format) -> io::Result<serde_json::Value>
```

`format` nhận `AUTO`, `JPEG`, `PNG`, `MP4`, `MOV`. AUTO chỉ nhận dạng header ở
logical offset 0 của view; không tự quét toàn đĩa hoặc nhận mọi kiểu file.
PDF/ZIP không được quảng bá là đã có validator chỉ vì scanner phát hiện header.

### PNG

- Xác minh signature, IHDR đầu tiên/duy nhất, dimensions, bit depth/color type,
  compression/filter/interlace flags.
- Xác minh giới hạn size và type của chunk, critical chunks được hỗ trợ,
  palette/indexed-color constraints và dãy IDAT liên tục.
- Streaming CRC32 trên type + payload của **mọi chunk đã đọc**, không chỉ header.
- Kiểm tra zlib header gộp từ chuỗi IDAT, không chấp nhận preset dictionary.
- IEND có length 0 và CRC đúng sau image data cho `candidate_length_bytes`.
  `trailing_bytes` được tách riêng: dữ liệu tiếp theo không tự thành một phần PNG.
- Không inflate pixel data, không kiểm tra Adler32 của toàn zlib stream, không
  decode filter/scanline và không xác minh các frame APNG. Có `animated_png` flag
  nếu gặp acTL; CRC/layout pass không chứng minh animation còn nguyên.

### JPEG

- SOI phải ở đầu; duyệt marker và segment lengths, không tìm EOI bằng chuỗi byte
  đơn giản. EOI giả trong APP/thumbnail/comment payload không cắt file chính.
- Hỗ trợ cấu trúc baseline/extended/progressive Huffman SOF0/SOF1/SOF2 với tối đa
  4 component, dimensions khác 0, precision được hỗ trợ và selector hợp lệ.
- Kiểm tra DQT không zero, DHT lengths/symbol-count/oversubscription và table
  references từ SOS. SOS spectral/successive parameters được kiểm tra ở mức header.
- Duyệt entropy data với byte stuffing `FF00`, restart interval và chu kỳ RST0..7;
  yêu cầu có entropy payload trước marker kết thúc scan. EOI sau frame/scan cho
  ranh giới ứng viên chính xác theo marker stream.
- Không giải mã coefficients/Huffman stream hoặc pixel, không chứng minh MCU
  count đúng. Các profile arithmetic/lossless/hierarchical, deferred-height DNL,
  abbreviated streams dùng bảng ngoài trả `UNSUPPORTED_FEATURE` thay vì giả pass.

### MP4/MOV và các mảnh không xác định

MP4/MOV được chuyển sang `iso_bmff::analyze` hiện có. Ranh giới video dùng length
caller đã cung cấp, báo `boundary_evidence=caller_supplied_view`; module không tự
suy ra chiều dài từ mdat hoặc tự tìm moov phù hợp. Thiếu metadata hoặc mảnh có thể
cần tìm thêm ứng viên, evidence filesystem, reference cùng thiết bị/codec hoặc
công cụ chuyên biệt. Không tự thử mọi hoán vị và không copy moov của video khác.

## Giới hạn công việc và phân loại kết quả

PNG/JPEG được giới hạn **64 MiB byte đã đọc**, 16.384 PNG chunks hoặc 65.536 JPEG
markers. Bộ đệm đọc tối đa 64 KiB, segment JPEG tối đa 65.533 bytes. Nguồn đọc
được chặn bằng `Take` theo view length và byte budget; payload lớn không tạo một
allocation theo size do nguồn tùy ý cung cấp. Ảnh lớn hơn budget có thể cần decoder
ngoài hoặc phân tích chuyên biệt; `BUDGET_EXCEEDED` không có nghĩa file hỏng.
MP4 dùng budget box/offset riêng của analyzer hiện có.

| classification | Ý nghĩa |
| --- | --- |
| STRUCTURE_CONSISTENT | Các kiểm tra cấu trúc trong phạm vi module đã pass; `decode_verified=false` |
| INVALID_STRUCTURE | CRC/order/segment/selectors không nhất quán |
| TRUNCATED | View thiếu bytes của một field/payload/terminator cần đọc |
| BUDGET_EXCEEDED | Chưa hoàn tất trong budget, không được kết luận hỏng |
| UNSUPPORTED_FEATURE | Định dạng nhận được nhưng có profile/feature ngoài phạm vi |
| UNKNOWN_OR_UNSUPPORTED_FORMAT | AUTO không nhận dạng header được hỗ trợ tại offset 0 |

Lỗi I/O thật như permission denied được propagate; không đổi thành báo cáo
corruption. Candidate length chỉ có khi bộ phân tích đạt cấu trúc phù hợp. Thứ tự
mảnh vẫn cần evidence độc lập; PNG CRC hoặc JPEG EOI không chứng minh nội dung
đúng, và `decode_verified` không bao giờ được set true trong module này.

## Kiểm thử

Unit tests bao gồm PNG thật tối thiểu với zlib data/CRC, IEND và trailing bytes,
CRC sai, chunk bị cắt, thứ tự critical không đúng, critical chunk chưa hỗ trợ và
chunk-count budget; JPEG 1x1 được tạo tại máy thử, EOI trong APP, EOI bị cắt,
stuffing, restart cycle đúng/sai, SOI/EOI giả thiếu frame và lỗi I/O được giữ nguyên.
Bộ fixture JPEG là dữ liệu thử được tạo tại máy, không phải code upstream.
Root tích hợp thêm MCP/CLI và kiểm thử byte-view xuyên các mảnh ở cấp toàn dự án.
