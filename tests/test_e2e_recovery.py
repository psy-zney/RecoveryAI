"""End-to-end regression tests using disposable images and real filesystem metadata."""
import hashlib
import json
import os
from pathlib import Path
import queue
import struct
import subprocess
import sys
import tempfile
import threading
import time

sys.dont_write_bytecode = True
from synthetic_filesystems import create_ntfs, create_fat32, create_gpt
from create_sample_images import create_mbr_partition_table, create_ntfs_vbr

MCP_BINARY = os.environ.get("RESC_MCP_BIN") or os.environ.get("RECOVERYAI_MCP_BIN") or str(Path(__file__).resolve().parents[1] / "target" / "release" / ("mcp-server.exe" if os.name == "nt" else "mcp-server"))

def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

class McpClient:
    def __init__(self, binary_path=MCP_BINARY):
        environment=os.environ.copy()
        environment.setdefault("RESC_SESSION_ROOT",str(Path(__file__).resolve().parents[1]/"target"/"test-tmp"/"sessions"))
        self.process = subprocess.Popen([binary_path],env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, encoding="utf-8", bufsize=1)
        self.msg_id = 0
        self.responses = queue.Queue()
        def read_responses():
            for line in self.process.stdout:
                self.responses.put(line)
            self.responses.put(None)
        self.reader = threading.Thread(target=read_responses, daemon=True)
        self.reader.start()
    def send_request(self, method, params=None):
        self.msg_id += 1
        self.process.stdin.write(json.dumps({"jsonrpc":"2.0", "id":self.msg_id,"method":method,"params":params or {}})+"\n")
        self.process.stdin.flush()
        response = self.responses.get(timeout=30)
        assert response is not None, "Server exited unexpectedly"
        response = json.loads(response)
        assert response["id"] == self.msg_id, response
        return response
    def call_tool(self, name, arguments):
        response = self.send_request("tools/call",{"name":name,"arguments":arguments})
        assert "error" not in response, response
        result = response["result"]
        if result.get("isError"):
            raise RuntimeError(result["content"][0]["text"])
        assert len(json.dumps(response).encode()) < 5000, "Interactive response exceeds 5 KB"
        content=json.loads(result["content"][0]["text"])
        if name=="extract_recovered_data" and content.get("status")=="job_started":
            return self.wait(content["job_id"])
        return content
    def error(self, name, args, message=None):
        try:
            self.call_tool(name,args)
        except RuntimeError as error:
            if message: assert message in str(error), str(error)
        else:
            raise AssertionError(f"{name} unexpectedly succeeded")
    def open(self,path):
        return self.call_tool("open_recovery_session",{"target_path":str(path)})["session_id"]
    def job(self,sid,kind,params=None):
        return self.call_tool("start_async_disk_job",{"session_id":sid,"job_type":kind,"params":params or {}})["job_id"]
    def wait(self,job_id):
        deadline=time.monotonic()+30
        while time.monotonic()<deadline:
            result=self.call_tool("poll_async_job",{"job_id":job_id})
            if "Completed" in result["status"]:return result["status"]["Completed"]["result"]
            if "Failed" in result["status"]:raise RuntimeError(result["status"]["Failed"]["error"])
            if result["status"]=="Cancelled":return "Cancelled"
            time.sleep(.01)
        raise AssertionError("Job did not complete within 30 seconds")
    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            self.process.wait(timeout=5)
        self.reader.join(timeout=1)
        self.process.stdout.close()


def run_all_e2e_tests():
    client=McpClient()
    checks=0
    def passed(label):
        nonlocal checks
        checks+=1
        print(f"[OK] {label}")
    try:
        init=client.send_request("initialize")
        assert init["result"]["serverInfo"]["name"]
        client.process.stdin.write(json.dumps({"jsonrpc":"2.0","method":"notifications/initialized"})+"\n")
        client.process.stdin.flush()
        tools=client.send_request("tools/list")["result"]["tools"]
        assert len(tools)>=9
        passed("MCP initialization, tool schemas, and notification without response")
        temporary_root = Path(os.environ.get("RESC_TEST_TMP_DIR", Path(__file__).resolve().parents[1]/"target"/"test-tmp"))
        temporary_root.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="resc-e2e-",dir=temporary_root) as temp:
            root=Path(temp)
            for filesystem,creator in [("NTFS",create_ntfs),("FAT32",create_fat32)]:
                source=root/f"{filesystem}.img"
                info=creator(source,corrupt_boot=True)
                before=digest(source)
                sid=client.open(source)
                patch=client.call_tool("virtual_patch_and_verify",{"session_id":sid,"copy_from_lba":info["backup"],"to_lba":info["start"],"partition_start_lba":info["start"]})
                assert patch["mount_status"]=="SUCCESS",patch
                assert patch["verification"]["root"]["valid_file_entries"]>=1
                output=root/f"{filesystem}-tree"
                result=client.call_tool("extract_recovered_data",{"session_id":sid,"mode":"TSK_TREE_EXTRACT","output_dir":str(output),"params":{"partition_start_lba":info["start"]}})
                assert result["files_extracted"]==1,result
                assert (output/info["name"]).read_bytes()==info["payload"]
                manifest=json.loads((output/(info["name"]+".manifest.json")).read_text())
                assert manifest["sha256_hash"]==hashlib.sha256(info["payload"]).hexdigest()
                assert manifest["applied_ram_patches"] and manifest["unreadable_bad_sectors_in_range"]==[]
                reconstructed=b"".join(source.read_bytes()[r["source_byte_offset"]:r["source_byte_offset"]+r["length_bytes"]] for r in manifest["source_lba_ranges"])
                assert reconstructed==info["payload"],manifest
                assert digest(source)==before,"Source changed by virtual patch or extraction"
                client.error("extract_recovered_data",{"session_id":sid,"mode":"TSK_TREE_EXTRACT","output_dir":str(output),"params":{"partition_start_lba":info["start"]}},"overwrite")
                passed(f"{filesystem}: overlay boot repair, real root traversal, exact file export, source hash and overwrite guard")
            source=root/"metadata.img"
            info=create_ntfs(source)
            root_lba=info["start"]+4*8+5*2
            with source.open("r+b") as disk:
                disk.seek(root_lba*512);record=disk.read(512)
                disk.seek((info["start"]+1000)*512);disk.write(record)
                disk.seek(root_lba*512);disk.write(b"BAAD")
            before=digest(source);sid=client.open(source)
            repaired=client.call_tool("virtual_patch_and_verify",{"session_id":sid,"copy_from_lba":info["start"]+1000,"to_lba":root_lba,"partition_start_lba":info["start"]})
            assert repaired["mount_status"]=="SUCCESS",repaired
            assert digest(source)==before
            passed("NTFS root-index metadata repair is actually read through RAM overlay")
            for field,value in [(104,0xffffffff),(96,0xffffffff),(88,0xffffffff)]:
                source=root/f"bad-index-{field}.img";info=create_ntfs(source)
                root_offset=(info["start"]+4*8+5*2)*512
                with source.open("r+b") as image:
                    image.seek(root_offset+field);image.write(struct.pack("<I",value))
                sid=client.open(source)
                result=client.call_tool("virtual_patch_and_verify",{"session_id":sid,"patches":[{"lba":0,"offset":0,"hex_data":"00"}],"partition_start_lba":info["start"]})
                assert result["mount_status"]=="UNVERIFIED",result
            passed("Malformed NTFS index bounds and record sizes fail without panic or oversized allocation")
            # Bare signatures must never imply a mountable filesystem.
            source=root/"mbr-only.img"
            data=bytearray(4096*512);data[510:512]=b"\x55\xaa";source.write_bytes(data)
            sid=client.open(source)
            result=client.call_tool("virtual_patch_and_verify",{"session_id":sid,"patches":[{"lba":0,"offset":0,"hex_data":"00"}]})
            assert result["mount_status"]=="UNVERIFIED",result
            for patch in [{"lba":0,"offset":513,"hex_data":"aa"},{"lba":0,"offset":511,"hex_data":"aabb"},{"lba":0,"offset":0,"hex_data":"xyz"},{"lba":999999,"offset":0,"hex_data":"aa"}]:
                client.error("virtual_patch_and_verify",{"session_id":sid,"patches":[patch]})
            client.error("virtual_patch_and_verify",{"session_id":sid},"No patches")
            client.error("scan_filesystem_journals",{"session_id":sid,"partition_start_lba":0},"NotImplemented")
            client.error("start_async_disk_job",{"session_id":sid,"job_type":"UNKNOWN"},"NotImplemented")
            passed("No fake mount success, malformed patches rejected, unknown jobs and journals fail explicitly")
            source=root/"recovered_file.bin";source.write_bytes(b"A"*4096)
            before=digest(source);sid=client.open(source)
            extract={"session_id":sid,"mode":"CARVE_BY_LBA_RANGES","output_dir":str(root),"params":{"ranges":[{"start_lba":0,"count":1}]}}
            client.error("extract_recovered_data",extract)
            assert digest(source)==before
            extract["output_dir"]=str(root/"bad-range")
            extract["params"]["ranges"][0]["start_lba"]=99999
            client.error("extract_recovered_data",extract,"capacity")
            assert not (root/"bad-range"/"recovered_file.bin").exists()
            passed("Source collision cannot truncate evidence; invalid ranges cannot produce fake recovered files")
            for label,kwargs,valid in [("valid",{},True),("header",{"bad_header":True},False),("array",{"bad_array":True},False),("range",{"out_of_bounds":True},False)]:
                source=root/f"gpt-{label}.img";create_gpt(source,**kwargs)
                sid=client.open(source);layout=client.call_tool("scan_disk_topology",{"session_id":sid})
                assert (layout["lba_1_gpt_primary"]["status"]=="VALID_GPT")==valid,layout
                assert layout["last_lba"]["gpt_backup_valid"]==valid,layout
            source=root/"mbr-range.img";source.write_bytes(create_mbr_partition_table(2048,99999)+bytes(4095*512))
            layout=client.call_tool("scan_disk_topology",{"session_id":client.open(source)})
            assert layout["lba_0_mbr"]["status"]=="CORRUPTED",layout
            passed("Both GPT headers and entry arrays require correct CRC32 and capacity; MBR range bounds checked")
            source=root/"scan.img";data=bytearray(12003*512)
            for offset in [65534,6000*512,11000*512]:data[offset:offset+4]=b"%PDF"
            source.write_bytes(data);sid=client.open(source)
            result=client.wait(client.job(sid,"SIMD_MAGIC_CARVE"))
            assert result["sectors_scanned"]==12003,result
            assert sorted(hit["byte_offset"] for hit in result["sample_hits"])==[65534,6000*512,11000*512],result
            result=client.wait(client.job(sid,"SIMD_MAGIC_CARVE",{"start_lba":10000,"end_lba":12003}))
            assert result["hits_count"]==1 and result["sample_hits"][0]["lba"]==11000,result
            passed("Overlap catches split signatures once; full scan includes tail chunk and requested ranges beyond old cap")
            clone=root/"clone.img"
            result=client.wait(client.job(sid,"DD_RESCUE_CLONE",{"destination_image":str(clone)}))
            assert result["status"]=="cloned",result
            assert digest(source)==digest(clone)
            assert clone.with_suffix(".img.map").exists()
            client.error("start_async_disk_job",{"session_id":sid,"job_type":"DD_RESCUE_CLONE","params":{"destination_image":str(clone)}})
            passed("Native clone produces byte-identical image and mapfile, refuses existing destination")
            result=client.wait(client.job(sid,"FRAGMENT_PREFILTER",{"first_fragment_end_lba":127,"candidate_start_lba":128,"candidate_end_lba":512}))
            assert result["candidates_scored"]==48,result
            assert len(result["top_candidates"])==5
            assert all(0<=candidate["confidence"]<=1 for candidate in result["top_candidates"])
            passed("Fragment prefilter returns real bounded statistical scores")
            dense=root/"dense.img";dense.write_bytes((b"%PDF"+bytes(508))*2000)
            dense_sid=client.open(dense);dense_job=client.job(dense_sid,"SIMD_MAGIC_CARVE")
            result=client.wait(dense_job)
            assert result["hits_count"]==2000 and len(result["sample_hits"])==8 and result["samples_truncated"],result
            cursor=0;hit_count=0
            while True:
                page=client.call_tool("read_scan_hits",{"session_id":dense_sid,"job_id":dense_job,"cursor":cursor,"limit":100})
                hit_count+=page["count"];cursor=page["next_cursor"]
                if not page["has_more"]:break
            assert hit_count==2000
            passed("All 2000 hits persist and paginate in reports under 5 KB")
            hunt=root/"hunt.img"
            with hunt.open("wb") as image:
                image.truncate(60000*512)
                for lba in [2049,55000]:image.seek(lba*512);image.write(create_ntfs_vbr(4000,4,2))
            sid=client.open(hunt);result=client.wait(client.job(sid,"SIMD_VBR_HUNT"))
            assert sorted(hit["lba"] for hit in result["sample_hits"])==[2049,55000],result
            job_id=client.job(sid,"SIMD_VBR_HUNT")
            client.call_tool("poll_async_job",{"job_id":job_id,"cancel":True})
            assert client.wait(job_id)=="Cancelled"
            passed("VBR hunt scans every sector across full range; cancellation acknowledged without blocking stdio")
            four_kn=root/"four-kn.img";four_kn.write_bytes(bytes(4096*32))
            opened=client.call_tool("open_recovery_session",{"target_path":str(four_kn),"sector_size":4096});four_sid=opened["session_id"]
            assert opened["total_sectors"]==32 and opened["lba_unit_bytes"]==4096,opened
            client.call_tool("virtual_patch_and_verify",{"session_id":four_sid,"partition_start_lba":0,"patches":[{"lba":1,"offset":4094,"hex_data":"abcd"}]})
            exported=client.call_tool("extract_recovered_data",{"session_id":four_sid,"mode":"CARVE_BY_LBA_RANGES","output_dir":str(root/"four-out"),"params":{"ranges":[{"start_lba":1,"count":1}]}})
            content=(root/"four-out"/"recovered_file.bin").read_bytes()
            assert len(content)==4096 and content[-2:]==bytes.fromhex("abcd")
            manifest=json.loads(Path(exported["manifest_path"]).read_text())
            assert manifest["sector_size"]==4096 and manifest["source_lba_ranges"][0]["start_lba"]==1
            client.close();client=McpClient()
            resumed=client.call_tool("open_recovery_session",{"target_path":str(four_kn),"sector_size":4096,"resume_session_id":four_sid})
            assert resumed["session_id"]==four_sid
            replayed=client.call_tool("extract_recovered_data",{"session_id":four_sid,"mode":"CARVE_BY_LBA_RANGES","output_dir":str(root/"four-replayed"),"params":{"ranges":[{"start_lba":1,"count":1}]}})
            assert Path(replayed["output_path"]).read_bytes()==content
            # Stored results remain available after process restart and image-session replay.
            client.call_tool("open_recovery_session",{"target_path":str(dense),"resume_session_id":dense_sid})
            page=client.call_tool("read_scan_hits",{"session_id":dense_sid,"job_id":dense_job,"limit":5})
            assert page["count"]==5
            passed("4Kn native LBAs, durable overlay replay, provenance manifest and hit retrieval survive process restart")
            # A sparse image is sufficient to keep a real scan busy without allocating GiBs.
            large=root/"large.img"
            with large.open("wb") as stream:stream.truncate(2*1024**3)
            large_sid=client.open(large)
            start=time.monotonic()
            response=client.send_request("tools/call",{"name":"extract_recovered_data","arguments":{"session_id":large_sid,"mode":"CARVE_BY_LBA_RANGES","output_dir":str(root/"large-out"),"params":{"ranges":[{"start_lba":0,"count":2*1024**3//512}]}}})
            result=json.loads(response["result"]["content"][0]["text"])
            assert result["status"]=="job_started" and time.monotonic()-start<2
            poll=client.call_tool("poll_async_job",{"job_id":result["job_id"],"cancel":True})
            assert client.wait(result["job_id"])=="Cancelled"
            assert not (root/"large-out"/"recovered_file.bin").exists()
            passed("Large extraction returns immediately, stdio polls remain live and cancellation removes incomplete export")
            empty=root/"empty.img";empty.touch();client.error("open_recovery_session",{"target_path":str(empty)})
            passed("Zero-length images fail explicitly")
    finally:
        client.close()
    print(f"All {checks} E2E groups passed.")

if __name__=="__main__":
    run_all_e2e_tests()
