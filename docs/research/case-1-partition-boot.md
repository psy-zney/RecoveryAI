# Case 1 — Mất phân vùng hoặc hỏng boot sector

Nghiên cứu và triển khai độc lập ngày 2026-10-26. Module:
`crates/mcp-server/src/partition_recovery.rs`.

## Bài toán và bằng chứng cần phân biệt

1. MBR/GPT bị mất hoặc một bản GPT có CRC/bounds sai.
2. Hai GPT hợp lệ riêng lẻ nhưng GUID/geometry/array CRC không thống nhất.
3. Boot NTFS/FAT32 chính bị hỏng, backup có thể còn dùng được.
4. Ảnh chỉ chứa phân vùng, không có partition table; ranh giới phải lấy từ
   acquisition hoặc được người dùng xác nhận.

Các trường hợp này không chứng minh dữ liệu đã bị xóa, và boot geometry hợp lệ
không chứng minh root directory mount được. Không tự ghi lên nguồn để thử.

## Nguồn đã kiểm tra

| Nguồn | Nội dung trực tiếp đối chiếu |
| --- | --- |
| [TianoCore EDK II UefiGpt.h](https://github.com/tianocore/edk2/blob/master/MdePkg/Include/Uefi/UefiGpt.h) | Primary GPT ở LBA 1; MyLBA, AlternateLBA, PartitionEntryLBA; CRC32 của entry array tính đúng NumEntries × EntrySize |
| [rust-fatfs 0.3.6 boot_sector.rs](https://github.com/rafalh/rust-fatfs/blob/3290f78d73b80e60cf18a6f5a0931fdcfe5b82d3/src/boot_sector.rs) | Backup boot pointer phải nằm trong reserved region; geometry, total clusters và boot BPB validation |
| [ntfs 0.4.0 boot_sector.rs](https://github.com/ColinFinck/ntfs/blob/3110cafc0440a84f2593b5d3691e0b23b68ccb8f/src/boot_sector.rs) | Bytes/sector, sectors/cluster, MFT/MFTMirr, record-size và volume-serial fields |
| [ntfs-3g ntfsfix.c](https://github.com/tuxera/ntfs-3g/blob/edge/ntfsprogs/ntfsfix.c), `check_alternate_boot` | Backup thường nằm ở sector cuối của partition; BPB number_of_sectors có thể nhỏ hơn số sector thực của partition, nhất là sau resize |

Nguồn được tải qua HTTPS với TLS verification; các crate còn được đối chiếu với
metadata/source đã cài theo Cargo.lock. Không sao chép mã của ntfsfix hoặc engine
GPL vào dự án. Thuật toán kiểm tra và tạo proposal là mã mới, tái dùng parser và
CRC32 đã có của dự án, không thêm dependency.

Để nhận diện chính xác bản nội dung đã đọc, SHA-256 của `UefiGpt.h` tải từ master:
`cc924f8228df2b1e04b4e2d4b0c7d4eca76b02ffb836f559b8ef7e67d1bc2aa1`.
Link `master`/`edge` là nguồn đang phát triển, không phải pin phiên bản upstream.

## Kết luận được đưa vào mã

### GPT

- Dùng validator hiện có kiểm tra header CRC32, entry-array CRC32, LBA và bounds.
- Nếu hai bản hợp lệ nhưng mâu thuẫn, báo `VALID_COPIES_CONFLICT`, không tự chọn
  bản thắng và không tạo repair proposal.
- Nếu chỉ một bản dùng được, proposal mô tả copy entry-array vào metadata region
  của bản còn lại và tạo **header mới** với MyLBA, AlternateLBA, PartitionEntryLBA
  cùng CRC32 đã tính lại. Không copy nguyên backup header vào primary.
- Chỉ proposal cho header tiêu chuẩn 92 bytes, array tối đa 64 logical sectors.
  Nếu vượt budget hoặc target lấn usable region thì không tạo proposal.
- Proposal chưa được áp dụng. GPT entries phù hợp không chứng nhận filesystem.

### NTFS

- Dùng partition boundary đã được bảng phân vùng hợp lệ xác nhận; nếu không có,
  dùng phần ảnh từ `partition_start_lba` đến cuối như một giả thuyết được ghi rõ.
- Backup được đọc tại sector cuối của boundary này. Không mặc định vị trí là
  `start + BPB.total_sectors - 1`: nghiên cứu ntfs-3g cho thấy công thức đó không
  bao quát trường hợp backup sector nằm ngoài số sector filesystem ghi trong BPB.
- Đối chiếu geometry, MFT/MFTMirr, record parameters và volume serial. Nếu primary
  hỏng và backup phù hợp, trả overlay-copy proposal với các cờ xác nhận identity
  và boundary. Các cờ không thay thế đối chiếu chứng cứ acquisition.
- Không đề xuất tái tạo backup tại vùng có thể là dữ liệu filesystem nếu chưa có
  ranh giới xác nhận và bằng chứng sector đó được dành cho backup.

### FAT32

- Nếu primary hợp lệ, đọc backup tại BPB pointer; nếu primary không đọc được,
  thử vị trí phổ biến `start + 6` như **ứng viên**, không coi đó là hằng số đúng
  cho mọi volume.
- Backup pointer phải khác zero và nằm trong reserved region; bytes/sector và
  volume range phải khớp. Kiểm tra tối thiểu FAT32 cluster count, FAT capacity và
  root cluster để tránh coi BPB FAT12/16 giả là FAT32.
- Hai boot hợp lệ nhưng geometry/serial khác nhau không nhận automatic repair.
- Chỉ đề xuất copy một boot sector; không giả vờ đã phục hồi FSInfo, FAT chains,
  deleted entries hoặc toàn bộ boot region.

## Interface và cách dùng sau khi tích hợp

```text
analyze(disk, sector_size, optional partition_start_lba) -> compact JSON
```

Root agent tích hợp module qua MCP/CLI. Phân tích không áp dụng patch, không mở
write handle tới nguồn và không tự chạy công cụ repair ngoài. Proposal yêu cầu
caller thực hiện riêng từng overlay action qua `virtual_patch_and_verify`, sau
đó chạy lại topology và native filesystem validation.

Giới hạn: tối đa 3 volume mỗi lần, report dưới 3500 JSON bytes, volume bị cắt được
báo `volumes_truncated`; dùng `partition_start_lba` để kiểm tra một volume cụ thể.
`volumes_examined` khác `volumes_returned` khi phải cắt báo cáo để giữ budget.

## Kiểm thử đã thực hiện

`cargo test --locked -p mcp-server partition_recovery -- --nocapture`:

- Primary NTFS hỏng signature: chỉ tạo proposal và overlay revision giữ nguyên.
- Sai dung lượng backup hoặc sai session geometry: không repair proposal.
- 4Kn dùng logical LBA đúng, không nhân thêm 8 khi trả địa chỉ MCP.
- BPB NTFS ghi 127 sector, partition thực 128: tìm backup đúng tại LBA 127.
- GPT backup dùng được: header proposal có địa chỉ/CRC được validator đọc lại.
- Hai GPT hợp lệ riêng lẻ nhưng xung đột: không có proposal.
- Áp dụng proposal GPT lên overlay của fixture rồi chạy validator độc lập:
  cả primary và backup hợp lệ; bytes nguồn không thay đổi.
- FAT32 backup hợp lệ và pointer ngoài reserved region bị từ chối.

Đây là fixture tổng hợp, không phải kiểm thử ổ vật lý lỗi hay mọi biến thể của
Windows/Linux. Chưa có MBR/EBR reconstruction, GPT resize, backup ở middle-sector
NTFS lịch sử, RAID/LVM hoặc đảm bảo mount đầy đủ. Không claim thư viện upstream
đã được nâng phiên bản chỉ vì nghiên cứu mã của chúng.
