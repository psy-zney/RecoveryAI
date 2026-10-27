---
name: file-carving-and-defrag
description: Chuyên gia khai phá tệp tin thô (Raw File Carving), ghép mảnh file bị phân mảnh không liên tục (Defragmentation) và phục hồi đa phương tiện / cơ sở dữ liệu.
---

> Implementation boundary: follow ../recovery-triage/SKILL.md and the live
> tools/list schema first. Historical playbooks below contain unsupported
> scenarios. General deleted-record/journal scanning, exFAT/EXT4 recovery and legacy
> external recovery adapters return NotImplemented. Fragment scores are
> heuristics, not proof of a match. Native cloning is not GNU ddrescue.
> All LBAs use session sector_size; never write repairs to original media.

# Kỹ Năng Khai Phá & Ghép Mảnh Tệp Tin (File Carving & Fragment Reassembly)

Khi các cấu trúc siêu dữ liệu của hệ thống tệp tin (`$MFT`, bảng FAT, Inode table) bị format, ghi đè hoặc hỏng nặng, việc khôi phục dữ liệu bắt buộc phải dựa vào việc phân tích trực tiếp dòng byte trên đĩa (Raw Carving).

Kỹ năng này trang bị cho AI Agent khả năng điều khiển bộ quét SIMD tốc độ cao của RecoveryAI, kết hợp với các thuật toán phân tích biên (Boundary Coherence) để tìm ứng viên. Việc xác định đúng thứ tự mảnh cần bằng chứng cấu trúc và kiểm tra nội dung độc lập.

---

## 3 Trụ Cột Kỹ Thuật
1. **SIMD-Accelerated Magic Byte Scanner:** Quét các chữ ký header được khai báo trong default_signatures() của JPEG, PNG, MP4, PDF, ZIP, SQLite, DOCX, XLSX qua AVX2/SSE4.
2. **Boundary Coherence & Entropy Continuity:** Đo độ chênh lệch entropy ($\Delta H = |H_A - H_B|$) và tính liên tục ngữ nghĩa tại điểm giao cắt giữa đuôi mảnh 1 và đầu mảnh 2 để lọc ra Top 5 ứng viên ghép nối tốt nhất.
3. **Internal Format Parsing:** Đọc cấu trúc container bên trong (như `moov`/`mdat` atoms trong MP4 hoặc B-Tree pages trong SQLite) để tính toán chính xác kích thước còn thiếu.

---

## Danh Mục Playbooks
- [01_boundary_coherence.md](playbooks/01_boundary_coherence.md): Thuật toán toán học chấm điểm tiếp giáp và lọc ứng viên mảnh ghép.
- [02_mp4_mov_atom_repair.md](playbooks/02_mp4_mov_atom_repair.md): Cứu video MP4/MOV bị phân mảnh atom `moov` và `mdat`.
- [03_sqlite_database_salvage.md](playbooks/03_sqlite_database_salvage.md): Khôi phục cơ sở dữ liệu SQLite từ các B-Tree leaf pages mồ côi.

## Current video workflow

Use ISO_BMFF_BOX_SCAN for candidate box headers, analyze_video_candidate for
ordered byte ranges, CARVE_BY_BYTE_RANGES for exact export, and the optional
FFmpeg verifier. Missing moov regeneration and arbitrary fragment-order search
are not implemented. See docs/recovery-guide.vi.md from the repository root.
