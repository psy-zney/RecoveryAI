# Nghiên cứu tiếp: phục hồi video phân mảnh và mất metadata

Ngày kiểm tra nguồn: **26-10-2026**. Tài liệu này là nghiên cứu và đề xuất thiết kế;
không bổ sung implementation, không chứng nhận `resc` đã có các thuật toán dưới
đây. Không sửa cấu hình mạng hoặc chạy công cụ recovery trên dữ liệu thật.

## 1. Hai bài báo khoa học đã xác minh toàn văn

### 1.1. Casey và Zoun: tradeoff thực tế khi carving video phân mảnh

**Eoghan Casey, Rikkert Zoun (2014). “Design tradeoffs for developing fragmented
video carving tools.” Digital Investigation, 11, S30–S39.**
DOI: [10.1016/j.diin.2014.05.010](https://doi.org/10.1016/j.diin.2014.05.010).

- **Loại nguồn:** bài báo hội nghị DFRWS USA 2014, xuất bản trong Digital
  Investigation; không phải README hoặc bài quảng cáo công cụ.
- **Mức tiếp cận:** tải được PDF toàn văn của proceedings từ
  [kho chính thức DFRWS](https://github.com/dfrws/papers/blob/112bba7230d82a1a3d446b248ec4dcbe2487817c/2014%20USA/2014_USA_paper-design_tradeoffs_for_developing_fragmented_video_carving_tools.pdf).
  Đã đối chiếu tiêu đề, tác giả, năm, DOI ở trang đầu của bản publisher và đọc các
  phần về 3GP/MP4, unassigned fragments, validation và conclusions.
- **Nội dung liên quan trực tiếp:** kinh nghiệm từ Defraser/DC3Carver với AVI,
  MPEG và 3GP; dùng cấu trúc container và syntax codec để catalog/reassemble
  fragment, thay vì chỉ ghép sau một header.
- **Điểm quan trọng:** paper phân biệt việc tìm `ftyp`/`moov`/`mdat` với việc
  xác định frame/audio sample. H.264 không có những định danh cố định đủ độc nhất
  để tìm bằng signature thô mà không kiểm tra syntax. Việc đứt mảnh bên trong
  `mdat` làm đứt liên hệ giữa sample tables trong `moov` và dữ liệu media.
- **Reference header:** paper có ví dụ ghép header để render fragment và xây
  container từ frame/sample offsets đã tìm được. Một ví dụ MP4 cụ thể vẫn giả
  định sample tables gốc `stco`, `stsz`, `stsc` còn nguyên; không được diễn giải
  thành “copy moov bất kỳ là sửa được video mất toàn bộ moov”.
- **Giới hạn:** đây là phân tích thiết kế và các case thực nghiệm có phạm vi,
  không phải bằng chứng rằng mọi MP4 hiện đại hoặc mọi mức fragmentation đều
  giải được. Bài báo nói rõ không có một phương pháp tối ưu cho mọi case.
  Decoder/player khác nhau có thể render lượng nội dung khác nhau. Salvage được
  một số frame khác với khôi phục byte-identical cả video.
- **Áp dụng cho `resc`:** bổ sung codec-aware validation, catalog fragment, nhiều
  chiến lược recovery, storyboard và provenance đối với metadata được tổng hợp;
  không nâng confidence của heuristic entropy thành xác suất recovery.

### 1.2. Gloe, Fischer và Kirchner: cấu trúc video khác nhau theo thiết bị/phần mềm

**Thomas Gloe, André Fischer, Matthias Kirchner (2014). “Forensic analysis of
video file formats.” Digital Investigation, 11, S68–S76.**
DOI: [10.1016/j.diin.2014.03.009](https://doi.org/10.1016/j.diin.2014.03.009).

- **Loại nguồn:** bài báo DFRWS EU 2014, Digital Investigation.
- **Mức tiếp cận:** tải và đọc PDF toàn văn từ
  [kho chính thức DFRWS](https://github.com/dfrws/papers/blob/112bba7230d82a1a3d446b248ec4dcbe2487817c/2014%20EU/2014_EU_paper-forensic_analysis_of_video_file_formats.pdf).
  Metadata thư mục ở trên được xác minh ngay trên trang đầu publisher PDF.
- **Bộ dữ liệu được bài báo mô tả:** 19 model camera, 14 model điện thoại và
  6 bộ công cụ chỉnh sửa video. Đó là dữ liệu của bài báo, không phải benchmark
  đã chạy trong `resc`.
- **Nội dung:** khác biệt về container, codec, thông số acquisition, atom order,
  time scales và metadata có ích cho authentication/source attribution. Có
  nhiều thứ tự hợp lệ, gồm trường hợp `moov` cuối file và chuỗi `moof`/`mdat`.
- **Giới hạn cần giữ nguyên:** paper chủ yếu nghiên cứu authentication và
  đặc điểm định dạng, **không phải đánh giá tỷ lệ phục hồi dữ liệu**. Tác giả ghi
  nhận tập thử không có nhiều thiết bị cho mỗi model nên không khẳng định mọi
  quan sát tổng quát cho mọi máy cùng model. Nghiên cứu 2014 không tự xác minh
  hành vi của mọi điện thoại/codec/firmware năm 2026.
- **Áp dụng cho `resc`:** reference video nên được so khớp theo codec parameters,
  dimensions, sample entries, audio config và capture settings; “cùng đuôi mp4”
  hoặc thậm chí “cùng model camera” chưa đủ chứng minh tương thích. Không ép
  mọi MP4 thành một thứ tự atom chuẩn duy nhất.

## 2. Tài liệu kỹ thuật upstream, tách khỏi bài báo khoa học

### 2.1. Untrunc: repair video bị cắt với một reference video khỏe

- **Nguồn:** README upstream của dự án `ponchio/untrunc` và fork
  `anthwlock/untrunc`; đây là tài liệu phần mềm nguồn mở, **không phải paper**,
  không có DOI và không có năm xuất bản được README xác lập.
- **Bản đã kiểm tra:**
  [original README tại commit ebd80fb](https://github.com/ponchio/untrunc/blob/ebd80fb0183eaab7ed911ad78247ccb0c4e82151/README.md)
  và [fork README tại commit 9d86ec9](https://github.com/anthwlock/untrunc/blob/9d86ec9ef2ffed1bf8131abe80742c0574db52b6/README.md).
  Đã tải nội dung trực tiếp từ các commit này.
- **Điều tài liệu thực sự nói:** cần video hỏng và một video không hỏng tương
  tự, tốt nhất từ cùng camera/app; chính README cũng nêu khả năng thất bại.
  Luồng sử dụng tạo file output mới. Fork mô tả thêm logging, skip unknown bytes,
  sửa invalid atom lengths và hỗ trợ một số thiết bị/track cụ thể.
- **Điều chưa xác minh trong đợt nghiên cứu:** chưa build/run Untrunc, chưa đo
  recovery rate hoặc tốc độ, chưa review toàn bộ source và giấy phép để tích hợp.
  Các claim tốc độ/supported devices trong README là claim của maintainer, không
  phải kết quả thử độc lập của dự án này.
- **Hướng tích hợp:** adapter tùy chọn chạy trên bản sao/ứng viên đã export và
  reference được user cung cấp, có timeout, quota, output mới và manifest ghi rõ
  đây là **repaired derivative**. Không bật tự động cho mọi hit và không nhúng
  mã upstream vào dự án trước khi review licensing.

### 2.2. FFmpeg MOV/MP4 demuxer: timeline và các box fMP4

- **Nguồn:** tài liệu chính thức trong
  [FFmpeg doc/demuxers.texi tại commit ceb466c](https://github.com/FFmpeg/FFmpeg/blob/ceb466cbcd7f2366d2dc738cda02e47f0ac59f7d/doc/demuxers.texi),
  phần `mov/mp4/3gp`; bản rendered thông thường ở
  [FFmpeg formats documentation](https://ffmpeg.org/ffmpeg-formats.html#mov_002fmp4_002f3gp).
  Đã đọc source tài liệu ở GitHub, không dựa vào một mirror không xác định.
- **Loại nguồn:** tài liệu kỹ thuật có thể thay đổi theo commit, không phải
  paper, không có DOI hoặc tập thử recovery trong đoạn tài liệu này.
- **Bằng chứng kỹ thuật:** demuxer có xử lý edit lists; fragmented input có thể
  lấy timestamp từ `tfdt`, `sidx`, `mfra`, với các lựa chọn PTS/DTS khác nhau.
  Đó là cơ sở để thiết kế kiểm tra timeline của fMP4, không phải thuật toán tự
  ánh xạ physical fragments từ ảnh đĩa.
- **Đọc nguồn không tin cậy:** tài liệu nêu `enable_drefs` và
  `use_absolute_path` mặc định tắt; bật chúng có rủi ro đọc external tracks và
  lộ thông tin. Decoder adapter phải giữ giới hạn protocol và không tự bật
  external references chỉ để một file hỏng bắt đầu chạy.

## 3. Những bước mạnh hơn bộ kiểm tra box hiện tại

Đây là **đề xuất công việc tiếp theo**, chưa được implementation trong đợt
nghiên cứu này. Cần mỗi bước có corpus và kiểm thử độc lập trước khi quảng bá.

| Thứ tự | Năng lực đề xuất | Bằng chứng đầu vào và đầu ra |
| --- | --- | --- |
| 1 | Kiểm tra toàn bộ sample extents | Kết hợp `stsc`, `stsz`/`stz2`, `stco`/`co64`, sample descriptions trong `stsd`; tính byte range từng sample và đối chiếu bounds của `mdat`, không chỉ chunk-start |
| 2 | Kiểm tra timeline/dependencies | `stts`, `ctts`, `stss`, codec config; tách PTS/DTS, sync samples và dependencies, tránh ghép hai đoạn chỉ vì timestamps gần nhau |
| 3 | Hỗ trợ fMP4 thực chất | Parse `tfhd`/`trun` defaults, sizes, data offsets, `tfdt`, `mfhd`; so với `sidx`/`mfra` nếu có. Báo thiếu field/budget, không giả định moof gần mdat là cùng fragment |
| 4 | Codec-aware H.264 carving | Phân biệt NAL length-prefixed trong MP4 với Annex B; lấy NAL length size/SPS/PPS từ `avcC` khi có; validate NAL/slice syntax, frame-number/POC và IDR constraints bằng parser bị giới hạn |
| 5 | Candidate graph có decoder feedback | Dựng graph của các đoạn có bằng chứng tương thích; bounded search/beam, điểm từ cú pháp+decoder và evidence nguồn. Giữ nhiều giả thuyết và nêu ambiguity; không diễn giải score thành xác suất chưa được calibration |
| 6 | Missing-moov/reference-assisted repair | So reference codec/settings, phát hiện sample offsets/sizes trong media, dựng container/index mới có provenance. Không sao chép nguyên moov của video khác và gọi là dữ liệu gốc |

NAL, SPS/PPS và timestamps có thể giảm không gian tìm kiếm nhưng không thay thế
các bytes đã mất hoặc bị ghi đè. H.265/HEVC, AV1, audio interleaving, encrypted
tracks và codec profile khác cần parser/corpus riêng; không suy rộng kết quả
H.264 sang tất cả video. Với fragment chỉ chứa dependent P/B frames mà thiếu
reference frames, có thể chỉ salvage một phần hoặc không decode được.

## 4. Tiêu chí đánh giá nên dùng

- Corpus phải có original hash và extent map biết trước; gồm contiguous,
  split giữa box/sample/NAL, multi-fragment, đảo thứ tự vật lý, partial overwrite,
  thiếu `moov`, reference tương thích/không tương thích và nhiều video trộn lẫn.
- Báo riêng: byte-identical recovery, số frame/audio sample decode được, timeline
  consistency, false fragment association, duplicate output, runtime và quota
  exhaustion. Không thay mọi tiêu chí bằng một “recovery percentage”.
- Sau decoder pass, tạo storyboard và human content review khi nghi ghép sai
  video; decoder có thể conceal lỗi hoặc vẫn render file chỉ còn một phần.
- Manifest phân biệt bytes gốc với headers/timestamps/sample tables được dựng
  lại; giữ original evidence, version công cụ, reference identity và repair log.
- Paper 2014 là nền tảng thiết kế đáng đọc, không phải baseline hiệu năng hiện
  hành. Cần tái lập corpus của `resc` trước khi nhận một claim tốt hơn upstream.

## 5. Dấu vết kiểm tra nguồn

PDF từ official archive được pin ở commit
`112bba7230d82a1a3d446b248ec4dcbe2487817c` và chuyển bằng `pdftotext` để đối chiếu:

| Nguồn đã tải | SHA-256 |
| --- | --- |
| Casey & Zoun 2014 PDF | `d200e034330e7ff2f342511f5f1280866daaa9ccb310c0102ab9654a03828318` |
| Gloe et al. 2014 PDF | `92e97a16d17de5aab29817fea47fc426fa5c652e75b196ebba4074ba7df7b405` |
| Original Untrunc README | `988ee0f1f459fffa0ee781c263a865182fada3d887b41d1a8c870aa60f2bcb77` |
| Fork Untrunc README | `5c8f9e2fac45b930b291354257491c2ec008002f2f297d41dc5e011d931b4f1a` |
| FFmpeg demuxers.texi | `b0f35446a217ffbf3abb4547312dd0ae61ed7f941332411e8158cf3cb816d261` |

Không thử truy cập nguồn bị chặn qua mirror/proxy khác và không đổi network
allowlist. Đọc được metadata/DOI từ PDF publisher chính thức không đồng nghĩa đã
reproduce thực nghiệm hoặc có quyền copy code/figures của paper vào mã nguồn.
