"""Integration of independently developed recovery cases through real MCP/CLI."""
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zlib

sys.dont_write_bytecode = True
from test_e2e_recovery import McpClient
from synthetic_filesystems import create_fat32, create_gpt

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def png():
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"\0\xff\0\0")) + chunk(b"IEND", b""))


class ThreeCasesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="resc-three-cases-")
        self.root = Path(self.temp.name)
        self.environment = patch.dict(os.environ, {"RESC_SESSION_ROOT": str(self.root / "sessions")})
        self.environment.start()
        self.client = McpClient()

    def tearDown(self):
        self.client.close()
        self.environment.stop()
        self.temp.cleanup()

    def test_boot_overlay_to_deleted_extent_to_content_validation_and_export(self):
        source = self.root / "damaged-boot-deleted-image.img"
        create_fat32(source)
        payload = png()
        root_lba = 32 + 2 * 1024
        with source.open("r+b") as stream:
            stream.seek(510); stream.write(b"\0\0")
            stream.seek(root_lba * 512); stream.write(b"\xe5MAGE   PNG")
            stream.seek(root_lba * 512 + 28); stream.write(struct.pack("<I", len(payload)))
            for fat_lba in [32, 32 + 1024]:
                stream.seek(fat_lba * 512 + 3 * 4); stream.write(bytes(4))
            stream.seek((root_lba + 1) * 512); stream.write(payload)
        before = digest(source)
        sid = self.client.open(source)
        diagnosis = self.client.call_tool("diagnose_recovery", {"session_id": sid})
        self.assertEqual(len(diagnosis["cases"]), 3)
        evidence = self.client.call_tool("analyze_partition_recovery", {"session_id": sid, "partition_start_lba": 0})
        candidates = evidence["volumes"][0]["backup_candidates"]
        proposal = next(c["overlay_proposal"] for c in candidates if c["filesystem"] == "FAT32")
        self.assertFalse(evidence["repairs_applied"])
        self.assertEqual(proposal["copy_from_lba"], 6)
        self.client.call_tool("virtual_patch_and_verify", {"session_id": sid, "copy_from_lba": 6, "to_lba": 0, "partition_start_lba": 0})
        job = self.client.job(sid, "FAT32_DELETED_SCAN", {"partition_start_lba": 0})
        summary = self.client.wait(job)
        self.assertEqual(summary["candidate_export_plans"], 1)
        hits = self.client.call_tool("read_scan_hits", {"session_id": sid, "job_id": job})
        candidate = hits["hits"][0]
        self.assertFalse(candidate["content_verified"])
        ranges = candidate["candidate_export_plan"]["ranges"]
        content = self.client.call_tool("analyze_file_candidate", {"session_id": sid, "ranges": ranges, "format": "AUTO"})
        self.assertEqual(content["report"]["classification"], "STRUCTURE_CONSISTENT")
        self.assertEqual(content["report"]["candidate_length_bytes"], len(payload))
        self.assertFalse(content["report"]["decode_verified"])
        result = self.client.call_tool("extract_recovered_data", {"session_id": sid, "mode": "CARVE_BY_BYTE_RANGES",
            "output_dir": str(self.root / "recovered"), "params": {"ranges": ranges, "filename": "candidate.png"}})
        self.assertEqual(Path(result["output_path"]).read_bytes(), payload)
        self.assertEqual(result["sha256_hash"], hashlib.sha256(payload).hexdigest())
        self.assertEqual(digest(source), before)
        manifest = json.loads(Path(result["manifest_path"]).read_text())
        self.assertTrue(manifest["applied_ram_patches"])

    def test_gpt_backup_proposal_can_restore_primary_in_overlay_only(self):
        source = self.root / "gpt.img"; create_gpt(source)
        with source.open("r+b") as stream:
            stream.seek(512 + 16); byte = stream.read(1)
            stream.seek(512 + 16); stream.write(bytes([byte[0] ^ 1]))
        before = digest(source)
        sid = self.client.open(source)
        report = self.client.call_tool("analyze_partition_recovery", {"session_id": sid})
        self.assertFalse(report["gpt"]["primary_valid"])
        self.assertTrue(report["gpt"]["backup_valid"])
        proposal = report["gpt"]["overlay_proposal"]
        array = proposal["array_copy"]
        for offset in range(array["count_sectors"]):
            self.client.call_tool("virtual_patch_and_verify", {"session_id": sid,
                "copy_from_lba": array["copy_from_lba"] + offset, "to_lba": array["to_lba"] + offset})
        self.client.call_tool("virtual_patch_and_verify", {"session_id": sid, "patches": [proposal["header_patch"]]})
        verified = self.client.call_tool("scan_disk_topology", {"session_id": sid})
        self.assertEqual(verified["lba_1_gpt_primary"]["status"], "VALID_GPT")
        self.assertTrue(verified["last_lba"]["gpt_backup_valid"])
        self.assertEqual(digest(source), before)

    def test_content_cli_and_partial_failures_are_reported(self):
        source = self.root / "raw.img"
        payload = png(); source.write_bytes(payload + bytes(512 - len(payload)))
        command = [sys.executable, str(ROOT / "scripts/recovery.py")]
        result = subprocess.run(command + ["analyze-file", str(source), "--length", "512"], capture_output=True, text=True, check=True)
        content = json.loads(result.stdout)["report"]
        self.assertEqual(content["candidate_length_bytes"], len(payload))
        self.assertFalse(content["decode_verified"])
        result = subprocess.run(command + ["scan", str(source), "--kind", "FAT32_DELETED_SCAN", "--partition-start-lba", "0"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(json.loads(result.stdout)["scan_failures"], 1)
        result = subprocess.run(command + ["analyze-partition", str(source)], capture_output=True, text=True, check=True)
        self.assertFalse(json.loads(result.stdout)["repairs_applied"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
