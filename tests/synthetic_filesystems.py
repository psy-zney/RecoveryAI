"""Deterministic filesystem fixtures with real directory metadata and file data.
These are disposable test images, never physical devices.
"""
import struct
import zlib

PAYLOAD = b"resc: verified native filesystem recovery\n" * 3

def align8(n):
    return (n + 7) & ~7

def resident_attribute(kind, value, name=""):
    encoded = name.encode("utf-16le")
    offset = align8(24 + len(encoded))
    result = bytearray(align8(offset + len(value)))
    struct.pack_into("<IIBBHHH", result, 0, kind, len(result), 0, len(name), 24 if name else 0, 0, 0)
    struct.pack_into("<IHBB", result, 16, len(value), offset, 0, 0)
    result[24:24+len(encoded)] = encoded
    result[offset:offset+len(value)] = value
    return result

def file_record(attributes, number, directory=False):
    record = bytearray(1024)
    record[:4] = b"FILE"
    struct.pack_into("<HH", record, 4, 48, 3)
    struct.pack_into("<HHHHII", record, 16, 1, 1, 56, 3 if directory else 1, 56 + sum(map(len,attributes)) + 8, 1024)
    struct.pack_into("<I", record, 44, number)
    offset = 56
    for attribute in attributes:
        record[offset:offset+len(attribute)] = attribute
        offset += len(attribute)
    struct.pack_into("<I", record, offset, 0xffffffff)
    # Update Sequence Array: preserve each sector trailer before replacing it.
    record[48:50] = b"\x5a\xa5"
    for i, position in enumerate([510,1022]):
        record[50+2*i:52+2*i] = record[position:position+2]
        record[position:position+2] = record[48:50]
    return record

def create_ntfs(path, corrupt_boot=False):
    from create_sample_images import create_ntfs_vbr, create_mbr_partition_table
    start, sectors, mft_lcn = 2048, 8192, 4
    data = bytearray((start+sectors)*512)
    boot = create_ntfs_vbr(sectors, mft_lcn, 2)
    data[:512] = create_mbr_partition_table(start,sectors)
    data[start*512:(start+1)*512] = boot
    data[-512:] = boot
    # $MFT's nonresident $DATA stream covers 24 file records in six clusters.
    attribute = bytearray(72)
    struct.pack_into("<IIBBHHH",attribute,0,0x80,72,1,0,0,0,0)
    struct.pack_into("<QQHHIQQQ",attribute,16,0,5,64,0,0,24576,24576,24576)
    attribute[64:68] = bytes([0x11,6,mft_lcn,0])
    records = {0:file_record([attribute],0)}
    name = "evidence.txt".encode("utf-16le")
    key = bytearray(66+len(name))
    struct.pack_into("<Q",key,0,5 | (1<<48))
    struct.pack_into("<QQII",key,40,len(PAYLOAD),len(PAYLOAD),0x20,0)
    key[64],key[65] = len(name)//2,1
    key[66:] = name
    entry = bytearray(align8(16+len(key)))
    struct.pack_into("<QHHHH",entry,0,16 | (1<<48),len(entry),len(key),0,0)
    entry[16:16+len(key)] = key
    end = struct.pack("<QHHHH",0,16,0,2,0)
    index = struct.pack("<IIIB3x",0x30,1,4096,1)
    index += struct.pack("<IIIB3x",16,16+len(entry)+len(end),16+len(entry)+len(end),0)
    index += entry+end
    records[5] = file_record([resident_attribute(0x90,index,"$I30")],5,True)
    records[16] = file_record([resident_attribute(0x80,PAYLOAD)],16)
    for number,record in records.items():
        offset = start*512+mft_lcn*4096+number*1024
        data[offset:offset+1024] = record
    if corrupt_boot:
        data[start*512+510:start*512+512] = b"\0\0"
    path.write_bytes(data)
    return {"start":start,"backup":start+sectors-1,"name":"evidence.txt","payload":PAYLOAD}

def create_fat32(path, corrupt_boot=False):
    sectors,fat_size,reserved = 131072,1024,32
    boot=bytearray(512)
    boot[:3]=b"\xeb\x58\x90";boot[3:11]=b"MSDOS5.0"
    struct.pack_into("<HBHBHHBHHHII",boot,11,512,1,reserved,2,0,0,0xf8,0,63,255,0,sectors)
    struct.pack_into("<IHHIHH",boot,36,fat_size,0,0,2,1,6)
    boot[64]=0x80;boot[66]=0x29
    struct.pack_into("<I",boot,67,0x12345678)
    boot[71:82]=b"RESC TEST  ";boot[82:90]=b"FAT32   ";boot[510:512]=b"\x55\xaa"
    fsinfo=bytearray(512)
    struct.pack_into("<I",fsinfo,0,0x41615252)
    struct.pack_into("<IIII",fsinfo,484,0x61417272,0xffffffff,0xffffffff,0)
    struct.pack_into("<I",fsinfo,508,0xaa550000)
    fat=bytearray(fat_size*512)
    struct.pack_into("<IIII",fat,0,0x0ffffff8,0x0fffffff,0x0fffffff,0x0fffffff)
    root=bytearray(512);root[:11]=b"EVIDENCETXT";root[11]=0x20
    struct.pack_into("<H",root,26,3);struct.pack_into("<I",root,28,len(PAYLOAD))
    with path.open("wb") as output:
        output.truncate(sectors*512)
        for lba,chunk in [(0,boot),(6,boot),(1,fsinfo),(7,fsinfo),(reserved,fat),(reserved+fat_size,fat),(reserved+2*fat_size,root),(reserved+2*fat_size+1,PAYLOAD)]:
            output.seek(lba*512);output.write(chunk)
        if corrupt_boot:output.seek(510);output.write(b"\0\0")
    return {"start":0,"backup":6,"name":"EVIDENCE.TXT","payload":PAYLOAD}

def create_gpt(path, bad_header=False, bad_array=False, out_of_bounds=False):
    sectors=8192
    image=bytearray(sectors*512)
    image[510:512]=b"\x55\xaa"
    image[450]=0xee;struct.pack_into("<II",image,454,1,sectors-1)
    entries=bytearray(128*128)
    entries[:16]=bytes(range(1,17));entries[16:32]=bytes(range(17,33))
    struct.pack_into("<QQ",entries,32,2048,sectors+1 if out_of_bounds else 4095)
    crc=zlib.crc32(entries)
    for lba,alternate,array_lba in [(1,sectors-1,2),(sectors-1,1,sectors-33)]:
        header=bytearray(512)
        struct.pack_into("<8sIIIIQQQQ16sQIII",header,0,b"EFI PART",0x10000,92,0,0,lba,alternate,34,sectors-34,bytes(range(16)),array_lba,128,128,crc)
        struct.pack_into("<I",header,16,zlib.crc32(header[:92]))
        if bad_header:header[24]^=1
        image[lba*512:(lba+1)*512]=header
        image[array_lba*512:array_lba*512+len(entries)]=entries
        if bad_array:image[array_lba*512+80]^=1
    path.write_bytes(image)
