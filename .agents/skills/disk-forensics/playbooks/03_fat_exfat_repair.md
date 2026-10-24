# Playbook 03: Khôi phục Phân Vùng FAT32 & exFAT (FAT32/exFAT Repair)

## 1. Dấu hiệu nhận biết
- Thẻ nhớ SD, USB Flash Drive định dạng FAT32 hoặc exFAT cắm vào máy bị yêu cầu format.
- Phân tích sector đầu tiên của phân vùng thấy chữ ký bị ghi đè hoặc giá trị `bytes_per_sector` bằng 0.

## 2. Đặc điểm Kiến trúc Phục hồi của FAT32 & exFAT

### 2.1. FAT32
- **Sector 0 (VBR):** Chứa BIOS Parameter Block (BPB). OEM ID thường là `MSDOS5.0`.
- **Sector 1 (FSInfo):** Chứa thông tin số cluster trống (`0x52 0x52 0x61 0x41` và `0x72 0x72 0x41 0x61`).
- **Sector 6 (Backup VBR):** Bản sao chính xác 100% của Sector 0.
- **Sector 7 (Backup FSInfo):** Bản sao của Sector 1.
- **Bảng FAT 1 & Bảng FAT 2:** FAT32 duy trì 2 bảng File Allocation Table song song. Nếu FAT 1 bị hỏng, FAT 2 có thể được sao chép đè lên.

### 2.2. exFAT
- **Sectors 0..11 (Main Boot Region):** Gồm 12 sectors (Boot Sector, Extended Boot Sectors, OEM Parameter Sector, Boot Checksum Sector).
- **Sectors 12..23 (Backup Boot Region):** Bản sao chính xác của 12 sectors đầu.
- Checksum sector (Sector 11 và Sector 23) phải khớp chính xác với thuật toán XOR checksum của 11 sector trước đó.

## 3. Quy trình thực hiện qua MCP

### Trường hợp 1: FAT32 hỏng Boot Sector
1. Đọc Sector 0: `inspect_sector_structure(session_id, lba=start_lba, parse_as="FAT32_BPB")`.
2. Nếu phát hiện sai lệch (ví dụ: `bytes_per_sector != 512`, thiếu signature `0xAA55`):
3. Kiểm tra Sector 6: `inspect_sector_structure(session_id, lba=start_lba + 6, parse_as="FAT32_BPB")`.
4. Nếu Sector 6 nguyên vẹn, thực hiện bản vá ảo:
   ```json
   virtual_patch_and_verify(
     session_id="...",
     copy_from_lba=start_lba + 6,
     to_lba=start_lba
   )
   ```

### Trường hợp 2: exFAT hỏng Main Boot Region
1. Đọc và kiểm tra Main Boot Sector tại `Start LBA`.
2. Nếu checksum hoặc signature `EXFAT   ` bị hỏng, kiểm tra Backup Boot Sector tại `Start LBA + 12`.
3. Sao chép 12 sectors từ `Start LBA + 12` về `Start LBA` thông qua `virtual_patch_and_verify`.
