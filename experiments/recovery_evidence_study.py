#!/usr/bin/env python3
"""Controlled counterexamples and a bounded carving baseline, not a recovery rate.

Run after cargo build --release --locked. Requires FFmpeg/ffprobe with libx264.
Generated media/images remain in a temporary directory; only JSON is retained.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import random
import struct
import subprocess
import sys
import tempfile
import time
import zlib

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("resc_workbench", ROOT / "scripts/recovery.py")
workbench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workbench)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def run(command):
    return subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True,
                          timeout=60, check=True)


def chunk(kind, payload):
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))


def png(color, noise=False):
    header = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 16, 16, 8, 2, 0, 0, 0))
    # A complete ancillary chunk aligns the IDAT boundary to the search grid.
    prefix = header + chunk(b"tEXt", b"note\0" + b"x" * (256 - len(header) - 17))
    assert len(prefix) == 256
    rng = random.Random(3107)
    pixels = b"".join(b"\0" + (rng.randbytes(48) if noise else bytes(color) * 16) for _ in range(16))
    return prefix + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b"")


def trim(ranges, length):
    result = []
    for span in ranges:
        count = min(length, span["length_bytes"])
        if count:
            result.append({"source_byte_offset": span["source_byte_offset"], "length_bytes": count})
        length -= count
    if length:
        raise ValueError("Candidate length exceeds reconstruction")
    return result


def search_png(client, sid, image, budget=512):
    """No access to original payload, true extents, split, or fixture case name.

    Assumes a PNG header, intact IEND, a 256-byte search grid and at most two
    extents. Prefix cuts are restricted to 1..4 grid units. Not SHT/PUP replication.
    """
    header = image.find(b"\x89PNG\r\n\x1a\n")
    if header < 0:
        return {"validations": 0, "candidates": [], "budget_exhausted": False}
    terminator = chunk(b"IEND", b"")
    ends = [at + len(terminator) for at in range(len(image)) if image.startswith(terminator, at)]
    plans = [[{"source_byte_offset": header, "length_bytes": end - header}]
             for end in ends if end > header]
    for prefix in range(256, 1025, 256):
        if header + prefix > len(image):
            continue
        for start in range(0, len(image), 256):
            for end in ends:
                if end <= start or (start < header + prefix and end > header):
                    continue
                plans.append([{"source_byte_offset": header, "length_bytes": prefix},
                              {"source_byte_offset": start, "length_bytes": end - start}])
    candidates = {}
    trials = 0
    for plan in plans[:budget]:
        trials += 1
        report = client.call("analyze_file_candidate", {"session_id": sid, "ranges": plan, "format": "PNG"})["report"]
        if report["classification"] == "STRUCTURE_CONSISTENT":
            bounded = trim(plan, report["candidate_length_bytes"])
            key = json.dumps(bounded, sort_keys=True)
            data = b"".join(image[r["source_byte_offset"]:r["source_byte_offset"] + r["length_bytes"]] for r in bounded)
            candidates[key] = {"ranges": bounded, "sha256": digest(data)}
    return {"validations": trials, "candidates": list(candidates.values()),
            "budget_exhausted": len(plans) > budget}


def fragment_study(client, directory):
    rows = []
    for index, case in enumerate(["contiguous", "forward_two_fragments", "reverse_two_fragments",
                                  "two_valid_different_tails", "overwritten_tail", "three_fragments"]):
        original = png((255, 0, 0), noise=case == "three_fragments")
        image = bytearray(random.Random(700 + index).randbytes(8192))
        head, tail = (5120, 512) if case == "reverse_two_fragments" else (1024, 4096)
        image[head:head + 256] = original[:256]
        if case == "contiguous":
            image[head:head + len(original)] = original
        elif case == "three_fragments":
            image[tail:tail + 256] = original[256:512]
            image[6400:6400 + len(original) - 512] = original[512:]
        else:
            image[tail:tail + len(original) - 256] = original[256:]
            if case == "two_valid_different_tails":
                other = png((0, 0, 255))
                image[6144:6144 + len(other) - 256] = other[256:]
            if case == "overwritten_tail":
                image[tail + 10] ^= 0xff
        source = directory / (case + ".img")
        source.write_bytes(image)
        before = workbench.sha256(source)
        sid = client.call("open_recovery_session", {"target_path": str(source)})["session_id"]
        started = time.monotonic()
        found = search_png(client, sid, image)
        # Ground truth is used only after the complete search, never to stop it.
        exact = sum(c["sha256"] == digest(original) for c in found["candidates"])
        rows.append({"case": case, "validations": found["validations"],
                     "budget_exhausted": found["budget_exhausted"],
                     "candidate_plans": len(found["candidates"]),
                     "distinct_candidate_contents": len({c["sha256"] for c in found["candidates"]}),
                     "exact_content_plans": exact,
                     "wrong_content_plans": len(found["candidates"]) - exact,
                     "seconds": round(time.monotonic() - started, 4),
                     "source_unchanged": before == workbench.sha256(source)})
    return rows


def boxes(data):
    at = 0
    result = []
    while at < len(data):
        if len(data) - at < 8:
            raise ValueError("Truncated fixture box")
        size = int.from_bytes(data[at:at + 4], "big")
        header = 8
        if size == 1:
            header = 16
            size = int.from_bytes(data[at + 8:at + 16], "big")
        elif size == 0:
            size = len(data) - at
        if size < header or at + size > len(data):
            raise ValueError("Invalid fixture box")
        result.append((data[at + 4:at + 8], at, at + header, at + size))
        at += size
    return result


def packet_map(path):
    result = run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_packets",
                  "-show_entries", "packet=pos,size", "-of", "json", str(path)])
    return [(int(p["pos"]), int(p["size"])) for p in json.loads(result.stdout)["packets"]]


def video_study(client, directory):
    encoder = {"codec": "libx264", "preset": "ultrafast", "pixel_format": "yuv420p",
               "qp": 20, "x264_params": "keyint=1:scenecut=0", "b_frames": 0,
               "threads": 1, "size": "64x64", "fps": 6, "seconds": 2}
    originals = {}
    for color in ["red", "blue"]:
        path = directory / (color + ".mp4")
        run(["ffmpeg", "-nostdin", "-v", "error", "-f", "lavfi", "-i",
             f"color=c={color}:s=64x64:r=6", "-t", "2", "-an", "-c:v", "libx264",
             "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-qp", "20",
             "-x264-params", "keyint=1:scenecut=0", "-bf", "0", "-threads", "1",
             str(path)])
        originals[color] = path
    red = originals["red"].read_bytes()
    blue = originals["blue"].read_bytes()
    red_packets, blue_packets = packet_map(originals["red"]), packet_map(originals["blue"])
    if len(red_packets) != 12 or [n for _, n in red_packets] != [n for _, n in blue_packets]:
        raise RuntimeError("Encoder did not produce the equal-size sample fixture; cannot claim the splice experiment ran")
    top = boxes(red)
    mdat = next(box for box in top if box[0] == b"mdat")
    moov = next(box for box in top if box[0] == b"moov")
    wrong = bytearray(red)
    for (target, size), (donor, _) in zip(red_packets[6:], blue_packets[6:]):
        wrong[target:target + size] = blue[donor:donor + size]
    corrupt = bytearray(red)
    corrupt[mdat[2]:mdat[3]] = bytes(mdat[3] - mdat[2])
    missing = bytearray(red)
    missing[moov[1] + 4:moov[1] + 8] = b"free"
    truncated = red[:mdat[3] - 10]
    payloads = {"intact": red, "missing_moov": missing, "truncated_mdat": truncated,
                "zeroed_media_same_boxes": corrupt, "cross_video_payload_splice": wrong}
    rows = []
    for name, data in payloads.items():
        path = directory / (name + ".mp4"); path.write_bytes(data)
        image = directory / (name + "-video.img")
        image.write_bytes(bytes(data) + bytes((-len(data)) % 512))
        before = workbench.sha256(image)
        sid = client.call("open_recovery_session", {"target_path": str(image)})["session_id"]
        structure = client.call("analyze_video_candidate", {"session_id": sid,
            "ranges": [{"source_byte_offset": 0, "length_bytes": len(data)}]})["report"]
        probe = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-count_frames",
            "-show_entries", "stream=nb_read_frames", "-of", "json", str(path)],
            capture_output=True, timeout=30)
        decoded = subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-xerror", "-threads", "1",
            "-i", str(path), "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
            capture_output=True, timeout=30)
        try:
            frames = [s.get("nb_read_frames") for s in json.loads(probe.stdout).get("streams", [])]
        except ValueError:
            frames = []
        rows.append({"case": name, "structure": structure["classification"],
                     "probe_exit": probe.returncode, "reported_frames": frames,
                     "decode_exit": decoded.returncode, "decoded_bytes": len(decoded.stdout),
                     "decoded_rgb_sha256": digest(decoded.stdout) if decoded.returncode == 0 else None,
                     "file_matches_original": digest(data) == digest(red),
                     "source_unchanged": before == workbench.sha256(image)})
    baseline_hash = rows[0]["decoded_rgb_sha256"]
    for row in rows:
        row["decoded_content_matches_original"] = (row["decoded_rgb_sha256"] == baseline_hash
                                                    if row["decode_exit"] == 0 else None)
    return {"fixture": {"colors": ["red", "blue"], "encoder": encoder,
                        "red_packet_sizes": [size for _, size in red_packets],
                        "blue_packet_sizes": [size for _, size in blue_packets]},
            "cases": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, help="New JSON result file; never overwritten")
    args = parser.parse_args()
    if Path(args.output).exists():
        parser.error("Output already exists")
    with tempfile.TemporaryDirectory(prefix="resc-evidence-") as temporary:
        directory = Path(temporary)
        previous = os.environ.get("RESC_SESSION_ROOT")
        os.environ["RESC_SESSION_ROOT"] = str(directory / "sessions")
        client = None
        try:
            client = workbench.Client(workbench.DEFAULT_BINARY)
            results = {"study": "controlled evidence checks, not a recovery benchmark",
                       "script_sha256": digest(Path(__file__).read_bytes()),
                       "ffmpeg": run(["ffmpeg", "-version"]).stdout.decode().splitlines()[0],
                       "python": sys.version.split()[0],
                       "fragment_search": fragment_study(client, directory),
                       "video_validation": video_study(client, directory)}
        finally:
            if client is not None:
                client.close()
            if previous is None:
                os.environ.pop("RESC_SESSION_ROOT", None)
            else:
                os.environ["RESC_SESSION_ROOT"] = previous
    with Path(args.output).open("x", encoding="utf-8") as stream:
        json.dump(results, stream, indent=2)
        stream.write("\n")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
