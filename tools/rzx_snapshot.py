#!/usr/bin/env python3
"""Take the snapshot out of an RZX recording, or put a different one in.

    rzx_snapshot.py get RECORDING.rzx OUT.z80
    rzx_snapshot.py put RECORDING.rzx SNAPSHOT.(z80|szx|sna) OUT.rzx

A recording is a snapshot and every byte the machine read from a port after
it. Putting in a snapshot of a rebuilt program, with the input left exactly as
it was, is how a rebuild is checked against somebody's play: if it is the same
program it reads the same keys at the same moments and stays in step. Only the
first snapshot is replaced; a recording carrying later ones (SPIN writes one
after a fast load) is refused, since those would put the old program back.
"""
import struct
import sys
import zlib


def blocks(data):
    if data[:4] != b"RZX!":
        raise SystemExit("not an RZX recording")
    at = 10
    while at + 5 <= len(data):
        kind = data[at]
        length = struct.unpack("<I", data[at + 1:at + 5])[0]
        yield kind, data[at:at + length]
        at += length


def snapshot_of(block):
    flags, ext, size = struct.unpack("<I4sI", block[5:17])
    body = block[17:]
    if flags & 1:
        raise SystemExit("the recording names a snapshot file rather than carrying one")
    if flags & 2:
        body = zlib.decompress(body)
    return ext.rstrip(b"\0").decode().lower(), body


def main():
    mode = sys.argv[1]
    data = open(sys.argv[2], "rb").read()
    found = [block for kind, block in blocks(data) if kind == 0x30]
    if not found:
        raise SystemExit("no snapshot in the recording")
    if mode == "get":
        ext, body = snapshot_of(found[0])
        open(sys.argv[3], "wb").write(body)
        print(f"{ext}, {len(body)} bytes")
        return
    if len(found) > 1:
        raise SystemExit(f"{len(found)} snapshots in the recording: the later ones would put the old program back")
    snap_path, out_path = sys.argv[3], sys.argv[4]
    snap = open(snap_path, "rb").read()
    ext = snap_path.rsplit(".", 1)[1].lower().encode()
    packed = zlib.compress(snap, 9)
    body = struct.pack("<I4sI", 2, ext.ljust(4, b"\0"), len(snap)) + packed
    new = bytes([0x30]) + struct.pack("<I", len(body) + 5) + body
    out = bytearray(data[:10])
    for kind, block in blocks(data):
        # The signature blocks sign the old contents; they would be a lie.
        if kind in (0x20, 0x21):
            continue
        out += new if kind == 0x30 else block
    open(out_path, "wb").write(out)
    print(f"{out_path}: the input of {sys.argv[2]} after a {ext.decode()} of {len(snap)} bytes")


if __name__ == "__main__":
    main()
