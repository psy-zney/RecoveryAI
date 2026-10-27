# Nghiên cứu chuyên ngành: resc còn thiếu gì để phục hồi dữ liệu khó?

Ngày khảo sát: **26-10-2026**. Ba nhóm case trước là cách tổ chức bài toán, chưa
phải ba thuật toán giải quyết trọn vẹn. Lượt này khảo sát **14 công trình**, gồm
các bài xuất bản và một bản tác giả DFRWS APAC 2021, từ PDF trong
[kho chính thức DFRWS](https://github.com/dfrws/papers). Không dùng bài quảng cáo
công cụ làm bằng chứng khoa học. Tài liệu Untrunc/FFmpeg được tách riêng.

Đã đọc metadata, abstract và các mục phương pháp/kết quả/giới hạn liên quan trong
full text. Đây **không phải systematic review bao phủ mọi bài đến năm 2026** và
không xác lập thuật toán nào là state of the art hiện nay. Publisher, Crossref,
OpenAlex và website DFRWS trực tiếp bị policy mạng chặn trong phiên; bản PDF do
DFRWS công bố trên GitHub vẫn truy cập được. DOI lấy từ PDF, không đoán từ tên bài.

## 1. Đánh giá hiện trạng dự án

resc có lớp I/O chỉ đọc, overlay/WAL, parser và validator có giới hạn, export đúng
byte kèm provenance, và MCP/CLI điều phối. Đó là nền để thử nghiệm reconstruction.
Phần còn thiếu lớn nhất là **tự tìm tập mảnh và thứ tự của chúng khi metadata đã
mất**, **phục hồi metadata/codec chuyên sâu**, và **đánh giá trên corpus độc lập**.

74 unit tests, 16 nhóm E2E và 13 kiểm thử workbench/tích hợp đã pass xác nhận các
fixture đã chạy. Chúng chưa cho biết phần trăm phục hồi file phân mảnh ngoài
thực tế. Hai mảnh được fixture cung cấp đúng thứ tự không chứng minh engine tự
suy ra được thứ tự đó.

## 2. Các bài báo nói gì, và điều đó thay đổi thiết kế ra sao?

### [P1] Object validation để tìm gap, không chỉ tìm chữ ký

**Simson L. Garfinkel (2007), _Carving contiguous and fragmented files with fast
object validation_. Digital Investigation 4S, S2–S12.**
[DOI 10.1016/j.diin.2007.06.017](https://doi.org/10.1016/j.diin.2007.06.017).

Bài tổ chức kiểm tra rẻ trước, container validation và giải nén sau; validator
có thể trả thiếu dữ liệu thay vì chỉ đúng/sai. Bifragment gap carving thử các view
ghép hai đoạn rồi dùng validator loại ứng viên; mô hình tìm kiếm trong bài có
chi phí bậc hai và không giải mọi số mảnh trên toàn ảnh lớn.

**Cho resc:** thêm job tìm gap có budget trên ByteView, cache kết quả validator,
giữ các trạng thái reject/incomplete/unsupported và nhiều ứng viên. Scanner header
hiện tại mới hỗ trợ bước tìm đầu vào; nó chưa phải engine tìm thứ tự mảnh.

### [P2] Tìm điểm phân mảnh và giữ nhiều giả thuyết

**Anandabrata Pal, Husrev T. Sencar, Nasir Memon (2008), _Detecting file
fragmentation point using sequential hypothesis testing_. Digital Investigation
5, S2–S13.** [DOI 10.1016/j.diin.2008.05.015](https://doi.org/10.1016/j.diin.2008.05.015).

Tác giả tách tìm đầu file, chọn block và xếp thứ tự thành các vấn đề khác nhau.
Sequential hypothesis testing hỗ trợ quyết định tiếp tục/loại/chưa đủ bằng chứng;
paper kết hợp với tìm đường ghép và tìm ứng viên cục bộ. Figure 2 minh họa JPEG
**decode được nhưng ghép sai**. Thử nghiệm liên quan JPEG DFRWS, không phải mọi
MP4 hoặc mọi trường hợp overwrite.

**Cho resc:** candidate graph theo format và metadata, search giới hạn và ghi
ambiguity. Beam/best-first search là đề xuất thiết kế cho dự án, chưa phải bản
triển khai thuật toán paper. Không dùng decoder exit code hoặc entropy làm tiêu
chí duy nhất để tự chọn mảnh.

### [P3] JPEG thiếu header/mảnh cần kiến thức codec

**Husrev T. Sencar, Nasir Memon (2009), _Identification and recovery of JPEG files
with missing fragments_. Digital Investigation 6, S88–S98.**
[DOI 10.1016/j.diin.2009.06.007](https://doi.org/10.1016/j.diin.2009.06.007).

Huffman/quantization tables, MCU và chroma subsampling giúp đánh giá tương thích
và dựng pseudo-header trong một số tình huống. Cùng bảng Huffman không chứng minh
hai mảnh cùng một file; header dựng lại có thể tạo ảnh lệch hoặc chỉ phục hồi một phần.

**Cho resc:** JPEG-specific header/table bank, decoder state và kiểm tra nối ảnh,
với donor identity và các byte sinh thêm được đánh dấu riêng. Marker parser hiện
có chưa thực hiện JPEG fragment reconstruction.

### [P4] Video carving phải hiểu sample và codec

**Eoghan Casey, Rikkert Zoun (2014), _Design tradeoffs for developing fragmented
video carving tools_. Digital Investigation 11, S30–S39.**
[DOI 10.1016/j.diin.2014.05.010](https://doi.org/10.1016/j.diin.2014.05.010).

Paper bàn kinh nghiệm Defraser/DC3Carver, semantic/codec-aware carving và validation
tradeoffs. Phát hiện container box khác với tìm được sample/frame. Đứt đoạn trong
mdat có thể phá ánh xạ sample tables; một ví dụ repair trong bài vẫn giả định các
sample table gốc còn nguyên. Không có phương pháp tối ưu cho mọi case.

**Cho resc:** kết hợp stsc, stsz/stz2, stco/co64 và stsd để tính toàn bộ sample
extents; thêm timeline stts/ctts/stss, fMP4 tfhd/trun/tfdt và parser NAL/slice cho
codec cụ thể. H.264 length-prefixed trong MP4 không được xử lý như Annex B một
cách mặc định. H.265/AV1 và audio cần triển khai/fixtures riêng.

### [P5] Reference video cần tương thích thực sự

**Thomas Gloe, André Fischer, Matthias Kirchner (2014), _Forensic analysis of
video file formats_. Digital Investigation 11, S68–S76.**
[DOI 10.1016/j.diin.2014.03.009](https://doi.org/10.1016/j.diin.2014.03.009).

Bài khảo sát đặc điểm container/metadata từ 19 model camera, 14 model điện thoại
và 6 bộ công cụ chỉnh sửa. Đây là nghiên cứu authentication/source attribution,
**không phải benchmark tỷ lệ recovery**. Có nhiều thứ tự atom hợp lệ và khác biệt
theo thiết bị/phần mềm; dữ liệu năm 2014 không bao quát mọi thiết bị hiện nay.

**Cho resc:** reference-assisted missing-moov repair cần so codec parameters,
dimensions, sample entries, audio config và capture settings. Untrunc là ứng viên
adapter nguồn mở, chưa được build/đánh giá/tích hợp trong lượt này. Output phải
ghi repaired derivative, không coi header/index dựng lại là byte nguồn gốc.

### [P6] Metadata và carving nên nằm trong một mô hình reconstruction

**Janine Schneider, Hans-Peter Deifel, Stefan Milius, Felix Freiling (2020),
_Unifying Metadata-Based Storage Reconstruction and Carving with LAYR_.
Forensic Science International: Digital Investigation 33, 301006.**
[DOI 10.1016/j.fsidi.2020.301006](https://doi.org/10.1016/j.fsidi.2020.301006).

Paper mô hình hóa storage layers và composition operators để kết hợp metadata-
based reconstruction với carving. Metadata còn dùng được có độ xác định khác
với pattern-based heuristic; hai nguồn bằng chứng có thể bổ sung nhau.

**Cho resc:** nâng byte plan thành reconstruction plan có source mappings, thứ
tự logic, assumptions, validation evidence và derived ranges. Dùng cùng mô hình
cho partition → filesystem → file → container/codec, thay vì mỗi tool xuất một
dạng kết quả khó kiểm lại. Không cần sao chép LAYR để áp dụng nguyên tắc thiết kế.

### [P7] File phân mảnh có thể nằm ngược thứ tự vật lý

**Vincent van der Meer, Hugo Jonker, Jeroen van den Bos, _A Contemporary
Investigation of NTFS File Fragmentation_. Bản tác giả DFRWS APAC 2021.**
[PDF chính thức, bản tác giả](https://github.com/dfrws/papers/blob/112bba7230d82a1a3d446b248ec4dcbe2487817c/2021%20APAC/2021_APAC_paper-a_contemporary_investigation_of_ntfs_file_fragmentation.pdf).

Nghiên cứu corpus **220 laptop Windows**, thu dữ liệu khoảng 10/2018–1/2019.
Discussion ghi out-of-order files chiếm gần một nửa nhóm file đã phân mảnh trong
corpus đó; đa số file toàn corpus lại không phân mảnh. Không được đổi mẫu số thành
“gần nửa mọi file” hoặc áp tỷ lệ đó cho dữ liệu người dùng năm 2026.

**Cho resc:** thuật toán tìm mảnh phải xét ứng viên ở cả trước và sau offset hiện
tại, nhiều gap và nhiều mảnh. Truy hồi theo vị trí tăng dần chỉ xử lý một phần bài
toán. File NTFS compressed/sparse cần biết storage semantics; raw concatenation
không luôn tái tạo logical content. PDF được ghi rõ là author preprint, không có
DOI trong bản đã tải nên không gán một DOI suy đoán.

### [P8] Nhận dạng fragment không có header

**William C. Calhoun, Drue Coles (2008), _Predicting the types of file fragments_.
Digital Investigation 5, S14–S20.**
[DOI 10.1016/j.diin.2008.05.005](https://doi.org/10.1016/j.diin.2008.05.005).

Paper thử Fisher linear discriminant và longest common subsequences trên fragment
không chứa header, dùng các đặc trưng như byte statistics, entropy và correlation.
Kết quả phụ thuộc loại file, mẫu và giả định mô hình; không xác minh một classifier
có thể luôn phân loại mọi dạng nén/mã hóa.

**Cho resc:** xây baseline fragment classifier để giảm không gian candidate trước
search. Học máy có thể xếp hạng nhưng cần unknown/reject, dữ liệu âm và kiểm tra
ngoài phân bố. Tách train/test theo file/nguồn, không để fragment cùng file rò
sang cả hai tập. Đây là đề xuất đánh giá cho dự án, chưa có model được huấn luyện.

### [P9] ZIP/DEFLATE hỏng cần salvage chuyên biệt

**Ralf D. Brown (2013), _Improved recovery and reconstruction of DEFLATEd files_.
Digital Investigation 10, S21–S29.**
[DOI 10.1016/j.diin.2013.06.003](https://doi.org/10.1016/j.diin.2013.06.003).

Paper giới thiệu recovery sau vùng DEFLATE bị hỏng và reconstruction với n-grams,
triển khai trong ZipRec 1.0. Thực nghiệm gồm văn bản Europarl 21 ngôn ngữ và các
khoảng corruption được kiểm soát. Không thể dùng số cải thiện trên văn bản đó làm
tỷ lệ phục hồi ZIP/DOCX/video nói chung.

**Cho resc:** thêm nhánh ZIP/OOXML salvage: tìm local headers/member boundaries,
khôi phục member còn nguyên, kiểm tra CRC khi đủ byte, và giữ unknown regions.
Nếu dùng reconstruction thống kê, byte được suy đoán phải tách khỏi byte thu hồi
trực tiếp; không ghi chúng thành nguồn forensic gốc trong manifest hiện tại.

### [P10] Ext4 recovery cần extents và journal

**Kevin D. Fairbanks (2012), _An analysis of Ext4 for digital forensics_.
Digital Investigation 9, S118–S130.**
[DOI 10.1016/j.diin.2012.05.010](https://doi.org/10.1016/j.diin.2012.05.010).

Extents, extent trees và journal có thể giữ manh mối metadata sau xóa trong các
thử nghiệm của bài, nhưng không bảo đảm dữ liệu còn hoặc chưa bị tái sử dụng.
Kernel/feature set được nghiên cứu không đại diện mọi Ext4 hiện đại.

**Cho resc:** mở rộng filesystem-specific recovery, ưu tiên NTFS deleted MFT/
runlists/allocation checks và Ext4 extent/journal parser có fixture độc lập.
Đề xuất NTFS là bước thiết kế của dự án, không phải kết luận thực nghiệm NTFS
được bài Ext4 chứng minh. FAT32 root scanner hiện có là phạm vi nhỏ trong nhóm này.

### [P11] SSD/TRIM đặt giới hạn lên dữ liệu thực còn trong nguồn

**Christopher King, Timothy Vidas (2011), _Empirical analysis of solid state disk
data retention when used with contemporary operating systems_. Digital
Investigation 8, S111–S117.**
[DOI 10.1016/j.diin.2011.05.013](https://doi.org/10.1016/j.diin.2011.05.013).

144 thử nghiệm trên 15 SSD và một HDD đối chứng cho thấy retention phụ thuộc
thiết bị/OS/TRIM. Không ngoại suy tỷ lệ từ phần cứng năm 2011 sang SSD/NVMe hiện nay.

**Cho resc:** giữ acquisition context và phân biệt metadata candidate, bytes đọc
được và nội dung còn đúng. Không kết luận TRIM chỉ từ một vùng zero; cũng không
hứa dùng AI khôi phục chính xác bytes đã không còn trong ảnh.

### [P12] Acquisition hỏng có thể làm mất cả sector vẫn đọc được

**James R. Lyle, Mark Wozar (2007), _Issues with imaging drives containing faulty
sectors_. Digital Investigation 4S, S13–S15.**
[DOI 10.1016/j.diin.2007.06.002](https://doi.org/10.1016/j.diin.2007.06.002).

Trong một số cấu hình thử nghiệm, tool bỏ sót readable sectors cạnh faulty sector.
Không được dùng kết quả tool/interface năm 2007 để cáo buộc công cụ hiện nay.

**Cho resc:** fault-injection benchmark cho đọc block lớn, fallback từng sector,
mapfile và readable neighbors; adapter GNU ddrescue có resume/multi-pass là một
hướng riêng. Native clone hiện tại chưa có các thuật toán đó.

### [P13] Corpus và ground truth quyết định giá trị của kết quả

**Simson Garfinkel, Paul Farrell, Vassil Roussev, George Dinolt (2009), _Bringing
science to digital forensics with standardized forensic corpora_. Digital
Investigation 6, S2–S11.**
[DOI 10.1016/j.diin.2009.06.016](https://doi.org/10.1016/j.diin.2009.06.016).

Test/sampled/realistic/real data có vai trò và bias khác nhau. Corpus dùng chung,
ground truth và cách đo rõ giúp kiểm chứng và so sánh được.

**Cho resc:** giữ unit fixtures nhưng thêm DFRWS/ground-truth corpus, nhiều nguồn
thiết bị và negative cases. Trong lượt này mới đọc mô tả DFRWS 2006; **chưa tải
hay chạy benchmark dataset đó**.

### [P14] Không có một “error rate” chung tự suy ra từ số test pass

**James R. Lyle (2010), _If error rate is such a simple concept, why don't I have
one for my forensic tool yet?_. Digital Investigation 7, S135–S139.**
[DOI 10.1016/j.diin.2010.05.017](https://doi.org/10.1016/j.diin.2010.05.017).

Paper phân biệt lỗi ngẫu nhiên với implementation errors có tính systematic,
kích hoạt bởi điều kiện cụ thể. Nó thảo luận vấn đề xây error rates, không đưa ra
một tỷ lệ chuẩn dùng được cho mọi forensic tool.

**Cho resc:** báo metric theo thao tác, format và corruption class; không lấy
74 tests pass để gán “accuracy 99%” hay đổi heuristic score thành xác suất.

## 3. Lộ trình có thể kiểm chứng, thay cho thêm nhiều tool tên mới

Các mục sau là **đề xuất chưa triển khai trong lượt nghiên cứu**. Thứ tự này là
đánh giá kỹ thuật cho resc, không phải xếp hạng do một paper công bố.

| Ưu tiên | Nhánh triển khai | Tiêu chí nghiệm thu |
| --- | --- | --- |
| P0 | Corpus độc lập và benchmark runner | Original hashes/extents; đo false joins, bytes đúng, partials, runtime/RAM; không leakage |
| P1 | NTFS deleted metadata và logical extents | MFT fixups/runlists/bounds/allocation conflicts; phân biệt compressed/sparse/unsupported |
| P1 | MP4 sample map/timeline/fMP4 | Tính mọi sample extent và offsets, không chỉ chunk-start; kiểm tra bằng demuxer độc lập |
| P1 | Bifragment search có budget | Tự tìm gap/đảo thứ tự trên corpus chưa biết trước plan; cancel/quota/ambiguity đầy đủ |
| P2 | Candidate graph nhiều mảnh | Không reuse extents; nhiều giả thuyết; format/codec evidence; benchmark ghép nhầm |
| P2 | Reference-assisted video/JPEG repair | So donor compatibility; output derivative; byte tạo lại có provenance riêng |
| P2 | ZIP/OOXML/DEFLATE partial salvage | Member/CRC/unknown-region accounting; không lẫn bytes suy đoán với bytes gốc |
| P2 | Ext4/journal và acquisition adapters | Filesystem-specific fixtures, readable-neighbor tests, mapfile/resume thật |
| P3 | Headerless fragment classification/ML | Open-set reject, calibrated evaluation trên file/thiết bị tách biệt |

### Mô hình bằng chứng chung cần giữ

Mỗi candidate nên có: source extents, thứ tự logic, assumptions, format/codec
state, validation evidence, competing candidates, missing ranges, derived ranges,
tool version và budget outcomes. AI agent điều phối giả thuyết và giải thích
bằng chứng; nó không được tự tạo byte thiếu rồi công bố là dữ liệu gốc.

Phân biệt ít nhất các kết quả sau:

1. Tìm thấy chữ ký/header.
2. Có metadata hoặc candidate extent plan.
3. Cấu trúc container phù hợp.
4. Decoder đọc được một phần/toàn phần trong budget.
5. Bytes khớp ground truth khi có bản gốc.
6. Người dùng xác nhận đúng nội dung cần lấy.

Không dùng một cờ `recovered=true` thay cho sáu mức bằng chứng này.

## 4. Tài liệu đối chiếu và provenance nguồn

- [Carving/fragmentation](literature-carving.md): P1–P3, sections đã đọc,
  checksums PDF và chi tiết giới hạn thuật toán.
- [Video](literature-video.md): P4–P5; Untrunc/FFmpeg docs được phân loại riêng,
  không gọi README là bài báo khoa học.
- [Filesystem/SSD/acquisition/corpus](literature-filesystem.md): P10–P13,
  thiết kế thực nghiệm, giới hạn lịch sử và dataset chưa chạy.

P6–P9/P14 được root tải từ archive tại commit
`112bba7230d82a1a3d446b248ec4dcbe2487817c`, chuyển bằng pdftotext và đọc các phần
liên quan. PDF nằm ngoài checkout, không nhúng lại vào mã nguồn.

| Nguồn | SHA-256 PDF |
| --- | --- |
| Calhoun/Coles 2008 | `8aac3ff37636ae6ede22a20aaff3f1587b5ba095537ef643eaa4456a38b2cfb1` |
| Lyle 2010 | `8d5c58215c9e49baf2316947c03f1eca5d5d3b85503e3b080dd8eb901e3cacac` |
| Brown 2013 | `036de484e874f6db6e80c621c8504f8238254ad383c699ca28d5a9b94ef51abc` |
| Schneider et al. / LAYR 2020 | `24b7bbaf4b26bbfbb4bb0e796151e0f10c839cfb0ebed350826bc8238dbf793d` |
| van der Meer et al., bản tác giả APAC 2021 | `afd684047aace78f65145276f8669b269098b17b5604fce4972e0973e7109e06` |

Các định danh DOI phía trên đối chiếu từ chính PDF. Không build/run ZipRec, LAYR,
Untrunc hoặc benchmark ngoài trong lượt đọc tài liệu này. Các đề xuất chưa trở
thành tính năng chỉ vì đã được nghiên cứu. Thí nghiệm đối chứng tổng hợp chạy sau
lượt đọc được ghi riêng tại
[`deep-evidence-study.vi.md`](deep-evidence-study.vi.md); nó không thay thế corpus
độc lập hoặc biến các đề xuất roadmap thành tính năng đã hoàn thành.
