#!/usr/bin/env python3
"""A Z80 assembler for the emulator's own exports, taught by its disassembler.

    z80asm.py SOURCE.asm OUT.bin [--map OUT.map] [--mcp ./target/release/mcp]

No assembler is in the build, and an export only writes as a mnemonic an
instruction that has one encoding, so the encodings are not typed in here: every
opcode, with every prefix, is put in front of the emulator's disassembler with
known operand bytes behind it, and what comes back is the template. An
instruction assembles to whichever opcode the disassembler spells that way,
the shortest if more than one does. That makes it exactly as right as the
emulator's decoder, which zexdoc and zexall check, and no righter.

What it reads: ORG, EQU, labels on lines of their own ("name:"), DEFB, DEFW,
DEFM "text", DEFS count[,fill], and instructions whose operands are
expressions of labels, $hex, %binary and decimal with + - * / and brackets.
It writes the bytes from the lowest ORG to the highest address written, and
with --map every label's address, one "name $ADDR" a line.

BANK n, on a line of its own, puts what follows into the 128K's RAM bank n,
which is only ever seen at $C000-$FFFF, until a BANK with no number. Each
bank is written to OUT.bank<n>.bin, all 16K of it, and its labels go in the
map as "name $ADDR bank<n>": the same address can be in two banks, and the
bank says which.
"""
import json
import os
import re
import subprocess
import sys

MARK_D, MARK_N = 0x5A, 0xA5


def templates(binary):
    """{text template: (byte template, slots)}, built from the disassembler."""
    p = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    n = [0]

    def call(tool, args):
        n[0] += 1
        msg = {"jsonrpc": "2.0", "id": n[0], "method": "tools/call",
               "params": {"name": tool, "arguments": args}}
        p.stdin.write(json.dumps(msg) + "\n")
        p.stdin.flush()
        reply = json.loads(p.stdout.readline())
        return "\n".join(x.get("text", "") for x in reply["result"]["content"])

    found = {}
    for prefix in ([], [0xCB], [0xED], [0xDD], [0xFD], [0xDD, 0xCB], [0xFD, 0xCB]):
        base = 0x8000
        image = []
        for op in range(256):
            if len(prefix) == 2:
                ins = prefix + [MARK_D, op]
            else:
                ins = prefix + [op, MARK_D, MARK_N]
            image += ins + [0] * (8 - len(ins))
        for at in range(0, len(image), 256):
            call("write_memory", {"address": base + at, "bytes": image[at:at + 256]})
        lines = []
        at = base
        while at < base + len(image):
            out = call("disassemble", {"address": at, "count": 200, "comments": False}).splitlines()
            lines += out
            at = int(re.search(r'next address \$([0-9A-F]{4})', out[-1]).group(1), 16)
        for line in lines:
            m = re.match(r'^\$([0-9A-F]{4})  ((?:[0-9A-F]{2} )+)\s*(.*?)\s*(;.*)?$', line)
            if not m:
                continue
            addr = int(m.group(1), 16)
            if (addr - base) % 8:
                continue
            op = (addr - base) // 8
            code = [int(x, 16) for x in m.group(2).split()]
            text = m.group(3)
            if len(prefix) == 2:
                want = prefix + [MARK_D, op]
                if code != want:
                    continue
                slots = [("d", 2)]
            else:
                head = prefix + [op]
                if code[:len(head)] != head:
                    continue
                slots = []
            if text.startswith(("DEFB", "??")):
                continue
            tpl = text
            if re.search(r'\((I[XY])\+\$5A\)', tpl):
                tpl = re.sub(r'\((I[XY])\+\$5A\)', r'(\1+{d})', tpl)
                if len(prefix) != 2:
                    slots.append(("d", len(prefix) + 1))
            if "$A55A" in tpl:
                tpl = tpl.replace("$A55A", "{nn}")
                slots.append(("nn", len(code) - 2))
            rel = f"${addr + 2 + MARK_D:04X}"
            if len(code) == 2 and len(prefix) == 0 and tpl.endswith(rel):
                tpl = tpl[:-len(rel)] + "{e}"
                slots.append(("e", 1))
            if "$5A" in tpl:
                tpl = tpl.replace("$5A", "{n}")
                slots.append(("n", len(code) - 1))
            if "$A5" in tpl:
                tpl = tpl.replace("$A5", "{n}")
                slots.append(("n", len(code) - 1))
            fixed = [b for i, b in enumerate(code)]
            for kind, at in slots:
                for k in range(2 if kind == "nn" else 1):
                    fixed[at + k] = None
            known = found.get(tpl)
            # DD CB d op reads every op whose low three bits are not 6 as the
            # same BIT n,(IX+d) as the one that are, so the documented one
            # has to be asked for rather than taken as the first seen.
            documented = len(prefix) == 2 and op & 7 == 6
            if known is None or len(fixed) < len(known[0]) or documented:
                found[tpl] = (fixed, slots)
    p.stdin.close()
    p.wait()
    return found


def compile_templates(found):
    """By mnemonic: (regex, fixed bytes, slots), literal templates first."""
    by = {}
    for tpl, (fixed, slots) in found.items():
        pattern = ""
        names = []
        for piece in re.split(r'(\{\w+\})|(\+\{d\})', tpl):
            if not piece:
                continue
            if piece == "+{d}":
                pattern += r'(?P<d>[+-][^)]*)'
                names.append("d")
            elif piece in ("{n}", "{nn}", "{e}"):
                name = piece[1:-1]
                group = name if name not in names else name + "2"
                pattern += rf'(?P<{group}>[^,()]+?)'
                names.append(group)
            else:
                pattern += re.escape(piece)
        mnemonic = tpl.split()[0]
        by.setdefault(mnemonic, []).append((re.compile("^" + pattern + "$"), fixed, slots, names, tpl))
    for options in by.values():
        options.sort(key=lambda o: (len(o[3]), len(o[1])))
    return by


def strip_comment(s):
    quoted = False
    for i, ch in enumerate(s):
        if ch == '"':
            quoted = not quoted
        if ch == ';' and not quoted:
            return s[:i].strip()
    return s.strip()


class Assembler:
    def __init__(self, table):
        self.table = table
        self.names = {}

    def value(self, expr, pc, final):
        text = expr.strip()

        def name(m):
            word = m.group(0)
            if word in self.names:
                return str(self.names[word])
            if final:
                raise ValueError(f"unknown name {word}")
            return "0"
        text = re.sub(r'\$([0-9A-Fa-f]+)', lambda m: str(int(m.group(1), 16)), text)
        text = re.sub(r'%([01]+)', lambda m: str(int(m.group(1), 2)), text)
        text = re.sub(r'\$', str(pc), text)
        text = re.sub(r'\b[A-Za-z_]\w*\b', name, text)
        if not re.fullmatch(r'[0-9+\-*/() ]+', text):
            raise ValueError(f"cannot read {expr!r}")
        return int(eval(text.replace("/", "//")))

    def encode(self, line, pc, final):
        mnemonic = line.split()[0]
        options = self.table.get(mnemonic, [])
        problem = f"no instruction is spelt {line!r}"
        for regex, fixed, slots, names, tpl in options:
            m = regex.match(line)
            if not m:
                continue
            try:
                out = list(fixed)
                groups = [m.group(g) for g in names]
                for (kind, at), got in zip(slots, groups):
                    v = self.value(got, pc, final)
                    if kind == "nn":
                        if final and not -0x8000 <= v <= 0xFFFF:
                            raise ValueError(f"{v} does not fit a word")
                        out[at], out[at + 1] = v & 255, (v >> 8) & 255
                    elif kind == "e":
                        off = v - (pc + len(out))
                        if final and not -128 <= off <= 127:
                            raise ValueError(f"{got} is {off} bytes away, out of reach")
                        out[at] = off & 255
                    else:
                        if final and not -128 <= v <= 255:
                            raise ValueError(f"{v} does not fit a byte")
                        out[at] = v & 255
                return out
            except ValueError as e:
                problem = f"{line!r}: {e}"
        raise ValueError(problem)

    def run(self, lines):
        for final in (False, True, True):
            pc = None
            bank = None
            image = {}
            starts = []
            self.listing = []
            self.banks = {}
            for number, raw in lines:
                l = strip_comment(raw)
                if not l:
                    continue
                try:
                    m = re.match(r'^(\w+)\s+EQU\s+(.+)$', l)
                    if m:
                        self.names[m.group(1)] = self.value(m.group(2), pc or 0, final)
                        continue
                    m = re.match(r'^ORG\s+(.+)$', l)
                    if m:
                        pc = self.value(m.group(1), 0, final)
                        continue
                    m = re.match(r'^BANK(\s+(.+))?$', l)
                    if m:
                        bank = self.value(m.group(2), 0, final) if m.group(2) else None
                        if bank is not None and not 0 <= bank <= 7:
                            raise ValueError(f"there is no bank {bank}: a 128K has 0 to 7")
                        continue
                    m = re.match(r'^(\w+):$', l)
                    if m:
                        self.names[m.group(1)] = pc
                        if bank is not None:
                            self.banks[m.group(1)] = bank
                        continue
                    word = l.split()[0]
                    rest = l[len(word):].strip()
                    if word == "DEFB":
                        out = [self.value(x, pc, final) & 255 for x in split_args(rest)]
                    elif word == "DEFW":
                        out = []
                        for x in split_args(rest):
                            v = self.value(x, pc, final)
                            out += [v & 255, (v >> 8) & 255]
                    elif word == "DEFM":
                        out = list(rest[1:-1].encode("latin-1"))
                    elif word == "DEFS":
                        args = split_args(rest)
                        count = self.value(args[0], pc, final)
                        fill = self.value(args[1], pc, final) if len(args) > 1 else 0
                        # DEFS $D200-$ is how a rebuild keeps what follows on
                        # its page; counted backwards, what comes before has
                        # grown past it, and a silent nothing moved it.
                        if final and count < 0:
                            raise ValueError(f"DEFS runs {-count} bytes backwards at ${pc:04X}: "
                                             "what comes before it is too long")
                        out = [fill & 255] * max(count, 0)
                    else:
                        out = self.encode(l, pc, final)
                        starts.append((pc, number))
                except ValueError as e:
                    raise SystemExit(f"line {number}: {e}")
                self.listing.append((number, pc, len(out)))
                if final and pc + len(out) > 0x10000:
                    raise SystemExit(f"line {number}: runs past $FFFF")
                if final and bank is not None and out and pc < 0xC000:
                    raise SystemExit(f"line {number}: ${pc:04X} is in bank {bank}, which is only "
                                     "seen from $C000")
                for i, b in enumerate(out):
                    key = (bank, pc + i)
                    if final and key in image:
                        where = f" in bank {bank}" if bank is not None else ""
                        raise SystemExit(f"line {number}: ${pc + i:04X}{where} is written twice")
                    image[key] = b
                pc += len(out)
        return image, starts


def split_args(s):
    out, depth, cur, quoted = [], 0, "", False
    for ch in s:
        if ch == '"':
            quoted = not quoted
        if ch == "," and depth == 0 and not quoted:
            out.append(cur)
            cur = ""
            continue
        depth += ch == "("
        depth -= ch == ")"
        cur += ch
    out.append(cur)
    return [x.strip() for x in out if x.strip()]


def load_table(binary):
    cache = os.path.join(os.path.dirname(os.path.abspath(binary)), "z80asm-templates.json")
    if os.path.exists(cache) and os.path.getmtime(cache) > os.path.getmtime(binary):
        found = {k: (v[0], [tuple(s) for s in v[1]]) for k, v in json.load(open(cache)).items()}
    else:
        found = templates(binary)
        json.dump(found, open(cache, "w"))
    return compile_templates(found)


def main():
    args = sys.argv[1:]
    binary = "./target/release/mcp"
    out_map = None
    if "--mcp" in args:
        i = args.index("--mcp")
        binary = args[i + 1]
        del args[i:i + 2]
    if "--map" in args:
        i = args.index("--map")
        out_map = args[i + 1]
        del args[i:i + 2]
    source, out = args
    asm = Assembler(load_table(binary))
    lines = list(enumerate(open(source, encoding="latin-1").read().splitlines(), 1))
    image, starts = asm.run(lines)
    main_image = {a: b for (bank, a), b in image.items() if bank is None}
    lo, hi = min(main_image), max(main_image)
    data = bytes(main_image.get(a, 0) for a in range(lo, hi + 1))
    open(out, "wb").write(data)
    stem = out[:-4] if out.endswith(".bin") else out
    banks = sorted({bank for bank, _ in image if bank is not None})
    for bank in banks:
        page = bytes(image.get((bank, a), 0) for a in range(0xC000, 0x10000))
        open(f"{stem}.bank{bank}.bin", "wb").write(page)
    if out_map:
        with open(out_map, "w") as f:
            for name, addr in sorted(asm.names.items(), key=lambda kv: (kv[1], kv[0])):
                bank = asm.banks.get(name)
                f.write(f"{name} ${addr:04X}" + (f" bank{bank}" if bank is not None else "") + "\n")
    print(f"${lo:04X}-${hi:04X}, {len(data)} bytes, {len(starts)} instructions, {len(asm.names)} names"
          + (f", banks {', '.join(str(b) for b in banks)}" if banks else ""))


if __name__ == "__main__":
    main()
