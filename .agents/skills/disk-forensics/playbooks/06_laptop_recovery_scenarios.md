# Cứu laptop bằng Linux Live và máy thứ hai

Hướng dẫn chuẩn hiện tại: [kết nối/acquisition hai máy](../../../../docs/two-computer-rescue.vi.md).
Đọc tài liệu đó trước khi truy cập ổ vật lý. Đây là CLI acquisition trước MCP
recovery, không có MCP remote-device tool.

1. Nếu máy bật được nhưng OS hỏng, boot Linux Live USB; không cài OS lên SSD.
2. Chạy `python3 scripts/rescue.py devices`, đối chiếu model/serial/capacity/geometry.
3. Chọn ổ khỏe khác để chứa ảnh. Source backing không được mounted hoặc swap.
4. Xem plan `rescue.py acquire --source DEVICE --case DESTINATION_CASE`, rồi dùng
   `--run` sau khi xác minh đúng nguồn/đích. Fast dùng GNU ddrescue --no-scrape.
5. Giữ case/map và dùng `--resume` sau gián đoạn; retry có giới hạn với
   `--phase retry`. `--seal` chốt ảnh/map/hash và cấm acquisition tiếp vào case đó.
6. Nếu cần máy thứ hai điều khiển, dùng `rescue.py remote HOST devices|acquire`
   qua SSH key đã kiểm tra fingerprint. Máy nguồn cần healthy staging storage.
7. Dùng `rescue.py pull HOST REMOTE_CASE --case LOCAL_CASE`, thêm `--resume` khi
   mạng ngắt. Chỉ dùng ảnh sau khi transfer hash verified.
8. Chạy `recovery.py triage IMAGE --acquisition-manifest RECEIVED_JSON` rồi MCP
   recovery theo recovery-triage/SKILL.md. Giữ unresolved bytes và mapfile.

Nếu máy không cấp nguồn, SSD tháo được thì dùng adapter/khe đúng SATA/NVMe trên
máy cứu hộ; nếu SSD không nhận thiết bị hoặc NAND/controller hỏng thì cần phần
cứng/chuyên gia. Chế độ Target Disk/Share Disk chỉ dùng trên máy thực sự hỗ trợ.
Cáp USB nối hai laptop thông thường không cung cấp quyền đọc SSD của máy kia.

Windows `list_physical_drives` chỉ là enumeration/đọc thiết bị được hỗ trợ;
raw-device export/clone vẫn NotImplemented. Không hướng dẫn dùng lệnh extraction
WinPE giả định hoặc khẳng định resc ngăn OS ghi lên nguồn. Nguồn được resc mở
read-only, nhưng OS/phần mềm khác vẫn có thể ghi. Chưa tạo hoặc xác nhận boot ISO
hay thử nghiệm trên laptop/SSD lỗi thật trong dự án.
