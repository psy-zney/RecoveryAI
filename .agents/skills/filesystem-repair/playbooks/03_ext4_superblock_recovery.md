# Playbook: Tìm Kiếm & Áp Dụng Backup Superblock Cho Linux EXT4

## 1. Cấu Trúc Linux EXT4 Superblock
Trong hệ thống Linux EXT2/EXT3/EXT4:
- Primary Superblock nằm ở byte offset `1024` từ đầu phân vùng (Sector 2 nếu sector size là 512 bytes).
- Kích thước Superblock: `1024` bytes.
- Magic signature tại offset `0x38` (`0x56` bytes tính từ đầu Superblock): `0xEF53` (`53 EF` in little-endian).

## 2. Vị Trí Các Bản Backup Superblock (Sparse Superblock Feature)
EXT4 mặc định kích hoạt tính năng `sparse_super`. Các bản sao dự phòng của Superblock chỉ được lưu trữ ở các Block Group:
- Block Group 0: Primary Superblock
- Block Group 1: Backup Superblock 1
- Block Group 3: Backup Superblock 2
- Block Group 5: Backup Superblock 3
- Block Group 7: Backup Superblock 4
- Các lũy thừa của 3, 5, 7 (ví dụ: Group 9, 25, 27, 49, 81...)

### Các Vị Trí Block Thông Thường (Khi Block Size = 4096 bytes):
- Block `32768` (LBA = `start_lba + 32768 * 8`)
- Block `98304` (LBA = `start_lba + 98304 * 8`)
- Block `163840` (LBA = `start_lba + 163840 * 8`)
- Block `229376` (LBA = `start_lba + 229376 * 8`)

## 3. Quy Trình Khôi Phục Superblock EXT4 Trên RAM Overlay
1. Kiểm tra vị trí Primary Superblock tại `start_lba + 2` qua `inspect_sector_structure`.
2. Nếu byte tại offset `0x38` không phải `53 EF`, thực hiện quét nhanh tại các LBA tương ứng với Block 32768 hoặc 98304.
3. Khi tìm thấy chữ ký `0xEF53`, trích xuất 1024 bytes của Backup Superblock.
4. Áp dụng đè lên `start_lba + 2` qua `virtual_patch_and_verify`.
5. Kiểm tra tính mountable bằng cách phân tích Block Group Descriptors và root inode (Inode 2).
