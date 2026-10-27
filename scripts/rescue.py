#!/usr/bin/env python3
"""Linux Live rescue/acquisition and authenticated SSH image transfer (Python 3.11+).

Standalone on the source computer; recovery.py and Rust run on acquired images.
No source writes, no automatic mounting, no SSH server or firewall changes.
"""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile

SCHEMA = 1
ARTIFACTS = ("image.img", "rescue.map")
SSH_OPTIONS = ["-T", "-oBatchMode=yes", "-oStrictHostKeyChecking=yes",
               "-oConnectTimeout=15", "-oServerAliveInterval=15", "-oServerAliveCountMax=3"]


def linux():
    if not sys.platform.startswith("linux"):
        raise ValueError("Acquisition requires Linux Live; use recovery.py for image recovery on Windows/macOS")


def hash_file(path):
    with open(path, "rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, data):
    with open(path, "x", encoding="utf-8") as stream:
        json.dump(data, stream, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())


def read_json(path):
    with open(path, "rb") as stream:
        data = stream.read(1024 * 1024 + 1)
    if len(data) > 1024 * 1024:
        raise ValueError("Metadata exceeds 1 MiB")
    return json.loads(data)


def block_roots(device, seen=None):
    """Resolve partitions, dm/LVM and md slaves; fail closed for unknown backing."""
    seen = set() if seen is None else set(seen)
    if device in seen:
        raise ValueError("Cyclic block-device topology")
    seen.add(device)
    node = Path("/sys/dev/block", device).resolve(strict=True)
    if (node / "partition").exists():
        return block_roots((node.parent / "dev").read_text().strip(), seen)
    slaves = list((node / "slaves").iterdir())
    if slaves:
        roots = set()
        for slave in slaves:
            roots |= block_roots((slave / "dev").read_text().strip(), seen)
        return roots
    if node.name.startswith(("loop", "zram", "ram", "nbd")):
        raise ValueError("Unsupported virtual backing device: " + node.name)
    return {device}


def inventory():
    linux()
    result = subprocess.run(["lsblk", "--json", "--bytes", "--list", "--output",
        "NAME,PATH,TYPE,SIZE,RO,TRAN,MODEL,SERIAL,MOUNTPOINTS,LOG-SEC,MAJ:MIN"],
        check=True, capture_output=True, text=True, timeout=30)
    rows = json.loads(result.stdout)["blockdevices"]
    return {"platform": sys.platform, "devices": rows,
            "tools": {name: shutil.which(name) for name in ("ddrescue", "ssh", "lsblk")},
            "note": "Inventory is local to this running OS, not a scan for powered-off computers"}


def source_info(path, output_parent, sector_size=None):
    linux()
    path = Path(path).resolve(strict=True)
    info = path.stat()
    if stat.S_ISREG(info.st_mode):
        sector_size = sector_size or 512
        if info.st_size <= 0 or info.st_size % sector_size:
            raise ValueError("Image source must be nonempty and sector-aligned")
        return {"path": str(path), "kind": "image", "size": info.st_size,
                "device": info.st_dev, "inode": info.st_ino, "mtime_ns": info.st_mtime_ns,
                "sector_size": sector_size}
    if not stat.S_ISBLK(info.st_mode):
        raise ValueError("Source must be a regular raw image or Linux block device")
    device = f"{os.major(info.st_rdev)}:{os.minor(info.st_rdev)}"
    rows = inventory()["devices"]
    row = next((item for item in rows if item["maj:min"] == device), None)
    if not row or not row["size"] or row["log-sec"] not in (512, 4096):
        raise ValueError("Unknown source capacity/geometry")
    if sector_size and sector_size != row["log-sec"]:
        raise ValueError("Requested sector size does not match physical device")
    roots = block_roots(device)
    destination = output_parent.stat().st_dev
    target_roots = block_roots(f"{os.major(destination)}:{os.minor(destination)}")
    if roots & target_roots:
        raise ValueError("Destination shares source physical backing; choose another disk")
    for item in rows:
        if any(item.get("mountpoints") or []):
            if roots & block_roots(item["maj:min"]):
                raise ValueError("Source or related volume mounted/in swap: " + item["path"])
    return {"path": str(path), "kind": "block", "size": row["size"],
            "device": device, "roots": sorted(roots), "serial": row["serial"],
            "model": row["model"], "sector_size": row["log-sec"]}


def map_summary(path, size):
    """Require complete ordered coverage; '+' alone means acquired bytes."""
    if path.stat().st_size > 16 * 1024 * 1024:
        raise ValueError("Map exceeds 16 MiB validation budget")
    counts = dict.fromkeys("+?-*/", 0)
    end = 0
    for line in path.read_text().splitlines():
        fields = line.split()
        if not fields or line.lstrip().startswith("#"):
            continue
        if len(fields) != 3:
            raise ValueError("Malformed ddrescue map")
        if fields[1] in counts:  # current-position/status/pass header
            continue
        start, length = int(fields[0], 0), int(fields[1], 0)
        status = fields[2]
        if status not in counts or start != end or length <= 0 or start + length > size:
            raise ValueError("Invalid map status, gap, overlap or capacity")
        counts[status] += length
        end += length
    if end != size:
        raise ValueError("Map does not cover full source capacity")
    return {"status": "COMPLETE" if counts["+"] == size else "PARTIAL",
            "rescued_bytes": counts["+"], "unresolved_bytes": size - counts["+"],
            "bytes_by_map_status": counts}


def regular(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise ValueError("Expected regular artifact with one link: " + str(path))
    return info


def private_case(path):
    path = Path(path).absolute()
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise ValueError("Case directory must be owned by this user, mode 0700, and not a symlink")
    return path.resolve(strict=True)


@contextlib.contextmanager
def case_lock(case):
    import fcntl
    descriptor = os.open(case / ".lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
    finally:
        os.close(descriptor)


def acquire(args):
    linux()
    case = Path(args.case).absolute()
    parent = case.parent.resolve(strict=True)
    case = parent / case.name
    geometry = getattr(args, "sector_size", None)
    source = source_info(args.source, parent, geometry)
    if args.resume and source["kind"] == "block" and not source["serial"]:
        raise ValueError("Block-device resume needs a reported serial; cannot bind an unidentified device")
    report = {"source": source, "case": str(case), "phase": args.phase,
              "status": "PLAN", "engine": "GNU ddrescue (external)"}
    if not args.run:
        return report
    executable = shutil.which("ddrescue")
    if not executable:
        raise ValueError("Install GNU ddrescue (Debian/Ubuntu package gddrescue), not dd_rescue")
    if args.resume:
        case = private_case(case)
    else:
        if shutil.disk_usage(parent).free < source["size"] + 16 * 1024 * 1024:
            raise ValueError("Insufficient destination space for image plus map")
        case.mkdir(mode=0o700)
    with case_lock(case):
        if (case / "sealed.json").exists():
            raise ValueError("Case is sealed; acquisition cannot modify it")
        if args.resume:
            saved = read_json(case / "source.json")
            if saved["source"] != source:
                raise ValueError("Source identity/capacity changed; refusing resume")
            image = regular(case / "image.img")
            if [image.st_dev, image.st_ino, image.st_size] != saved["image_identity"]:
                raise ValueError("Image identity/capacity changed")
            regular(case / "rescue.map")
            if (case / "rescue.map").stat().st_size:
                map_summary(case / "rescue.map", source["size"])
        else:
            with (case / "image.img").open("xb") as image:
                image.truncate(source["size"])  # sparse zeros remain unknown unless map says '+'
            (case / "rescue.map").touch(exist_ok=False, mode=0o600)
            image = regular(case / "image.img")
            saved = {"schema": SCHEMA, "source": source,
                     "image_identity": [image.st_dev, image.st_ino, image.st_size]}
            write_json(case / "source.json", saved)
        # Hold read-only source and output handles. Pin directory for ddrescue's
        # map/temp-map renames, so changing a path cannot redirect these writes.
        with contextlib.ExitStack() as stack:
            src = os.open(source["path"], os.O_RDONLY | os.O_NOFOLLOW)
            stack.callback(os.close, src)
            dst = os.open(case / "image.img", os.O_RDWR | os.O_NOFOLLOW)
            stack.callback(os.close, dst)
            directory = os.open(case, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            stack.callback(os.close, directory)
            if source_info(source["path"], parent, geometry) != source:
                raise ValueError("Source changed before acquisition")
            opened = os.fstat(src)
            expected = Path(source["path"]).stat()
            if (opened.st_dev, opened.st_ino, opened.st_rdev) != (expected.st_dev, expected.st_ino, expected.st_rdev):
                raise ValueError("Source changed while opening")
            if (os.fstat(dst).st_dev, os.fstat(dst).st_ino) != (image.st_dev, image.st_ino):
                raise ValueError("Destination changed while opening")
            options = ["--no-scrape"] if args.phase == "fast" else ["--retry-passes=2"]
            command = [executable, "--quiet", f"--sector-size={source['sector_size']}",
                       f"--size={source['size']}", *options, f"/proc/self/fd/{src}",
                       f"/proc/self/fd/{dst}", f"/proc/self/fd/{directory}/rescue.map"]
            version = subprocess.run([executable, "--version"], capture_output=True,
                                     check=True, text=True, timeout=10).stdout.splitlines()[0]
            result = subprocess.run(command, stdin=subprocess.DEVNULL, pass_fds=(src, dst, directory))
            os.fsync(dst)
        if result.returncode:
            raise RuntimeError(f"ddrescue exited {result.returncode}; preserve case and use --resume")
        if source["kind"] == "image" and source_info(source["path"], parent, geometry) != source:
            raise ValueError("Source image changed during acquisition")
        report.update(map_summary(case / "rescue.map", source["size"]))
        report["engine"] = version
        if args.seal:
            files = {}
            for name in ARTIFACTS:
                info = regular(case / name)
                files[name] = {"size": info.st_size, "sha256": hash_file(case / name),
                               "inode": info.st_ino, "mtime_ns": info.st_mtime_ns}
            report.update(schema=SCHEMA, files=files, sealed=True)
            write_json(case / "sealed.json", report)
        return report


def inspect_case(case):
    case = private_case(case)
    manifest = read_json(case / "sealed.json")
    if manifest.get("schema") != SCHEMA or set(manifest.get("files", {})) != set(ARTIFACTS):
        raise ValueError("Invalid sealed manifest")
    for name in ARTIFACTS:
        info = regular(case / name)
        entry = manifest["files"][name]
        if (info.st_size, info.st_ino, info.st_mtime_ns) != (entry["size"], entry["inode"], entry["mtime_ns"]):
            raise ValueError("Sealed artifact changed: " + name)
    return manifest


def ssh_command(host, arguments):
    if not re.fullmatch(r"(?:[A-Za-z0-9_][A-Za-z0-9_.-]*@)?[A-Za-z0-9][A-Za-z0-9.-]*", host):
        raise ValueError("Use user@hostname or IPv4; SSH options/metacharacters are not allowed")
    return ["ssh", *SSH_OPTIONS, host, shlex.join(["python3", "-", *arguments])]


def agent_script():
    # Remote operations do not need this file installed on the source computer.
    return Path(__file__).read_bytes()


def remote(host, arguments):
    command = ssh_command(host, arguments)
    with tempfile.TemporaryFile() as output:
        result = subprocess.run(command, input=agent_script(), stdout=output)
        if result.returncode not in (0, 2):
            raise RuntimeError(f"Remote operation failed ({result.returncode}); acquisition may be resumable")
        if output.tell() > 1024 * 1024:
            raise ValueError("Remote JSON exceeds budget")
        output.seek(0)
        return json.load(output)


def pull(args):
    linux()
    manifest = remote(args.host, ["inspect-case", args.remote_case])
    case = Path(args.case).absolute()
    if args.resume:
        case = private_case(case)
    else:
        case.mkdir(mode=0o700)
        write_json(case / "transfer.json", {"host": args.host, "case": args.remote_case, "manifest": manifest})
    with case_lock(case):
        binding = read_json(case / "transfer.json")
        if binding != {"host": args.host, "case": args.remote_case, "manifest": manifest}:
            raise ValueError("Remote case changed; cannot resume")
        for name in ARTIFACTS:
            target = case / name
            expected = manifest["files"][name]
            if target.exists():
                regular(target)
                if target.stat().st_size != expected["size"] or hash_file(target) != expected["sha256"]:
                    raise ValueError("Existing completed transfer does not match manifest")
                continue
            part = case / (name + ".part")
            if part.exists():
                regular(part)
            else:
                part.touch(mode=0o600, exist_ok=False)
            offset = part.stat().st_size
            if offset > expected["size"]:
                raise ValueError("Partial transfer exceeds remote size")
            command = ssh_command(args.host, ["stream", args.remote_case, name, str(offset)])
            with part.open("ab") as output, tempfile.TemporaryFile() as errors:
                process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors)
                try:
                    process.stdin.write(agent_script())
                    process.stdin.close()
                    remaining = expected["size"] - offset
                    while remaining:
                        chunk = process.stdout.read(min(1024 * 1024, remaining))
                        if not chunk:
                            raise RuntimeError("SSH stream interrupted; use pull --resume")
                        output.write(chunk)
                        remaining -= len(chunk)
                    if process.stdout.read(1) or process.wait() != 0:
                        raise RuntimeError("Invalid SSH stream; preserve partial file")
                    output.flush()
                    os.fsync(output.fileno())
                finally:
                    if process.poll() is None:
                        process.terminate()
                    process.wait()
                    process.stdout.close()
            if hash_file(part) != expected["sha256"]:
                raise ValueError("Transferred SHA-256 mismatch; preserve .part and investigate")
            # Exclusive hard link publishes without replacing an existing path.
            os.link(part, target)
            part.unlink()
        receipt = case / "received.json"
        if not receipt.exists():
            write_json(receipt, {"status": "TRANSFER_VERIFIED", "acquisition": manifest,
                                 "note": "Hash confirms transfer; unresolved map regions are not recovered data"})
        return {"status": "TRANSFER_VERIFIED", "case": str(case),
                "acquisition_status": manifest["status"], "transfer_hash_verified": True}


def bundle(output):
    linux()
    root = Path(__file__).resolve().parents[1]
    target = Path(output)
    target.mkdir(mode=0o700)
    files = {}
    for name in ("rescue.py", "recovery.py", "setup-rescue.sh"):
        shutil.copyfile(root / "scripts" / name, target / name)
        files[name] = hash_file(target / name)
    binary = root / "target/release/mcp-server"
    if sys.platform.startswith("linux") and binary.is_file():
        shutil.copyfile(binary, target / "mcp-server")
        (target / "mcp-server").chmod(0o755)
        files["mcp-server"] = hash_file(target / "mcp-server")
    write_json(target / "bundle.json", {"platform": sys.platform, "architecture": os.uname().machine,
               "files": files, "note": "Portable toolkit directory, not a bootable ISO; install Python/ddrescue separately"})
    return {"output": str(target.resolve()), "files": files}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("devices", help="Read local Linux block-device inventory")
    p = sub.add_parser("acquire", help="Plan (default) or run/resume ddrescue")
    p.add_argument("--source", required=True)
    p.add_argument("--sector-size", type=int, choices=(512, 4096), help="Image geometry; physical geometry is detected and must match")
    p.add_argument("--case", required=True, help="New private case directory on destination disk")
    p.add_argument("--phase", choices=["fast", "retry"], default="fast")
    p.add_argument("--run", action="store_true")
    p.add_argument("--resume", action="store_true")
    p.add_argument("--seal", action="store_true", help="Finalize snapshot/hash; no further acquisition")
    p = sub.add_parser("remote", help="Run devices/acquire on a booted Linux source via SSH")
    p.add_argument("host")
    p.add_argument("arguments", nargs=argparse.REMAINDER)
    p = sub.add_parser("pull", help="Transfer sealed acquisition over SSH, resume and verify SHA-256")
    p.add_argument("host")
    p.add_argument("remote_case")
    p.add_argument("--case", required=True)
    p.add_argument("--resume", action="store_true")
    p = sub.add_parser("bundle", help="Copy portable toolkit for an existing Linux Live USB")
    p.add_argument("--output", required=True)
    p = sub.add_parser("inspect-case", help="Inspect sealed acquisition metadata")
    p.add_argument("case")
    p = sub.add_parser("stream", help="Internal binary artifact stream for SSH transfer")
    p.add_argument("case")
    p.add_argument("artifact", choices=ARTIFACTS)
    p.add_argument("offset", type=int)
    args = parser.parse_args(argv)
    if args.command == "devices":
        report = inventory()
    elif args.command == "acquire":
        report = acquire(args)
    elif args.command == "remote":
        if not args.arguments or args.arguments[0] not in ("devices", "acquire", "inspect-case"):
            raise ValueError("Remote allows only devices, acquire, inspect-case")
        report = remote(args.host, args.arguments)
    elif args.command == "pull":
        report = pull(args)
    elif args.command == "bundle":
        report = bundle(args.output)
    elif args.command == "inspect-case":
        report = inspect_case(args.case)
    else:
        manifest = inspect_case(args.case)
        size = manifest["files"][args.artifact]["size"]
        if not 0 <= args.offset <= size:
            raise ValueError("Stream offset out of bounds")
        with open(Path(args.case) / args.artifact, "rb") as stream:
            stream.seek(args.offset)
            shutil.copyfileobj(stream, sys.stdout.buffer, 1024 * 1024)
        return 0
    print(json.dumps(report, indent=2))
    return 2 if report.get("status") == "PARTIAL" else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        sys.exit(1)
    except KeyboardInterrupt:
        print("Interrupted; preserve case/map/.part and resume explicitly", file=sys.stderr)
        sys.exit(130)
