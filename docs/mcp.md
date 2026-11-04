# The emulator as an MCP server

`cargo run --release --bin mcp` speaks [MCP](https://modelcontextprotocol.io)
over stdin and stdout: JSON-RPC 2.0, one message a line. It is the emulator
without a window — the same `Spectrum`, the same observer, the same
disassembler — driven by a program rather than by a person.

What it is for is taking a game apart: load it, run it under control, watch
what it does, and write down what has been worked out.

```jsonc
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":3,"method":"tools/call",
 "params":{"name":"load_tape","arguments":{"path":"tapes/Jetpac.tzx"}}}
```

Every reply is text. The thing reading it is a language model, and a table it
can quote back is worth more than a structure it has to re-serialise.

## The tools

**Getting a machine going.** `machine_info`, `set_machine`, `reset`,
`load_tape`, `load_snapshot`, `load_recording`, `play_recording`.

`load_tape` starts the tape and runs it to its end with the ROM loader
handed over, so a four-minute tape loads in a second or two of real time; pass
`autoload: false` to put it in the deck without starting it. `load_snapshot` is
quicker still and starts in the middle of a game, which is usually where the
interesting code is.

It types the same `LOAD ""` on every model, and the menu on a 128K, +2A or +3
ignores the letters and takes the `ENTER` as its first line, the tape loader. If the
machine never calls the ROM's loader at `$0556`, the tool fails rather than
pretending. That only says it started reading, though, and a game can still fail
after that. Jetpac does on a 128K: from the menu, its stub has its `LDIR` at `$5B89`,
on a byte the ROM decrements every other frame, so it becomes a `CALL` and resets the
machine. From 48 BASIC, with paging locked, it runs, so `basic48: true` goes there
from the menu first and fails if the paging doesn't lock. When PC ends up in the ROM
the tool says so, and it's worth looking at the screen.

**The map, before disassembling.** `memory_activity`, `tape_blocks`, `loader`.

**The +3's drive.** `mount_disk`, `new_disk`, `eject_disk`, `disk_info`,
`disk_speed`, `disk_catalogue`, `read_sector`, `disk_activity`. `mount_disk` takes a `.dsk` or a `.zip` with one inside. A disk is
mounted read-only unless asked otherwise, and a writable mount says where the
writes go — `copy_to` writes the image to a new file first, and is the only way
to write a disk that came out of an archive. `read_sector` reads
the image rather than driving the drive, so it works whatever the machine is
doing. See [`disks.md`](disks.md).

`memory_activity` is the access map: reads, writes and whether anything ran
there, per 256-byte page, with a guess at what each page is for and whether a
back buffer has been detected. `tape_blocks` decodes the headers — what loads
where — which is the memory map before there is one.

**Running it.** `step` (with `over` for stepping over a `CALL`), `run_frames`,
`run_tstates`, `run_until`, `watch_events`.

`run_until` is the breakpoint, and `watch_events`'s `write_to` is the
watchpoint: stop at whatever writes to an address, which — once
`changed_since` has found the address — is how the code behind a variable is
found. `watch_events` is the more useful one for
finding your way around a program nobody has documented: it stops on *what the
program did* rather than on where it is — a write to the display file, the
beeper or the sound chip being touched, the frame interrupt, a call into the
ROM, any `IN`, any `OUT`. Switch on `screen` and run, and the machine stops at
the instruction that drew something.

**Looking at it.** `screen`, `graphics`.

`screen` sends a PNG in an image block — a model with eyes sees the picture —
along with a sketch of the 32x24 character cells and what the attributes are
set to, for one that cannot. `graphics` reads memory as characters and sprites,
eight bytes each, which is how graphics are found: point it at a candidate
address and see whether letters, a sprite or rubbish comes out.

**Typing at it.** `press_keys`, `type_text`, and `mouse` for whichever mouse is
fitted — moves in the machine's pixels, buttons, then a few frames. For an AMX
mouse the movement is queued as steps that its PIO delivers as interrupts,
once the program has turned them on.

**The printer.** `printout` gives what a fitted ZX Printer or Alphacom 32 has
printed: the paper as a PNG, a pixel a dot, and any text on it read back
through the font CHARS points at, as Fuse reads it.

The ROM scans the keyboard once a frame and wants a key on two scans running
before it believes in it, so a key is held for ten frames by default. Several
keys at once is a chord: `["CAPS SHIFT", "1"]`. This is how a game is driven to
the level worth looking at, and how the input routine is found — press
something and see who reads port $FE.

**Reading it.** `registers`, `read_memory`, `write_memory`, `disassemble`,
`load_symbols`, `symbols`, `identify`.

`load_symbols` reads the symbol files for the machine out of the preferences
directory — `tools/rom-symbols.py` and `tools/skool-symbols.py` write them —
and builds the table of routines recognisable by their first twelve bytes.
After it, `disassemble` names the address a `CALL` names, and `identify` will
tell you that the code a game copied to $9000 is the ROM's own CLS. A name
typed with `set_comment` always wins over one from a file.

Addresses may be numbers or strings: `32768`, `"$8000"`, `"0x8000"`, `"8000h"`.
A bare string of digits is decimal, deliberately — reading decimal addresses as
hex is how `tools/skool-symbols.py` once found nothing at all and said so
quietly.

`write_memory` pokes the running machine and nothing else; it is for testing a
theory (freeze the counter and see what stops moving), not for patching a file.

**Watching it.** `watch_routines`, then `routines`, `routine`, `call_graph`,
`code_map`, `xrefs`, `blocks`, `profile`, `frame_timing`, `autodoc`.

`xrefs` answers "what refers to this address" two ways at once, and labels
which is which: what the machine was *watched* doing is fact, and what the
*code* says is a search whose hits include data that happens to look like an
instruction.

The observer attributes every write, read, port access and loop to the routine
on top of the call stack, worked out from what the CPU did rather than by
decoding opcodes. Nothing is counted until `watch_routines` switches it on, and
the tools say so rather than answering with an empty table that reads like an
answer. It costs a branch on every access, so switch it off when finished.

**How far it has got.** `coverage`, `indirect_jumps`.

Code is what has run, so a disassembly built from watching stops at whatever
was never played. `coverage` says where that edge is: what has run, what the
code that has run could reach but has not, what has only been read, and which
conditionals have only ever gone one way. What a conditional did is recorded as
it runs, because "its target ran" does not mean it was ever taken. A one-sided
branch says what to make happen next, and a stretch not reached says what that
will find.

`indirect_jumps` is for the jumps the code does not name. Every `JP (HL)`,
`JP (IX)` and `JP (IY)` is written down with where it went and what ran just
before it, then those targets are looked for side by side in memory, which finds
the jump table with the entries nothing has jumped to yet. Those entries lead
`coverage` to handlers nothing else mentions. Pointed at Jetpac without being
told anything, it finds the 18 object handlers at `$6374`, ten of them seen.

Both only know what they were watching for. A `PUSH HL : RET` used as a jump is
not seen as one, since nothing about a `RET` says which it is. Unreached code is
decoded, not run, so a stretch can run on into something that isn't code. And
code that has since been overwritten still counts as having run: Jetpac's first
four bytes ran as a bootstrap before its loader moved the program over them, so
what's there now decodes as nonsense that `coverage` follows. Where a table ends
is a guess, and a word beyond it may get listed.

**What's left to explain.** `unexplained`.

For each labelled stretch of run code it says whether there's a description and how
many instructions have a comment of their own, biggest gap first, and which called
routines have no name. It only counts. A bad comment scores the same as a good one,
so what it's for is choosing what to read next, and it can't tell you you've
finished.

`export_listing` adds what was measured to what was written, so a listing with few
comments still says something. Above each label, `; used by:` names the routines that
mention it and `; seen:` says how often it was called and what it wrote. Beside an
instruction, `[never taken]` and `[always taken]` are branches that only went one way,
`[forced]` is code that only ran under `force_branch`, and `[rewritten by …]` was
changed after it ran. They're told apart from comments by being in that form, and
`facts: false` leaves them out.

**Pictures to keep.** `export_flowchart`, `export_call_graph`, `export_sprites`.

`export_flowchart` draws one routine as a flow chart: top to bottom in address order,
from the code that ran, a box for each run of instructions with what it calls and what
the notes say, a diamond for each decision with how often each way was taken (dashed
if never), jumps forward down the right and loops back up the left, and jumps out to
other routines named at the side. It's the picture of a game loop, which the call graph
can't give, because a main loop is jumped round and never called. A handler run
through a table — the return address pushed, then `JP (HL)`, or a `JP` to a lone
`JP (HL)` somewhere else — is drawn as a call that comes back where the push said, and
a jump back to code the chart has already fallen into is a loop, even if something
else calls that code too.

`export_call_graph` writes who called whom as an SVG: the busiest routines (60 by
default) in layers from left to right, named from the notes, a thicker line for a call
made more often and a dashed one going back for a loop, with each routine's
description as its tooltip. It's the same layout the Call flow window draws, kept as a
file. `export_sprites` writes every bitmap described with `set_data` as a PNG of its
own, named by its label: ink black, paper white, and outside a masked sprite's mask
transparent, the right way up.

**What it does to itself.** `self_modifying`, `watch_value`, `value_log`.

A listing shows one moment's bytes, and on a program that patches its own code,
that moment isn't always the truth. `self_modifying` lists what has rewritten code
that had already run: where, how many times, by which instruction and with which
values. A couple of values is a switch being thrown; one instruction and thousands
of bytes is a loader moving a block over its own start. Code written before it
first ran isn't counted, so unpacking doesn't bury the real ones, and nor is a
write of the value already there.

`watch_value` logs every change to a byte or a few, and `value_log` says which
instructions made them, how many each, and then the changes in order with the frame
and T-state. That's how you find out what takes a life without stopping the machine
at each write. The latest 4,096 changes are kept per watch, and it tells you how
many it let go.

**The other side of a branch.** `force_branch`.

`coverage` says which conditionals have only gone one way. `force_branch` makes one
go the other way: it runs until the machine reaches it, turns over the flag it
tests (or sets `B`, for `DJNZ`), runs on, and tells you what code that reached.

This happens on a copy, so the real machine comes back exactly as it was, and of
everything the copy did only the record of which bytes ran is kept. Those bytes
show as code in the listing and are marked as forced, and `coverage` counts them
separately and goes on calling the branch one-sided. That matters. Only one flag was
changed, so what follows may be something the program could never do: the registers
are whatever they were, not whatever the other route would have made them. It tells
you what's there to read, not what happens.

**What the data is.** `set_data`, `define_struct`, `struct_array`, `bind_struct`.

A listing of hex tells you nothing about a sprite. `set_data` says what a stretch
holds, and the listing and `export_asm` then show it as that: words as `DEFW` with
names for the addresses, text as `DEFM`, and a bitmap as rows of `DEFB` with the
picture above them:

```text
;       ####            ####
; ################################
$8000  DEFB $00,$02,$02
$8003  DEFB $18,$18
$8005  DEFB $FF,$FF
```

It's drawn at two characters a pixel, because a character is about twice as tall as
it's wide and one a pixel squashes a sprite to a sliver. Width, rows and how many
bytes of header come first are yours to give, because a game's sprite format is its
own. A game that draws up the screen keeps its sprites upside down, and `bottom_up`
turns the picture the right way up while the rows stay in memory order. A sprite
drawn over a background usually keeps a mask byte before each byte of picture;
`masked` pictures only the picture bytes, so the shape is not lost in the mask. To compare
a byte with its pixels, `bitmap_rows` on `export_listing` puts each row's pixels
beside it instead.

You are making a claim and nothing checks it,
but it can't damage anything: however a stretch is shown, its rows stand for the
same bytes, so the export still assembles back into the program.

`define_struct` names the fields of a record. `struct_array` says records lie side
by side at an address, so `LD A,($5D31)` reads `LD A,(rocket.x)` if that record has
a label, or `object[6].x` if it doesn't. `bind_struct` says that over a range of
code a register points at one, so `(IX+$05)` reads `(IX+speed)`. The narrowest
binding wins, because one object table's `+5` is a speed to one handler and a
countdown to the next. The listing replaces the numbers with the names. The export
can't, since an assembler needs the numbers, so it says so in a comment beside them.

**Searching it.** `find_bytes`, `changed_since`.

The variable hunt: `save_state`, lose a life, `changed_since` — the counter is
among the handful of addresses that come back. The display file is left out
unless asked for, since it changes every frame and would bury the answer.

**Hearing it.** `sound_state`.

The beeper bit of port $FE, and on a 128K the AY's registers with what they
mean channel by channel — tone, noise, volume, period turned into a pitch.
`watch_events` with `beeper` or `ay` finds the routine doing it.

**Keeping it.** `save_state`, `restore_state` — by name in the session, or as a
`.sna` file. An experiment that goes wrong is undone by putting the machine
back.

**Writing it down.** `set_comment`, `comments`, `save_comments`,
`export_listing`, `export_asm`.

`export_asm` writes the program in RAM as assembly source that assembles back into
the same bytes: code where it has run, DEFB elsewhere, every label and comment,
and the ROMs it ran against named by CRC32 in its header. `from` and `to` narrow it
to the program itself, and anything it names outside that becomes an EQU. It carries
what the listing does, as comments: descriptions above their labels, the pictures,
the `seen:` and `used by:` lines and the marks. So it can be the disassembly and not
only something to build from.

`export_listing` writes the whole annotated disassembly to a file — the thing
being built. What ran while watching is disassembled; what did not is left as
bytes rather than turned into instructions nobody executed. Notes go to
`<name>.zxrs.txt` beside the tape or snapshot: plain text, one line an address,
readable and diffable without the emulator. A guess made by AutoDoc is marked
as a guess and never overwrites something typed.

**Placing it among other games.** `lineage`, `export_lineage`.

`lineage` reads every `.asm` in `disassembly/`, the file `export_asm` writes, and draws
the games as a family tree, oldest first: each game hangs off the earlier game holding
the most of its code, with how much. A game is in the tree once it has been
disassembled and not before. Its code is cut into fragments of five instructions with
every number and name blanked, and the ROM's own fragments and the tape loader are left
out. Title, year and publisher come from the file's TOSEC-style name, and authors from
ZXDB, through `zxdb-games.txt` in the preferences directory, which
`tools/zxdb-games.py` writes. With `game`, it also shows that game's routines found in
an earlier game, by the names the disassembly gives them, and who is suspected of
writing them. `export_lineage` writes the same as an HTML page. What it measures, and
what it was checked against, is in [`lineage.md`](lineage.md).

## Things worth knowing

**A machine with no ROM looks like a game that crashed.** A fresh `Spectrum`
has `$FF` where the ROM should be, which is `RST $38` forty thousand times
over: it runs, it fills the screen with rubbish, and it sits at `$0038`. The
session tracks whether a real ROM has been put in rather than looking at the
bytes, because "not all zeros" is true of an empty machine. Every game
"loaded" against that and ran for thousands of frames of nothing, and the
report said so cheerfully — which is what the end-to-end test against a real
tape was written to catch.

**The observer needs calls to watch.** An edge in the call graph is a call made
from inside something. Code entered by a jump has no frame for a call to come
from, so a main loop that was never itself called is not a routine and its
calls are attributed to whatever is under it.

**Where in the frame a routine ran decides what is seen.** The ULA draws
between T-state 14,336 and 57,344 on a 48K. A write above the beam is on the
screen this frame; one below it waits for the next. `run_tstates` is how to
stop part-way through a frame and look.

**A recording is played by fetch count.** A frame of an RZX is a number of
opcode fetches, and the interrupt comes at the recording's own frame boundary
rather than on the T-state clock. `play_recording` does that; see
[`timing.md`](timing.md). Anything else that runs the machine — `run_frames`,
`run_tstates`, `run_until`, `press_keys`, `type_text` — lets go of the recording
first and says so, because the machine can only follow one clock: with the
recording in place, a frame ended nowhere but at a recorded boundary, so
`run_frames` used to run for ever inside one frame with no interrupt and report
none run. `force_branch` runs its fork on the machine's own clock and leaves
the recording where it was.

**There is no JSON crate in the lock file**, and none can be added, so
`src/mcp/json.rs` is a small implementation of exactly what JSON-RPC uses —
the same bargain as `src/svg.rs`.

## Driving the machine's own commands

A Spectrum's keywords are one key each rather than words to spell out, so
`type_text` reads a line the way the machine would have it typed: `LOAD` is the
L key, `CAT` is extended mode with a shift on the 9, and the symbols are behind
SYMBOL SHIFT. That is the only way to reach the extended-mode words at all, and
it is what makes `LOAD *"m";1;"prog"` and `FORMAT "m";1;"cart"` typable. Text
inside quotes is typed letter by letter, since a file called `"info"` is not
`IN` followed by `fo`. A line starts with a keyword, and a letter asked for at
the start of a line is an error rather than a keyword nobody wanted: the mode
is read out of FLAGS at $5C3B rather than guessed.

## What is not exposed

Still in the emulator and not offered here:

- Race the Beam (`race.rs`) — replaying a frame instruction by instruction to
  watch the picture being built, and the tints that say which side of the beam
  a write landed on;
- the timeline (`timeline.rs`) — a turn of the loop drawn against the frames it
  ran in, which `frame_timing` answers a flatter version of;
- the CRT and composite rendering (`crt.rs`), since what a model needs from the
  screen is what is on it rather than what a television did to it;
- the ZX81's tape deck;
- the RZX playback's visited-address set.

The ZX81 answers the tools that mean something on it — loading a `.p`,
running, registers, memory, disassembly, the screen and the notes. The rest say
what they are about and how to get back to a Spectrum, rather than reporting
zeros: an empty answer reads like a finding.
