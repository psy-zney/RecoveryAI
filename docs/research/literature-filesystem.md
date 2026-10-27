# Nghiên cứu: filesystem, dữ liệu xóa, SSD/TRIM và kiểm chứng acquisition

Ngày kiểm tra nguồn: 2026-10-26. Đây là ghi chú nghiên cứu, không thêm thuật toán
hay thay đổi application code. Bốn công trình dưới đây được đọc từ **full text
PDF trong kho GitHub chính thức của DFRWS**, gồm abstract, phương pháp, kết quả
và phần giới hạn/kết luận liên quan. DOI được xác minh ngay trong PDF, không suy
đoán từ tên bài hoặc từ một công cụ AI.

## 1. SSD và TRIM: còn metadata không có nghĩa còn bytes nguồn

**Christopher King và Timothy Vidas (2011).** *Empirical analysis of solid state
disk data retention when used with contemporary operating systems.*
Digital Investigation 8, S111–S117. DOI:
[10.1016/j.diin.2011.05.013](https://doi.org/10.1016/j.diin.2011.05.013).
[Full text chính thức DFRWS](https://github.com/dfrws/papers/blob/main/2011%20USA/2011_USA_paper-empirical_analysis_of_solid_state_disk_data_retention_when_used_with_contemporary_operating_systems.pdf).

- **Thiết kế:** 15 SSD từ 10 hãng, một HDD đối chứng, tổng 144 phép thử với hệ
  điều hành và các tình huống sử dụng/xóa/format của thời kỳ nghiên cứu.
- **Kết luận của bài:** khả năng thu lại block sau deletion khác mạnh theo
  controller, OS và việc TRIM có hiệu lực. Các cấu hình TRIM trong thử nghiệm
  làm giảm rõ khả năng lấy lại dữ liệu so với nhiều cấu hình không TRIM.
- **Giới hạn:** kết quả trên phần cứng, firmware và OS khoảng năm 2011 không phải
  xác suất phục hồi của SSD/NVMe năm 2026. Không suy ra mọi SSD đã TRIM đều trả
  zero giống nhau, không dùng một tỷ lệ trong bài làm tỷ lệ thành công của resc.
- **Áp dụng resc:** một deleted directory entry hoặc extent hợp lý chỉ là bằng
  chứng metadata. Phải tách `metadata candidate`, bytes thực đọc được, cấu trúc
  định dạng và content validation. Khi nguồn ảnh trả toàn zero, chưa được tự kết
  luận riêng từ mẫu đó rằng nguyên nhân chắc chắn là TRIM. Cần ghi acquisition,
  thời điểm thu ảnh, thiết bị/controller/OS nếu biết; không có thuật toán carving
  nào tái tạo được bytes đã không còn trong nguồn chỉ bằng signature cũ.

## 2. Deleted recovery phải hiểu cấu trúc từng filesystem

**Kevin D. Fairbanks (2012).** *An analysis of Ext4 for digital forensics.*
Digital Investigation 9, S118–S130. DOI:
[10.1016/j.diin.2012.05.010](https://doi.org/10.1016/j.diin.2012.05.010).
[Full text chính thức DFRWS](https://github.com/dfrws/papers/blob/main/2012%20USA/2012_USA_paper-an_analysis_of_ext4_for_digital_forensics.pdf).

- **Nội dung:** phân tích extents, extent trees, HTrees, flex block groups và
  journal của Ext4; thử nghiệm image với Linux kernel 2.6.32-38.
- **Kết luận liên quan:** trong các thử nghiệm, metadata extent trong inode bị
  xóa khác nhau theo kích thước/phân mảnh và việc phải tạo extent tree. Các extent
  index node ngoài inode không nhất thiết bị zero ngay; nếu chưa tái sử dụng,
  chúng có thể giữ manh mối tới dữ liệu. Journal còn có thể chứa bản metadata cũ,
  nhưng đây là nguồn hữu ích **có giới hạn**, không phải bản sao bảo đảm của file.
- **Giới hạn:** một nghiên cứu Ext4/phiên bản kernel cụ thể không xác minh hành
  vi mọi feature hiện đại như metadata checksums, encryption hoặc mọi mode
  journaling; cũng không chứng minh quy tắc xóa của NTFS/FAT.
- **Áp dụng resc:** bước tiếp theo đáng làm là parser metadata/extent/journal
  theo từng filesystem, với bounds, allocation-state và sequence/checksum
  validation, trước khi suy đoán thứ tự mảnh bằng entropy. Hỗ trợ Ext4 thực sự
  phải có fixtures riêng và kiểm tra independent extents; không đổi nhãn
  `NotImplemented` thành `supported` chỉ vì có chữ ký superblock hoặc bài báo.

## 3. Acquisition có bad sectors: thử xem có mất cả vùng đọc được hay không

**James R. Lyle và Mark Wozar (2007), NIST.** *Issues with imaging drives
containing faulty sectors.* Digital Investigation 4S, S13–S15. DOI:
[10.1016/j.diin.2007.06.002](https://doi.org/10.1016/j.diin.2007.06.002).
[Full text chính thức DFRWS](https://github.com/dfrws/papers/blob/main/2007%20USA/2007_USA_paper-issues_with_imaging_drives_containing_faulty_sectors.pdf).

- **Thiết kế:** ba ổ ATA có tập bad sector ổn định, cùng ổ đối chứng có nội dung
  tương ứng; so hành vi một số imaging tools và đường giao tiếp. Nghiên cứu nằm
  trong hoạt động phát triển phương pháp kiểm thử CFTT của NIST.
- **Kết quả:** một số công cụ/cấu hình bỏ sót cả readable sectors nằm cạnh bad
  sector; trong cấu hình dd-based Linux/ATA được thử, có thể mất tới bảy sector
  đọc được bên cạnh một sector hỏng. Giao tiếp/write blocker cũng ảnh hưởng
  hành vi. Chỉ có file image hoàn thành không đủ để kết luận đã lấy đủ mọi vùng
  có thể đọc được.
- **Giới hạn:** bài chủ động ghi nhận chỉ thử một tập tool/interface giới hạn từ
  năm 2007; không phải kết quả đánh giá dd, GNU ddrescue hay phần cứng hiện nay.
  Không phải chứng nhận rằng resc đã đáp ứng NIST/CFTT.
- **Áp dụng resc:** mở rộng fault-injection benchmark ở cả đọc theo block lớn và
  fallback từng sector, khẳng định readable neighbors vẫn được lấy đúng, mapfile
  phân biệt chính xác vùng lỗi với zero-fill. Báo cáo `partial` phải ghi rõ các
  vùng thiếu; output hash chỉ kiểm chứng bytes đã xuất, không làm dữ liệu thiếu
  trở thành acquisition đầy đủ. Tiếp tục phân biệt native block copier với thuật
  toán multi-pass/resume của GNU ddrescue.

## 4. Benchmark phải có ground truth, không chỉ những ảnh do chính mình dựng

**Simson Garfinkel, Paul Farrell, Vassil Roussev và George Dinolt (2009).**
*Bringing science to digital forensics with standardized forensic corpora.*
Digital Investigation 6, S2–S11. DOI:
[10.1016/j.diin.2009.06.016](https://doi.org/10.1016/j.diin.2009.06.016).
[Full text chính thức DFRWS](https://github.com/dfrws/papers/blob/main/2009%20USA/2009_USA_paper-bringing_science_to_digital_forensics_with_standardized_forensic_corpora.pdf).

- **Luận điểm:** corpus dùng chung, metadata rõ và ground truth giúp kết quả
  kiểm chứng/tái lập được giữa các nhóm nghiên cứu và so sánh bằng metric khách
  quan. Bài phân biệt test data, sampled data, realistic data và real data cùng
  các vấn đề bias, quyền riêng tư và phân phối.
- **Giới hạn:** corpus không tự trở thành đại diện mọi tình huống. Fixture đơn
  giản hữu ích cho unit test nhưng không đo được chất lượng ngoài phân bố của
  chính fixture; dữ liệu thực còn có hạn chế riêng tư/bản quyền.
- **Áp dụng resc:** giữ regression fixtures hiện có, thêm benchmark độc lập với
  ground-truth file hashes và extents. Báo riêng phát hiện, khôi phục đúng bytes,
  false positives, partials, thời gian, peak RAM và dung lượng hit spool. Không
  biến số lượng tests pass thành phần trăm phục hồi thực tế hoặc gộp skipped
  test với pass. Chưa chạy corpus ngoài thì phải ghi `unrun`.

## Dataset tham chiếu bổ sung đã đọc mô tả, chưa chạy benchmark

[DFRWS 2006 Challenge — kho chính thức](https://github.com/dfrws/dfrws2006-challenge)
và [ground-truth layout](https://github.com/dfrws/dfrws2006-challenge/blob/main/file-image-layout.md).
Đây là tài liệu dataset, không phải công trình thứ năm trong danh sách trên.

Mô tả xác nhận ảnh raw 50 MB không có filesystem, chứa JPEG, ZIP, HTML, text,
Office cùng fragments. Layout nêu 32 file không tính embedded files, 22 tình
huống. Các submission phải ghi file type/size/hash và sector nguồn. Ban tổ chức
cũng chỉ ra nguy cơ benchmark bị ảnh hưởng khi dùng public-domain source files
đã biết trước. Điều đó hỗ trợ việc công bố ground truth và hạn chế data leakage.

README trỏ ảnh dataset tới Dropbox; **chưa tải và chưa chạy ảnh đó**, không đổi
network allowlist và không tuyên bố đã benchmark resc trên DFRWS. Không lấy số
512-byte sector của dataset này làm geometry mặc định của mọi acquisition.

## Giới hạn truy cập và kiểm chứng nguồn

- Truy cập thành công: `github.com/dfrws/...` và full text PDF qua
  `raw.githubusercontent.com/dfrws/papers/...` trong policy hiện có.
- NIST SP 800-86 tại `nvlpubs.nist.gov/.../nistspecialpublication800-86.pdf` trả
  HTTP 403 trong phiên này; không có bản full text từ endpoint đó để xác minh.
  Không suy đoán nguyên nhân chi tiết chỉ từ HTTP status, không bypass proxy,
  không đổi network draft. DOI links ở trên dùng làm định danh từ PDF, không có
  claim đã resolve publisher/Crossref endpoints.
- Không thêm executable, dependency, test hoặc thuật toán dựa trên các paper
  trong lượt nghiên cứu này. Các bước áp dụng là đề xuất phát triển tiếp.

SHA-256 bốn PDF thực sự đã đọc, để đối chiếu bản nguồn:

| PDF | SHA-256 |
| --- | --- |
| King/Vidas 2011 | `ebed8c4a72ad91df0e4eeafe62b4ce9895ecf384f8d200823b3d1e6270ab34a8` |
| Fairbanks 2012 | `6cacf29e2a63227ad85f28f8069908d26cf92073f3d45cd259d54d6ccc84e5cb` |
| Lyle/Wozar 2007 | `3d8490450606a1a2f15a035cdb68d4baabd6a3a2afdbe490445616c6cda9681a` |
| Garfinkel et al. 2009 | `1a6adc7d768df83740db2907e5b08581bc63a5b7e51d2b9789d9fbab1b8f9bdd` |
