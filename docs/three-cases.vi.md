# Ba nhóm recovery và cách phối hợp

Ba nhóm dưới đây là cách tổ chức các tình huống phổ biến, không phải thống kê
khẳng định mọi ca mất dữ liệu đều thuộc một nhóm duy nhất. Một ảnh có thể đồng
thời mất boot, chứa file đã xóa và có dữ liệu phân mảnh. Dự án vì vậy kết hợp các
route trên cùng session/overlay thay vì yêu cầu người dùng chọn đúng lỗi từ đầu.

| Nhóm | Agent nghiên cứu độc lập | Phần triển khai |
| --- | --- | --- |
| 1. Mất phân vùng hoặc hỏng boot | partition_boot_case | Đối chiếu GPT và backup NTFS/FAT32, đề xuất repair trong overlay với địa chỉ/CRC đúng |
| 2. Xóa nhầm hoặc mất metadata file | deleted_metadata_case | Quét deleted SFN ở root FAT32, đối chiếu FAT/allocation/live references, xuất candidate extent có điều kiện |
| 3. File phân mảnh hoặc hỏng nội dung | fragment_content_case | Kiểm tra JPEG marker/PNG chunk CRC, tìm chiều dài ứng viên theo terminator, phối hợp MP4/MOV analyzer |

Mỗi agent sở hữu module và báo cáo nghiên cứu riêng. Phần tổng hợp thống nhất
session, geometry, ordered byte ranges, callback hit JSONL, report budget, MCP,
CLI, provenance và kiểm thử tích hợp. Không tạo ba công cụ rời rạc có ba quy tắc
ghi dữ liệu khác nhau.

## Bắt đầu mà không chọn trước một nhóm

```bash
python scripts/recovery.py triage /mnt/evidence/disk.img --report /mnt/results/triage.json
python scripts/recovery.py triage /mnt/evidence/disk.img --deep --report /mnt/results/deep.json
```

Triage trả `diagnosis.cases` gồm cả ba nhóm và phân tích backup/partition riêng.
Deep scan chạy signature, VBR và video-box routes. Với **toàn ảnh** và FAT32 được
nhận diện, nó thêm deleted-entry scan ở root các volume đã lấy mẫu. Nếu dùng
`--start-lba`/`--end-lba`, deep scan không tự đọc root FAT32 ngoài phạm vi đó;
dùng `deleted-scan` riêng sau khi xác định volume.

Mỗi scan có trạng thái, job ID, trang mẫu và file hit đầy đủ. Một scan thất bại
được giữ trong báo cáo để các route độc lập tiếp tục; exit code 2 báo kết quả
chưa đầy đủ. Timeout chỉ xác lập yêu cầu hủy, không phải xác nhận job đã hoàn tất.
Không có bước tự vá boot hay tự ghép mảnh trên dữ liệu thật.

## Nhóm 1: kiểm tra đề xuất, chỉ áp dụng vào overlay khi có bằng chứng

```bash
python scripts/recovery.py analyze-partition /mnt/evidence/disk.img
python scripts/recovery.py analyze-partition /mnt/evidence/disk.img --partition-start-lba 2048
```

GPT backup không thể copy nguyên header sang vị trí primary: `my_lba`, alternate
header, địa chỉ entry array và CRC cần đúng cho vị trí mới. Module trả riêng kế
hoạch copy array và patch header. Đề xuất không đồng nghĩa đã thực hiện.

Boot backup khi primary hỏng vẫn có thể là cấu trúc cũ hoặc thuộc volume khác.
Đọc các trường yêu cầu xác nhận identity/boundary; không lấy vị trí 6 hoặc cuối
ảnh làm chân lý. Sau repair overlay, kiểm tra lại topology và native root.
Không ghi sửa lên ảnh hay ổ gốc.

## Nhóm 2: phân biệt entry xóa với bytes thực sự còn phục hồi được

```bash
python scripts/recovery.py deleted-scan /mnt/evidence/disk.img --partition-start-lba 2048 --max-clusters 256 --report /mnt/results/deleted.json
```

`FAT32_DELETED_SCAN` là background job có cancel, limits và persisted hits. Nó
chỉ xét deleted short-name entries ở **root FAT32**; không dựng tên LFN, duyệt mọi
thư mục con, NTFS deleted records hay journal. Ký tự đầu của tên xóa không còn
được xác định chắc chắn.

Plan hiện chỉ được đưa ra cho file vừa trong một cluster, cluster đang FAT-free,
không xung đột live root metadata và không bị các kiểm tra/quota làm scan thiếu.
Cluster đã tái cấp phát, FAT mirrors bất đồng, chain nhiều cluster mất hoặc root
scan chưa đầy đủ không được đoán contiguous để xuất. FAT-free vẫn không chứng
minh bytes chưa bị ghi đè; `content_verified` ban đầu luôn false.

Lấy `candidate_export_plan` từ hit JSONL, kiểm tra nguồn và nội dung bằng nhóm 3,
rồi dùng cùng plan với `recover`. Mọi output vẫn có manifest và không ghi đè.

## Nhóm 3: kiểm tra nội dung trước khi công bố thành công

```bash
python scripts/recovery.py analyze-file /mnt/evidence/disk.img --offset 1048576 --length 10485760 --format AUTO
python scripts/recovery.py analyze-file /mnt/evidence/disk.img --plan /mnt/results/plan.json --format PNG
```

AUTO chỉ nhận dạng các định dạng được hỗ trợ: JPEG, PNG, MP4/MOV. Không tự chọn
định dạng gần giống để tạo kết quả pass. JPEG kiểm tra marker/segment/stuffing và
EOI; PNG kiểm tra cấu trúc chunk/CRC và IEND. Khi có terminator hợp lệ,
`candidate_length_bytes` chỉ rõ bytes ứng viên trong view và `trailing_bytes`
phân biệt bytes ngoài file; đừng tự export toàn bộ khoảng quét.

JPEG/PNG có budget 64 MiB và giới hạn số marker/chunk. Report phân biệt cấu trúc
sai, truncated, vượt budget và feature/format chưa hỗ trợ. Kiểm tra cấu trúc không
giải mã pixel và không chứng minh các mảnh thuộc cùng ảnh. MP4/MOV giữ giới hạn
riêng của ISO BMFF analyzer và cần ffprobe/FFmpeg để kiểm tra decode output.

Nếu plan gồm nhiều ranges và terminator nằm trước cuối view, rút ngắn plan theo
**thứ tự byte output** trước khi export; không cộng chiều dài vào source offset
đầu tiên qua các khoảng trống vật lý. Không tự suy ra thứ tự mảnh chưa biết.

## Chuỗi tích hợp cả ba nhóm

Một session có thể đi qua:

1. `analyze_partition_recovery` → đối chiếu backup và xác nhận đề xuất.
2. `virtual_patch_and_verify` → chỉ thay đổi overlay/WAL.
3. `FAT32_DELETED_SCAN` → đọc filesystem qua overlay đã kiểm tra.
4. `read_scan_hits` → lấy candidate plan có giới hạn bằng chứng.
5. `analyze_file_candidate` → kiểm tra cấu trúc trên cùng ordered byte view.
6. `CARVE_BY_BYTE_RANGES` → xuất đúng bytes, manifest, hash; xác minh output độc lập.

`tests/test_three_cases.py` thực hiện chuỗi này trên fixture tổng hợp, cùng kiểm
thử tái tạo primary GPT từ backup hoàn toàn trong overlay. Đây không phải bằng
chứng phục hồi được mọi ổ/ảnh hỏng thực tế.

## Nghiên cứu và giới hạn

Ba nhóm này còn nhiều hướng sâu hơn. Xem [tổng hợp 14 công trình chuyên ngành và
lộ trình nghiên cứu](research/literature-review-and-roadmap.vi.md), phân biệt rõ
khả năng hiện có với những thuật toán mới được đề xuất, chưa triển khai.

Đọc [case 1](research/case-1-partition-boot.md),
[case 2](research/case-2-deleted-metadata.md) và
[case 3](research/case-3-fragments-content.md) để xem nguồn, suy luận và phạm vi
triển khai của từng agent. [Hướng dẫn thao tác](recovery-guide.vi.md) mô tả cài đặt,
chọn nguồn/đích và xác minh. Những khả năng như RAID, giải mã không có khóa, NTFS
undelete, exFAT/EXT4 recovery, tái tạo moov mất hoàn toàn và tự suy ra mọi thứ tự
mảnh vẫn cần engine hoặc bằng chứng khác.
