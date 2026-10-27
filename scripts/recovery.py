#!/usr/bin/env python3
"""Read-only recovery workbench. Python 3.11+, standard library only.

Use --help and docs/recovery-guide.vi.md. Never guesses fragment order or repairs
original media. FFmpeg is an optional, separately installed verification engine.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BINARY = ROOT / "target" / "release" / ("mcp-server.exe" if os.name == "nt" else "mcp-server")
IMAGE_EXTENSIONS = {".img", ".dd", ".raw", ".bin", ".001"}


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def source_path(args):
    if args.choose and args.source:
        raise ValueError("Chỉ dùng source hoặc --choose, không dùng cả hai")
    if args.choose:
        candidates = sorted(p for p in Path(args.choose).resolve().iterdir()
                            if p.is_file() and p.suffix.lower() in IMAGE_EXTENSIONS)
        if not candidates:
            raise ValueError("Không tìm thấy ảnh đĩa .img/.dd/.raw/.bin/.001 trong thư mục")
        for index, path in enumerate(candidates, 1):
            print(f"{index}. {path.name} ({path.stat().st_size:,} bytes)", file=sys.stderr)
        selected = int(input("Chọn số ảnh đĩa nguồn (chỉ đọc): "))
        if not 1 <= selected <= len(candidates):
            raise ValueError("Số lựa chọn không hợp lệ")
        source = candidates[selected - 1]
    elif args.source:
        source = Path(args.source).resolve(strict=True)
    else:
        raise ValueError("Cần đường dẫn nguồn hoặc --choose THƯ_MỤC")
    if not source.is_file():
        raise ValueError("Chỉ nhận file ảnh đĩa thông thường; không nhận raw device/thư mục")
    size = source.stat().st_size
    if not size or size % args.sector_size:
        raise ValueError("Ảnh đĩa phải khác rỗng và chia hết cho sector-size. Với video riêng lẻ dùng verify-media, không thêm padding vào nguồn.")
    # Reject common container formats that must first be converted by trusted tools.
    with source.open("rb") as stream:
        magic = stream.read(8)
    if magic.startswith((b"EVF", b"vhdxfile", b"QFI\xfb", b"KDMV")):
        raise ValueError("Nguồn là container E01/VHDX/QCOW/VMDK; chuyển thành raw bằng công cụ phù hợp trước")
    return source


class Client:
    def __init__(self, binary, timeout=30):
        self.timeout = timeout
        self.responses = queue.Queue(maxsize=4)
        self.identifier = 0
        self.process = subprocess.Popen([str(Path(binary).resolve(strict=True)), "--read-only-enforced"],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.DEVNULL, text=True, encoding="utf-8")

        def read():
            try:
                while True:
                    line = self.process.stdout.readline(1024 * 1024)
                    if not line:
                        break
                    if len(line) >= 1024 * 1024:
                        raise ValueError("MCP response exceeds 1 MiB limit")
                    self.responses.put(json.loads(line), timeout=timeout)
            except Exception as error:
                self.responses.put(error, timeout=timeout)
            finally:
                self.responses.put(None, timeout=timeout)

        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()
        try:
            self.request("initialize", {})
            self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
            self.process.stdin.flush()
        except BaseException:
            self.close()
            raise

    def request(self, method, params):
        self.identifier += 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.identifier,
                                            "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        try:
            response = self.responses.get(timeout=self.timeout)
        except queue.Empty as error:
            raise TimeoutError(f"MCP request {method} timed out after {self.timeout}s") from error
        if isinstance(response, Exception):
            raise response
        if response is None:
            raise RuntimeError("MCP server exited unexpectedly")
        if response.get("id") != self.identifier or "error" in response:
            raise RuntimeError(str(response))
        return response["result"]

    def call(self, name, arguments):
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            raise RuntimeError(result["content"][0]["text"])
        return json.loads(result["content"][0]["text"])

    def wait(self, job, seconds):
        deadline = time.monotonic() + seconds
        try:
            while time.monotonic() < deadline:
                status = self.call("poll_async_job", {"job_id": job})["status"]
                if isinstance(status, dict) and "Completed" in status:
                    return status["Completed"]["result"]
                if isinstance(status, dict) and "Failed" in status:
                    raise RuntimeError(status["Failed"]["error"])
                if status == "Cancelled":
                    raise RuntimeError("Job cancelled; no successful recovery claimed")
                time.sleep(.1)
            raise TimeoutError("Job exceeded --job-timeout")
        except (KeyboardInterrupt, TimeoutError):
            self.call("poll_async_job", {"job_id": job, "cancel": True})
            raise

    def close(self):
        if self.process.poll() is None:
            try:
                self.process.stdin.close()
            except BrokenPipeError:
                pass
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        self.reader.join(timeout=1)
        self.process.stdout.close()


def read_plan(args, source):
    if args.plan:
        if args.length is not None or args.offset != 0:
            raise ValueError("Chỉ dùng --plan hoặc --offset/--length")
        plan = json.loads(Path(args.plan).read_text(encoding="utf-8"))
        if not isinstance(plan, dict) or not isinstance(plan.get("ranges"), list):
            raise ValueError('Plan phải là object {"ranges": [...]}')
        if plan.get("source_sha256") and sha256(source) != plan["source_sha256"]:
            raise ValueError("SHA-256 ảnh nguồn không khớp plan")
        return plan
    if args.length is None:
        raise ValueError("Cần --plan hoặc cả --offset/--length; không đoán độ dài file từ signature")
    return {"ranges": [{"source_byte_offset": args.offset, "length_bytes": args.length}]}


def verify_media(path, decode=False, seconds=120):
    """Use upstream FFmpeg tools without shell, remote protocols or output writes."""
    source = Path(path).resolve(strict=True)
    if not source.is_file():
        raise ValueError("Cần file thông thường đã xuất ra")
    executable = shutil.which("ffprobe")
    if not executable:
        raise RuntimeError("Thiếu ffprobe: cài FFmpeg từ package manager chính thức rồi chạy lại")
    sidecar = source.with_name(source.name + ".manifest.json")
    manifest_verified = None
    if sidecar.exists():
        manifest = json.loads(sidecar.read_text(encoding="utf-8"))
        manifest_verified = sha256(source) == manifest["sha256_hash"] and source.stat().st_size == manifest["size_bytes"]
        if not manifest_verified:
            raise ValueError("File không khớp SHA-256/kích thước trong provenance manifest")
    # Temporary files avoid unbounded subprocess pipes; -show_entries bounds normal
    # output. Large/malicious probe output is rejected before loading into memory.
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
        result = subprocess.run([executable, "-v", "error", "-protocol_whitelist", "file",
                                 "-format_whitelist", "mov", "-probesize", "8388608", "-analyzeduration", "10000000",
                                 "-show_entries", "format=format_name,duration,size:stream=index,codec_type,codec_name,width,height,duration",
                                 "-of", "json", str(source)], stdin=subprocess.DEVNULL, stdout=output, stderr=errors, timeout=seconds)
        if output.tell() > 1024 * 1024 or errors.tell() > 1024 * 1024:
            raise RuntimeError("ffprobe output exceeds bounded diagnostic budget")
        output.seek(0); errors.seek(0)
        info = json.load(output) if result.returncode == 0 else {}
        report = {"engine": "FFmpeg ffprobe (external executable)", "probe_passed": result.returncode == 0 and any(s.get("codec_type") == "video" for s in info.get("streams", [])),
                  "manifest_hash_verified": manifest_verified, "decode_tested": False,
                  "decode_passed": None, "media": info, "errors": errors.read(4096).decode("utf-8", "replace")}
    if decode and report["probe_passed"]:
        ffmpeg = shutil.which("ffmpeg")
        if not ffmpeg:
            raise RuntimeError("Thiếu ffmpeg cho kiểm tra decode")
        with tempfile.TemporaryFile() as errors:
            result = subprocess.run([ffmpeg, "-nostdin", "-v", "error", "-xerror", "-threads", "2",
                                     "-protocol_whitelist", "file", "-format_whitelist", "mov", "-i", str(source),
                                     "-map", "0:v?", "-map", "0:a?", "-f", "null", "-"],
                                    stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=errors, timeout=seconds)
            errors.seek(0)
            report.update(decode_tested=True, decode_passed=result.returncode == 0,
                          decode_errors=errors.read(4096).decode("utf-8", "replace"))
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default=os.environ.get("RECOVERYAI_MCP_BIN", str(DEFAULT_BINARY)))
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("doctor", help="Kiểm tra công cụ, không đọc giá trị secrets")
    config = commands.add_parser("mcp-config", help="In JSON MCP đúng đường dẫn máy hiện tại")
    config.add_argument("--session-root", required=True)
    media = commands.add_parser("verify-media", help="Kiểm tra MP4/MOV đã xuất bằng FFmpeg/ffprobe")
    media.add_argument("file")
    media.add_argument("--decode", action="store_true")
    media.add_argument("--timeout", type=int, default=120)
    for command in ["triage", "scan", "analyze-video", "analyze-file", "analyze-partition", "deleted-scan", "recover", "extract-tree", "clone"]:
        sub = commands.add_parser(command)
        sub.add_argument("source", nargs="?")
        sub.add_argument("--choose", metavar="DIRECTORY", help="Chọn ảnh đĩa bằng danh sách số")
        sub.add_argument("--sector-size", type=int, choices=[512, 4096], default=512)
        sub.add_argument("--job-timeout", type=int, default=3600)
        sub.add_argument("--report", help="Ghi JSON vào file mới, không ghi đè")
        if command in {"triage", "scan"}:
            sub.add_argument("--start-lba", type=int, default=0)
            sub.add_argument("--end-lba", type=int)
        if command == "triage":
            sub.add_argument("--deep", action="store_true", help="Quét toàn bộ các route signature/VBR/video; có thể mất nhiều giờ")
        if command == "scan":
            sub.add_argument("--kind", choices=["SIMD_MAGIC_CARVE", "SIMD_VBR_HUNT", "ISO_BMFF_BOX_SCAN", "FAT32_DELETED_SCAN"], default="SIMD_MAGIC_CARVE")
        if command in {"analyze-video", "analyze-file", "recover"}:
            sub.add_argument("--plan")
            sub.add_argument("--offset", type=int, default=0)
            sub.add_argument("--length", type=int)
        if command == "analyze-file":
            sub.add_argument("--format", choices=["AUTO", "JPEG", "PNG", "MP4", "MOV"], default="AUTO")
        if command in {"scan", "deleted-scan", "analyze-partition"}:
            sub.add_argument("--partition-start-lba", type=int, required=command == "deleted-scan")
        if command in {"scan", "deleted-scan", "triage"}:
            sub.add_argument("--max-clusters", type=int, default=256)
        if command in {"recover", "extract-tree"}:
            sub.add_argument("--output-dir", required=True)
        if command == "recover":
            sub.add_argument("--filename", default="recovered_file.bin")
        if command == "extract-tree":
            sub.add_argument("--partition-start-lba", type=int, required=True)
        if command == "clone":
            sub.add_argument("--destination", required=True)
            sub.add_argument("--mapfile")
    args = parser.parse_args(argv)
    if args.command == "doctor":
        report = {"python": sys.version.split()[0], "platform": sys.platform,
                  "binary_exists": Path(args.binary).is_file(),
                  "tools": {name: shutil.which(name) for name in ["cargo", "rustc", "node", "ffprobe", "ffmpeg"]},
                  "session_root_configured": bool(os.environ.get("RESC_SESSION_ROOT")),
                  "next_step": "Build: cargo build --release --locked. No pip/npm dependencies. See docs/recovery-guide.vi.md"}
    elif args.command == "mcp-config":
        report = {"mcpServers": {"resc": {"command": str(Path(args.binary).resolve()),
                    "args": ["--read-only-enforced"], "env": {"RESC_SESSION_ROOT": str(Path(args.session_root).resolve())}}}}
    elif args.command == "verify-media":
        report = verify_media(args.file, args.decode, args.timeout)
    else:
        source = source_path(args)
        client = Client(args.binary)
        try:
            opened = client.call("open_recovery_session", {"target_path": str(source), "sector_size": args.sector_size})
            sid = opened["session_id"]
            if args.command in {"triage", "scan"}:
                report = {"source": str(source), "session": opened}
                if args.command == "triage":
                    report["diagnosis"] = client.call("diagnose_recovery", {"session_id": sid})
                    try:
                        report["case1_partition_boot"] = client.call("analyze_partition_recovery", {"session_id": sid})
                    except RuntimeError as error:
                        report["case1_partition_boot"] = {"status": "FAILED", "error": str(error)}
                    report["analysis_failures"] = int(report["case1_partition_boot"].get("status") == "FAILED")
                kinds = [args.kind] if args.command == "scan" else (["SIMD_MAGIC_CARVE", "SIMD_VBR_HUNT", "ISO_BMFF_BOX_SCAN"] if args.deep else [])
                report["scans"] = []
                requests = []
                for kind in kinds:
                    if kind == "FAT32_DELETED_SCAN":
                        if args.partition_start_lba is None:
                            raise ValueError("FAT32_DELETED_SCAN cần --partition-start-lba đã xác minh")
                        params = {"partition_start_lba": args.partition_start_lba, "max_clusters": args.max_clusters}
                    else:
                        params = {"start_lba": args.start_lba}
                        if args.end_lba is not None:
                            params["end_lba"] = args.end_lba
                    requests.append((kind, params))
                if args.command == "triage" and args.deep and args.start_lba == 0 and args.end_lba is None:
                    for volume in report["diagnosis"]["volumes"]:
                        if volume["classification"] == "FAT32" and volume["boot_geometry_valid"]:
                            requests.append(("FAT32_DELETED_SCAN", {"partition_start_lba": volume["start_lba"], "max_clusters": args.max_clusters}))
                failures = 0
                for kind, params in requests:
                    try:
                        job = client.call("start_async_disk_job", {"session_id": sid, "job_type": kind, "params": params})
                        print(f"{kind}: {job['job_id']}", file=sys.stderr)
                        result = client.wait(job["job_id"], args.job_timeout)
                        report["scans"].append({"kind": kind, "params": params, "status": "COMPLETED", "job_id": job["job_id"], "result": result,
                            "first_page": client.call("read_scan_hits", {"session_id": sid, "job_id": job["job_id"], "limit": 100}),
                            "all_hits_file": str(Path(opened["session_dir"]) / (job["job_id"] + "_hits.jsonl"))})
                    except (RuntimeError, TimeoutError) as error:
                        failures += 1
                        report["scans"].append({"kind": kind, "params": params, "status": "FAILED", "error": str(error)})
                report["scan_failures"] = failures
            elif args.command == "analyze-partition":
                params = {"session_id": sid}
                if args.partition_start_lba is not None:
                    params["partition_start_lba"] = args.partition_start_lba
                report = client.call("analyze_partition_recovery", params)
            elif args.command == "deleted-scan":
                job = client.call("start_async_disk_job", {"session_id": sid, "job_type": "FAT32_DELETED_SCAN",
                    "params": {"partition_start_lba": args.partition_start_lba, "max_clusters": args.max_clusters}})
                report = {"job_id": job["job_id"], "session": opened, "result": client.wait(job["job_id"], args.job_timeout),
                    "first_page": client.call("read_scan_hits", {"session_id": sid, "job_id": job["job_id"], "limit": 100}),
                    "all_hits_file": str(Path(opened["session_dir"]) / (job["job_id"] + "_hits.jsonl"))}
            elif args.command == "extract-tree":
                job = client.call("extract_recovered_data", {"session_id": sid, "mode": "TSK_TREE_EXTRACT",
                    "output_dir": str(Path(args.output_dir).resolve()),
                    "params": {"partition_start_lba": args.partition_start_lba}})
                report = client.wait(job["job_id"], args.job_timeout)
            elif args.command == "clone":
                params = {"destination_image": str(Path(args.destination).resolve())}
                if args.mapfile:
                    params["mapfile"] = str(Path(args.mapfile).resolve())
                job = client.call("start_async_disk_job", {"session_id": sid, "job_type": "DD_RESCUE_CLONE", "params": params})
                report = client.wait(job["job_id"], args.job_timeout)
            else:
                plan = read_plan(args, source)
                if args.command in {"analyze-video", "analyze-file"}:
                    params = {"session_id": sid, "ranges": plan["ranges"]}
                    if args.command == "analyze-file":
                        params["format"] = args.format
                    report = client.call("analyze_file_candidate" if args.command == "analyze-file" else "analyze_video_candidate", params)
                else:
                    job = client.call("extract_recovered_data", {"session_id": sid, "mode": "CARVE_BY_BYTE_RANGES",
                        "output_dir": str(Path(args.output_dir).resolve()), "params": {"ranges": plan["ranges"], "filename": args.filename}})
                    report = client.wait(job["job_id"], args.job_timeout)
        finally:
            client.close()
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if getattr(args, "report", None):
        with Path(args.report).open("x", encoding="utf-8") as stream:
            stream.write(rendered)
    print(rendered, end="")
    if args.command == "verify-media" and (not report["probe_passed"] or (args.decode and report["decode_passed"] is not True)):
        return 2
    if args.command in {"triage", "scan"} and (report.get("scan_failures", 0) or report.get("analysis_failures", 0)):
        return 2
    if args.command in {"analyze-video", "analyze-file"} and report["report"]["classification"] != "STRUCTURE_CONSISTENT":
        return 2
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, TimeoutError, queue.Empty, subprocess.SubprocessError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print("Đã yêu cầu hủy; kiểm tra session và các file .part trước khi chạy lại.", file=sys.stderr)
        sys.exit(130)
