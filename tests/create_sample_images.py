"""
Tạo các file ảnh đĩa mẫu phục vụ kiểm thử tự động hệ thống cứu dữ liệu RecoveryAI.
Tạo 4 kịch bản lỗi:
1. case1_lost_partition.img: Mất MBR/GPT (LBA 0..33 bị zeroed), VBR tại LBA 2048 còn nguyên
2. case2_ntfs_raw_corrupted_vbr.img: Lỗi RAW NTFS (VBR tại LBA 2048 bị hỏng OEM ID, Backup VBR ở cuối đĩa còn tốt)
3. case3_fat32_corrupted_boot.img: FAT32 hỏng Boot Sector 0, Backup Boot Sector 6 còn tốt
4. case4_fragmented_mp4.img: File MP4 bị phân mảnh thành 2 đoạn cách nhau bởi khoảng trống
"""

import os
import struct

SECTOR_SIZE = 512
TOTAL_SECTORS = 20480  # 10 MB = 20480 * 512 bytes
OUTPUT_DIR = os.path.join(os.path.dirname(__file__), "sample_corrupted_images")


def create_ntfs_vbr(total_partition_sectors=18432, mft_lcn=786, mft_mirr_lcn=2):
    """Tạo NTFS Boot Sector (512 bytes) chuẩn."""
    buf = bytearray(SECTOR_SIZE)
    # Jump instruction: 0xEB, 0x52, 0x90
    buf[0:3] = b"\xeb\x52\x90"
    # OEM ID: 'NTFS    '
    buf[3:11] = b"NTFS    "
    # Bytes per sector: 512
    struct.pack_into("<H", buf, 11, 512)
    # Sectors per cluster: 8 (4KB)
    buf[13] = 8
    # Reserved sectors: 0
    struct.pack_into("<H", buf, 14, 0)
    # Media descriptor: 0xF8 (Fixed disk)
    buf[21] = 0xF8
    # Sectors per track: 63
    struct.pack_into("<H", buf, 24, 63)
    # Number of heads: 255
    struct.pack_into("<H", buf, 26, 255)
    # Hidden sectors: 2048
    struct.pack_into("<I", buf, 28, 2048)
    # Total sectors:
    struct.pack_into("<Q", buf, 40, total_partition_sectors)
    # $MFT LCN:
    struct.pack_into("<Q", buf, 48, mft_lcn)
    # $MFTMirr LCN:
    struct.pack_into("<Q", buf, 56, mft_mirr_lcn)
    # Clusters per MFT record: -10 = 2^10 = 1024 bytes
    struct.pack_into("<b", buf, 64, -10)
    # Clusters per Index record: 1
    struct.pack_into("<b", buf, 68, 1)
    # Volume Serial: 0x12345678AABBCCDD
    struct.pack_into("<Q", buf, 72, 0x12345678AABBCCDD)
    # Boot signature: 0xAA55
    struct.pack_into("<H", buf, 510, 0xAA55)
    return buf


def create_fat32_vbr(total_partition_sectors=18432):
    """Tạo FAT32 Boot Sector (512 bytes) chuẩn."""
    buf = bytearray(SECTOR_SIZE)
    # Jump instruction
    buf[0:3] = b"\xeb\x58\x90"
    # OEM ID: 'MSDOS5.0'
    buf[3:11] = b"MSDOS5.0"
    # Bytes per sector: 512
    struct.pack_into("<H", buf, 11, 512)
    # Sectors per cluster: 8
    buf[13] = 8
    # Reserved sectors: 32
    struct.pack_into("<H", buf, 14, 32)
    # Number of FATs: 2
    buf[16] = 2
    # Media descriptor: 0xF8
    buf[21] = 0xF8
    # Hidden sectors: 2048
    struct.pack_into("<I", buf, 28, 2048)
    # Total sectors 32:
    struct.pack_into("<I", buf, 32, total_partition_sectors)
    # Sectors per FAT: 288
    struct.pack_into("<I", buf, 36, 288)
    # Root cluster: 2
    struct.pack_into("<I", buf, 44, 2)
    # FSInfo sector: 1
    struct.pack_into("<H", buf, 48, 1)
    # Backup boot sector: 6
    struct.pack_into("<H", buf, 50, 6)
    # File system type: 'FAT32   '
    buf[82:90] = b"FAT32   "
    # Boot signature: 0xAA55
    struct.pack_into("<H", buf, 510, 0xAA55)
    return buf


def create_mbr_partition_table(start_lba=2048, total_sectors=18432, part_type=0x07):
    """Tạo MBR (Sector 0) chứa 1 partition entry chuẩn."""
    buf = bytearray(SECTOR_SIZE)
    # Partition entry 1 at offset 446 (0x1BE)
    offset = 446
    buf[offset] = 0x80  # Bootable
    buf[offset + 4] = part_type  # 0x07 = NTFS/exFAT, 0x0C = FAT32 LBA
    struct.pack_into("<I", buf, offset + 8, start_lba)
    struct.pack_into("<I", buf, offset + 12, total_sectors)
    # MBR Signature: 0xAA55
    struct.pack_into("<H", buf, 510, 0xAA55)
    return buf


def generate_all_test_images():
    os.makedirs(OUTPUT_DIR, exist_ok=True)
    print(f"Generating test disk images into: {OUTPUT_DIR}")

    # --- Test Case 1: Mất MBR/GPT (Unallocated) ---
    path1 = os.path.join(OUTPUT_DIR, "case1_lost_partition.img")
    disk1 = bytearray(TOTAL_SECTORS * SECTOR_SIZE)
    # LBA 0..33 bị zeroed (không có MBR / GPT)
    # VBR nằm mồ côi tại LBA 2048
    vbr_ntfs = create_ntfs_vbr(18432)
    disk1[2048 * SECTOR_SIZE : 2049 * SECTOR_SIZE] = vbr_ntfs
    # Backup VBR tại LBA 2048 + 18432 - 1 = 20479
    disk1[20479 * SECTOR_SIZE : 20480 * SECTOR_SIZE] = vbr_ntfs
    with open(path1, "wb") as f:
        f.write(disk1)
    print(f"[OK] Created {path1} (Size: {len(disk1)} bytes)")

    # --- Test Case 2: NTFS RAW (Corrupted Primary VBR, Valid Backup VBR) ---
    path2 = os.path.join(OUTPUT_DIR, "case2_ntfs_raw_corrupted_vbr.img")
    disk2 = bytearray(TOTAL_SECTORS * SECTOR_SIZE)
    # Valid MBR at LBA 0
    disk2[0:SECTOR_SIZE] = create_mbr_partition_table(2048, 18432, 0x07)
    # Corrupted Primary VBR at LBA 2048: OEM ID bị đè bởi zero và rác
    corrupted_vbr = bytearray(vbr_ntfs)
    corrupted_vbr[3:11] = b"\x00\x00\x00\x00\x00\x00\x00\x00"
    struct.pack_into("<H", corrupted_vbr, 11, 0)  # bytes_per_sector = 0 (hỏng nặng)
    disk2[2048 * SECTOR_SIZE : 2049 * SECTOR_SIZE] = corrupted_vbr
    # Valid Backup VBR ở cuối phân vùng (LBA 20479)
    disk2[20479 * SECTOR_SIZE : 20480 * SECTOR_SIZE] = vbr_ntfs
    with open(path2, "wb") as f:
        f.write(disk2)
    print(f"[OK] Created {path2} (Size: {len(disk2)} bytes)")

    # --- Test Case 3: FAT32 RAW (Corrupted Boot Sector 0, Valid Backup Sector 6) ---
    path3 = os.path.join(OUTPUT_DIR, "case3_fat32_corrupted_boot.img")
    disk3 = bytearray(TOTAL_SECTORS * SECTOR_SIZE)
    disk3[0:SECTOR_SIZE] = create_mbr_partition_table(2048, 18432, 0x0C)
    fat32_vbr = create_fat32_vbr(18432)
    # Sector 0 tại LBA 2048 bị hỏng
    corrupted_fat_vbr = bytearray(fat32_vbr)
    corrupted_fat_vbr[3:11] = b"CORRUPT!"
    corrupted_fat_vbr[510:512] = b"\x00\x00"  # Mất signature 0xAA55
    disk3[2048 * SECTOR_SIZE : 2049 * SECTOR_SIZE] = corrupted_fat_vbr
    # Backup Boot Sector tại LBA 2048 + 6 = 2054 hoàn hảo
    disk3[2054 * SECTOR_SIZE : 2055 * SECTOR_SIZE] = fat32_vbr
    with open(path3, "wb") as f:
        f.write(disk3)
    print(f"[OK] Created {path3} (Size: {len(disk3)} bytes)")

    # --- Test Case 4: MP4 Fragmented Carving ---
    path4 = os.path.join(OUTPUT_DIR, "case4_fragmented_mp4.img")
    disk4 = bytearray(TOTAL_SECTORS * SECTOR_SIZE)
    # Fragment A: LBA 4000..4009 (10 sectors = 5120 bytes)
    # MP4 Header: 00 00 00 18 'ftyp' 'mp42' ...
    mp4_head = bytearray(5120)
    mp4_head[0:4] = struct.pack(">I", 24)
    mp4_head[4:8] = b"ftyp"
    mp4_head[8:12] = b"mp42"
    # Giả lập payload video (high entropy ~ 7.8)
    import random
    rng = random.Random(42)
    for i in range(12, 5120):
        mp4_head[i] = rng.randint(0, 255)
    disk4[4000 * SECTOR_SIZE : 4010 * SECTOR_SIZE] = mp4_head

    # Gap: LBA 4010..4200 (190 sectors rác / zeroed)

    # Fragment B: LBA 4200..4210 (moov atom)
    mp4_tail = bytearray(5120)
    mp4_tail[0:4] = struct.pack(">I", 5120)
    mp4_tail[4:8] = b"moov"
    for i in range(8, 5120):
        mp4_tail[i] = rng.randint(0, 255)
    disk4[4200 * SECTOR_SIZE : 4210 * SECTOR_SIZE] = mp4_tail
    with open(path4, "wb") as f:
        f.write(disk4)
    print(f"[OK] Created {path4} (Size: {len(disk4)} bytes)")


if __name__ == "__main__":
    generate_all_test_images()
