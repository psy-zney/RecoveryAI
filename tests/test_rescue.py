"""Acquisition guards, real optional ddrescue, SSH process protocol and resume.

Only synthetic regular images are read; physical disks are never opened.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("rescue", ROOT / "scripts/rescue.py")
rescue = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rescue)
spec_workbench = importlib.util.spec_from_file_location("recovery", ROOT / "scripts/recovery.py")
workbench = importlib.util.module_from_spec(spec_workbench)
spec_workbench.loader.exec_module(workbench)


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux acquisition only")
class RescueTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="resc-acquisition-")
        self.root = Path(self.temp.name)
        self.source = self.root / "original.img"
        self.source.write_bytes(bytes(range(256)) * 8192)
        self.before = rescue.hash_file(self.source)

    def tearDown(self):
        self.assertEqual(rescue.hash_file(self.source), self.before)
        self.temp.cleanup()

    def args(self, **kwargs):
        values = dict(source=str(self.source), case=str(self.root / "case"), phase="fast",
                      run=True, resume=False, seal=False)
        values.update(kwargs)
        return argparse.Namespace(**values)

    def seal(self):
        case = self.root / "remote-case"
        case.mkdir(mode=0o700)
        shutil.copyfile(self.source, case / "image.img")
        (case / "rescue.map").write_text(f"0x0 + 1\n0x0 {self.source.stat().st_size:#x} +\n")
        files = {}
        for name in rescue.ARTIFACTS:
            info = (case / name).stat()
            files[name] = dict(size=info.st_size, sha256=rescue.hash_file(case / name),
                               inode=info.st_ino, mtime_ns=info.st_mtime_ns)
        rescue.write_json(case / "sealed.json", dict(schema=1, status="COMPLETE", files=files))
        return case

    def transport(self):
        original = rescue.ssh_command
        def local(host, arguments):
            command = original(host, arguments)
            return [sys.executable, str(ROOT / "tests/helpers/ssh_transport.py"), *command[1:]]
        return patch.object(rescue, "ssh_command", side_effect=local)

    def test_plan_performs_no_writes_or_engine_invocation(self):
        with patch.object(rescue.subprocess, "run", side_effect=AssertionError("spawned")):
            report = rescue.acquire(self.args(run=False))
        self.assertEqual(report["status"], "PLAN")
        self.assertFalse((self.root / "case").exists())

    def test_invalid_image_sources_and_existing_case_rejected(self):
        invalid = self.root / "bad.img"
        invalid.write_bytes(b"not raw")
        with self.assertRaises(ValueError):
            rescue.acquire(self.args(source=str(invalid), run=False))
        (self.root / "case").mkdir()
        with patch.object(rescue.shutil, "which", return_value="ddrescue"):
            with self.assertRaises(FileExistsError):
                rescue.acquire(self.args())

    def test_map_requires_ordered_capacity_and_tracks_unresolved_bytes(self):
        path = self.root / "test.map"
        path.write_text("# map\n0x0 ? 1\n0x0 0x200 +\n0x200 0x200 -\n")
        summary = rescue.map_summary(path, 1024)
        self.assertEqual(summary["status"], "PARTIAL")
        self.assertEqual(summary["unresolved_bytes"], 512)
        for text in ["0 512 +\n", "0 512 +\n256 768 +\n", "0 1024 X\n", "0 -1 +\n"]:
            path.write_text(text)
            with self.assertRaises(ValueError):
                rescue.map_summary(path, 1024)

    def test_physical_isolation_mount_and_swap_guards(self):
        info = argparse.Namespace(st_mode=0o060000, st_rdev=os.makedev(8, 0))
        disk = {"maj:min": "8:0", "size": 4096, "log-sec": 512, "serial": "test",
                "model": "synthetic", "path": "/dev/fake", "mountpoints": []}
        # Mock only source stat; output parent's filesystem remains real.
        real_stat = Path.stat
        def fake_stat(path, *args, **kwargs):
            return info if path == self.source else real_stat(path, *args, **kwargs)
        with patch.object(Path, "stat", fake_stat), patch.object(rescue, "inventory", return_value={"devices": [disk]}):
            with patch.object(rescue, "block_roots", return_value={"8:0"}):
                with self.assertRaisesRegex(ValueError, "shares source"):
                    rescue.source_info(self.source, self.root)
            for mount in ("/mnt/source", "[SWAP]"):
                disk["mountpoints"] = [mount]
                with patch.object(rescue, "block_roots", side_effect=[{"8:0"}, {"8:16"}, {"8:0"}]):
                    with self.assertRaisesRegex(ValueError, "mounted/in swap"):
                        rescue.source_info(self.source, self.root)
        with patch.object(rescue, "source_info", return_value={"kind": "block", "serial": None}):
            with self.assertRaisesRegex(ValueError, "reported serial"):
                rescue.acquire(self.args(resume=True, run=False))

    def test_host_validation_and_required_ssh_security_options(self):
        for bad in ("-oProxyCommand=evil", "user@host;id", "user@host $(id)", "a\nb"):
            with self.assertRaises(ValueError):
                rescue.ssh_command(bad, ["devices"])
        command = rescue.ssh_command("rescue@192.168.1.20", ["acquire", "--source", "/dev/disk with spaces"])
        self.assertIn("-oStrictHostKeyChecking=yes", command)
        self.assertIn("-oBatchMode=yes", command)
        self.assertIn("'/dev/disk with spaces'", command[-1])

    def test_sealed_artifact_mutation_and_symlinks_rejected(self):
        case = self.seal()
        self.assertEqual(rescue.inspect_case(case)["status"], "COMPLETE")
        (case / "image.img").write_bytes(b"changed")
        with self.assertRaises(ValueError):
            rescue.inspect_case(case)
        link = self.root / "alias"
        link.symlink_to(case, target_is_directory=True)
        with self.assertRaises(ValueError):
            rescue.private_case(link)

    def test_case_lock_excludes_second_worker(self):
        case = self.root / "locked"
        case.mkdir(mode=0o700)
        with rescue.case_lock(case):
            with self.assertRaises(BlockingIOError):
                with rescue.case_lock(case):
                    pass

    def test_recovery_triage_binds_image_and_acquisition_map(self):
        case = self.seal()
        evidence = workbench.acquisition_evidence(case / "sealed.json", case / "image.img")
        self.assertTrue(evidence["hashes_verified"])
        (case / "rescue.map").write_text("tampered")
        with self.assertRaisesRegex(ValueError, "hash/size mismatch"):
            workbench.acquisition_evidence(case / "sealed.json", case / "image.img")

    def test_remote_process_protocol_and_interrupted_transfer_resume(self):
        remote = self.seal()
        target = self.root / "received"
        args = argparse.Namespace(host="rescue@source", remote_case=str(remote), case=str(target), resume=False)
        with self.transport():
            with patch.dict(os.environ, {"RESC_TEST_DROP_STREAM": "1"}):
                with self.assertRaises(RuntimeError):
                    rescue.pull(args)
            self.assertEqual((target / "image.img.part").stat().st_size, 1024)
            self.assertFalse((target / "image.img").exists())
            args.resume = True
            self.assertEqual(rescue.pull(args)["status"], "TRANSFER_VERIFIED")
            self.assertEqual(rescue.hash_file(target / "image.img"), self.before)
            self.assertTrue((target / "received.json").is_file())
            self.assertEqual(rescue.pull(args)["status"], "TRANSFER_VERIFIED")

    def test_corrupted_partial_never_publishes_success(self):
        remote = self.seal()
        target = self.root / "received-bad"
        args = argparse.Namespace(host="source", remote_case=str(remote), case=str(target), resume=False)
        with self.transport():
            with patch.dict(os.environ, {"RESC_TEST_DROP_STREAM": "1"}):
                with self.assertRaises(RuntimeError):
                    rescue.pull(args)
            (target / "image.img.part").write_bytes(b"x" * 1024)
            args.resume = True
            with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                rescue.pull(args)
        self.assertFalse((target / "received.json").exists())
        self.assertFalse((target / "image.img").exists())

    @unittest.skipUnless(shutil.which("ddrescue"), "Install gddrescue to exercise real engine")
    def test_real_ddrescue_fast_resume_retry_seal_and_source_immutability(self):
        first = rescue.acquire(self.args())
        self.assertEqual(first["status"], "COMPLETE")
        self.assertEqual(rescue.hash_file(self.root / "case/image.img"), self.before)
        final = rescue.acquire(self.args(resume=True, phase="retry", seal=True))
        self.assertEqual(final["unresolved_bytes"], 0)
        self.assertEqual(rescue.inspect_case(self.root / "case")["files"]["image.img"]["sha256"], self.before)
        with self.assertRaisesRegex(ValueError, "sealed"):
            rescue.acquire(self.args(resume=True))

    @unittest.skipUnless(shutil.which("ddrescue"), "Install gddrescue to exercise real engine")
    def test_real_ddrescue_fault_injection_seals_partial_without_claiming_full_recovery(self):
        fault = self.root / "fault.map"
        size = self.source.stat().st_size
        fault.write_text(f"0x0 + 1\n0x0 0x400 +\n0x400 0x200 -\n0x600 {size - 1536:#x} +\n")
        real_run = subprocess.run
        def inject(command, **kwargs):
            if "--version" not in command:
                command = [command[0], f"--test-mode={fault}", *command[1:]]
            return real_run(command, **kwargs)
        with patch.object(rescue.subprocess, "run", side_effect=inject):
            report = rescue.acquire(self.args(seal=True))
        self.assertEqual(report["status"], "PARTIAL")
        self.assertEqual(report["unresolved_bytes"], 512)
        self.assertNotEqual(rescue.hash_file(self.root / "case/image.img"), self.before)
        evidence = workbench.acquisition_evidence(self.root / "case/sealed.json", self.root / "case/image.img")
        self.assertEqual(evidence["status"], "PARTIAL")
        self.assertEqual(evidence["unresolved_bytes"], 512)
        environment = dict(os.environ, RESC_SESSION_ROOT=str(self.root / "sessions"))
        cli = subprocess.run([sys.executable, str(ROOT / "scripts/recovery.py"), "triage",
            str(self.root / "case/image.img"), "--acquisition-manifest", str(self.root / "case/sealed.json")],
            capture_output=True, text=True, check=True, env=environment)
        self.assertEqual(json.loads(cli.stdout)["acquisition"]["unresolved_bytes"], 512)

    @unittest.skipUnless(shutil.which("ddrescue"), "Install gddrescue to exercise real engine")
    def test_changed_source_identity_rejects_resume(self):
        rescue.acquire(self.args())
        replacement = self.root / "replacement.img"
        replacement.write_bytes(self.source.read_bytes())
        replacement.replace(self.source)
        with self.assertRaisesRegex(ValueError, "identity/capacity changed"):
            rescue.acquire(self.args(resume=True))

    def test_bundle_contains_hashed_portable_scripts(self):
        report = rescue.bundle(self.root / "bundle")
        self.assertEqual(report["files"]["rescue.py"], rescue.hash_file(ROOT / "scripts/rescue.py"))
        with self.assertRaises(FileExistsError):
            rescue.bundle(self.root / "bundle")


if __name__ == "__main__":
    unittest.main(verbosity=2)
