# Cứu máy không khởi động bằng máy thứ hai

`scripts/rescue.py` bổ sung acquisition trước bước recovery. Nó chạy độc lập với
Rust trên Linux Live, nhận diện thiết bị, tạo ảnh bằng GNU ddrescue, tiếp tục bằng
mapfile và truyền ảnh đã chốt sang máy khác qua SSH. `scripts/recovery.py` và MCP
xử lý ảnh sau acquisition. Không cần cài Rust lên máy hỏng.

## 1. Chọn phương thức theo tình trạng máy

| Tình trạng | Phương thức | Phần resc hỗ trợ |
| --- | --- | --- |
| Bật được, không vào Windows | Boot Linux Live USB; đọc SSD nội bộ | `devices`, `acquire`, resume/retry |
| Boot Live được, muốn điều khiển từ máy mới | Ethernet/Wi-Fi + SSH có xác thực | `remote devices`, `remote acquire`, `pull` |
| Tắt nguồn hoàn toàn, SSD tháo được | Lắp SSD vào máy cứu hộ hoặc enclosure đúng chuẩn | Boot Linux trên máy cứu hộ rồi acquisition |
| Mac hỗ trợ Target Disk Mode/Share Disk | Dùng đúng chế độ và cáp Apple hỗ trợ | Đọc thiết bị/ảnh mà OS thực sự cung cấp; không tự kích hoạt chế độ Apple |
| SSD không được firmware nhận, controller/NAND hỏng | Thiết bị/chuyên gia phần cứng | Không có byte-source để phần mềm đọc |
| BitLocker/FileVault/LUKS | Giữ ảnh mã hóa và khóa; unlock bản sao bằng công cụ phù hợp | Acquisition bytes; chưa giải mã volume native |
| RAID/LVM, exFAT/Ext4, NTFS undelete | Acquisition đủ thành viên rồi công cụ chuyên biệt | Chưa phục hồi logic các dạng này trong resc |

USB A–A/C–C thông thường không biến một laptop thành ổ đĩa của laptop kia. Cáp
Ethernet kết nối mạng, nhưng máy nguồn phải chạy được Live OS/SSH. Không cấp nguồn
được thì phải tiếp cận ổ bằng phần cứng hoặc chế độ firmware có hỗ trợ.

Luồng SSH hiện tại là **acquisition vào ổ khỏe ở máy nguồn → chốt ảnh → tải về máy
mới**. Máy nguồn cần ổ đích thứ hai đủ dung lượng. Chưa có luồng ddrescue trực tiếp
từ SSD từ xa vào ổ máy mới khi máy nguồn không có chỗ chứa ảnh; không dùng pipe
`ssh ... dd | ddrescue` vì pipe không hỗ trợ seek/resume theo mapfile như block source.

## 2. Chuẩn bị USB và bộ công cụ

Tải ISO Linux Live từ nhà phát hành chính thức, tạo USB boot bằng công cụ phù hợp
trên một USB dành riêng. Thao tác ghi ISO sẽ xóa USB được chọn; kiểm tra đúng USB.
resc tạo **thư mục toolkit**, chưa tạo/phát hành ISO boot được.

Trên máy Linux chuẩn bị, tại checkout đã build:

```bash
python3 scripts/rescue.py bundle --output /media/rescue-usb/resc-toolkit
```

Bundle chứa `rescue.py`, `recovery.py`, `setup-rescue.sh`, `bundle.json` với SHA-256; thêm
`mcp-server` nếu có release Linux tại thời điểm đóng gói. Binary chỉ dùng trên CPU
và runtime Linux tương thích. Bundle không kèm Python/ddrescue/SSH hay thư viện hệ
thống; giữ bản gốc gói cài nếu dự kiến cứu hộ offline.

Boot máy hỏng vào **Try Linux**, không chọn cài hệ điều hành lên SSD. Không mở SSD
trong file manager. Cài công cụ vào Live OS (không cài lên ổ đang cứu):

```bash
sudo apt-get update
sudo apt-get install python3 util-linux gddrescue openssh-client
bash scripts/setup-rescue.sh
```

Nếu dùng bundle, chạy các file ngay trong thư mục bundle; không có tiền tố
`scripts/`. Acquisition chỉ cần Python >=3.11, `lsblk`, GNU ddrescue và Linux.

## 3. Nhận diện SSD và ổ đích

```bash
python3 scripts/rescue.py devices
```

Đối chiếu path, model, serial, capacity, transport, logical sector size và
mountpoints. `/dev/nvme0n1` hay `/dev/sda` chỉ là ví dụ, không phải số ổ cố định.
USB enclosure NVMe và M.2 SATA không mặc định tương thích nhau.

Cần một ổ khỏe khác được mount làm đích, ví dụ `/media/live/HEALTHY`. Nguồn và các
volume có cùng backing không được mounted hoặc dùng làm swap. `rescue.py` kiểm
tra ancestry của partition, dm/LVM và md qua sysfs; chặn cùng backing, mounted
source/swap, và filesystem đích mà không truy ra backing (như overlay/NFS/SSHFS).
Nó không tự unmount, swapoff, thay trạng thái read-only hay mount nguồn.
Nguồn block tự lấy geometry; nếu dùng ảnh raw 4Kn làm nguồn acquisition, chỉ định
`--sector-size 4096` (ảnh raw mặc định 512, không thể tự nhận geometry chỉ từ đuôi file).

Nếu mount/swap được báo, xác minh đúng thiết bị trước khi xử lý bằng công cụ OS.
Phần mềm chỉ mở nguồn read-only; không ngăn một tiến trình khác hoặc firmware ghi
vào nguồn. Hardware write blocker vẫn là một biện pháp độc lập.

## 4. Tạo ảnh và tiếp tục acquisition

Xem plan trước (chưa tạo output hoặc đọc nội dung SSD):

```bash
sudo python3 scripts/rescue.py acquire \
  --source /dev/nvme0n1 --case /media/live/HEALTHY/laptop-001
```

Sau khi đối chiếu nguồn/đích, chạy lượt nhanh:

```bash
sudo python3 scripts/rescue.py acquire \
  --source /dev/nvme0n1 --case /media/live/HEALTHY/laptop-001 --run
```

Case mới phải chưa tồn tại. Chương trình tạo `image.img` theo capacity nguồn,
`rescue.map`, `source.json` và lock. Dung lượng trống phải đủ cho toàn ảnh và map.
ddrescue chạy `--no-scrape` ở lượt nhanh. Vùng chưa lấy được giữ zero trong ảnh
sparse; **mapfile mới phân biệt byte đã đọc và vùng chưa biết**.

Nếu bị ngắt, giữ nguyên case và nguồn. Chạy lại cùng lệnh thêm `--resume`. Nguồn
bị đổi identity/capacity sẽ bị từ chối; không sửa tay `source.json` để bỏ chặn.
Nếu thiết bị được đánh số lại sau reboot, cần xác minh và acquisition mới; chưa
có tự rebind theo serial. Identity check không phải xác thực bằng phần cứng.
Nếu thiết bị/enclosure không báo serial, acquisition mới vẫn được phép nhưng
block-device resume bị chặn vì không đủ thông tin ràng buộc nguồn.

Chỉ thử thêm khi thiết bị còn ổn định; retry không bắt buộc:

```bash
sudo python3 scripts/rescue.py acquire \
  --source /dev/nvme0n1 --case /media/live/HEALTHY/laptop-001 \
  --run --resume --phase retry
```

Lượt retry dùng `--retry-passes=2`; không thử vô hạn. Khi muốn chốt để chuyển ảnh,
chạy một lượt fast/resume với `--seal` (hoặc thêm `--seal` ngay lượt đầu nếu đã
quyết định không retry). `sealed.json` lưu phiên bản ddrescue, nguồn, map coverage,
unresolved bytes, SHA-256 của image/map. Sau seal, acquisition không được chạy
tiếp trên case đó. Hash chỉ đọc ảnh đích, không đọc lại toàn SSD lỗi.

Exit code `0`: plan/operation hoàn tất; `2`: acquisition còn vùng chưa đọc;
`1`: lỗi; `130`: ngắt. `PARTIAL` vẫn có thể seal, chuyển và recovery phần còn lại.
Một lần đứt SSH có thể khiến acquisition dừng hoặc còn chạy trên nguồn; kiểm tra
case/tiến trình, lock sẽ chặn chạy trùng. Chưa có background daemon hoặc auto resume.

## 5. Điều khiển máy nguồn qua SSH

Trên Live OS máy nguồn, chuẩn bị OpenSSH server và tài khoản/khóa có quyền đọc
block source, ghi ổ đích. Dịch vụ SSH do người dùng quản lý; resc không mở port,
tự bật root login hoặc thay firewall. Trên console nguồn, xem fingerprint host
key công khai và so sánh khi máy mới kết nối lần đầu:

```bash
ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
```

Trên máy mới, kết nối SSH thường một lần để xác nhận đúng fingerprint và kiểm tra
key login. Các lệnh resc dùng `BatchMode=yes`, `StrictHostKeyChecking=yes` nên từ
chối host chưa tin cậy, host key đổi hoặc yêu cầu password. Không tắt kiểm tra key.
`rescue@192.168.1.20` dưới đây là ví dụ; thay bằng tài khoản có quyền thích hợp.

```bash
python3 scripts/rescue.py remote rescue@192.168.1.20 devices
python3 scripts/rescue.py remote rescue@192.168.1.20 acquire \
  --source /dev/nvme0n1 --case /media/live/HEALTHY/laptop-001
python3 scripts/rescue.py remote rescue@192.168.1.20 acquire \
  --source /dev/nvme0n1 --case /media/live/HEALTHY/laptop-001 --run --seal
```

Script được gửi qua stdin SSH; máy nguồn không cần clone repo hoặc cài Rust.
Đường dẫn `--case` trong `remote acquire` là **trên máy nguồn**, không phải máy mới.
Không có tự khám phá máy LAN; phải biết hostname/IPv4 và tài khoản. Phiên bản này
chưa nhận IPv6 literal, port tùy chọn hoặc interactive sudo; có thể dùng SSH config
cho hostname đã thiết lập, và tài khoản phải có sẵn quyền phù hợp.

## 6. Chuyển ảnh đã seal về máy mới và recovery

Chạy trên Linux/WSL máy mới (native Windows acquisition/pull chưa có):

```bash
python3 scripts/rescue.py pull rescue@192.168.1.20 \
  /media/live/HEALTHY/laptop-001 --case /mnt/recovered/laptop-001
```

Nếu mạng đứt, `.part` được giữ. Dùng lại lệnh thêm `--resume`. Mỗi artifact chỉ
được công bố sau khi đủ size và hash khớp nguồn; cuối cùng có `received.json`.
Nếu hash lệch, không công bố success; giữ case lỗi để điều tra và dùng case đích
mới khi cần tải lại. Không xóa ảnh nguồn sau transfer.

```bash
python3 scripts/recovery.py triage /mnt/recovered/laptop-001/image.img \
  --acquisition-manifest /mnt/recovered/laptop-001/received.json --deep
```

`--acquisition-manifest` xác minh lại hash image/map, giữ `unresolved_bytes` trong
triage; có thể mất thời gian trên ảnh lớn. Geometry dùng giá trị `source.sector_size`
trong acquisition, thêm `--sector-size 4096` nếu là 4Kn. Native MCP không tự nhập
mapfile vào bad-sector map của nó; agent phải giữ acquisition evidence bên cạnh
manifest export. Bundle chạy recovery bằng `python3 recovery.py --binary ./mcp-server ...`.

Xem [hướng dẫn recovery](recovery-guide.vi.md) cho phân vùng/metadata/file video.
Acquisition thành công không có nghĩa tất cả file bên trong còn đúng.
