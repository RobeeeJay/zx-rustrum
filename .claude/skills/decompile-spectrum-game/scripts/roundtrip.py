"""Read an exported .asm back the way an assembler would, against the memory it came from.

    roundtrip.py FILE.asm STATE.sna FROM TO [MCP-BINARY]
    roundtrip.py FILE.asm live FROM TO [MCP-BINARY]

FROM and TO are hex, as given to export_asm. Every ORG, EQU and label is resolved,
every DEFB, DEFW and DEFM is turned into bytes and compared with memory, and every
instruction, with its names put back as addresses, is compared with the emulator's
own disassembly of the bytes at that address. It starts a server of its own, so it
does not disturb the one being worked in.

The memory to compare with is a .sna, or "live": what the server being worked in
holds now, read through mcp_call, which is the state the export was just made from.
A .sna pushes PC into the two bytes below SP, while export_asm writes what the
program had there (restore_state puts those bytes back), so a difference in just
those two bytes of a .sna is reported as the format's and not counted.

What this proves: the layout, the names and the data. What it does not: that a real
assembler spells every mnemonic the same way. For that, assemble it with sjasmplus
(--raw) and compare the binary with the same memory.
"""
import subprocess,json,re,sys
asm_path,sna_path=sys.argv[1],sys.argv[2]
lo,hi=int(sys.argv[3].lstrip("$"),16),int(sys.argv[4].lstrip("$"),16)
binary=sys.argv[5] if len(sys.argv)>5 else "./target/release/mcp"
pushed=set()
if sna_path=="live":
    import os
    sys.path.insert(0,os.path.dirname(os.path.abspath(__file__)))
    from mcp_call import call as live_call
    mem=bytearray(0x4000)
    for a in range(0x4000,0x10000,4096):
        for row in live_call("read_memory",{"address":f"${a:04X}","length":4096}).splitlines()[1:]:
            mem+=bytes(int(x,16) for x in row[7:54].split())
    mem=bytes(mem)
else:
    sna=open(sna_path,'rb').read(); mem=bytes(0x4000)+sna[27:27+0xC000]
    sp=sna[23]|sna[24]<<8
    pushed={sp,(sp+1)&0xFFFF}
p=subprocess.Popen([binary],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
n=0
def call(t,a):
    global n; n+=1
    p.stdin.write(json.dumps({"jsonrpc":"2.0","id":n,"method":"tools/call","params":{"name":t,"arguments":a}})+"\n"); p.stdin.flush()
    return "\n".join(x.get("text","") for x in json.loads(p.stdout.readline())["result"]["content"])
for a in range(0x4000,0x10000,256):
    call("write_memory",{"address":a,"bytes":list(mem[a:a+256])})
def disasm(a):
    line=call("disassemble",{"address":a,"count":1,"comments":False}).splitlines()[0]
    m=re.match(r'^\$([0-9A-F]{4})  ((?:[0-9A-F]{2} )+)\s*(.*?)\s*$',line)
    return len(m.group(2).split()), m.group(3)
def strip_comment(s):
    q=False
    for i,ch in enumerate(s):
        if ch=='"': q=not q
        if ch==';' and not q: return s[:i].strip()
    return s.strip()
lines=[strip_comment(l) for l in open(asm_path)]
equ={}; labels={}; body=[]
pc=None
# pass 1: addresses, using the length of what is in memory for each instruction
for l in lines:
    if not l: continue
    m=re.match(r'^(\w+)\s+EQU\s+\$([0-9A-F]{4})$',l)
    if m: equ[m.group(1)]=int(m.group(2),16); continue
    m=re.match(r'^ORG\s+\$([0-9A-F]{4})$',l)
    if m: pc=int(m.group(1),16); continue
    m=re.match(r'^(\w+):$',l)
    if m: labels[m.group(1)]=pc; continue
    if l.startswith(('DEFB','DEFW','DEFM')):
        if l.startswith('DEFB'): size=len(l[5:].split(','))
        elif l.startswith('DEFW'): size=2
        else: size=len(l[6:-1])
        body.append((pc,l,size)); pc+=size; continue
    size,_=disasm(pc); body.append((pc,l,size)); pc+=size
names={**equ,**labels}
def value(tok):
    return names[tok] if tok in names else int(tok[1:],16)
errors=[]; notes=[]; data=code=0
for pc,l,size in body:
    if l.startswith('DEFB'):
        got=bytes(int(x.strip()[1:],16) for x in l[5:].split(',')); data+=size
    elif l.startswith('DEFW'):
        v=value(l[5:].strip()); got=bytes([v&255,v>>8]); data+=size
    elif l.startswith('DEFM'):
        got=l[6:-1].encode(); data+=size
    else:
        code+=1
        text=re.sub(r'\b[A-Za-z_]\w*\b',lambda m: f"${names[m.group(0)]:04X}" if m.group(0) in names else m.group(0),l)
        _,want=disasm(pc)
        if text!=want: errors.append(f"${pc:04X}: wrote {l!r} which is {text!r}; memory disassembles as {want!r}")
        continue
    if got!=mem[pc:pc+size]:
        differ={pc+i for i in range(size) if got[i]!=mem[pc+i]}
        if differ<=pushed: notes.append(f"${min(differ):04X}: the .sna's pushed PC, not the program's bytes; not counted")
        else: errors.append(f"${pc:04X}: {l!r} gives {got.hex()} where memory holds {mem[pc:pc+size].hex()}")
end=body[-1][0]+body[-1][2]
print(f"{code} instructions and {data} bytes of data, ${body[0][0]:04X} to ${end-1:04X}; {len(labels)} labels, {len(equ)} EQUs")
print("covers exactly the range asked for:", body[0][0]==lo and end==hi+1)
inside=[name for name,a in equ.items() if lo<=a<=hi]
if inside: print(f"{len(inside)} names inside the range are EQUs, not labels: a claim or an instruction hides where they start:", ", ".join(inside[:10]))
[print(n) for n in notes]
print(f"{len(errors)} mismatches"); [print(" ",e) for e in errors[:20]]
