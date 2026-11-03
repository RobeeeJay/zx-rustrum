#!/usr/bin/env python3
"""Compare the code a tape carries with memory in a .sna, to prove an export is the game.

    tape_blocks.py TAPE                          list the standard blocks
    tape_blocks.py TAPE SNA BLOCK ADDRESS [SKIP]  compare block BLOCK's data, less
                                                  SKIP bytes from its start, with
                                                  memory from ADDRESS

Only standard-speed blocks (TZX ID $10, or any TAP block) are read; a turbo loader's
blocks need that loader's own decoding. Blocks are numbered from 0 as tape_blocks
lists them, header blocks included. A 48K .sna holds 27 bytes of registers and then
$4000-$FFFF.
"""
import sys


def blocks(data):
    out = []
    if data[:7] == b"ZXTape!":
        i = 10
        skip = {0x30: lambda i: 2 + data[i + 1], 0x32: lambda i: 3 + (data[i + 1] | data[i + 2] << 8),
                0x20: lambda i: 3, 0x21: lambda i: 2 + data[i + 1], 0x22: lambda i: 1}
        while i < len(data):
            kind = data[i]
            if kind == 0x10:
                n = data[i + 3] | data[i + 4] << 8
                out.append(data[i + 5:i + 5 + n])
                i += 5 + n
            elif kind in skip:
                i += skip[kind](i)
            else:
                raise SystemExit(f"block ID ${kind:02X} at {i} is not read here")
    else:
        i = 0
        while i + 2 <= len(data):
            n = data[i] | data[i + 1] << 8
            out.append(data[i + 2:i + 2 + n])
            i += 2 + n
    return out


def main():
    tape = blocks(open(sys.argv[1], "rb").read())
    if len(sys.argv) == 2:
        for k, b in enumerate(tape):
            check = 0
            for v in b[:-1]:
                check ^= v
            kind = "header" if b[0] == 0 else "data"
            ok = "ok" if check == b[-1] else "BAD CHECKSUM"
            extra = ""
            if b[0] == 0 and len(b) == 19:
                name = b[2:12].decode("latin-1").rstrip()
                length = b[12] | b[13] << 8
                param = b[14] | b[15] << 8
                extra = f' type {b[1]} "{name}" {length} bytes, param ${param:04X}'
            print(f"{k:3} {kind:6} {len(b) - 2:6} bytes  checksum {ok}{extra}")
        return
    sna = open(sys.argv[2], "rb").read()
    memory = bytes(0x4000) + sna[27:27 + 0xC000]
    block = tape[int(sys.argv[3])][1:-1]
    at = int(sys.argv[4].lstrip("$"), 16)
    skip = int(sys.argv[5]) if len(sys.argv) > 5 else 0
    want = block[skip:]
    got = memory[at:at + len(want)]
    differ = [at + k for k in range(len(want)) if got[k] != want[k]]
    print(f"{len(want)} bytes of block {sys.argv[3]} against ${at:04X}-${at + len(want) - 1:04X}: "
          f"{len(differ)} differ")
    for a in differ[:20]:
        print(f"  ${a:04X}: memory ${memory[a]:02X}, tape ${want[a - at]:02X}")


main()
