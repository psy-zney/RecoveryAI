# Case 2 — Xóa nhầm và mất metadata: bằng chứng FAT32 deleted entry

## Nghiên cứu độc lập và phạm vi triển khai

Nhánh này nghiên cứu cấu trúc directory/FAT thay vì suy luận từ chữ ký nội dung.
Mã `fatfs` đang được dự án khóa ở **0.3.6** đã có sẵn trong Cargo registry cache,
được cài bằng `Cargo.lock` có checksum. Đọc trực tiếp ba file upstream:

- [dir_entry.rs, upstream commit 3290f78d](https://github.com/rafalh/rust-fatfs/blob/3290f78d73b80e60cf18a6f5a0931fdcfe5b82d3/src/dir_entry.rs):
  `DIR_ENTRY_DELETED_FLAG = 0xE5`, `is_deleted`, `is_end`, short-entry layout,
  first-cluster high/low và byte size. Byte đầu SFN bị thay bằng 0xE5 nên tên gốc
  không còn xác định hoàn toàn.
- [boot_sector.rs cùng commit](https://github.com/rafalh/rust-fatfs/blob/3290f78d73b80e60cf18a6f5a0931fdcfe5b82d3/src/boot_sector.rs):
  geometry, số sector/FAT, root cluster, active FAT khi tắt mirroring và validation.
- [table.rs cùng commit](https://github.com/rafalh/rust-fatfs/blob/3290f78d73b80e60cf18a6f5a0931fdcfe5b82d3/src/table.rs):
  FAT32 chỉ dùng 28 bit thấp, 0 là free, 0x0FFFFFF7 là bad, 0x0FFFFFF8..F là EOC.
- [fs.rs cùng commit](https://github.com/rafalh/rust-fatfs/blob/3290f78d73b80e60cf18a6f5a0931fdcfe5b82d3/src/fs.rs):
  FAT32 được phân loại theo cluster count, ngưỡng tối thiểu 65525; không chỉ nhìn
  chuỗi nhãn `FAT32` trong BPB.

Commit ở trên lấy từ `.cargo_vcs_info.json` của package đã cài. Đây là nghiên cứu
source upstream trong cache, không phải xác nhận phiên bản mới nhất trên mạng.
`fatfs` có giấy phép MIT. Phần scanner mới là implementation độc lập của việc đọc
trường và kiểm tra bounds, không sao chép mã GPL từ công cụ undelete khác và
không cần thêm dependency hay sửa lockfile.

Kết luận kỹ thuật quan trọng: marker đã xóa giữ lại first cluster và size, nhưng
FAT chain thường bị xóa. Vì vậy không được lấy số byte rồi đoán mọi cluster kế
tiếp là liên tục, nhất là với file lớn/phân mảnh. FAT-free hiện tại cũng không
chứng minh dữ liệu chưa từng bị ghi đè.

## Mã nguồn và API

`crates/mcp-server/src/deleted_recovery.rs` cung cấp scanner chỉ đọc; root agent
tích hợp thành job bất đồng bộ `FAT32_DELETED_SCAN`:

```json
{
  "session_id": "ID_FROM_OPEN",
  "job_type": "FAT32_DELETED_SCAN",
  "params": {"partition_start_lba": 0, "max_clusters": 256}
}
```

`partition_start_lba` là đầu volume đã xác minh, tính theo session sector size.
Job đọc **root directory** theo FAT chain, kiểm tra vòng lặp, bounds, geometry,
FAT capacity, FAT copies hoặc active FAT theo BPB. Nếu partition table hợp lệ có
entry đúng đầu volume, scanner còn kiểm tra BPB không vượt partition đó. Volume
mồ côi do người dùng chỉ rõ vẫn có thể được phân tích khi không có entry tương ứng.

Mỗi deleted short-entry được lưu thành hit JSONL, đọc bằng `read_scan_hits` và
cursor. Long-filename records, deleted directories và volume-label records
không bị hiểu nhầm thành file thường. Trường `name_hint` được làm sạch để hiển thị,
không dùng như đường dẫn output và luôn ghi `name_complete: false`.

| allocation_state | Ý nghĩa |
| --- | --- |
| `SINGLE_CLUSTER_EXTENT_CONTENT_UNVERIFIED` | First cluster free trong FAT được chọn/các mirror, size nằm trong một cluster, root scan không bị cắt, không thấy live root reference; có candidate export plan |
| `CURRENTLY_ALLOCATED_OR_RESERVED` | FAT không free; không cung cấp plan vì có thể đã được cấp phát lại |
| `FAT_COPIES_DISAGREE` | Bằng chứng allocation mâu thuẫn; không chọn tùy ý bản có lợi |
| `REFERENCED_BY_LIVE_ROOT_METADATA` | Cluster trùng root-chain hoặc một first-cluster reference đang sống ở root; không cung cấp plan |
| `FREE_FIRST_CLUSTER_LOST_CHAIN_NO_PLAN` | File cần nhiều cluster; chain chưa biết, không đoán contiguous |
| `ROOT_SCAN_INCOMPLETE_NO_PLAN` | Chạm budget trước khi đọc xong root; chưa loại trừ được live references |
| `INVALID_DIRECTORY_METADATA` / `INVALID_FIRST_CLUSTER` | Trường metadata không nhất quán hoặc vượt data region |
| `EMPTY_FILE_NO_DATA_EXTENT` | Size bằng 0; không có byte payload để xuất |

Plan có đúng dạng `{ "ranges": [{"source_byte_offset": ..., "length_bytes": ...}] }`.
Chỉ là candidate extents của entry hiện quan sát, không bảo đảm bytes thuộc file
gốc, format đúng hoặc tên đúng. Người dùng có thể đưa plan vào
`CARVE_BY_BYTE_RANGES`, nhận SHA-256/provenance và kiểm tra định dạng/nội dung độc lập.

## Giới hạn và an toàn

- Hỗ trợ raw image 512/4096-byte logical sectors và volume FAT32 đúng geometry.
- Chỉ root-directory SFN; chưa duyệt subdirectories, ghép LFN, NTFS deleted MFT,
  journals, FAT12/FAT16/exFAT hoặc phục hồi fragmented deleted chains.
- Giới hạn `max_clusters` 1..4096, root bytes <=32 MiB, entries <=65536, candidates
  <=4096 và FAT-sector cache 128 sectors. Report có `scan_truncated` và số đếm.
- Khi bất kỳ budget nào cắt scan, không phát candidate export plan.
- Root directory FAT chain free/bad/reserved/vòng lặp hoặc mirror bất đồng đều
  là lỗi tường minh; không biến một lần scan không hoàn tất thành thành công.
- Cancellation được kiểm tra trước/sau mỗi read và mỗi entry/candidate. Worker
  tích hợp gắn cancel token vào overlay; các nguồn I/O vẫn theo giới hạn kernel
  đã nêu trong capability document.
- Scanner không vá metadata, không đánh dấu FAT, không ghi nguồn và không tự
  xuất candidate. Poll/hit pagination dùng job manager hiện có.

## Kiểm chứng

Unit tests nội bộ dùng sparse synthetic storage, không dùng physical media:
one-cluster candidate 512/4096, allocated/lost chain, mirror disagreement,
live-root reference, active FAT selection, bounds/geometry, cancellation,
root FAT bị free và cycle. E2E riêng ở `tests/test_deleted_recovery.py` tạo ảnh
FAT32 có deleted entry, chạy job thật, đọc hit, xuất exact-byte candidate và
kiểm tra output payload/SHA-256/manifest cùng hash nguồn không đổi. Test âm
kiểm tra reallocated cluster, FAT mismatch, file nhiều cluster và BPB vượt ảnh.

Đây là phạm vi khôi phục có bằng chứng cụ thể; không phải tuyên bố hỗ trợ mọi
trường hợp xóa nhầm, quick-format hoặc dữ liệu đã bị ghi đè.
