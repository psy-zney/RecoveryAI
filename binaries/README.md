# Thư mục Binaries Mã nguồn mở (External Forensics Engines)

Hệ thống RecoveryAI tích hợp chặt chẽ với các công cụ điều tra và cứu dữ liệu mã nguồn mở thông qua Crate `external-bridges`.

## Cấu trúc thư mục chuẩn:

```text
binaries/
├── sleuthkit/
│   └── bin/
│       ├── mmls.exe       # Phân tích partition layout
│       ├── fsstat.exe     # Đọc metadata chi tiết NTFS/FAT/ext4
│       ├── fls.exe        # Liệt kê tệp tin và các entry đã bị xóa trong MFT/FAT
│       └── icat.exe       # Trích xuất dữ liệu tệp tin theo inode / MFT record
├── testdisk/
│   ├── photorec_win.exe  # Engine file carving hơn 480 định dạng
│   └── testdisk_win.exe   # Phục hồi partition table chuyên dụng
└── ddrescue/
    └── ddrescue.exe       # Image dump an toàn khi gặp Bad Sector vật lý
```

## Nguồn tải chính thức:
1. **The Sleuth Kit (TSK):** https://github.com/sleuthkit/sleuthkit/releases
2. **TestDisk & PhotoRec:** https://www.cgsecurity.org/wiki/TestDisk_Download
3. **GNU ddrescue for Windows:** Có sẵn qua MSYS2 (`pacman -S ddrescue`) hoặc các bản build độc lập.
