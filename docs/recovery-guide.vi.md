# Hướng dẫn cài đặt, chọn nguồn và phục hồi dữ liệu

`resc` làm việc trên **ảnh đĩa raw, chỉ đọc** và xuất dữ liệu vào một thư mục khác.
Bạn không cần chọn trước một giả thuyết lỗi: bắt đầu bằng triage, sau đó kết hợp
metadata filesystem, quét chữ ký và phân tích từng ứng viên. Không có thuật toán
nào bảo đảm phục hồi mọi trường hợp; dữ liệu đã bị ghi đè hoặc TRIM có thể không còn.

Xem thêm [ba nhóm recovery tích hợp](three-cases.vi.md): lost partition/boot,
FAT32 deleted metadata và file-content validation. Các nhóm có thể cùng xuất hiện
trên một ảnh; không cần chọn một giả thuyết duy nhất.

## 1. Cài đặt và kiểm tra

Cần Rust **stable hiện hành**, trình liên kết C, Node.js >=18, Python >=3.11.
Python không cần pip; Node launcher không cần npm install. Cargo dùng `Cargo.lock`.
Một số tài liệu cũ ghi Rust 1.80 và panic=abort: không dùng các chỉ dẫn đó.
Engine hiện dùng API Rust stable mới hơn và bắt buộc `panic=unwind`.

Trên cloud đã chuẩn bị trong phiên này:

```bash
source /workspace/.setup/resc/env.sh
cd /workspace/resc
bash scripts/setup.sh
python scripts/recovery.py doctor
```

Trên Debian/Ubuntu riêng, cài `build-essential`, `curl`, `ca-certificates`,
`python3`, Node.js và Rust stable từ nhà cung cấp chính thức. Dùng rustup theo
https://rustup.rs/; giữ xác minh TLS/checksum. Nếu hệ thống chỉ có `python3`, thay
`python` bằng `python3` trong các lệnh dưới đây. Chạy build trực tiếp:

```bash
rustup toolchain install stable --component clippy --component rustfmt
cargo +stable build --release --workspace --locked
python3 scripts/recovery.py doctor
```

Trên Windows PowerShell, cài Rust stable MSVC cùng Visual Studio Build Tools C++,
Node.js và Python >=3.11. Tại thư mục dự án:

```powershell
cargo build --release --workspace --locked
python scripts/recovery.py doctor
$env:RESC_SESSION_ROOT = "D:\resc-state"
node bin/resc-mcp.js --version
```

FFmpeg là thành phần **tùy chọn** để kiểm tra MP4/MOV đã xuất: cài gói `ffmpeg`
từ package manager chính thức, kiểm tra `ffprobe -version` và `ffmpeg -version`.
Cloud hiện có FFmpeg 7.1.5. Đây là tiến trình ngoài, không đóng gói lại mã FFmpeg.
Xem [nguồn, giấy phép và giới hạn](open-source-engines.md).

Kiểm thử phát triển:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --workspace --locked
python tests/test_e2e_recovery.py
python tests/test_recovery_workbench.py
python tests/test_deleted_recovery.py
python tests/test_three_cases.py
```

Test video thật chạy khi có cả ffmpeg và ffprobe; nếu thiếu sẽ hiện **skipped**.
Các ảnh thử nghiệm là tổng hợp, không phải dữ liệu cá nhân của bạn.

## 2. Chọn đúng file nguồn

| Bạn đang có gì? | Chọn gì / làm gì |
| --- | --- |
| Ảnh toàn ổ `.img`, `.dd`, `.raw` | Chọn chính ảnh đó; giữ bản nguồn và hash độc lập |
| Ảnh một phân vùng | Vẫn mở được; VBR thường ở LBA 0, không mặc định 2048 |
| Video `.mp4`/`.mov` còn mở được một phần | Dùng `verify-media`; không coi video là ảnh đĩa |
| File `.E01`, VHDX, QCOW2, VMDK | Chuyển thành raw bằng công cụ hiểu container trước; đổi đuôi không phải chuyển đổi |
| Ảnh chia nhiều đoạn `.001`, `.002`… | Hợp nhất bằng công cụ acquisition phù hợp trước; chỉ chọn `.001` không bảo đảm đủ nguồn |
| USB/HDD/SSD thật | Tạo ảnh acquisition bằng công cụ tin cậy trước; CLI này chỉ nhận file thường |
| File bị xóa trên ổ đang dùng | Ngừng ghi vào nguồn; làm việc trên ảnh, không cài công cụ lên chính ổ cần cứu |

Lưu kết quả lên **ổ đích khác** với ổ vật lý chứa chứng cứ khi có thể. Engine chặn
ghi đè file nguồn/file đích có sẵn, nhưng không chứng minh hai đường dẫn trên hai
ổ vật lý độc lập. Không chạy công cụ sửa filesystem trực tiếp lên nguồn.

Mở bằng đường dẫn đầy đủ, luôn đặt trong dấu nháy nếu có khoảng trắng:

```bash
python scripts/recovery.py triage "/mnt/evidence/USB image.img" --report /mnt/results/triage.json
```

Hoặc chọn theo danh sách số, không cần GUI:

```bash
python scripts/recovery.py triage --choose /mnt/evidence
```

Danh sách chỉ gồm file ngay trong thư mục, không tự duyệt thư mục con. Phần mở
rộng không xác nhận định dạng. Ảnh phải khác rỗng và có độ dài chia hết cho
`--sector-size 512` hoặc `4096`; lấy geometry từ quá trình acquisition, không đoán
và không thêm padding vào ảnh gốc. File output/report phải có tên mới.

## 3. Phân tích tổng quát trước, quét sâu khi cần

```bash
python scripts/recovery.py triage /mnt/evidence/disk.img --report /mnt/results/triage.json
python scripts/recovery.py triage /mnt/evidence/disk.img --deep --job-timeout 14400 --report /mnt/results/deep.json
```

Triage nhanh kiểm tra MBR/GPT và các boot sector được lấy mẫu: LBA 0, tối đa 8
phân vùng và vị trí phổ biến 2048. Kết quả không thay thế quét toàn ảnh.
`--deep` thực hiện ba lượt discovery cơ bản: chữ ký mọi loại file hỗ trợ, boot sector NTFS/FAT32
thất lạc và box MP4/MOV. Ảnh lớn có thể mất nhiều giờ, cần dung lượng lưu hit JSONL.
Không tự vá boot, không tự đoán thứ tự mảnh và không tự xuất mọi hit. Khi quét toàn
ảnh và nhận diện FAT32, deep scan thêm deleted-entry scan có budget ở root volume
đã lấy mẫu; xem hướng dẫn ba nhóm cho giới hạn. Thất bại từng route được giữ trong
JSON, các route độc lập tiếp tục và exit code 2 báo phần chưa hoàn tất.

Để giới hạn phạm vi hoặc chạy từng lượt:

```bash
python scripts/recovery.py scan /mnt/evidence/disk.img --kind SIMD_MAGIC_CARVE
python scripts/recovery.py scan /mnt/evidence/disk.img --kind SIMD_VBR_HUNT --start-lba 0 --end-lba 100000
python scripts/recovery.py scan /mnt/evidence/disk.img --kind ISO_BMFF_BOX_SCAN
```

`end-lba` không bao gồm sector cuối. Mọi LBA tính theo sector-size của session;
byte offset luôn tính theo byte. Báo cáo chứa tổng số hit, một trang mẫu và đường
dẫn `all_hits_file` chứa đầy đủ các dòng JSONL, không chỉ vài hit đầu. Với agent
từ xa, lấy trang bằng `read_scan_hits`, không yêu cầu toàn file trong một response.

| Bằng chứng | Hướng tiếp theo | Không được kết luận |
| --- | --- | --- |
| GPT CRC/bounds hợp lệ, NTFS/FAT boot hợp lệ | Đọc metadata, ưu tiên native tree extraction | Mọi file bên trong còn nguyên |
| Mất bảng phân vùng, tìm thấy VBR ứng viên | Kiểm tra geometry/backup và ranh giới phân vùng | VBR signature là volume mount được |
| Boot chính hỏng, backup phù hợp | Thử copy trong overlay bằng MCP, kiểm tra root index | Đã sửa ổ vật lý |
| Tìm thấy chữ ký JPEG/PDF/ZIP/video… | Xác định chiều dài, extents, kiểm tra định dạng | Đã phục hồi một file hoàn chỉnh |
| exFAT/BitLocker marker | Dùng engine/decryption phù hợp với quyền và khóa sẵn có | resc đã giải mã hoặc phục hồi filesystem đó |
| Entropy cao hoặc toàn zero tại mẫu | Ghi nhận và kiểm tra thêm acquisition/mapfile | Chắc chắn mã hóa, TRIM, ghi đè hoặc không cứu được |
| I/O lỗi | Giữ chẩn đoán và mapfile, ưu tiên acquisition | Chắc chắn lỗi phần cứng thay vì ảnh/quyền/môi trường |

Nếu NTFS/FAT32 còn đọc được, xuất file allocated theo metadata trước; ví dụ
phân vùng bắt đầu ở LBA đã xác minh (không mặc định mọi ảnh là 2048):

```bash
python scripts/recovery.py extract-tree /mnt/evidence/disk.img --partition-start-lba 2048 --output-dir /mnt/recovered/tree-001
```

Lệnh này xuất cây file được hỗ trợ, không có lựa chọn riêng từng tên file và không
khôi phục deleted records. Chọn file cần giữ từ thư mục kết quả sau khi kiểm tra.
Lỗi/quotas có thể để lại các file hoàn tất trước đó; xem kết quả và manifest.

Nếu cần bản sao từ **ảnh file**, native clone có mapfile nhưng không có resume hay
các retry passes của GNU ddrescue:

```bash
python scripts/recovery.py clone /mnt/evidence/disk.img --destination /mnt/recovered/clone.img --mapfile /mnt/recovered/clone.map
```

`partial` nghĩa là có vùng không đọc được và đã điền zero; không coi là bản sao
nguyên vẹn. Hai file đích phải mới; giữ mapfile và `.part` khi có lỗi.

## 4. MP4/MOV: phân biệt mất moov, thiếu media và phân mảnh

Video thường gồm `ftyp` (nhận diện), `moov` (metadata/index) và `mdat` (media).
QuickTime cũ có thể không có `ftyp`; fragmented MP4 dùng `moof`/`traf` và cần
demuxer hiểu fragment. Một file có thể chứa nhiều `mdat` hoặc nằm ở cuối `moov`.
Box tìm được ở gần nhau trên đĩa chưa chắc thuộc cùng video.

Quét `ISO_BMFF_BOX_SCAN` cho các **ứng viên header** `ftyp/moov/mdat/moof`, kiểm tra
size 32/64-bit và báo `declared_length_bytes`, `fits_scan_range`. Size 0 phụ thuộc
ranh giới file chưa biết, nên không tự gán phần còn lại của đĩa làm chiều dài video.
Trong payload nén vẫn có thể xuất hiện header giả.

Khi có byte offset và chiều dài dựa trên metadata hoặc bằng chứng khác:

```bash
python scripts/recovery.py analyze-video /mnt/evidence/disk.img --offset 1048576 --length 7340032
```

Nếu file bị chia mảnh, viết plan JSON theo **thứ tự byte của file cần dựng**, không
phải thứ tự vị trí trên đĩa. Ví dụ minh họa, không dùng các con số này cho ảnh thật:

```json
{
  "ranges": [
    {"source_byte_offset": 2000013, "length_bytes": 35001},
    {"source_byte_offset": 700019, "length_bytes": 42007}
  ]
}
```

Có thể thêm `source_sha256` để CLI chặn nếu chọn nhầm nguồn; hash sẽ được tính
toàn bộ ảnh và có thể mất thời gian. Plan chỉ mô tả extents, không chứng minh mảnh
thuộc cùng file. Giới hạn 4096 ranges, không chồng lấn, không vượt dung lượng.
Không bắt buộc offset/length chia hết cho sector.

```bash
python scripts/recovery.py analyze-video /mnt/evidence/disk.img --plan /mnt/results/plan.json
```

| Classification | Ý nghĩa / bước tiếp |
| --- | --- |
| `MISSING_MOOV` | Chưa có metadata trong view; tìm moov khác, metadata filesystem hoặc bản mẫu cùng thiết bị; không tự ghép bất kỳ moov nào |
| `MISSING_MEDIA_DATA` | Có metadata nhưng không có mdat trong view |
| `CHUNK_OFFSETS_OUTSIDE_MDAT` | Chunk trỏ ra ngoài payload; kiểm tra thiếu/sai thứ tự mảnh hoặc sai ranh giới file |
| `INVALID_OR_BUDGET_EXCEEDED` | Header bị cắt/sai, sai plan hoặc vượt budget; đọc `issues`, không quy tất cả thành hỏng file |
| `FRAGMENTED_MP4_REQUIRES_DEMUXER` | Phát hiện cấu trúc fragmented MP4; chuyển sang FFmpeg để xác minh |
| `INSUFFICIENT_SAMPLE_TABLES` | Thiếu track/chunk table để xác minh cấu trúc ở mức này |
| `STRUCTURE_CONSISTENT` | Box bounds và chunk-start offsets phù hợp; **chưa xác minh sample extents, codec hay nội dung đúng** |

Phân tích có budget 4096 boxes, depth 12, 65536 chunk offsets, lấy tối đa 12 box
cấp cao trong báo cáo. Không sửa stco/co64, không tái tạo moov thất lạc và không
tự thử hoán vị vô hạn các mảnh. Fragment prefilter cũ chỉ là heuristic entropy.

## 5. Xuất ứng viên và xác minh độc lập

```bash
python scripts/recovery.py recover /mnt/evidence/disk.img --plan /mnt/results/plan.json --output-dir /mnt/recovered/candidate-001 --filename candidate.mp4
python scripts/recovery.py verify-media /mnt/recovered/candidate-001/candidate.mp4
python scripts/recovery.py verify-media /mnt/recovered/candidate-001/candidate.mp4 --decode --timeout 600
```

`recover` dùng asynchronous byte-range extraction, chép đúng ranges trong plan,
có cancel và manifest; nó vẫn có thể xuất ứng viên không hợp lệ để điều tra.
Lệnh không tuyên bố định dạng đã được xác minh. Không ghi đè output cũ.

`verify-media` giới hạn demuxer MOV/MP4 và protocol file, chạy ffprobe không qua
shell và so hash/size nếu có manifest. `--decode` chạy FFmpeg với `-xerror`, audio
và video vào null output; không remux, không ghi lại file. Timeout là chưa hoàn tất,
không phải pass. Decode pass vẫn cần người dùng xem/nghe nội dung nếu nghi ghép
nhầm các mảnh có cấu trúc hợp lệ. Công cụ này không phải trình xác minh PDF/JPEG/ZIP.

Manifest `<filename>.manifest.json` ghi SHA-256 của output, các đoạn byte nguồn,
thứ tự byte output, geometry, patch ảo và bad sectors liên quan. Hash xác minh
đúng bytes đã xuất; không chứng minh một video hoàn chỉnh hay chain of custody.

Exit code CLI: `0` = thao tác hoàn tất; `1` = lỗi thao tác/đối số/I/O; `2` = phân
tích video chưa đạt `STRUCTURE_CONSISTENT` hoặc probe/decode thất bại; `130` = hủy.
Triage/scan thành công không đồng nghĩa dữ liệu đã recovery. Khi hủy hoặc lỗi,
giữ các `.part`/manifest/mapfile mà engine báo cần điều tra.

## 6. MCP cho AI agent

Sinh cấu hình đúng với máy hiện tại:

```bash
python scripts/recovery.py mcp-config --session-root /mnt/recovery-state
```

Copy object `mcpServers` vào cấu hình MCP của client. Windows dùng đường dẫn như
`D:\resc-state`. Lệnh sinh cấu hình in JSON ra stdout, không tự sửa file client.
Server dùng stdio, không có port HTTP. Nếu dùng Node launcher, đặt đường dẫn tuyệt
đối đến `bin/resc-mcp.js`; không dùng `npx resc` để tránh lấy nhầm package.

Agent đọc [AGENTS.md](../AGENTS.md), gọi `tools/list` rồi theo
[playbook triage](../.agents/skills/recovery-triage/SKILL.md). Một ví dụ chuỗi gọi:

```json
{"name":"open_recovery_session","arguments":{"target_path":"/mnt/evidence/disk.img","sector_size":512}}
{"name":"diagnose_recovery","arguments":{"session_id":"ID_FROM_OPEN"}}
{"name":"start_async_disk_job","arguments":{"session_id":"ID_FROM_OPEN","job_type":"ISO_BMFF_BOX_SCAN","params":{}}}
{"name":"poll_async_job","arguments":{"job_id":"ID_FROM_START"}}
{"name":"read_scan_hits","arguments":{"session_id":"ID_FROM_OPEN","job_id":"ID_FROM_START","cursor":0,"limit":20}}
```

Đây là objects truyền vào `tools/call`, không phải đầy đủ JSON-RPC envelope.
Đọc `next_cursor` cho trang tiếp theo. Chỉ ghép/xuất khi có plan rõ ràng, báo cáo
riêng cấu trúc/provenance/decoder. Giữ `RESC_SESSION_ROOT` giữa các lần chạy để
replay overlay WAL và lấy lại hit; live jobs không tự resume.

## 7. Phạm vi thực tế và trường hợp cần engine khác

| Nhóm | Hiện trạng |
| --- | --- |
| MBR/GPT, NTFS/FAT32 boot và backup | Kiểm tra hình học/CRC; sửa thử trong overlay |
| File allocated NTFS/FAT | Native extraction qua extents filesystem, không phụ thuộc kiểu nội dung |
| Chữ ký JPEG/PNG/PDF/ZIP/SQLite/MP4 và các loại mặc định | Phát hiện, không tự suy ra độ dài hoặc tái tạo mọi file |
| Mảnh có extents đã xác định, mọi kiểu nội dung | Byte-range export chính xác + manifest; không đoán thứ tự |
| JPEG/PNG | Marker/chunk CRC validation và ranh giới ứng viên, không decode pixel |
| MP4/MOV | Box diagnostics, chunk starts, tìm moov/mdat, kiểm tra FFmpeg trên output |
| FAT32 root deleted SFN | Bounded scan, phân biệt allocation và candidate single-cluster plan; chưa xác minh nội dung |
| NTFS deleted records/journals, exFAT/EXT4 recovery, RAID/LVM | Chưa hỗ trợ native; dùng công cụ chuyên biệt |
| Hỏng phần cứng, acquisition nhiều lượt có resume | GNU ddrescue/thiết bị acquisition chuyên biệt; native clone không thay thế thuật toán đó |
| Moov mất hoàn toàn, video bị overwrite, mã hóa thiếu khóa | Không bảo đảm dựng lại; cần bằng chứng/codec metadata hoặc khóa phù hợp |

Xem [capabilities](engine-capabilities.md) cho quotas, panic/I/O và transaction limits.
