# Playbook 06: Cứu Dữ Liệu Laptop Bị Hỏng & Kết Nối Thiết Bị Ngoài (USB / Ổ Di Động)

Tài liệu này hướng dẫn chi tiết quy trình kết nối và phục hồi dữ liệu từ một chiếc laptop bị sự cố (hỏng phần cứng hoặc hỏng hệ điều hành) bằng RecoveryAI.

---

## 1. Liệt Kê & Nhận Diện Ổ Đĩa Tự Động (`list_physical_drives`)

Trước khi thực hiện bất kỳ thao tác nào, hãy gọi công cụ:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "list_physical_drives",
    "arguments": {}
  }
}
```

### Kết quả trả về mẫu:
- `PHYSICALDRIVE0`: Ổ đĩa nội bộ của máy tính đang chạy (`is_external: false`).
- `PHYSICALDRIVE1`: Ổ cứng ngoài, USB Flash drive hoặc Box ổ cứng laptop cắm qua cổng USB (`is_external: true`).

---

## 2. Kịch Bản A: Laptop Bị Hỏng Phần Cứng Nặng
*(Laptop không bật được nguồn, cháy mainboard, chết chip, vô nước, rơi vỡ, hỏng màn hình)*

### Các Bước Thực Hiện:
1. **Tháo ổ cứng khỏi laptop bị hỏng:**
   - Dùng tuốc-nơ-vít mở nắp đáy laptop.
   - Tháo ổ cứng:
     - **Nếu là SSD M.2 (NVMe hoặc SATA):** Kích thước thanh nhỏ (2280 / 2242).
     - **Nếu là HDD hoặc SSD 2.5 inch:** Kích thước hộp chữ nhật chuẩn SATA.
2. **Lắp vào Box chuyển đổi USB (Enclosure):**
   - Mua/sử dụng **Box di động M.2 PCIe NVMe/SATA to USB** (hãng Orico, Ugreen, Baseus...) hoặc **Cáp chuyển SATA sang USB 3.0**.
   - Gắn ổ cứng của laptop vào Box và đậy nắp lại.
3. **Kết nối vào máy cứu hộ:**
   - Cắm dây USB của Box vào máy tính đang chạy RecoveryAI.
4. **Khởi động phiên phục hồi:**
   - Mở Terminal/IDE dưới quyền **Administrator (Run as Administrator)** để Windows cấp quyền đọc sector vật lý thô.
   - Gọi `list_physical_drives` để xác nhận số thứ tự ổ đĩa (ví dụ: `\\.\PhysicalDrive1`).
   - Mở phiên:
     ```json
     {
       "name": "open_recovery_session",
       "arguments": { "target_path": "\\\\.\\PhysicalDrive1" }
     }
     ```
   - RecoveryAI lập tức kích hoạt chế độ **100% Read-Only DMA**, ngăn chặn hoàn toàn Windows tự ý ghi đè, format lại hoặc làm nặng thêm lỗi của ổ đĩa.

---

## 3. Kịch Bản B: Laptop Lỗi Hệ Điều Hành (Không Vào Được Windows)
*(Laptop vẫn bật nguồn được nhưng bị màn hình xanh BSOD, treo logo, hỏng boot EFI, lỗi Winload.efi, hỏng phân vùng C)*

> Không cần tháo ốc vít hay tháo rời ổ cứng!

### Các Bước Thực Hiện:
1. **Chuẩn bị USB Cứu hộ (WinPE Bootable):**
   - Sử dụng USB cứu hộ thông dụng (Anhdv Boot, DLC Boot, Hirens BootCD, hoặc Ventoy).
   - Copy thư mục `RecoveryAI` (chứa file `target/release/mcp-server.exe` và `bin/recoveryai-mcp.js`) vào USB cứu hộ.
2. **Khởi động Laptop từ USB:**
   - Cắm USB vào laptop bị lỗi.
   - Nhấn nút nguồn và liên tục bấm phím chọn Boot Menu:
     - **Dell:** F12
     - **HP:** F9 hoặc Esc
     - **Lenovo:** F12 hoặc nút Novo
     - **Asus:** F8 hoặc Esc
     - **Acer:** F12
   - Chọn khởi động vào **Windows PE (WinPE)** từ USB.
3. **Chạy RecoveryAI trên WinPE:**
   - Trong môi trường WinPE, ổ cứng nội bộ của laptop sẽ hiển thị là `\\.\PhysicalDrive0`.
   - Cắm thêm 1 ổ cứng di động hoặc USB thứ hai có đủ dung lượng trống để làm nơi lưu dữ liệu cứu được.
   - Chạy `mcp-server.exe` và trích xuất dữ liệu:
     `extract_recovered_data(source_type="TSK_TREE_EXTRACT", target_dir="E:/DuLieuLaptop")`.

---

## 4. Lưu Ý Quan Trọng Về Quyền Hạn Windows
Khi làm việc với các ổ đĩa vật lý trực tiếp (`\\.\PhysicalDriveX`), hệ điều hành Windows bắt buộc chương trình phải có quyền **Administrator (Elevated Privileges)**.
- Nếu gặp lỗi: `Access is denied (os error 5)`:
  -> Nhấp chuột phải vào biểu tượng Terminal / PowerShell / Antigravity / Cursor -> Chọn **"Run as administrator"**.
- Nếu không có quyền Administrator:
  -> Hãy dùng một công cụ tạo ảnh đĩa (Disk Image) để dump toàn bộ ổ cứng ra file `.img` hoặc `.dd`, sau đó mở file ảnh bằng `open_recovery_session("D:/backup.img")`.
