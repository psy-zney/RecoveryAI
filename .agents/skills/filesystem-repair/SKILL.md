---
name: filesystem-repair
description: Chuyên gia phân tích cấu trúc nội tại và sửa lỗi filesystem (NTFS, FAT32, exFAT, EXT4) trên RAM Copy-on-Write overlay.
---

# Kỹ Năng Sửa Chữa Cấu Trúc Hệ Thống Tệp Tin Ảo (Virtual Filesystem Repair)

Kỹ năng này cung cấp các giao thức chuyên sâu để chẩn đoán, tính toán lại thông số hình học (BPB, LCN, Cluster Size) và sửa chữa các hư hỏng nội tại của hệ thống tệp tin hoàn toàn trên RAM Overlay thông qua công cụ `virtual_patch_and_verify`.

## Nguyên Tắc Thực Thi Bắt Buộc
1. **Không ghi trực tiếp lên ổ cứng:** Mọi bản vá boot sector, bảng chỉ mục hay superblock chỉ được áp dụng trên RAM Overlay.
2. **Đối chiếu Checksum và Backup:** Luôn kiểm tra các vị trí dự phòng mặc định của file system trước khi thử nghiệm tái tạo thủ công:
   - **NTFS:** Backup VBR tại LBA cuối cùng của phân vùng; `$MFTMirr` (chứa bản sao của 4 record quan trọng nhất: `$MFT`, `$MFTMirr`, `$LogFile`, `$Volume`).
   - **FAT32:** Backup Boot Sector tại `Start LBA + 6`; FSInfo backup tại `Start LBA + 7`.
   - **exFAT:** Main Boot Region tại `Start LBA + 0..11`, Backup Boot Region tại `Start LBA + 12..23` với 32-bit checksum ở sector 11.
   - **EXT4:** Primary Superblock tại byte offset 1024; Backup Superblocks tại Block 32768, 98304, 163840, v.v.

---

## Danh Mục Playbooks
- [01_ntfs_mft_recovery.md](file:///c:/Users/admin/MyProject/RecoveryAI/.agents/skills/filesystem-repair/playbooks/01_ntfs_mft_recovery.md): Phục hồi NTFS VBR, khai thác `$MFTMirr` và khôi phục cây chỉ mục `$I30`.
- [02_fat32_exfat_fsinfo.md](file:///c:/Users/admin/MyProject/RecoveryAI/.agents/skills/filesystem-repair/playbooks/02_fat32_exfat_fsinfo.md): Tái tạo cấu trúc FAT32 FSInfo và xác thực checksum 12 sector của exFAT.
- [03_ext4_superblock_recovery.md](file:///c:/Users/admin/MyProject/RecoveryAI/.agents/skills/filesystem-repair/playbooks/03_ext4_superblock_recovery.md): Tìm kiếm và áp dụng Backup Superblock cho phân vùng Linux EXT4.
