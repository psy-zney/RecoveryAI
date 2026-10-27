"""Recovery regressions: synthetic fragments, exact provenance, real FFmpeg decode."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
from test_e2e_recovery import McpClient

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("workbench", ROOT / "scripts" / "recovery.py")
workbench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workbench)


def atom(kind, payload):
    return struct.pack(">I", 8 + len(payload)) + kind + payload


def structural_video():
    ftyp = atom(b"ftyp", b"isom" + bytes(4) + b"isom")
    mdat = atom(b"mdat", bytes(70003))
    table = atom(b"stco", bytes(4) + struct.pack(">II", 1, len(ftyp) + 8))
    for kind in [b"stbl", b"minf", b"mdia", b"trak", b"moov"]:
        table = atom(kind, table)
    return ftyp + mdat + table


class RecoveryWorkbenchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="resc-workbench-")
        self.root = Path(self.temp.name)
        self.environment = patch.dict(os.environ, {"RESC_SESSION_ROOT": str(self.root / "sessions")})
        self.environment.start()
        self.client = McpClient()

    def tearDown(self):
        self.client.close()
        self.environment.stop()
        self.temp.cleanup()

    def fragments(self, payload):
        split = len(payload) // 2 + 3
        # Store the second fragment before the first, with unrelated bytes between.
        second = 19
        first = second + len(payload) - split + 1097
        image = bytearray(first + split + 512)
        image[second:second + len(payload) - split] = payload[split:]
        image[first:first + split] = payload[:split]
        image.extend(bytes((-len(image)) % 512))
        source = self.root / "fragments.img"
        source.write_bytes(image)
        plan = [{"source_byte_offset": first, "length_bytes": split},
                {"source_byte_offset": second, "length_bytes": len(payload) - split}]
        return source, plan

    def test_unaligned_reordered_fragments_analyze_export_and_hash_provenance(self):
        payload = structural_video()
        source, ranges = self.fragments(payload)
        before = workbench.sha256(source)
        sid = self.client.open(source)
        report = self.client.call_tool("analyze_video_candidate", {"session_id": sid, "ranges": ranges})["report"]
        self.assertEqual(report["classification"], "STRUCTURE_CONSISTENT")
        self.assertFalse(report["decode_verified"])
        self.assertEqual(report["chunk_offsets_checked"], 1)
        exported = self.client.call_tool("extract_recovered_data", {"session_id": sid,
            "mode": "CARVE_BY_BYTE_RANGES", "output_dir": str(self.root / "output"),
            "params": {"ranges": ranges, "filename": "candidate.mp4"}})
        self.assertEqual(Path(exported["output_path"]).read_bytes(), payload)
        manifest = json.loads(Path(exported["manifest_path"]).read_text())
        self.assertEqual(manifest["sha256_hash"], hashlib.sha256(payload).hexdigest())
        self.assertEqual([r["source_byte_offset"] for r in manifest["source_lba_ranges"]], [r["source_byte_offset"] for r in ranges])
        self.assertEqual(workbench.sha256(source), before)
        self.client.error("extract_recovered_data", {"session_id": sid, "mode": "CARVE_BY_BYTE_RANGES",
            "output_dir": str(self.root / "output"), "params": {"ranges": ranges, "filename": "candidate.mp4"}})

    def test_invalid_plans_and_missing_metadata_are_not_claimed_as_recovery(self):
        source, ranges = self.fragments(structural_video())
        sid = self.client.open(source)
        for invalid in [[], ranges * 2, [{"source_byte_offset": source.stat().st_size, "length_bytes": 1}]]:
            self.client.error("analyze_video_candidate", {"session_id": sid, "ranges": invalid})
            self.client.error("extract_recovered_data", {"session_id": sid, "mode": "CARVE_BY_BYTE_RANGES",
                "output_dir": str(self.root / "bad"), "params": {"ranges": invalid}})
        report = self.client.call_tool("analyze_video_candidate", {"session_id": sid, "ranges": ranges[:1]})["report"]
        self.assertNotEqual(report["classification"], "STRUCTURE_CONSISTENT")

    def test_general_triage_and_cross_chunk_video_box_discovery(self):
        image = bytearray(70000)
        for offset, kind in [(13, b"ftyp"), (65530, b"moov"), (67001, b"mdat"), (68003, b"moof")]:
            image[offset:offset + 16] = atom(kind, bytes(8))
        image[69000:69008] = struct.pack(">I", 3) + b"mdat"  # impossible size
        image.extend(bytes((-len(image)) % 512))
        source = self.root / "boxes.img"; source.write_bytes(image)
        sid = self.client.open(source)
        diagnosis = self.client.call_tool("diagnose_recovery", {"session_id": sid})
        self.assertTrue(diagnosis["read_only"])
        self.assertIn("ISO_BMFF_BOX_SCAN", [r["action"] for r in diagnosis["routes"]])
        job = self.client.job(sid, "ISO_BMFF_BOX_SCAN")
        self.assertEqual(self.client.wait(job)["hits_count"], 4)
        hits = self.client.call_tool("read_scan_hits", {"session_id": sid, "job_id": job})
        self.assertEqual({h["box_type"] for h in hits["hits"]}, {"ftyp", "moov", "mdat", "moof"})
        self.assertTrue(all(h["validation"] == "plausible_box_header_only" for h in hits["hits"]))

    def test_command_line_plan_recovery_and_mcp_config(self):
        source, ranges = self.fragments(structural_video())
        plan = self.root / "plan.json"
        plan.write_text(json.dumps({"ranges": ranges, "source_sha256": workbench.sha256(source)}))
        cli = [sys.executable, str(ROOT / "scripts/recovery.py")]
        result = subprocess.run(cli + ["analyze-video", str(source), "--plan", str(plan)], capture_output=True, text=True, check=True)
        self.assertEqual(json.loads(result.stdout)["report"]["classification"], "STRUCTURE_CONSISTENT")
        result = subprocess.run(cli + ["recover", str(source), "--plan", str(plan), "--output-dir", str(self.root / "cli-output")], capture_output=True, text=True, check=True)
        self.assertEqual(Path(json.loads(result.stdout)["output_path"]).read_bytes(), structural_video())
        result = subprocess.run(cli + ["mcp-config", "--session-root", str(self.root / "state")], capture_output=True, text=True, check=True)
        self.assertTrue(Path(json.loads(result.stdout)["mcpServers"]["resc"]["command"]).is_absolute())

    def test_multi_route_cli_tree_export_and_native_clone(self):
        from synthetic_filesystems import create_ntfs
        source = self.root / "allocated.img"
        info = create_ntfs(source)
        cli = [sys.executable, str(ROOT / "scripts/recovery.py")]
        # A bounded deep scan still runs all three independent scan types.
        result = subprocess.run(cli + ["triage", str(source), "--deep", "--end-lba", "32"], capture_output=True, text=True, check=True)
        report = json.loads(result.stdout)
        self.assertEqual(len(report["scans"]), 3)
        self.assertTrue(all(Path(s["all_hits_file"]).exists() for s in report["scans"]))
        result = subprocess.run(cli + ["extract-tree", str(source), "--partition-start-lba", str(info["start"]),
            "--output-dir", str(self.root / "tree")], capture_output=True, text=True, check=True)
        self.assertEqual(json.loads(result.stdout)["files_extracted"], 1)
        self.assertEqual((self.root / "tree" / info["name"]).read_bytes(), info["payload"])
        clone = self.root / "clone.img"
        result = subprocess.run(cli + ["clone", str(source), "--destination", str(clone)], capture_output=True, text=True, check=True)
        self.assertEqual(json.loads(result.stdout)["status"], "cloned")
        self.assertEqual(workbench.sha256(source), workbench.sha256(clone))

    def test_single_sector_source_triage_and_cli_negative_video_exit_status(self):
        source = self.root / "one-sector.img"; source.write_bytes(bytes(512))
        sid = self.client.open(source)
        report = self.client.call_tool("diagnose_recovery", {"session_id": sid})
        self.assertTrue(report["first_sector_zeroed"])
        result = subprocess.run([sys.executable, str(ROOT / "scripts/recovery.py"), "analyze-video", str(source), "--length", "512"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertNotEqual(json.loads(result.stdout)["report"]["classification"], "STRUCTURE_CONSISTENT")

    @unittest.skipUnless(shutil.which("ffmpeg") and shutil.which("ffprobe"), "Optional FFmpeg tools unavailable")
    def test_real_video_fragment_reconstruction_demux_and_full_decode(self):
        video = self.root / "original.mp4"
        subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=64x64:rate=2",
                        "-t", "1", "-c:v", "mpeg4", "-movflags", "+faststart", str(video)], check=True, timeout=30)
        source, ranges = self.fragments(video.read_bytes())
        sid = self.client.open(source)
        analysis = self.client.call_tool("analyze_video_candidate", {"session_id": sid, "ranges": ranges})
        self.assertEqual(analysis["report"]["classification"], "STRUCTURE_CONSISTENT")
        result = self.client.call_tool("extract_recovered_data", {"session_id": sid, "mode": "CARVE_BY_BYTE_RANGES",
            "output_dir": str(self.root / "real-output"), "params": {"ranges": ranges, "filename": "restored.mp4"}})
        report = workbench.verify_media(result["output_path"], decode=True, seconds=30)
        self.assertTrue(report["probe_passed"])
        self.assertTrue(report["decode_passed"])
        self.assertTrue(report["manifest_hash_verified"])
        self.assertEqual(workbench.sha256(video), workbench.sha256(Path(result["output_path"])))


if __name__ == "__main__":
    unittest.main(verbosity=2)
