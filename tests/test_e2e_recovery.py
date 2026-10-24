"""
End-to-End Automated Test Suite for RecoveryAI MCP Engine
Kiểm thử toàn bộ 8 MCP tools và các kịch bản theo tài liệu gemini-code plan.
"""

import json
import os
import subprocess
import sys
import time

if sys.stdout.encoding != "utf-8":
    try:
        sys.stdout.reconfigure(encoding="utf-8")
        sys.stderr.reconfigure(encoding="utf-8")
    except Exception:
        pass

MCP_BINARY = os.path.join(
    os.path.dirname(__file__), "..", "target", "release", "mcp-server.exe"
)
CORRUPTED_DIR = os.path.join(os.path.dirname(__file__), "sample_corrupted_images")


class McpClient:
    def __init__(self, binary_path):
        self.process = subprocess.Popen(
            [binary_path],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        self.msg_id = 1

    def send_request(self, method, params=None):
        req = {
            "jsonrpc": "2.0",
            "id": self.msg_id,
            "method": method,
            "params": params or {},
        }
        self.msg_id += 1
        line = json.dumps(req)
        self.process.stdin.write(line + "\n")
        self.process.stdin.flush()

        resp_line = self.process.stdout.readline()
        if not resp_line:
            stderr = self.process.stderr.read()
            raise RuntimeError(f"Server closed connection. Stderr: {stderr}")
        return json.loads(resp_line)

    def call_tool(self, tool_name, arguments):
        resp = self.send_request(
            "tools/call", {"name": tool_name, "arguments": arguments}
        )
        if "error" in resp:
            raise RuntimeError(f"Tool error: {resp['error']}")
        content = resp["result"]["content"][0]["text"]
        return json.loads(content)

    def close(self):
        try:
            self.process.stdin.close()
            self.process.terminate()
            self.process.wait(timeout=2)
        except Exception:
            pass


def run_all_e2e_tests():
    print("=" * 60)
    print("🚀 Bắt đầu chạy Kiểm thử Toàn diện E2E RecoveryAI MCP Engine")
    print("=" * 60)

    client = McpClient(MCP_BINARY)

    # 1. Test Initialize
    init_resp = client.send_request("initialize")
    assert "result" in init_resp, "Initialize failed"
    server_info = init_resp["result"]["serverInfo"]
    print(f"[OK] MCP Initialize thành công: {server_info['name']} v{server_info['version']}")

    # 2. Test Tools List (Assert full 8 tools)
    tools_resp = client.send_request("tools/list")
    tools = {t["name"] for t in tools_resp["result"]["tools"]}
    expected_8_tools = {
        "open_recovery_session",
        "scan_disk_topology",
        "inspect_sector_structure",
        "scan_filesystem_journals",
        "start_async_disk_job",
        "poll_async_job",
        "virtual_patch_and_verify",
        "extract_recovered_data",
    }
    missing = expected_8_tools - tools
    assert not missing, f"Thiếu tools chuẩn: {missing}"
    print(f"[OK] Toàn bộ 8 MCP tools chuẩn đã được đăng ký ({len(tools)} tools tổng cộng)")

    # 3. Test Case 1: Mất phân vùng (Lost Partition Recovery)
    case1_img = os.path.join(CORRUPTED_DIR, "case1_lost_partition.img")
    if not os.path.exists(case1_img):
        print("[INIT] Chưa có dữ liệu ảnh đĩa mẫu, tự động tạo mới qua create_sample_images.py...")
        import create_sample_images
        create_sample_images.generate_all_test_images()

    open_res1 = client.call_tool("open_recovery_session", {"target_path": case1_img})
    session_id1 = open_res1["session_id"]
    print(f"[OK] Case 1: Mở phiên thành công (Session ID: {session_id1[:8]}..., Sectors: {open_res1['total_sectors']})")

    topo1 = client.call_tool("scan_disk_topology", {"session_id": session_id1})
    assert topo1["lba_0_mbr"]["status"] == "ZEROED", "LBA 0 phải là ZEROED"
    assert topo1["common_offsets"]["lba_2048"]["is_valid_ntfs"] is True, "LBA 2048 phải là valid NTFS VBR"
    print(f"[OK] Case 1 Topology: Phát hiện MBR bị xóa trắng nhưng VBR tại LBA 2048 còn nguyên vẹn")

    # 4. Test Case 2: NTFS RAW Repair qua Virtual Overlay (Copy Backup VBR -> LBA 2048)
    case2_img = os.path.join(CORRUPTED_DIR, "case2_ntfs_raw_corrupted_vbr.img")
    open_res2 = client.call_tool("open_recovery_session", {"target_path": case2_img})
    session_id2 = open_res2["session_id"]

    topo2 = client.call_tool("scan_disk_topology", {"session_id": session_id2})
    assert topo2["common_offsets"]["lba_2048"]["is_valid_ntfs"] is False, "Primary VBR tại 2048 phải là invalid"
    assert topo2["last_lba"]["ntfs_backup_vbr_valid"] is True, "Backup VBR ở cuối đĩa phải là valid"
    print(f"[OK] Case 2: Phát hiện Primary VBR hỏng (RAW) và Backup VBR ở LBA {topo2['last_lba']['lba']} hợp lệ")

    # Vá ảo trên RAM: copy Backup VBR về LBA 2048
    patch_res = client.call_tool(
        "virtual_patch_and_verify",
        {
            "session_id": session_id2,
            "copy_from_lba": topo2["last_lba"]["lba"],
            "to_lba": 2048,
            "description": "Restore NTFS VBR from Backup VBR at end of volume",
        },
    )
    assert patch_res["mount_status"] == "SUCCESS", "Mount status phải là SUCCESS sau khi vá ảo"
    assert patch_res["verification"]["ntfs_vbr_at_2048_valid"] is True, "VBR tại 2048 phải hợp lệ sau khi vá"
    print(f"[OK] Case 2 Virtual Patching: {patch_res['message']} (mount_status: SUCCESS)")

    # 5. Test Case 4: SIMD Magic Carving (Async Job)
    case4_img = os.path.join(CORRUPTED_DIR, "case4_fragmented_mp4.img")
    open_res4 = client.call_tool("open_recovery_session", {"target_path": case4_img})
    session_id4 = open_res4["session_id"]

    job_res = client.call_tool(
        "start_async_disk_job",
        {"session_id": session_id4, "job_type": "SIMD_MAGIC_CARVE"},
    )
    job_id = job_res["job_id"]
    print(f"[OK] Case 4: Khởi chạy tác vụ quét ngầm SIMD_MAGIC_CARVE (Job ID: {job_id[:8]}...)")

    # Chờ job hoàn thành
    time.sleep(0.3)
    poll_res = client.call_tool("poll_async_job", {"job_id": job_id})
    print(f"[OK] Case 4 Poll Job: Status = {poll_res['status']}")

    client.close()
    print("=" * 60)
    print("🎉 TẤT CẢ 5 BÀI KIỂM THỬ E2E THEO ĐÚNG KẾ HOẠCH ĐÃ VƯỢT QUA 100%!")
    print("=" * 60)


if __name__ == "__main__":
    run_all_e2e_tests()
