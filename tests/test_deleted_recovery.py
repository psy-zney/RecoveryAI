"""FAT32 root deleted-entry evidence and exact candidate-export regressions."""
import hashlib
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
from synthetic_filesystems import create_fat32
from test_e2e_recovery import McpClient


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


class DeletedRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="resc-deleted-case-")
        self.root = Path(self.temp.name)
        self.environment = patch.dict(os.environ, {"RESC_SESSION_ROOT": str(self.root / "sessions")})
        self.environment.start()
        self.client = McpClient()

    def tearDown(self):
        self.client.close()
        self.environment.stop()
        self.temp.cleanup()

    def deleted_image(self, name="deleted.img", size=None, allocation=0, mismatch=False):
        source = self.root / name
        info = create_fat32(source)
        root_lba = 32 + 2 * 1024
        with source.open("r+b") as image:
            image.seek(root_lba * 512)
            image.write(b"\xe5")
            if size is not None:
                image.seek(root_lba * 512 + 28)
                image.write(struct.pack("<I", size))
            for index, lba in enumerate([32, 32 + 1024]):
                image.seek(lba * 512 + 3 * 4)
                image.write(struct.pack("<I", allocation if not mismatch or index == 0 else 0x0fffffff))
        return source, info

    def scan(self, source):
        sid = self.client.open(source)
        job = self.client.job(sid, "FAT32_DELETED_SCAN", {"partition_start_lba": 0, "max_clusters": 256})
        summary = self.client.wait(job)
        page = self.client.call_tool("read_scan_hits", {"session_id": sid, "job_id": job, "limit": 100})
        return sid, summary, page["hits"]

    def test_deleted_free_single_cluster_exact_export_and_source_hash(self):
        source, info = self.deleted_image()
        before = digest(source)
        sid, summary, hits = self.scan(source)
        self.assertEqual(summary["scope"], "FAT32_ROOT_DIRECTORY_SHORT_ENTRIES_ONLY")
        self.assertEqual(summary["deleted_entries_found"], 1)
        self.assertEqual(summary["candidate_export_plans"], 1)
        self.assertFalse(summary["scan_truncated"])
        self.assertFalse(hits[0]["content_verified"])
        self.assertFalse(hits[0]["name_complete"])
        plan = hits[0]["candidate_export_plan"]
        result = self.client.call_tool("extract_recovered_data", {
            "session_id": sid, "mode": "CARVE_BY_BYTE_RANGES",
            "output_dir": str(self.root / "output"), "params": {**plan, "filename": "candidate.txt"},
        })
        recovered = Path(result["output_path"])
        self.assertEqual(recovered.read_bytes(), info["payload"])
        manifest = json.loads(Path(result["manifest_path"]).read_text())
        self.assertEqual(manifest["sha256_hash"], hashlib.sha256(info["payload"]).hexdigest())
        self.assertEqual(manifest["source_lba_ranges"][0]["length_bytes"], len(info["payload"]))
        self.assertEqual(digest(source), before)

    def test_allocated_mismatched_and_lost_chains_have_no_export_plan(self):
        cases = [
            ({"allocation": 0x0fffffff}, "CURRENTLY_ALLOCATED_OR_RESERVED"),
            ({"mismatch": True}, "FAT_COPIES_DISAGREE"),
            ({"size": 513}, "FREE_FIRST_CLUSTER_LOST_CHAIN_NO_PLAN"),
        ]
        for index, (options, expected) in enumerate(cases):
            with self.subTest(expected=expected):
                source, _ = self.deleted_image(name=f"risk-{index}.img", **options)
                _, summary, hits = self.scan(source)
                self.assertEqual(summary["candidate_export_plans"], 0)
                self.assertEqual(hits[0]["allocation_state"], expected)
                self.assertNotIn("candidate_export_plan", hits[0])

    def test_invalid_bpb_fails_explicitly_without_claiming_scan_success(self):
        source, _ = self.deleted_image()
        with source.open("r+b") as image:
            image.seek(32)
            image.write(struct.pack("<I", 0xffffffff))
        sid = self.client.open(source)
        job = self.client.job(sid, "FAT32_DELETED_SCAN", {"partition_start_lba": 0})
        with self.assertRaisesRegex(RuntimeError, "capacity"):
            self.client.wait(job)


if __name__ == "__main__":
    unittest.main(verbosity=2)
