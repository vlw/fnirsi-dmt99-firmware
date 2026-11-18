"""Put a firmware .BIN onto the DMT-99 bootloader disk from Linux.

The WCH bootloader exposes a 4096-byte-sector disk whose MBR/BPB make Linux truncate the
partition and mount it read-only. This edits the FAT12 volume in a dump of the disk the way
Windows would (8.3 entry, contiguous clusters, both FATs) and writes back only changed blocks.

  python3 bootdisk.py build DUMP.img FIRMWARE.BIN TARGET.img     # as user
  sudo python3 -I bootdisk.py write /dev/sdX DUMP.img TARGET.img  # as root, device unmounted
  sudo python3 -I bootdisk.py flash FIRMWARE.BIN [BACKUP_DIR]     # all of the above, device found automatically
"""
import os
import struct
import sys
import time

BLK = 4096


def volume(img):
    assert img[510:512] == b"\x55\xaa", "no MBR signature"
    start = struct.unpack_from("<I", img, 446 + 8)[0] * BLK  # MBR LBA is in the device's 4K blocks
    b = img[start:start + 512]
    bps, spc, res, nf, rootent, tot16, _, fatsz = struct.unpack_from("<HBHBHHBH", b, 11)
    assert (bps, spc, b[54:62]) == (BLK, 1, b"FAT12   "), "unexpected volume format"
    fat = start + res * bps
    root = fat + nf * fatsz * bps
    data = root + rootent * 32
    return dict(start=start, fat=fat, nf=nf, fatsz=fatsz * bps, root=root, rootent=rootent, data=data)


def fat_get(img, v, n):
    o = v["fat"] + n * 3 // 2
    x = img[o] | img[o + 1] << 8
    return x >> 4 if n & 1 else x & 0xFFF


def fat_set(img, v, n, val):
    for c in range(v["nf"]):
        o = v["fat"] + c * v["fatsz"] + n * 3 // 2
        x = img[o] | img[o + 1] << 8
        x = (x & 0x000F) | val << 4 if n & 1 else (x & 0xF000) | val
        img[o], img[o + 1] = x & 0xFF, x >> 8


def entries(img, v):
    for i in range(v["rootent"]):
        o = v["root"] + i * 32
        yield i, o, img[o:o + 32]


def read_file(img, v, name):
    for _, _, e in entries(img, v):
        if e[0] == 0:
            break
        if e[:11] == name and e[0] != 0xE5:
            clus, size = struct.unpack_from("<H", e, 26)[0], struct.unpack_from("<I", e, 28)[0]
            out = b""
            while clus < 0xFF8:
                o = v["data"] + (clus - 2) * BLK
                out += img[o:o + BLK]
                clus = fat_get(img, v, clus)
            return out[:size]
    raise KeyError(name)


def remove_bins(img, v):
    """Delete leftover .BIN files (a flash that was never confirmed with AUTO)."""
    for _, o, e in entries(img, v):
        if e[0] in (0, 0xE5) or e[11] == 0x0F or e[11] & 0x18 or e[8:11] != b"BIN":
            continue
        clus = struct.unpack_from("<H", e, 26)[0]
        while 2 <= clus < 0xFF8:
            nxt = fat_get(img, v, clus)
            fat_set(img, v, clus, 0)
            clus = nxt
        img[o] = 0xE5
        print(f"removed leftover {bytes(e[:11]).decode(errors='replace')}")


def build_image(dump, fw, name=b"DMT99FW BIN", replace=False):
    img = bytearray(dump)
    v = volume(img)
    if replace:
        remove_bins(img, v)
    used = {n for n in range(2, len(img) // BLK) if fat_get(img, v, n)}
    live = [e for _, _, e in entries(img, v) if e[0] not in (0, 0xE5)]
    used |= {struct.unpack_from("<H", e, 26)[0] for e in live}  # e.g. a dir whose FAT entry is free
    assert all(e[:11] != name for e in live), "file already on the disk"
    assert not any(e[11] != 0x0F and e[8:11] == b"BIN" for e in live), "another .BIN already on the disk"
    need = -(-len(fw) // BLK)
    last_phys = 2 + (len(img) - v["data"]) // BLK - 1  # the volume claims more space than the device has
    first = next(c for c in range(2, last_phys - need + 2) if not used & set(range(c, c + need)))
    for i, c in enumerate(range(first, first + need)):
        fat_set(img, v, c, c + 1 if i < need - 1 else 0xFFF)
        o = v["data"] + (c - 2) * BLK
        chunk = fw[i * BLK:(i + 1) * BLK]
        img[o:o + BLK] = chunk + b"\x00" * (BLK - len(chunk))
    slot = next(o for i, o, e in entries(img, v) if e[0] in (0, 0xE5))  # first free slot, like Windows
    t = time.localtime()
    ftime = t.tm_hour << 11 | t.tm_min << 5 | t.tm_sec // 2
    fdate = (t.tm_year - 1980) << 9 | t.tm_mon << 5 | t.tm_mday
    img[slot:slot + 32] = name + struct.pack("<BBBHHHHHHHI", 0x20, 0, 0, ftime, fdate, fdate, 0, ftime, fdate, first, len(fw))
    assert read_file(img, v, name) == fw, "self-check: file does not read back through FAT"
    print(f"{name.decode()}: {len(fw)} bytes at clusters {first}..{first + need - 1}")
    return img


def build(dump_path, bin_path, out_path, name=b"DMT99SNDBIN"):
    img = build_image(open(dump_path, "rb").read(), open(bin_path, "rb").read(), name)
    open(out_path, "wb").write(img)


def find_device():
    """The WCH bootloader disk: model 'USB Special Disk', 400 x 4096-byte blocks."""
    for name in sorted(os.listdir("/sys/block")):
        try:
            model = open(f"/sys/block/{name}/device/model").read().strip()
            blocks = int(open(f"/sys/block/{name}/size").read())
        except OSError:
            continue
        if model == "USB Special Disk" and blocks == 3200:
            return f"/dev/{name}"
    return None


def flash(bin_path, backup_dir=None):
    """Find the meter's boot disk, back it up, add the firmware the way Windows would, verify."""
    fw = open(bin_path, "rb").read()
    assert fw[:2] == b"\x6f\x00" or fw[:2] == b"\x6f\x60", "not a DMT-99 application image"
    assert struct.unpack_from("<I", fw, 0x38)[0] == 0x6400, "not a DMT-99 application image"
    dev = find_device()
    assert dev, "meter not found: power it on holding AUTO/MENU and connect USB"
    for part in sorted(os.listdir(f"/sys/block/{os.path.basename(dev)}")):
        if part.startswith(os.path.basename(dev)):
            os.system(f"umount /dev/{part} 2>/dev/null")
    with open(dev, "rb") as f:
        dump = f.read()
    if backup_dir:
        os.makedirs(backup_dir, exist_ok=True)
        path = os.path.join(backup_dir, time.strftime("boot-%Y%m%d-%H%M%S.img"))
        open(path, "wb").write(dump)
        uid, gid = int(os.environ.get("SUDO_UID", -1)), int(os.environ.get("SUDO_GID", -1))
        if uid >= 0:
            os.chown(path, uid, gid)
            os.chown(backup_dir, uid, gid)
        print(f"backup: {path}")
    target = build_image(dump, fw, replace=True)
    write_blocks(dev, dump, bytes(target))


def write_blocks(dev, dump, target):
    name = os.path.basename(os.path.realpath(dev))
    model = open(f"/sys/block/{name}/device/model").read().strip()
    assert model == "USB Special Disk", f"{dev} is '{model}', not the DMT-99 boot disk"
    with open(dev, "rb") as f:
        assert f.read() == dump, "device content differs from the dump; re-dump and rebuild"
    v = volume(target)
    blocks = [i for i in range(len(target) // BLK) if target[i * BLK:(i + 1) * BLK] != dump[i * BLK:(i + 1) * BLK]]
    meta = {v["fat"] // BLK + k for k in range(v["nf"] * v["fatsz"] // BLK)}
    rootblks = set(range(v["root"] // BLK, v["data"] // BLK))
    order = [b for b in blocks if b not in meta | rootblks] + sorted(meta & set(blocks)) + sorted(rootblks & set(blocks))
    fd = os.open(dev, os.O_WRONLY | os.O_SYNC)
    try:
        for b in order:  # data first, then FATs, directory entry last
            os.pwrite(fd, target[b * BLK:(b + 1) * BLK], b * BLK)
        os.fsync(fd)
    finally:
        os.close(fd)
    os.system(f"blockdev --flushbufs {dev}")
    with open(dev, "rb") as f:
        assert f.read() == target, "read-back mismatch"
    print(f"wrote {len(order)} blocks; read-back OK")


def write(dev, dump_path, target_path):
    write_blocks(dev, open(dump_path, "rb").read(), open(target_path, "rb").read())


if __name__ == "__main__":
    cmd, *a = sys.argv[1:]
    {"build": build, "write": write, "flash": flash}[cmd](*a)
