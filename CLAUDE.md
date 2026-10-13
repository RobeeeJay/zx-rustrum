# Working notes

ZX-Rustrum: a ZX Spectrum (48K, 128K, +2A, +3) and ZX81 emulator in Rust, on
`eframe`/`egui` with the wgpu renderer. This file is what a new session needs to
know that the code does not say for itself.

## Verify Before Reporting Results

Never publish results, tables, or benchmark numbers until the underlying
pipeline has been end-to-end validated on a known-good case. If a result looks
surprising (e.g. an engine that never loses, a table that changes shape between
runs), treat it as a bug in the harness first, not a finding. State explicitly
which parts were measured and which were assumed.

## How to work on it

**Verify against the real machine, not against expectations.** The timing was
settled by running HALT2INT and comparing all thirty values with photographs of
a real 48K, and the border by rendering Border Break and diffing it against a
photograph pixel by pixel — 0 of 116,736 differ. When something looks wrong,
find a reference and measure; do not adjust a constant until it looks right.

**Every change gets a test, and the test is checked against the bug.** Revert
the fix, watch the test fail with a message that explains the failure, restore
it. Several bugs in this project were found because a test that "passed" was
passing for the wrong reason — one tape test was matching the toolbar's `100%`
button rather than the progress bar it meant to check.

**Tests are named as sentences** describing the behaviour, and their comments
say why the behaviour matters, not what the code does. Assertion messages carry
the actual values.

**`cargo fmt` and `cargo clippy --all-targets` stay clean.** CI gates on
`-D warnings`.

**No crate can be added that is not already in the lock file**, unless the
user says otherwise for a particular one. (The network itself does work — that
is how the ROM symbol files were fetched — but the build must stay
offline-reproducible.) That is why `src/svg.rs` exists rather than a
dependency, why `src/mcp/json.rs` implements only what JSON-RPC uses, and why
the logo is drawn in code rather than decoded from a file.

One crate has been allowed in since: **`gilrs`**, on 10 September 2026, to read
gamepads — which cannot be done through `eframe`, `winit` or anything else
already there. It is the only dependency in the tree that is not either
`eframe`'s or a file format's, and it came in by being asked for rather than
by being convenient.

A second has been allowed since: **`burn`**, on 14 September 2026, by the
user's decision, for training a network to play games inside the emulator
(`src/training/`). Only its `std`, `autodiff`, `ndarray` and `wgpu` features
are taken. Its GPU backend reaches `wgpu` through `cubecl-wgpu`, which asks for
the same `wgpu` 29 `eframe` already uses, so there is still one `wgpu` in the
tree; the `ndarray` backend is what the tests train on, since CI has no GPU.

**Prose:** plain, no salesmanship, British spelling. Say what happened,
including what did not work.

## Working Style

**Shipping vs. improving.** When a task's stated goal is met, stop and ship it:
commit, push and report. Do not propose additional refactors, cleanups or
enhancements unless they are asked for. Further improvements noticed on the way
go in a short "possible follow-ups" list after shipping, not before.

## Debugging

**State the hypothesis before fixing it**, along with the cheapest experiment
that would prove it wrong. One probe beats one speculative edit. For memory,
timing and emulator work, confirm an address or a read is stable across at
least two independent samples before building anything on it.

## Version Control

**Test and commit as you go.** After each self-contained change: run the tests
that cover it, then commit with a message describing what changed and why. Do
not batch several features into one commit. Where the area being changed has no
test, the change comes with a small one.

## Environment Setup

**Docker.** All `docker compose` instructions — in READMEs, in scripts and on
the command line — use `docker compose up -d --build`, never a bare `up -d`, so
that code changes are actually rebuilt into the image. A UI change is not done
until it has been seen in the running container.

## What is written down elsewhere

Two files hold what has been learned about the hardware, so this one can stay a
set of decisions rather than a manual:

- [`docs/timing.md`](docs/timing.md) — T-states, contention, the floating bus,
  snow, the painted frame and how recordings are paced. What was measured
  against what, and what is still open.
- [`docs/tape-loading.md`](docs/tape-loading.md) — the tape formats and their
  traps, the ROM loader's contract, and the three loading speeds.
- [`docs/build.md`](docs/build.md) — building from source, the tests, how a
  release is cut and signed, the source layout, and why the build is the
  shape it is.
- [`docs/training.md`](docs/training.md) — teaching a network to play: what it
  sees, the judge, the set-up file, what the settings do and what was
  measured.
- [`docs/missing.md`](docs/missing.md) — what other emulators have that this
  does not, checked against the source rather than remembered. A description of
  the gap and not a plan: nothing in it is promised.
- [`docs/agent-disassembly.md`](docs/agent-disassembly.md) — what a language
  model driving the MCP server would still need to disassemble a game fully
  and explain it. Also a description of the gap, not a plan.

## Boundaries

- `roms/` and `tapes/` are gitignored: the images and games are still under
  copyright. Tests that need them skip themselves when they are absent, so the
  suite is meaningful on a machine without them.
- `designs/` holds the user's artwork and is committed. Do not restyle or
  rebrand it — the cassette carries its own maker's marks, not the emulator's.
- Never commit `*.afdesign~lock~`.
- The preferences directory is still named `ZX Spectrum Emulator` after the
  rename to ZX-Rustrum, so nobody loses their ROM paths and window layout.
- Do not drive the keyboard with AppleScript to test the running app. It types
  into whatever has focus, and once typed into the user's editor.

## Decisions worth knowing

**Timing is counted per bus cycle**, not per instruction: contention lands at
the right T-state inside an instruction because the `Bus` trait is called at
each cycle. zexdoc and zexall pass, including the undocumented flags.

**The ZX81 has no video hardware.** The CPU walks the display file and the ULA
watches the bus; an opcode fetched above `$8000` with bit 6 clear is fed to the
CPU as a NOP while the ULA turns it into eight pixels. Emulating it at that
level is what makes hi-res programs work without special cases.

**Racing the beam replays a frame; it does not read the picture.** A copy of
the machine is taken at the interrupt that starts a frame and run forward to
wherever the cursor is (`src/race.rs`), so what is on screen is the machine as
it stood after every instruction up to that T-state and none after it: the
display file part-written, the border wherever it had been set, the registers
where they had got to. Behind the cursor is what the ULA painted on the way
there; ahead of it is what the display file holds at that moment, dimmed,
because the ULA has not put it out yet. Reading the live picture instead — what
this used to do — shows a single moment of the machine either way, which
answers nothing about when within the frame anything happened.

It only works on a stopped machine, and starting one switches it off: a running
machine is somewhere else by the time the cursor has been read. The copy is
silent and records nothing (a copy shares the sound queue, and would play its
own frame over the real one). Going down the screen runs the copy on from where
it is; going back up starts the frame again, because nothing can be
un-executed. A downward sweep of all 192 lines costs 59µs and an upward one
13ms, so no cleverness is needed.

**Race the Beam runs at five seconds a frame and fades the picture behind the
beam.** A pixel is at full brightness the moment the beam draws it and at the
slider's floor — half, by default — just before the beam comes round to draw it
again, so the trail down the screen says how long ago each part was drawn. At
fifty frames a second that would be a flicker nobody can see, which is why it
comes with the speed rather than as a switch of its own. The oldest part of the
picture is the line just *below* the beam, not the bottom of the screen: a
quarter of the frame is spent below the display in the border and the sync.
Hovering with the cursor is a different thing and is called Cursor Beam.

**Writes are marked by which side of the beam they landed on**, while Race the
Beam is on. Red says the beam had already been over that byte, so the change
will not be seen until the next frame — which is what a flickering sprite is;
green says it will be shown this frame. The colour blends into the colour the
write is going to show over two seconds of the user's time, which is worked out
in T-states from the speed the machine is being run at, so it is the same
number of instructions however the speed is changed.

A marked cell is drawn from the display file rather than from what the beam put
out: the mark is on its way to the colour the write will show, and for a late
write there is nothing on the screen yet. The mark itself lasts until the beam
goes over the byte — not until its colour has finished blending, or the picture
would pop back to the old content two seconds after every late write. The beam
crossing clears it, and by then the painted frame holds the same bytes anyway,
so nothing moves. Marking costs a branch on every write to the display file,
which is why `bus.tints` is `None` at every other time.

**The picture is the frame being painted while the machine crawls**, and the
last finished frame otherwise. Holding the finished frame is what stops a
repaint catching a picture half drawn at full speed; under slow draw it would
freeze the picture for the seconds an emulated frame takes while the beam
crawled over it. Both are what the ULA put out, never the display file as it
stands.

**The ULA snows when I points at RAM it is reading.** After every opcode fetch
the CPU puts I:R on the address bus, and the ULA — which tells the CPU's
accesses from its own by watching that bus — is disturbed by it. Two different
things happen, depending on where the last T-state of the M1 falls in the ULA's
eight-T-state cycle
([redcode's notes](https://github-wiki-see.page/m/redcode/ZXSpectrum/wiki/Snow-effect)):
on its third, the pixel fetch is made from the wrong address, bits 6..0 of R
standing in for the low seven of it — so snow is made of the program's own
graphics, from the same part of the screen, and the colours stay right. On its
fifth, the second cell of the pair is not fetched at all and the first goes out
again in its place: the "double effect", an eight-pixel bar repeated.

The address has to be RAM the ULA reads: $4000-$7FFF on a 48K, and on a 128K
also $C000-$FFFF when an odd page is banked there. The +2A/+3 drive the bus
themselves and do neither. The CPU only calls `Bus::refresh` when I could point
at either range, since it is on the busiest path there is — with the test a
screenful of NOPs runs a fifth slower, and real code about seven per cent.

**The ULA's fetch cycle starts a T-state after contention does.** The reference
FAQ says the first byte is displayed at 14336 while its own contention table
starts at 14335, and `snow.tap` settles it: its interrupt handler fills the
display with a solid field of NOPs after `LD R,A`, so every eight-T-state block
gets an M1 in the same place, and it lands on the snow window only with the
cycle anchored at 14336. All 3,072 blocks snow, which is what a program written
to demonstrate snow should do. Against that, `ula128.tap` sets I to $FE on a
128K and lands on the other parity, so it shows nothing here — worth checking
against a real machine before trusting the anchor further.

`tests/reference_48k.rs` holds the
[48K reference](https://worldofspectrum.org/faq/reference/48kreference.htm)'s
own numbers: the 69888-T frame, the 224-T line, the contention table T-state by
T-state from 14335, the four I/O patterns worked through by hand, and which
addresses are contended. All of them already matched; the file is there so they
go on matching.

**Sync is treated the way a television treats it.** A pulse held for at least a
line is a vertical sync and pulls the picture back to the top; a shorter one is
a line sync; one that arrives far too early is not a sync at all, so the beam
stays where it is and is merely blanked — which is what draws the ZX81's
loading pattern. The screen is painted over rather than wiped, so a display
that keeps restarting looks like one.

**An RZX recording is played back by fetch count, not by time.** A frame of a
recording is a number of *opcode fetches* — a prefixed instruction is two or
more — and every IN takes the next byte the recording holds instead of reading
the hardware. Counting whole instructions instead runs past the end of every
frame and reads input that was never recorded; that showed up as thousands of
"short reads" until it was fixed. The frame interrupt is raised at the
recording's frame boundary rather than by the T-state count, and the video
frame ends there too — nowhere else. The T-state clock used to end one as well,
so a recorded frame holding more instructions than fit in a frame of the
machine's own time painted the screen twice: once on the clock, once at the
boundary. Space Harrier's recording does that from about frame 1,100 on — its
frames hold 9,400 to 11,600 fetches where a video frame at the ~11 T-states a
fetch this game runs at holds about 6,300 — and at five seconds a frame the
second painting reads as flicker. Manic Miner's recording never does: its
boundaries land within a dozen T-states of the clock. What is not settled is
why those frames are so long — whether the game misses interrupts on real
hardware, in which case the ULA really did paint twice, or the recording was
made on something whose clock ran differently. The lengths are not near whole
multiples of a frame, which fits neither story exactly. In slow motion a
frame is played in pieces — Race the Beam runs at five seconds a frame, and
whole frames at a time would stand still for five seconds and then jump — but
only in slow motion: a 128K frame is a shade longer than a host frame, so a
budget-based rule split every frame at full speed for nothing. A frame is not
begun, and its input not put in front of the machine, until there is budget to
run some of it. `run_fetches` reports what it ran rather than what was asked
for: instructions run whole, so a prefixed one overshoots, and clamping the
count to the budget loses the overshoot on every piece. A recording that
asks for more input than it holds has come adrift, and the toolbar says so
rather than letting the picture look authentic. `recordings/` is gitignored for
the same reason as `tapes/`.

**Fastload answers the ROM instead of playing to it.** A tape loads at
1,500 baud however fast the machine is run — the pulses take as long as they
take — so Max speed shortens the wait and cannot remove it. `src/flashload.rs`
removes it: when the machine calls LD-BYTES at $0556, the next block with the
flag byte it asked for is copied into memory, the registers are left as the
routine would have left them, and the return is taken there and then. Border
Break loads in two frames instead of 2,605, ending in the same place with the
same screen.

Nothing else can be handed over: a game's own loader reads the tape itself and
often decrypts each byte as it arrives. What the emulator knows about those is
which sampling loop they count pulses in — seven of them, read off the tapes
rather than off a list, in `flashload::CORES` — so the tape window can say who
is reading. Speedlock, Bleepload, Microsphere, Paul Owens, Alkatraz, Dinamic,
the Search loader and its variant, Hewson's and Digital Integration's are all
recognised, and thirteen games are checked against their tapes.

It only works where the ROM is doing the loading, and only where that ROM is
the one paged in — the eight bytes at $0556 are checked against the routine
rather than assumed, since a 128K is running its own ROM there and a program
may put anything it likes at that address. A game with a loader of its own is
counting its own pulses and cannot be helped, which is why switching this on
switches Max speed on with it.

**Tapes are played as pulses**, never decoded, so turbo loaders and the ZX81's
own format work through the same path. Each machine has its own deck, because a
tape is timed in the T-states of the machine playing it and the two clocks
differ.

**Saving is read back off MIC, not trapped.** A blank tape arms
`src/recorder.rs`, which keeps when bit 3 of $FE changed and, after half a
second of quiet, reads the pulses back into a standard block at the ROM's
timings. Nothing about SA-BYTES is intercepted, so the machine saves the way it
always does and a program that calls the ROM from somewhere odd is recorded
the same. A saver with timings of its own is counted as unread rather than
written down wrongly. The recorder is flushed after the frame count moves on:
before it, the clock reads a frame early, which looks like a reset and drops
the block — the real ROM's SAVE in `tests/tape_save.rs` is what caught that.

**Nothing is written against an address until somebody says so.** The Call
flow window's detectors are asked to look — pressing *Main game loop* is what
starts the machine being watched — and what they find is offered with a score
and a reason. It goes into the notes only when *Label it* is pressed. The
emulator used to guess at everything continuously and write its guesses in; it
does not any more.

**AutoDoc guesses, and says so.** `src/autodoc.rs` reads the code from where
the machine is, names every routine that gets called, and applies ordered rules
to what each one touches — ports, screen ranges, ROM calls, block moves. It
hedges in the wording when the evidence is thin ("possibly a protection
check"), and a rule that always has an answer would be worse than none, so code
with no tell is left unnamed.

**Code is what has run.** The tracker keeps, per physical location, whether an
opcode has ever been fetched there since the last reset or snapshot, and the
debugger's Disassemble mode shows instructions only where one has, and every
other byte as `DEFB`. An opcode fetch is where an instruction starts, so one
that has run is shown whole and its operands never become rows of their own;
stepping back through the listing goes to an instruction that has run and ends
exactly there, or else a byte back.

**An exported program assembles back into the same bytes, or it is not an
export.** `src/asmexport.rs` writes RAM as source: code where it has run, data
elsewhere, labels made into names an assembler takes, the ROM left out and
named by CRC32. A Z80 instruction can have more than one encoding and an
assembler picks the usual one, so only documented instructions with the one
encoding are written as mnemonics — duplicates, stray prefixes and
undocumented ones are DEFB with the instruction in a comment. The only honest
test is an assembler, and none is in the build: `tests/asm_export.rs` uses
sjasmplus when ZXRS_Z80ASM names one or it is on the path, and skips that part
otherwise, as the ROM tests do.

**A conditional way out is not where a routine ends.** A taken `RET Z` ends the
call it is in; the next call through may fall straight past it, so the routine
carries on underneath. Only the exits that always end it — a plain `RET`, or a
jump that always jumps — are boundaries between blocks. Counting the
conditional ones drew every guarded routine as far as its first test, which is
how the coloured blocks in the debugger came to look smaller than the routines
they were of. Both kinds are still recorded as exits: knowing where a routine
can leave early is worth having, it is just not where it stops.

**Each routine carries three numbers: how big it is, what it wrote and what it
read.** The size is the distance between the lowest and highest address seen
executing inside it — where the routine reaches, which is where to start
looking rather than a promise, since a routine that jumps over a table reaches
past the table. The writes and reads are per call and count what the routines
it calls did as well: a routine whose whole job is to call the drawing routine
does nothing itself, and saying so would be the wrong thing to say about it.
Its own writes are not nothing either — a CALL pushes a return address, and
that is a write.

**The Call flow window draws the same calls four ways.** *Thread* is one turn
of the loop in the order it happened, nested as it nests. *Graph* is who calls
whom over everything watched, laid out in layers by `src/callgraph.rs`: a
routine sits to the right of what calls it, a thicker line is a call made more
often, and a line going back to the left is a call into something already
reached — which is what a loop in the program looks like from there. Cycles are
why the layering leaves out the edges that would push a routine past a layer it
already has, and only the busiest forty routines are drawn. *Flame* is one turn
as nested bars whose width is the work done. *Timeline* (`src/timeline.rs`) is
one turn against the frames it ran in, with the stretch where the ULA is
drawing the picture shaded: on this machine *when* a routine ran is the whole
question, since a write above the beam is seen this frame and one below it is
seen next.

**What a routine did beats what its code looks like.** `src/observe.rs`
attributes every write, port access, instruction and loop iteration to the
routine on top of the call stack, which `src/flow.rs` works out from what the
CPU did rather than by decoding opcodes (the profiler uses the same
classifier). AutoDoc prefers those measurements to its static rules and quotes
the numbers — "writes 6144 bytes into the display file per call, every frame" —
so the user can check the claim. Port $FE is the border, the beeper and the MIC
socket at once, so it proves nothing on its own: the beeper is told apart by
hammering the port while writing almost nothing to memory. Watching costs a
branch on every access, so it only runs while AutoDoc is on.

**The keyboard window is the machine's keyboard, both ways round.** Forty keys
wired as eight rows of five — the same matrix on every Spectrum and on the
ZX81, with different words printed on them, which is why `src/keyboard.rs`
holds two layouts and `src/ui/keyboard.rs` only draws them. Clicking a key
holds it down for a tenth of a second whatever the pointer does, because the
ROM scans the keyboard once a frame and wants a key on two scans running before
it believes in it; a click that lasted one host frame would type nothing. The
same tenth of a second is how long a key stays lit, so a key tapped on the desk
is a key that visibly flashes. Everything the machine can see down is lit,
whichever keyboard it came from. A shift clicked on its own waits for the key
it is shifting and goes down with it, since one pointer cannot hold two keys —
and it is pressed with that key rather than dropped at the moment it is
clicked, or the machine would see the key unshifted. The ZX81's legends are
from the keyboard table at <https://problemkaputt.de/zxdocs.htm>. The cursor
keys are written out as words: egui's fonts have no arrow glyphs, and an empty
box on a key says nothing. The +2A and +3 have typewriter keyboards whose cases
carry keys the rubber ones did not; what is drawn for them is the matrix they
share with the 48K, which is honest about what the machine reads and not a
picture of their case. The window's search finds a word on
the keys and rings the key and the shifts it takes; how each legend is
reached is decided by where it sits on the key — face, red on the key, over,
under — in `keyboard::search`, so it is tested without a window. The ZX81's
word above a key is function mode, SHIFT with NEWLINE, not extended mode.

**The +3 has a disk controller now, and no clock.** `src/fdc.rs` is a µPD765A
as far as +3DOS can tell — the three phases, and the commands the ROM uses —
and `src/disk.rs` reads and writes DSK files, which hold what the controller
would have read off the surface rather than a filesystem. Nothing is timed: a
real controller makes the program wait for the head and the motor, and this one
answers at once. +3DOS polls rather than counting so it cannot tell, but a
loader that measures the wait could. A disk is mounted read-only, writing to a
copy, or writing in place, and the question is asked rather than guessed at:
a game writes its high scores to the disk it loaded from. The drive has two
speeds — the waits a real one makes, or none — and its window draws the front
of the drive, a map of what has been read and written per sector, and the
catalogue. [`docs/disks.md`](docs/disks.md) has the rest.

**A reset reaches the disk controller, and the clock going backwards cancels
its waits.** Reset mid-command, the controller was left handing over data
nobody would take, so the next command found it talking rather than listening;
and `reset()` puts `tstates` and `frame` to zero, so a wait timed against
`total_t()` was left ending millions of T-states ahead and the drive reported
itself busy until the clock caught up. The +3's ROM then sat at `$211A` polling
the status register for the twenty-two seconds its timeout takes. Anything that
times against the machine's clock has to cope with it moving backwards — a
snapshot does it too.

**A reset lets go of the keyboard.** A shift clicked in the keyboard window
waits for the key it is shifting, and it used to go on waiting across a reset:
the machine came up with CAPS SHIFT held, answered every key with the shifted
one, and read as a machine ignoring the keyboard. `App::reset_machine` is the
one way through, and it releases everything the window is holding.

**The machine is deaf for a second after a reset, and that is the ROM.** The
48K ROM checks every byte of RAM before it does anything else, with interrupts
disabled — and the keyboard is read by the interrupt handler, so nothing is
scanned until it finishes. Measured at 85 frames on a 48K, 54 on a
128K and 57 on a +3 — the later ROMs are quicker about it — and it scales with
the emulator's speed: half speed, twice the wait. The status line says so rather
than leaving it looking like a hang; nothing about it is worth "fixing", since
the fix would be lying about the machine.

**An IPF is read at byte level, not at flux level.** Its streams hold decoded
bytes — sync elements carry the MFM sync words, data elements the bytes between
them — so `src/ipf.rs` walks the container, finds the address marks and checks
both CRCs, which is what says the reading is right rather than plausible. The
deleted-data marks and deliberate CRC errors that a protection leaves are kept
in the sector's ST1/ST2; the cell timing and the weak bits are not, and a
protection that measures either will not be fooled. Nothing is written back to
an IPF: what is taken out of one is the sectors, and an IPF is more than that.

**What is plugged into the back is a list that says how far each thing is
emulated.** The Interface 1 and its microdrives, the three Multifaces, the
µSpeech, the Fuller Audio Box, the SpecDrum and the Kempston mouse do
something; the Music Machine is a switch with nothing behind it yet, and the
Hardware window says so where somebody will see it. A switch that turns on nothing is worse than no
switch. The Interface 1 pages its own ROM in when the machine fetches from
$0008 or $1708 and out again at $0700, so without `roms/if1.rom` it pages in
nothing — which is what an empty socket does.
[`docs/peripherals.md`](docs/peripherals.md) has the rest.

**Nothing about an interface was known until its own ROM was run.** The first
Interface 1 here was written against the documentation and passed nine tests,
and with Sinclair's ROM in the socket it did not turn a drive: the shadow ROM
was paged out one fetch too early (the `RET` at $0700 has to come from the
shadow), the gap, sync and write-protect lines were in the wrong bits, and the
motor line was the wrong way up. All four were read back off the interface's
own code — the sector-finding loop at $165A, the write test at $136C — rather
than off a table. The tape moves as the ROM reads it rather than on the clock,
because a block is read with `INIR` at 21 T-states a byte and the tape hands
one over every 170. `tests/if1_rom.rs` formats a cartridge, saves a program to
it, loads it back and reads the listing off the screen; anything less than that
was passing for the wrong reason.

**The red button pages the Multiface in at the fetch from $0066**, not when the
CPU takes the NMI: the latch the button sets is clocked by /M1 with that
address on the bus, the same mechanism as the Interface 1's $0008. All three
models are emulated, each needing its own 8K — they page in and out on
different ports, and the 3 has its two the other way round from the 128, which
is the sort of thing only the ROM can settle. The menu does not run with the
box paged in: it puts a stub in the machine's RAM and pages itself in and out
several times a frame, so `paged` is a bad thing for a test to assert on. The
button itself is on the main window under *Buttons* rather than in the Hardware
window, because it is pressed while a game is running.
[`docs/peripherals.md`](docs/peripherals.md) has the ports.

**The µSpeech turns over on any access to $0038**, whichever kind of cycle it
is — a fetch, a read, a write, an `IN` or an `OUT`. The ULA's interrupt is a
fetch from that address, so the box pages itself in for the interrupt, runs its
handler and pages out again on the way back: fitting one to a running machine
is all it takes.

**The SP0256-AL2 is emulated as the chip, not as samples.** `src/sp0256.rs`
runs the microsequencer in its 2K ROM and the twelve-pole lattice filter it
drives, so the allophones are synthesised the way the hardware synthesises
them. It needs the chip's own dump at `roms/sp0256-al2.rom`; without it the
interface works and nothing is audible. Three traps, all of which cost time:
the sequencer addresses its ROM from $1000 rather than zero, so a dump loaded
at zero halts a sample later without a sound; bit order cannot be guessed and
has to be established by running it — right way round, all 64 allophones come
out within 3.5% of their published lengths, wrong way round the chip halts at
once or runs for a second and a half; and the arithmetic is deliberately narrow
and wraps, so widening it makes something that is not an SP0256. The chip runs
on its own oscillator, so it lives with the mixer and is clocked in the
machine's T-states. Checked against a recording of real hardware: the steady
sounds correlate at 0.94-0.98 with their formants inside a hundred hertz.

**Finding a graphic looks for one column of it, every way it could be kept.**
The Graphics window's Find takes the eight bytes of the 8x8 block clicked and
looks for them as eight bytes in a row and as every 2nd to 64th byte, each as
is, mirrored and inverted (`src/gfxfind.rs`): one column of a sprite is all a
block can be, and how far apart its rows are is the sprite's width, or half
it with a mask beside each byte. The viewer gained a row-by-row layout and a
mirror to show what it finds. A block of one repeated byte is refused rather
than found everywhere. Not found: a sprite drawn at a pixel position that is
not a multiple of eight, which straddles two cells; one stored upside down;
and anything in a 128K bank not paged in.

**The Kempston mouse takes the pointer when the screen is clicked.** Until
then the pointer is the desk's and the mouse sees nothing; after it, the
pointer is hidden and held — locked on macOS, confined elsewhere, since each
platform offers only one — and movement comes from the raw motion the host
reports, because a locked pointer does not move. Esc, the main window losing
focus, or taking the mouse off gives it back, and that is checked in `logic`:
on macOS eframe skips `ui` once the window is switched away from, which is
exactly when focus goes. Movement is divided by the display's scale so it
counts in the machine's pixels, with the remainder carried to the next frame. Its buttons' port is only partly
decoded and takes in the Kempston joystick's $1F, as in Fuse; the joystick is
asked first.

**The AMX mouse interrupts once a step, through its own vector.** It is a Z80
PIO: every step the mouse moves raises /INT, the PIO puts its vector on the bus
when the CPU takes it, and the handler reads which way the step went in bit 0
of $1F or $3F. `Bus::int_vector` is how the bus says what the vector's low
byte is — $FF, floating, for the ULA's interrupt — and the PIO holds its
request until it is taken rather than for the ULA's thirty-odd T-states. It
comes up with its interrupts off, so nothing happens until a program sets it
up. No AMX software is here to run, so the details stand on the Sinclair Wiki,
dsp-emulator and zx84 agreeing — where zx84 disagrees about the left button,
the other two win — and `tests/amx.rs` checks it with a small driver of the
same shape in machine code. Steps go no closer together than 1,000 T-states,
which is a choice rather than a measurement.

**The printer is where the stylus has got to, not a stream of lines.** The ROM
watches the ZX Printer's encoder and switches the stylus dot by dot, so
`src/printer.rs` works out the stylus's position from how long the motor has
run on the machine's clock (Fuse's `printer.c`). The Alphacom 32 is the same
device on the same port, so only one can be fitted; the difference the user
sees is the paper, which is a switch in the window rather than a property of
the machine. The ROM's `COPY` is checked against the display file bit for bit.
`Session::new` has no ROM in it: an MCP test that types at the machine has to
put one in, or it types into `RST $38` and looks like the peripheral failing.

**A network being trained sees the screen and nothing else.** `src/training/`
plays a game from a saved machine an action at a time, the actions chosen per
game from a joystick interface's switches or particular keys. What the network
is given is built in one place, `training::sight::look`, from what the ULA
painted — never the display file, never memory, never registers. The reward
has to come from somewhere, and the judge reads the score, lives and game-over
out of memory as the Arcade Learning Environment does for Atari games; what it
reads goes no further than the reward. A test puts the same picture in front
of two machines with different memory and registers and requires the network
to see no difference.

**Training runs on the graphics card unless told otherwise, because it was
measured.** At the default size an update took 4.7s through wgpu and 45s on
the processor, where the emulation is a small part of either. burn takes its
processor backend with that crate's defaults off, which leaves it on one core
without SIMD — the first run took over ten minutes an update — so
`burn-ndarray` is named in `Cargo.toml` to turn its threads on. A network is
kept as a directory with the set-up it was trained with, since it cannot be
rebuilt without knowing the picture and the actions it was made for. While a
kept network plays the machine on the screen the keyboard and stick on the
desk are not read at all, the same bargain as a recording: two hands on one
set of controls is neither playing nor watching. The discount is the setting
that decided whether the tests' tiny game was learnt in 25 updates or 140 —
[`docs/training.md`](docs/training.md) has why.

**A joystick is a choice, not a fact.** The machine has no joystick port, so
every interface solved it differently: Kempston on a port, Sinclair and Cursor
wired to five keys each — which is why they work with games that know nothing
about them — and the Fuller at $7F with its bits the other way up. No ROM reads
a joystick, so none of this can be measured off one: the masks and the key
mappings are Fuse's `joystick.c` and are written out in the tests so a change
has to be deliberate. A key bound to the stick is taken away from the machine's
own keyboard, or holding an arrow steers and types at once — but only while
the binding can do something: a direction while a stick interface is plugged
in, a mouse button while its mouse is fitted. Taking every bound key away
whatever was plugged in is how Space, the stick's fire by default, stopped
typing a space on a machine with no stick at all.

**The Hardware window is in sections, and a stick has one interface.** Mice,
Multiface, Printers, Audio, Joysticks and Drives, in that order; the MCP
`hardware` list uses the same. The Kempston, DK'Tronics and DK'Tronics
Programmable joystick interfaces are entries there as well as choices in the
Input window, and both windows go
through `SpectrumBus::set_joystick`: there is one stick, so fitting one
interface takes the other off, and choosing a key-wired interface in the Input
window takes both off. A preferences file from before this says only the
stick's kind, and loading it fits the interface that kind needs.

**Every window's keyboard is the machine's.** Each emulator window is a window
of its own to the window system, with its own keyboard, and the one clicked
last has it. What each window other than the main one is holding down is
gathered while it draws and read by the next frame's keyboard, so the machine
does not go deaf because the debugger was clicked — except while one of that
window's text fields is being typed into, since a label typed in the debugger
is not meant for the Spectrum. `egui_wants_keyboard_input` would have been the
wrong test: it is true for any focused widget, a clicked button included.
The quicksave keys stay the main window's, since the debugger's F5, F7 and F8
step. A gamepad cannot be
read through anything `eframe` brings with it, which is why `gilrs` was added
for it — the one crate in the tree that is there for a peripheral rather than
for the build. A binding is a source and an action so that a key and a pad
control are the same kind of thing: the pad's state is taken as a snapshot
each frame, which is what lets the deciding be tested on a machine with no pad
plugged in.

**`.szx` is a container, and what it cannot put back it says out loud.** The
blocks it does not know are stepped over and named in what `load` returns,
rather than being lost quietly — a machine that comes back missing its
microdrives should say so. `.sna` and `.z80` remain for the machines and
emulators that want them, and the extension chosen when saving decides which
is written.

**A quicksave is a copy of the machine, not a snapshot file.** `Spectrum` is
`Clone`, so each of the ten slots holds the whole of it: the peripherals, the
tape where it had got to, the chips mid-note — none of which `.sna` can hold
and not all of which `.szx` can. The kept copy is detached from the sound
queue, and the one restored takes the live machine's output over with
`Audio::take_output_from`, throwing away the samples it had not sent when it
was taken, which would otherwise come out as a blip from the past. Restoring
one also rolls back what is in the disk and microdrive drives, since those are
part of the machine; the files are only written when a drive is ejected. The
ZX81 is not `Clone`, so it has no quicksaves yet.

**The machine and what is plugged into it are remembered between launches.**
`machine`, `peripherals` and `microdrives` in the preferences; a ROM that has
since moved is not an error, and the machine the emulator can actually be is
what comes up.

**A watch can be on a place as well as on a kind of thing.** `Breaks` carries
watches for the sorts of thing a program does — a screen write, the beeper, an
`IN` — and `write_range`, which is a watch on an address. It costs one
comparison on every write when it is `None`, which is why it is an `Option` and
not a list. It is the answer to "what writes to this?", which is the question a
debugger is for.

**The MCP server is the emulator without a window.** `src/mcp/` and the `mcp`
binary expose the machine over JSON-RPC so a language model can drive it:
loading, running, breakpoints and event watches, registers, memory,
disassembly, the observer's measurements, snapshots and the notes. Everything a
tool returns is text, because the thing reading it is a model. There is no JSON
crate in the lock file, so `src/mcp/json.rs` implements what JSON-RPC uses and
no more, the same bargain as `src/svg.rs`. What is deliberately not exposed —
and why it would be worth exposing — is at the end of
[`docs/mcp.md`](docs/mcp.md).

**A machine with no ROM in it looks like a game that crashed.** `SpectrumBus`
fills its ROM space with `$FF`, which is `RST $38` over and over: the machine
runs, draws rubbish and sits at `$0038`. Anything that loads something has to
know whether a real ROM has been put in rather than testing the bytes for
emptiness.

**Anything else known goes in a file, not in the source.** `tools/rom-symbols.py`
turns a disassembly from
<https://github.com/ZXSpectrumVault/rom-disassemblies> into a symbol file in
the preferences directory. They are somebody else's work, so they are
generated rather than committed.

Three conventions are in use across those files and the converter reads all
three: `;; NAME` then `Lxxxx:` for the 48K and ZX81, a title between rules of
dashes for the 128K, and dZ80's `.lxxxx` with `defc NAME=$xxxx` for the +3.
A title only names a label within a dozen lines of it, or the pages of prose
at the top of a file end up naming the reset vector.

**A paged machine needs a symbol file per ROM.** The 128K's ROM image is two
16K ROMs and the +3's is four, each addressed `$0000-$3FFF` in its own right,
so a name means nothing without knowing which is in: they are
`symbols-128-rom0.txt` and so on, and only the one paged in at `$0000` is
read. `<name>.symbols.txt`
beside the notes takes `ADDR name ; comment` lines and `bytes …` signatures. A
full ROM disassembly cannot be shipped here and a signature that cannot be
checked should not be invented, so the mechanism is in the emulator and the
knowledge is the user's to supply.

**`tools/skool-symbols.py` reads the SkoolKit game disassemblies** at
<https://github.com/mrcook/zx-spectrum-games> — 6,677 routines across fifteen
games, 5,736 of them described. SkoolKit takes addresses either way, `$8000`
or `32768`, and four of those games are written in decimal: reading them as hex
finds nothing at all and says so quietly, which is how it looked at first.
Its `#R$8000` cross-reference markup means nothing in a text file and is taken
out, along with titles that only say "Routine at 32768".

**Signatures come from the ROM in the machine, not from a table somebody
typed.** Games copy ROM routines into RAM; the same bytes hash the same
wherever they land, so a copy is named after the original and said to be one.
Only the first twelve bytes are hashed, which stays in front of most absolute
addresses inside a routine. Anything else worth recognising can be added by
whoever has the code in front of them to check it against — inventing
signatures that cannot be verified would be worse than having none.

**A guess is marked with `@` and never overwrites a person.** Guesses are kept
in the same notes file as everything else, written as `@label ; @comment`. A
later guess replaces an earlier one — by then the program may have unpacked
itself, and the second look is the better one — but nothing the user typed is
ever replaced, and typing over a guess makes it theirs. Guesses are drawn in
the dim colour so the two are told apart at a glance.

**Listings are annotated in a file beside the tape.** Labels and comments
typed into the disassembly go to `<name>.zxrs.txt` next to the tape, or next to
the ROM when the deck is empty — plain text, one line per address, so it can be
read, edited and diffed without the emulator, and a badly edited line is
skipped rather than throwing the file away. A comment's line breaks
are written as `\n` and a backslash as `\\`, so a note stays one line: written
raw, a comment's second line was dropped on reading back, or taken for a note
of its own if it began with something that reads as an address.

**The cassette is rendered from the SVGs** in `designs/` by `src/svg.rs`, a
deliberately small renderer covering only what the artwork uses: groups with
matrix transforms, lines and cubic curves, rectangles, circles, solid and
linear-gradient fills, strokes, and the even-odd rule that makes the shell's
window a hole. It is not to grow into a general implementation. Re-exporting
the artwork changes the emulator without anyone redrawing anything in code.

## egui, learned the hard way

- **A viewport builder's geometry must be supplied every frame.** A window that
  has not been drawn for a while is retired and rebuilt, and a builder without
  a size gets the window system's default — which is why the debug windows used
  to open centred at 800x600 and resize on alt-tab.
- **Minimum and maximum sizes have to be sent as viewport commands**, not left
  to the builder, for the same reason. That is how the tape window's width is
  fixed rather than corrected after the fact.
- **A window can be laid out more than once for a frame.** Anything accumulated
  per draw — the cassette's hubs turning, for instance — must come from the
  clock, or it runs at two or three times the intended rate.
- **Anything the debug windows need must happen in `eframe::App::logic`, not
  `ui`.** eframe skips `ui` while the main window is not visible — which on
  macOS includes switching away from the application — and then prunes every
  viewport that frame did not declare, destroying the windows. `logic` runs
  either way. `App::draw` still does both, for the tests.
- **Accessibility labels are not where you expect.** A plain label's text and a
  combo box's selection are in the node's `value`, not its `label`. Tests query
  by value for those.
- Rasterising is slow enough to cache: the cassette is rasterised once per size,
  and the cogs once, then turned by rotating the quad they are drawn on.

## Verifying the running app

Screenshots are unreliable — the window loses focus to whatever the user is
doing. Prefer, in order: a headless test that writes a PNG (`tests/svg.rs` has
one); querying window geometry through System Events; a screenshot last. State
plainly in the report which of these was used.
