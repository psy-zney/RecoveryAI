"""
Script sinh ảnh động demo.gif chất lượng cao cho RecoveryAI README.md.
Mô phỏng màn hình chia đôi (Split-screen 960x540):
- Trái: Trạng thái ổ USB bị RAW -> Được vá bằng RAM Overlay -> Phục hồi dữ liệu
- Phải: Phiên làm việc Claude Desktop/Cursor gọi 8 MCP Tools tự động khôi phục trong 15s.
"""

import os
from PIL import Image, ImageDraw, ImageFont

WIDTH = 960
HEIGHT = 540
DIVIDER_X = 430

# Colors (Tailored modern dark theme)
BG_COLOR = (15, 17, 23)           # #0f1117 (Dark slate)
PANEL_LEFT_BG = (20, 23, 31)      # #14171f
PANEL_RIGHT_BG = (24, 27, 36)     # #181b24
HEADER_BG = (11, 13, 19)
TEXT_WHITE = (235, 240, 245)
TEXT_MUTED = (140, 150, 165)
TEXT_GREEN = (72, 199, 142)       # Success
TEXT_RED = (245, 101, 101)        # Error / RAW
TEXT_YELLOW = (236, 201, 75)      # Warning
TEXT_CYAN = (99, 179, 237)        # Tools / Tech
BORDER_COLOR = (38, 44, 58)
BOX_BG = (28, 32, 44)
ALERT_BG = (45, 25, 30)
TOOL_BOX_BG = (18, 22, 32)
USER_BUBBLE_BG = (35, 42, 60)

# Fonts
FONT_CONSOLAS_PATH = r"C:\Windows\Fonts\consola.ttf"
FONT_SEGOE_PATH = r"C:\Windows\Fonts\segoeui.ttf"
FONT_SEGOE_BOLD_PATH = r"C:\Windows\Fonts\segoeuib.ttf"

font_title = ImageFont.truetype(FONT_SEGOE_BOLD_PATH, 13)
font_ui = ImageFont.truetype(FONT_SEGOE_PATH, 12)
font_ui_bold = ImageFont.truetype(FONT_SEGOE_BOLD_PATH, 12)
font_ui_small = ImageFont.truetype(FONT_SEGOE_PATH, 11)
font_mono = ImageFont.truetype(FONT_CONSOLAS_PATH, 12)
font_mono_small = ImageFont.truetype(FONT_CONSOLAS_PATH, 11)
font_mono_bold = ImageFont.truetype(FONT_CONSOLAS_PATH, 12)

def draw_base_frame():
    img = Image.new("RGB", (WIDTH, HEIGHT), BG_COLOR)
    draw = ImageDraw.Draw(img)

    # Header bar
    draw.rectangle([(0, 0), (WIDTH, 34)], fill=HEADER_BG)
    draw.line([(0, 34), (WIDTH, 34)], fill=BORDER_COLOR, width=1)

    # Window controls (macOS style dots)
    draw.ellipse([(14, 12), (24, 22)], fill=(245, 101, 101))
    draw.ellipse([(32, 12), (42, 22)], fill=(236, 201, 75))
    draw.ellipse([(50, 12), (60, 22)], fill=(72, 199, 142))

    # Header Title
    draw.text((75, 10), "RecoveryAI Forensics Engine — MCP Autonomous Recovery Session", font=font_title, fill=TEXT_WHITE)
    draw.text((WIDTH - 250, 11), "100% Read-Only DMA | CoW Overlay", font=font_mono_small, fill=TEXT_CYAN)

    # Split background panels
    draw.rectangle([(0, 35), (DIVIDER_X, HEIGHT - 28)], fill=PANEL_LEFT_BG)
    draw.rectangle([(DIVIDER_X + 1, 35), (WIDTH, HEIGHT - 28)], fill=PANEL_RIGHT_BG)
    draw.line([(DIVIDER_X, 35), (DIVIDER_X, HEIGHT - 28)], fill=BORDER_COLOR, width=2)

    # Bottom status bar
    draw.rectangle([(0, HEIGHT - 28), (WIDTH, HEIGHT)], fill=HEADER_BG)
    draw.line([(0, HEIGHT - 28), (WIDTH, HEIGHT - 28)], fill=BORDER_COLOR, width=1)

    # Column Headers
    # Left Column Header
    draw.rectangle([(12, 44), (DIVIDER_X - 12, 70)], fill=BOX_BG, outline=BORDER_COLOR)
    draw.text((22, 49), "TARGET DRIVE: USB Cruzer 32GB (PhysicalDrive1)", font=font_ui_bold, fill=TEXT_WHITE)

    # Right Column Header
    draw.rectangle([(DIVIDER_X + 12, 44), (WIDTH - 12, 70)], fill=BOX_BG, outline=BORDER_COLOR)
    draw.text((DIVIDER_X + 22, 49), "CLAUDE DESKTOP / CURSOR — RECOVERYAI MCP", font=font_ui_bold, fill=TEXT_CYAN)

    return img, draw


def render_frame(phase: int, sub_tick: int = 0):
    """
    phase:
      0: Initial state (Disk RAW, user prompts)
      1: AI calls open_recovery_session & scan_disk_topology
      2: AI finds Backup VBR & calls virtual_patch_and_verify
      3: AI calls extract_recovered_data & completes extraction
      4: Success banner & final state
    """
    img, draw = draw_base_frame()

    # ================= LEFT COLUMN: PHYSICAL DISK INSPECTION =================
    y = 80
    # Drive info card
    draw.rectangle([(12, y), (DIVIDER_X - 12, y + 54)], fill=BOX_BG, outline=BORDER_COLOR)
    draw.text((22, y + 8), "Device: \\\\.\\PhysicalDrive1 [SanDisk USB 3.0]", font=font_ui, fill=TEXT_MUTED)
    draw.text((22, y + 28), "Capacity: 31,998 MB  |  Sectors: 62,499,840  |  Sector: 512B", font=font_mono_small, fill=TEXT_WHITE)
    y += 64

    if phase == 0:
        # RAW Error Alert Dialog
        draw.rectangle([(12, y), (DIVIDER_X - 12, y + 80)], fill=ALERT_BG, outline=TEXT_RED)
        draw.text((24, y + 10), "❌ CRITICAL: FILE SYSTEM RAW / UNREADABLE", font=font_ui_bold, fill=TEXT_RED)
        draw.text((24, y + 32), "Windows Alert: \"You need to format the disk in drive E:\nbefore you can use it. Volume unreadable.\"", font=font_ui_small, fill=(240, 180, 180))
        y += 90

        # Corrupted Hex Preview
        draw.rectangle([(12, y), (DIVIDER_X - 12, y + 120)], fill=HEADER_BG, outline=BORDER_COLOR)
        draw.text((22, y + 8), "Hex View: LBA 2048 (Partition VBR)", font=font_mono_bold, fill=TEXT_YELLOW)
        draw.text((22, y + 30), "00000800: 00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00", font=font_mono_small, fill=TEXT_MUTED)
        draw.text((22, y + 48), "00000810: 00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00", font=font_mono_small, fill=TEXT_MUTED)
        draw.text((22, y + 66), "00000820: 00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00", font=font_mono_small, fill=TEXT_MUTED)
        draw.text((22, y + 88), "Shannon Entropy: 0.0000 (All Zeros / Corrupted)", font=font_mono_small, fill=TEXT_RED)
        y += 130

        # Safety badge
        draw.rectangle([(12, y), (DIVIDER_X - 12, y + 40)], fill=BOX_BG, outline=BORDER_COLOR)
        draw.text((22, y + 12), "Physical Protection: GENERIC_READ ONLY", font=font_ui_bold, fill=TEXT_CYAN)

    elif phase == 1:
        # Scanning Topology
        draw.rectangle([(12, y), (DIVIDER_X - 12, y + 90)], fill=BOX_BG, outline=TEXT_YELLOW)
        draw.text((24, y + 10), "🔍 SCANNING DISK TOPOLOGY...", font=font_ui_bold, fill=TEXT_YELLOW)
        draw.text((24, y + 32), "• LBA 0 (MBR): Valid Signature [0x55AA]", font=font_mono_small, fill=TEXT_GREEN)
        draw.text((24, y + 50), "• LBA 2048 (Primary VBR): CORRUPTED (0x00)", font=font_mono_small, fill=TEXT_RED)
        draw.text((24, y + 68), "• LBA 62,499,999 (Backup VBR): INTACT [NTFS]", font=font_mono_small, fill=TEXT_GREEN)
        y += 100

        # Backup VBR Hex Preview
        draw.rectangle([(12, y), (DIVIDER_X - 12, y + 120)], fill=HEADER_BG, outline=BORDER_COLOR)
        draw.text((22, y + 8), "Hex View: LBA 62,499,999 (Backup VBR)", font=font_mono_bold, fill=TEXT_GREEN)
        draw.text((22, y + 30), "0773537F: EB 52 90 4E 54 46 53 20  00 02 08 00 00 00 00 00", font=font_mono_small, fill=TEXT_CYAN)
        draw.text((22, y + 48), "0773538F: F8 00 00 00 3F 00 FF 00  00 08 00 00 00 00 00 00", font=font_mono_small, fill=TEXT_CYAN)
        draw.text((22, y + 66), "OEM ID: \"NTFS    \"  |  Bytes/Sector: 512  |  Cluster: 4KB", font=font_mono_small, fill=TEXT_WHITE)
        draw.text((22, y + 88), "Shannon Entropy: 4.8124 (Healthy VBR Structure)", font=font_mono_small, fill=TEXT_GREEN)
        y += 130

    elif phase >= 2:
        # RAM Overlay Active Card
        draw.rectangle([(12, y), (DIVIDER_X - 12, y + 85)], fill=(18, 38, 28), outline=TEXT_GREEN)
        draw.text((24, y + 10), "⚡ RAM COPY-ON-WRITE OVERLAY ACTIVE", font=font_ui_bold, fill=TEXT_GREEN)
        draw.text((24, y + 30), "• Physical Drive: 100% UNTOUCHED (0 writes)", font=font_ui_small, fill=TEXT_WHITE)
        draw.text((24, y + 48), "• Virtual Patch: Copied LBA 62499999 -> 2048", font=font_mono_small, fill=TEXT_CYAN)
        draw.text((24, y + 66), "• In-Memory Mount: SUCCESS (NTFS 3.1 OK)", font=font_mono_small, fill=TEXT_GREEN)
        y += 95

        # Recovered files view
        draw.rectangle([(12, y), (DIVIDER_X - 12, y + 175)], fill=HEADER_BG, outline=BORDER_COLOR)
        draw.text((22, y + 8), "RECOVERED FILES (C:\\RecoveryOutput\\)", font=font_mono_bold, fill=TEXT_CYAN)
        y_f = y + 30
        files = [
            ("📁 DCIM/Photos/DSC_0842.JPG", "4.8 MB", "100% Intact"),
            ("📁 Documents/Financial_Report_2026.xlsx", "1.2 MB", "100% Intact"),
            ("📁 Projects/Customer_Database.sqlite", "14.5 MB", "100% Intact"),
            ("📁 Videos/Drone_Shot_4K.mp4", "52.8 MB", "Fragment-Rebuilt"),
            ("📁 System/Security_Keys.kdbx", "240 KB", "100% Intact"),
        ]
        for f_name, f_size, f_status in files:
            color = TEXT_GREEN if f_status == "100% Intact" else TEXT_YELLOW
            draw.text((22, y_f), f_name, font=font_mono_small, fill=TEXT_WHITE)
            draw.text((DIVIDER_X - 125, y_f), f_size, font=font_mono_small, fill=TEXT_MUTED)
            draw.text((DIVIDER_X - 55, y_f), "✔", font=font_ui_bold, fill=color)
            y_f += 24

        draw.line([(22, y_f + 4), (DIVIDER_X - 22, y_f + 4)], fill=BORDER_COLOR)
        draw.text((22, y_f + 10), "Total: 12 Files (73.5 MB) Restored Without Loss", font=font_ui_bold, fill=TEXT_GREEN)


    # ================= RIGHT COLUMN: AI AGENT WORKFLOW =================
    rx = DIVIDER_X + 16
    ry = 80
    r_width = WIDTH - rx - 16

    # User message bubble
    draw.rectangle([(rx, ry), (rx + r_width, ry + 42)], fill=USER_BUBBLE_BG, outline=BORDER_COLOR)
    draw.text((rx + 10, ry + 6), "User:", font=font_ui_bold, fill=TEXT_MUTED)
    draw.text((rx + 10, ry + 22), "\"USB của tôi bị báo lỗi RAW không mở được, hãy kiểm tra và cứu các file!\"", font=font_ui, fill=TEXT_WHITE)
    ry += 52

    # Agent Response & Tools sequence
    if phase >= 0:
        # Step 1: open_recovery_session
        color_step1 = TEXT_GREEN if phase >= 1 else TEXT_YELLOW
        draw.rectangle([(rx, ry), (rx + r_width, ry + 44)], fill=TOOL_BOX_BG, outline=BORDER_COLOR)
        draw.text((rx + 8, ry + 6), "⚙ tool: open_recovery_session", font=font_mono_bold, fill=TEXT_CYAN)
        if phase == 0:
            draw.text((rx + 8, ry + 24), "Opening \\\\.\\PhysicalDrive1 (read-only DMA)...", font=font_mono_small, fill=TEXT_YELLOW)
        else:
            draw.text((rx + 8, ry + 24), "✔ Session s_88b1 established. Sectors: 62,499,840.", font=font_mono_small, fill=TEXT_GREEN)
        ry += 52

    if phase >= 1:
        # Step 2: scan_disk_topology
        draw.rectangle([(rx, ry), (rx + r_width, ry + 56)], fill=TOOL_BOX_BG, outline=BORDER_COLOR)
        draw.text((rx + 8, ry + 6), "⚙ tool: scan_disk_topology", font=font_mono_bold, fill=TEXT_CYAN)
        draw.text((rx + 8, ry + 22), "• LBA 2048: Primary VBR corrupted (All zeros)", font=font_mono_small, fill=TEXT_RED)
        draw.text((rx + 8, ry + 38), "✔ LBA 62,499,999: Intact Backup NTFS VBR found!", font=font_mono_small, fill=TEXT_GREEN)
        ry += 64

    if phase >= 2:
        # Step 3: virtual_patch_and_verify
        draw.rectangle([(rx, ry), (rx + r_width, ry + 56)], fill=TOOL_BOX_BG, outline=BORDER_COLOR)
        draw.text((rx + 8, ry + 6), "⚙ tool: virtual_patch_and_verify", font=font_mono_bold, fill=TEXT_CYAN)
        draw.text((rx + 8, ry + 22), "params: {copy_from: 62499999, to: 2048}", font=font_mono_small, fill=TEXT_MUTED)
        draw.text((rx + 8, ry + 38), "✔ Virtual Patch verified. In-memory Mount: SUCCESS!", font=font_mono_small, fill=TEXT_GREEN)
        ry += 64

    if phase >= 3:
        # Step 4: extract_recovered_data
        draw.rectangle([(rx, ry), (rx + r_width, ry + 60)], fill=TOOL_BOX_BG, outline=BORDER_COLOR)
        draw.text((rx + 8, ry + 6), "⚙ tool: extract_recovered_data", font=font_mono_bold, fill=TEXT_CYAN)
        draw.text((rx + 8, ry + 22), "mode: TSK_TREE_EXTRACT -> C:\\RecoveryOutput\\", font=font_mono_small, fill=TEXT_MUTED)
        draw.text((rx + 8, ry + 38), "✔ 12/12 files exported (73.5 MB) in 14.8s. SHA256 verified.", font=font_mono_small, fill=TEXT_GREEN)
        ry += 68

    if phase >= 4:
        # AI final message
        draw.rectangle([(rx, ry), (rx + r_width, ry + 56)], fill=(20, 45, 30), outline=TEXT_GREEN)
        draw.text((rx + 10, ry + 8), "Claude (RecoveryAI):", font=font_ui_bold, fill=TEXT_GREEN)
        draw.text((rx + 10, ry + 26), "\"Toàn bộ dữ liệu USB đã được cứu an toàn ra thư mục\nC:\\RecoveryOutput mà không ghi đè bất kỳ 1 byte nào lên đĩa!\"", font=font_ui_small, fill=TEXT_WHITE)

    # Bottom status bar text
    status_time = ["00:01.2s", "00:04.5s", "00:09.1s", "00:13.4s", "00:14.8s"][min(phase, 4)]
    status_msg = f"Status: COMPLETED | Elapsed: {status_time} | RAM COW: 1.2 MB | Physical Writes: 0 BYTES (SAFE)"
    draw.text((16, HEIGHT - 20), status_msg, font=font_mono_small, fill=TEXT_GREEN if phase >= 3 else TEXT_YELLOW)
    draw.text((WIDTH - 240, HEIGHT - 20), "GitHub: psy-zney/RecoveryAI", font=font_mono_small, fill=TEXT_MUTED)

    return img


def generate_gif(output_path="assets/demo.gif"):
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    frames = []

    # Sequence of phases to simulate smooth 45-second feel compressed into compact GIF loop
    # Phase 0: 12 frames (RAW disk alert, initial state)
    for _ in range(12):
        frames.append(render_frame(0))

    # Phase 1: 10 frames (Scanning topology)
    for _ in range(10):
        frames.append(render_frame(1))

    # Phase 2: 12 frames (Virtual patching on RAM)
    for _ in range(12):
        frames.append(render_frame(2))

    # Phase 3: 12 frames (Extracting recovered files)
    for _ in range(12):
        frames.append(render_frame(3))

    # Phase 4: 20 frames (Final success report, stay visible)
    for _ in range(20):
        frames.append(render_frame(4))

    print(f"Generating optimized animated GIF with {len(frames)} frames...")

    # Convert to adaptive 256-color palette for crisp terminal text and tiny file size
    processed_frames = []
    for f in frames:
        p_frame = f.convert("P", palette=Image.ADAPTIVE, colors=256)
        processed_frames.append(p_frame)

    processed_frames[0].save(
        output_path,
        save_all=True,
        append_images=processed_frames[1:],
        duration=300, # 300ms per frame
        loop=0,
        optimize=True,
    )

    size_kb = os.path.getsize(output_path) / 1024
    print(f"[OK] Created {output_path} successfully! Size: {size_kb:.1f} KB")


if __name__ == "__main__":
    generate_gif()
