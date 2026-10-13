# ZX-Rustrum

A ZX Spectrum emulator in Rust — 48K, 128K, +2A and +3, plus the ZX81 — with a
Z80 that is timed by the bus cycle, beeper and AY sound, and a pile of
debugging windows you can drag onto a second monitor and watch while a game
runs.

The point of it is the watching. Plenty of emulators will play Jetpac. This
one is for seeing *how* Jetpac does what it does: which routine drew that
sprite, where the score lives, why the border flickers exactly there. The
timing has been checked against photographs of a real 48K rather than against
what seemed about right, so when it shows you something, the real machine did
it too.

## Quick Start

You need a stable Rust toolchain (this was last built with 1.97.1). Then:

```
cargo run --release             # 48K
cargo run --release -- --128    # 128K
cargo run --release -- --plus2a # +2A
cargo run --release -- --plus3  # +3
cargo run --release -- --zx81   # ZX81 with 16K (--zx81-1k for the bare machine)
```

The first build takes a while, since the release profile uses thin LTO and the
training code brings a fair bit of `burn` with it. After that an edit rebuilds
in under a minute: 42 seconds here for a one-crate change.

No ROMs are included, because they are still somebody's copyright. Without one
you get the built-in demo ROM, which is not as dull as it sounds (see
[Watching the Screen Being Drawn](#watching-the-screen-being-drawn)). To run
the real thing, put the images in `roms/`:

| File | Machine |
| --- | --- |
| `48.rom` | 48K (16K image) |
| `128.rom` | 128K (32K) |
| `plus3.rom` | +2A and +3 (64K) |
| `zx81.rom` | ZX81 (8K) |

Or use **File ▸ Load ROM…**, which also scans that folder for the others and
picks up what it finds. More on where it looks in
[ROMs, Snapshots and Recordings](#roms-snapshots-and-recordings).

Then load a game the way you would have in 1983. **File ▸ Load tape…**, type
`LOAD ""`, press ENTER, then press Play in the tape window. The tape doesn't
start on its own, just like a real one.

You can also name a file on the command line, and it's dispatched by extension:

```
cargo run --release -- "tapes/Jetpac (1983)(Ultimate Play The Game)[16K].tzx"
```

A `.p` on the command line brings up a ZX81 on its own, since there's nothing
else it could be for.

## Why wgpu and Not OpenGL

The renderer is `eframe`/`egui` on **wgpu**: Metal on macOS, Vulkan or DX12
elsewhere. The OpenGL path was tried first and dropped. On macOS it goes
through glutin, whose Cocoa backend panics with *"context to have a current
view"* when one of the extra windows loses its view. With a dozen extra
windows, this emulator found that out quickly.

If you're building somewhere wgpu can't find a backend, swap the `wgpu`
feature for `glow` in `Cargo.toml` and you're back on OpenGL. Just don't do it
on a Mac.

## The Main Window

There are two rows of controls above the screen.

The first holds **File** (loading and saving everything), **Run/Pause**, the
**Machine** dropdown, the **CPU** clock, the speed, **Late** and **I2** on a 48K
(both below), **Reset**, **Record** (an RZX recording or a video), and
**Quick** (ten in-memory quicksaves). A machine whose ROM is missing stays in
the dropdown, greyed out and saying "(no ROM)". It doesn't vanish and leave
you wondering where the +3 went.

The second row is **Windows**: RAM map, Debugger, Tape, Back buffer, Graphics,
Call flow, Profiler, Keyboard, Hardware and Input, plus Disk on a +3, Training
on a Spectrum, and Microdrive and Printer once the hardware for them is plugged
in. Each one is a real OS window (an egui *viewport*), and each opens where you
last left it.

Under the screen are the zoom, the video switches and the sound. The window
title always names the machine in use. When something fails, like a missing
ROM or an unreadable tape, it shows in red in the status bar instead of being
quietly swallowed.

## What It Does

**A Z80 timed by the bus cycle.** Timing isn't counted per instruction. The
CPU tells the machine how many T-states each fetch, read, write and internal
cycle takes, and the ULA applies memory contention at the exact T-state the
access happens. That covers the 48K pattern `6,5,4,3,2,1,0,0` over the 128
T-states of each pixel line, the four I/O contention cases, and the extra idle
cycles for `INC (HL)`, `(IX+d)` addressing, `ADD HL,rr`, the block
instructions, conditional returns and the rest.

The core does the full documented instruction set and the undocumented ones
too (`IXH`/`IXL`, `SLL`, the `DD CB d op` register copies), MEMPTR/WZ, the Q
register behind `SCF`/`CCF`'s F3/F5 behaviour, and interrupt modes 0/1/2 with
NMI.

**Checked against real hardware.** HALT2INT (Mark Woodmass) measures the R
register at the moment an interrupt is taken after a HALT reached at an exact
T-state. That pins down interrupt timing, contention, the halt state and the
floating bus all at once. The emulator reproduces a real 48K's output exactly,
in both timing variants. The reference photographs are in `tapes/`, and
`tests/halt2int.rs` runs the tape and compares all thirty values plus the
test's own Early/Late verdicts.

It caught two bugs, both fixed now. The halt state performs its M1 cycles at
PC, which is the byte *after* the HALT. So a HALT at `$7FFF` refreshes from the
uncontended `$8000`, while one at `$4000` is contended on every cycle. And the
floating bus is sampled at the start of the IORQ cycle, reading back `$FF` for
the four idle T-states in each group of eight.

**Early and late timing.** Real 48Ks came in two variants, one running the
display a T-state later relative to the interrupt. **Late**, beside the machine
dropdown, switches between them, and each matches its reference photograph.

**Issue 2 and issue 3 boards.** **I2** decides whether the EAR input hears the
MIC bit as well as the speaker. An issue 2 board does, and some tape
protections listen for exactly that. Head over Heels needs it on. Switch it off
for a strict issue 3.

**Border art.** The border is drawn by T-state rather than by scanline. The ULA
puts out two pixels per T-state, so a program hammering port `$FE` in a tight
loop can draw in the border at that resolution. Border Break (introspec/gonzy)
renders **pixel for pixel identically** to a real 48K: all 116,736 pixels of
`tapes/bb.png` match, border included.

Getting there took three things beyond sampling once a line:

- The ULA fetches two T-states ahead of the pixels it's emitting, so a border
  write lands on screen slightly before the fetch clock that contention counts.
- A frame drawn only part of the way still shows the *previous* frame below the
  point the ULA has reached, rather than going black.
- The border log has to be big enough for the hundreds of writes a frame this
  kind of code makes.

**Passes zexdoc and zexall.** Both exercisers pass every test, including
zexall's undocumented flag tests:

```
cargo run --release --bin zextest -- zexall.com
```

The `.com` files aren't included. They're easy to find, for example in the
`anotherlin/z80emu` repository under `testfiles/`.

## Watching the Picture

Four switches under the screen change what you see, and two of them are about
*when*.

**Race the Beam** runs the machine at five seconds a frame, so you can watch
the beam crawl down the screen. Everything it has drawn fades behind it (the
**fades to** slider says how far), so how long ago each part was drawn is
visible at a glance.

**Cursor Beam** is the paused version, and it's the one for reading a frame's
timing by hand. Stop the machine, turn it on and hover over the picture. It
replays the next frame from its interrupt. Above the cursor is the screen as it
stood when the beam got there, with every instruction up to that point
executed and no more. Below is what the display file holds at that moment,
dimmed, since the ULA hasn't put it out yet. The cursor position becomes a
T-state using the same arithmetic as the renderer, so the split lands exactly
where the beam would be.

**CRT** puts the picture on a tube: the curve of the glass and a gap under
every line. With it on, **Composite** takes the picture down an aerial lead,
with colour smeared sideways by the subcarrier and the herringbone that
subcarrier beats against the dot clock. An RGB monitor had neither, which is
why Composite only works with the tube.

**Overscan** chooses how much border you get. On (the default) shows the whole
384×304 the ULA puts out, which is where border-art demos live. Off crops to
304×240, roughly what a television showed. Only the amount of border changes,
and every pixel stays exactly where it was.

**Next frame**, while paused, runs to the end of the frame the machine is in
the middle of, so the picture on screen is one the ULA has finished.

## Debug Windows

### RAM Map

One pixel per byte, 256 bytes a row, updated every frame. There are two views:

- **Address space** is the 64K the CPU sees right now. Each 16K slot is
  labelled with what is paged into it (`$C000 RAM7`). Video RAM and any
  detected back buffer are outlined, and a white line marks the row PC is in.
- **All memory** is every RAM bank and ROM page the machine has, stacked as 16K
  blocks. Banks that *aren't* paged in are still there, greyed and marked
  "paged out". The paged-in ones get a bright outline saying which address they
  answer to (`RAM7 → $C000`), and the bank the ULA is displaying is marked
  "(screen)". On a 48K only the three reachable banks are shown.

Heat is recorded per physical location rather than per address. That way a
bank keeps its history while it's paged out, and two banks taking turns at
`$C000` don't smear into each other.

- **Green**: reads
- **Red**: writes
- **Blue**: instruction fetches

Each colour can be switched off on its own, so you can look at writes alone, or
separate code from data by hiding reads. Every access sets its pixel to full
brightness and it fades from there, with a fade rate per channel and an overall
gain. Hover over a pixel for the bank, the offset within it, the address it
answers to (or "not paged in"), the byte's value and its lifetime read and
write counts. Click it to jump the debugger there.

### Debugger

- Live disassembly around PC, aligned to real opcode boundaries. It searches
  backwards for a start address that tiles exactly onto PC, so the lines above
  the current instruction aren't garbage.
- Every register, including the shadow set, `IX`/`IY`, `I`, `R`, `IM`,
  `IFF1/2`, `WZ`, and the individual flags.
- **Pause / Run** (F5), **Step into** (F7), **Step over** (F8), **Step out**,
  and **Step back**. Step over runs `CALL`, `RST` and the repeating block
  instructions to completion. Step out runs until SP rises above where it is
  now. Step back undoes up to the last 20 instructions. It works because what
  each one changed was written down as it ran, not because the machine was
  copied. The sound already played and the tape's position aren't put back,
  since those live outside the machine's memory.
- Breakpoints. Click a disassembly line to toggle one, or type an address.
- **Disassemble** disassembles only where code has been seen to run since the
  last reset or snapshot, and shows every other byte as `DEFB`. Tables,
  graphics and text stop reading as nonsense code.
- **Export…** saves the program in RAM (`$4000`–`$FFFF`) as assembly source
  that assembles back into the same bytes, with every label and comment from
  your notes. The ROMs it ran against are named in the header by CRC32 and left
  out. Undocumented instructions, and ones with more than one encoding, go out
  as `DEFB` with the instruction beside them, so any assembler rebuilds the
  same bytes. Checked with sjasmplus against a real program, byte for byte.
- A hex/ASCII memory dump with shortcuts to follow HL or SP.

### Profiler

**Start** and **Stop** record a run. Each run is listed with when it started,
and when stopped it gets its length, both wall-clock and emulated time. Runs
are kept so you can compare them.

Beside the list is a bar graph of where the time went: one bar per function,
longest first, labelled with the entry point and showing its share, its time
and its call count. Hover over a bar to see whether the function is in ROM or a
RAM bank and how deeply it nested. **Click an entry point and the debugger
opens on that function.**

Calls are spotted from what the CPU *did*, not by decoding opcodes. A call is
any instruction that leaves SP two lower with the address of the next
instruction on the stack, which catches `CALL`, `RST` *and* interrupt
acceptance. So interrupt handlers get profiled like anything else. A return is
any instruction that pops the address it jumps to. Each function gets **self**
time and **inclusive** time (switch with **Rank by**). Anything still on the
stack when you press Stop keeps the time it had, and time outside any call is
reported separately.

### Graphics

The Graphics window reads memory as pictures. It works in cells (eight bytes a
character, the way the font is kept) or in rows (a row of pixels at a time,
the way most games keep their sprites), mirrored or inverted.

**Find…** is the useful bit. It pauses the machine and rings the 8×8 block
under the pointer on the main window. Click one and it looks for those eight
bytes everywhere in the 64K the machine can see: as a character, as a column of
a sprite up to 64 bytes a row (32 with a mask beside each byte), mirrored and
inverted. **Show** puts any match in the viewer. It beats squinting at a hex
dump.

### Call Flow, Back Buffer, Tape, Keyboard

**Call flow** draws one turn of whatever loop the program is going round: the
routines it calls in the order it calls them, nested as they nest, each box
sized by how much work it does. It's worked out from what ran, not read off the
code, and since that means sifting a few hundred thousand calls, it's done when
you ask rather than every frame. **Back buffer** is covered in the next
section, and **Tape** under [Tapes](#tapes).

**Keyboard** is the machine's keyboard, pressable and lit by the real one. A
box at its top right finds words on the keys (`print`, `merge`, `cat`, `not`,
`beep`), rings the key each is on in yellow and the shifts it needs in cyan,
and says how to type it: "BEEP: extended mode, then SYMBOL SHIFT with Z".
Commas look for several at once. If you've forgotten where `MERGE` lives,
you're not alone.

## Watching the Screen Being Drawn

**Slow draw** parks the CPU once it has written a set number of bytes to the
watched area during the current host frame. A screen that normally appears
instantly then assembles over several seconds. Set it to 8 writes a frame and
you can watch a routine fill the display byte by byte. The emulator still
renders 60 times a second, so the picture fills smoothly.

You choose what is watched:

- **video RAM**: `$4000`–`$5AFF`, the normal display file.
- **back buffer**: the region the detector found, or one you set by hand.

### Back-Buffer Detection

Plenty of programs build a frame in ordinary RAM and then blit it into `$4000`,
so the interesting drawing never touches video memory at all. The detector
looks for that:

- It counts writes per 256-byte page over a 25-frame window, looking for runs
  of at least 24 pages (6,144 bytes, one bitmap's worth) outside video RAM.
- Whenever a write into video RAM immediately follows a read from somewhere
  else, the source page's score goes up. That's the signature of `LDIR`, a
  stack blit or unrolled `LD` copies. A copy counts as much as 32 plain writes,
  so a buffer that is actually flipped to the screen beats one that is merely
  written to.
- The result comes with a confidence figure. A strong copy signal is enough on
  its own, even without a long contiguous run.

The back-buffer window renders the detected region as if it were video RAM, so
you can watch the *off-screen* image being built. Combined with slow draw on
the back buffer, you can watch it built a few bytes at a time. If the
heuristic guesses wrong, set the address by hand.

The demo ROM (`src/demo_rom.rs`) exists to show this off. It builds an
animated 6,912-byte screen at `$8000` and `LDIR`s it into `$4000`, and the
detector finds it within a couple of seconds.

## Tapes

**File ▸ Load tape…** opens `.tzx` and `.tap` (zipped or not), and the ZX81's
`.p`, `.81` and `.p81`. TZX is a pulse-level format, so the player doesn't
decode bytes. It turns blocks into a stream of pulse lengths in T-states and
drives the EAR bit through port `$FE`, exactly as a real tape feeds the ULA.
ROM loading, turbo loaders and custom pulse schemes all go through that one
path. Nothing is trapped or faked unless you ask for it.

The tape window's **Speed** is one of three:

- **Normal** plays the tape at the speed it was recorded at. You'll have time
  to make a cup of tea.
- **Max CPU** runs the machine as fast as it will go while the tape moves.
  Still real loading, just quicker.
- **Fastload** answers the ROM's `LD-BYTES` call instead of running it: the
  next block with the right flag byte is copied into memory and the routine
  returns as it would have. Border Break loads in 2 emulated frames instead of
  2,605. It can't help a game with a loader of its own, since that's counting
  pulses itself, so switching Fastload on switches Max speed on with it.
  Anabasis, which loads a BASIC stub and then reads the rest itself, only goes
  from 18,722 frames to 18,149.

[`docs/tape-loading.md`](docs/tape-loading.md) has the formats, their traps and
the ROM loader's contract.

Supported TZX blocks: standard speed ($10), turbo ($11), pure tone ($12), pulse
sequence ($13), pure data ($14), direct recording ($15), pause and
stop-the-tape ($20), group start/end ($21/$22), jump ($23), loops ($24/$25),
call sequence and return ($26/$27), stop-if-48K ($2A), set signal level ($2B),
and the informational blocks ($30–$35, $5A). CSW and generalised-data blocks
($18/$19) are skipped over rather than played.

### The Tape Window

- **Deck**: **Load…** puts a tape in. **Blank** puts in an empty one and
  records onto it, so `SAVE "name"` writes a block to the tape for each block
  the ROM sends. **Save…** writes the deck out as `.tzx` (pauses kept) or
  `.tap`. Loading a real tape stops the recording, so a SAVE never lands on the
  end of somebody's game. Only the ROM's own timings are read back. A program
  with a saver of its own is counted as not read, rather than written down
  wrongly.
- **Block list**: every block with its type, size and, for standard blocks, the
  decoded header (`Program "JETPAC"`, `Bytes "JPSP"`, …). Click a block to move
  the tape straight to it. The playing block is highlighted and kept in view.
  Turn off **Follow playing block** if you want to browse while it runs.
- **Progress**: one bar for the whole tape and one for the current block, with
  its name, how far through it is and the playing time left. The position comes
  from where the pulse generator has got to, not from the clock, so it stays
  right after a seek or a pause.
- **Transport**: Start, Rewind, Play/Pause, Stop, Fast forward. Rewind and fast
  forward step through the tape's sections, skipping the informational blocks
  that make no sound. A tape loads **stopped**. **Play on load** changes that
  if you'd rather it started straight away.
- **Oscilloscope**: the EAR waveform as it plays, drawn from the player's edge
  log, so it's exact rather than sampled. The sweep is edge-triggered (rising
  by default, or falling, or free-running) so the trace stands still instead
  of scrolling. The sweep runs from 50 µs, enough to see the individual pulses
  of a turbo loader, up to 40 ms.
- **Quality**: shows what the deck does wrong, meaning the motor's wobble, the
  head's alignment and the tape's hiss. It's three extra rows, so they're put
  away when you're not using them.

The cassette is the artwork in `designs/`, rendered as it is. Its hubs turn at
the speed a real deck's do. A compact cassette runs at 1⅞ inches a second and
a C60 winds out to about 25.7 mm from the hub, so a full pack comes round about
eighteen times a minute and a nearly empty one about thirty. Nobody asked for
that, but it would have bothered me otherwise.

### ZX81 Tapes

`.p`, `.81` and `.p81` files play as the pulse train a real ZX81 would have
saved, so they load through the ROM's own loader rather than being poked into
memory. Bits go out most significant first: a burst of four pulses for a 0 and
nine for a 1, each pulse a 150 µs high half and a 150 µs low half, with a
1,300 µs gap closing every bit. Those figures come from the ROM's SAVE routine
at `$031E`, which works out the burst length with `AND $05 / ADD A,$04`.

A `.p` has no file name, so one is made from the host file name in ZX81
character codes, with bit 7 marking the last. A `.p81` brings its own. Opening
one while a Spectrum is running switches to a ZX81 first. Then it's the 1982
ritual: `LOAD ""`, NEWLINE, Play. At roughly 50 bytes a second a 16K game takes
several minutes, so Max CPU is your friend.

Each machine has its own deck, because a tape is timed in the T-states of the
machine playing it, and the ZX81's 3.25 MHz isn't the Spectrum's 3.5 MHz. The
tape window follows whichever machine is running.

## The 128K, +2A and +3

The machine dropdown switches between the models at any time, provided the ROM
is there. Here's what the 128K changes over the 48K:

- **Memory**: eight 16K RAM banks paged through `$7FFD`, covering the bank at
  `$C000`, the shadow-screen select, the ROM select and the paging lock. Banks
  5 and 2 stay at `$4000` and `$8000`.
- **Timing**: 3.5469 MHz, 70,908 T-states a frame, 228 a line, first pixel at
  14,361. All of it is per model, so contention lands correctly on both.
- **Contention** follows the *bank*, not the address. Any odd-numbered bank is
  contended wherever it is paged, so `$C000` is contended only when bank 1, 3,
  5 or 7 is there.
- **Display**: the ULA draws from bank 5 or bank 7 depending on the
  shadow-screen bit, and the main window follows it.
- **AY**: `$FFFD` selects a register (and reads it back), and `$BFFD` writes it.

And what the +2A and +3 change again:

- **Four ROMs** instead of two. The low bit comes from `$7FFD` bit 4 and the
  high bit from `$1FFD` bit 2.
- **Port `$1FFD`**, the second paging latch. Bit 0 selects *special* paging:
  no ROM at all, and four RAM banks filling the address space in one of four
  fixed layouts chosen by bits 1–2 (`0,1,2,3` / `4,5,6,7` / `4,5,6,3` /
  `4,7,6,3`). Bit 3 is the disk motor.
- **Contention follows different banks**: the top four (4–7) rather than the
  odd ones, *and* it uses a different delay sequence, `1,0,7,6,5,4,3,2` where
  the earlier machines use `6,5,4,3,2,1,0,0`.
- **Stricter port decoding**: `$7FFD` needs A15 low and A14 high, so unlike on
  a 128K a write to `$3FFD` doesn't disturb paging.
- **No floating bus**: the gate array drives the bus high, so unattached ports
  read `$FF`.

The debugger shows the live memory map (`$0000:ROM0 $4000:RAM5 $8000:RAM2
$C000:RAM7`), both latches (including which all-RAM layout is active), which
bank is on screen, and an AY panel with the raw registers plus decoded
periods, frequencies, volumes, mixer routing and envelope settings.

### Disks

The +3 has a working disk interface: a µPD765A controller with `.dsk` images
(zipped or not) and `.ipf`. **File ▸ Load disk…** asks before the machine gets
the disk: read-only, write to a copy, or write back to the file. That's
because a game writes its high scores to the disk it loaded from, and that
shouldn't be the first you hear of it. **File ▸ Create blank disk…** makes one
formatted as the machine's own FORMAT would.

[`docs/disks.md`](docs/disks.md) says what is emulated and what isn't. IPFs,
for instance, are read-only.

## The ZX81

**ZX81 1K** and **ZX81 16K** in the machine dropdown switch to a ZX81, which
needs an 8K ROM at `roms/zx81.rom`.

The ZX81 has no video hardware to speak of. The picture comes from the CPU
walking the display file while the ULA watches the bus. When an opcode is
fetched from an address with A15 set and its byte has bit 6 clear, the CPU is
fed a NOP. Meanwhile the ULA takes that byte as a character code, fetches its
bitmap from `(I<<8) | (char&0x3F)<<3 | LCNT` during the refresh half of the
same M1 cycle, and shifts out eight pixels, two per T-state. A byte with bit 6
set is a `HALT`, which the CPU really executes, and that ends the line.

Emulating it at that level rather than drawing a character grid is what makes
it cycle exact. It also means the hi-res programs that abuse the mechanism need
no special handling. They get whatever their fetches put on the screen.

Around that sit the rest of the ULA's jobs:

- the three-bit line counter that picks the row within a character, held in
  reset while the vertical sync is low;
- the sync itself, started by reading port `$FE` and ended by any `OUT`;
- the NMI generator that times the borders in SLOW mode (`$FE` on, `$FD` off,
  firing once a line);
- the interrupt, which comes from bit 6 of the refresh register falling. That's
  how the ROM counts out a character row.

A line's T-states are counted from the interrupt that starts it, which is a
little before the visible part begins. So the picture is shifted back by the
difference when it's drawn, and the 256×192 picture sits in the middle of the
414×312 raster with 79 pixels of border either side, where a television shows
it.

**What's on screen is what a television would show**, rather than a tidy
picture of whatever the program drew. Each line is painted as the beam reaches
it and the rest of the screen is left alone, so a display that keeps
restarting doesn't flash fragments over an empty screen. The beam is blanked
while the sync is low. A television's line oscillator only locks to a sync that
arrives when a line is due. One that comes far too early is not a line at all,
so the beam stays where it is and is merely blanked.

That's what puts the ZX81's loading pattern on the screen. With the display off
in FAST mode, the ROM's tape loader pulses the sync hundreds of times a frame,
leaving black bars that shift with the data. With no sync at all, as in a FAST
computation with nothing driving the display, the beam sweeps a blank white
screen, which is what the real machine does.

Releasing the sync puts the beam at a fixed point in the line, because the ULA
holds its counters in reset while the sync is low. Only a sync held for at
least a line's worth of time pulls the picture back to the top. That fixed
point isn't the left edge: the sync pulse and the back porch take up the start
of a raster line. Getting it right is what makes a program that paces itself
land where the ROM's display does.

The hi-res games are why this matters. They raise and drop the sync once per
row themselves. Taking each of those pulses as a vertical sync restarts the
picture hundreds of times a second and draws nothing but the first row.
Ignoring them lets every row land wherever the code happened to reach, so the
picture skews and slides about. Two wrong answers, both tried, and neither
looked like a ZX81.

Memory follows the machine's sparse decoding. An 8K ROM appears twice in the
bottom page, 1K of RAM repeats sixteen times through its own page, and the
whole lot is mirrored above `$8000`, which is what lets the display routine
execute the display file with A15 set. A 16K ROM image fills the bottom page
instead of mirroring. A `.p` can also be loaded straight in as an image at
`$4009`. One too large for 1K is refused rather than silently truncated.

Tape input arrives on bit 7 of port `$FE`, which the ROM's loader tests with
`RLA` at `$035B`. Bit 6 is the 50/60 Hz jumper.

The debugger and RAM map work the same on a ZX81. Panels it has no use for (the
paging latch, the AY) aren't drawn, and the display file is outlined wherever
D_FILE points rather than at a fixed address. There's no sound hardware, so
what you hear while a tape loads is the recorder's monitor. Quicksaves and
Save snapshot are Spectrum-only for now. The ZX81 can't be copied yet.

## Sound

Sound is made on the emulated clock, not on a timer. The mixer integrates the
output level over each sample period, so a beeper edge lands in the right
sample even when it happens mid-instruction.

- **Beeper**: bit 4 of port `$FE`. Bit 3 (MIC) and the EAR input are mixed in
  quietly too, which is why you can hear a tape loading.
- **AY-3-8912** (128K and later, or a Fuller Box): three square-wave channels
  with 12-bit periods, the 17-bit noise generator, all sixteen envelope shapes
  and the logarithmic volume table. It runs at half the CPU clock and is
  averaged over each output sample rather than point-sampled, which keeps high
  tones from aliasing.

The tape's EAR line is mixed in at the T-state each edge happens. If you only
sample whenever the CPU last looked at the port, a burst of pilot tone collapses
into noise. The output is DC-blocked, since the real machine is AC coupled, so
a held beeper level decays to silence instead of thumping. Gain changes are
ramped so muting never clicks.

The controls are under the screen. **Beeper**, **AY** and **Hardware** (what
the add-ons make, like the SpecDrum) switch separately, because you don't always
want all three: the AY over a beeper that's only clicking, say. Then there's
the volume, **auto-mute**, and the **buffer**. Hover over the speaker icon for
the device, sample rate, buffer depth and dropped samples.

**auto-mute** (on by default) silences everything unless the machine runs at
about normal speed. Fast-forwarding otherwise produces a shriek, and you'll
only need to hear it once.

The emulator paces itself against the sound card. Each host frame it runs a bit
more or less emulation to hold the buffer near its target, 60 ms by default and
anywhere from 20 to 250 ms on the slider. That's what stops the queue drifting
into underruns or overflows, both of which crackle. If you hear crackling, raise
it. Lower it if you want a snappier beeper.

## What Plugs Into It

The **Hardware** window lists everything by section (Mice, Multiface, Printers,
Audio, Joysticks, Drives) and says, for each, how far it's emulated. A switch
that turns on nothing is worse than no switch, because it looks like it works.
So the RAM Music Machine, which can be fitted but does nothing yet, says so.

- **Interface 1** with up to eight microdrives, once `roms/if1.rom` is there.
  The **Microdrive** window takes `.mdr` cartridges (zipped or not): read-only,
  writing to a copy, or writing in place. `FORMAT`, `SAVE *`, `LOAD *` and
  `CAT` all work, and a test drives that whole round trip from the emulated
  keyboard.
- **Multiface One, 128 and 3**, each with its ROM in `roms/`. One **Red button**
  appears on the main window's second row for whichever is fitted, because a
  Multiface exists to be pressed while something else is running, and going to
  find a window first defeats that.
- **Currah µSpeech**, which really talks. The SP0256-AL2 is emulated as the chip
  it is, a microsequencer driving a twelve-pole filter, not a bank of samples.
- **Fuller Audio Box** and **Cheetah SpecDrum**.
- **Kempston mouse** or **AMX mouse** (one or the other, as both answer at
  `$DF`). Click the screen and the host's pointer is captured, and its movement
  and buttons go to the machine in the machine's own pixels. **Esc** gives it
  back, and so does switching away from the main window.
- **ZX Printer** or **Alphacom 32** (they share a port). Either adds a
  **Printer** window. The paper comes out of the bottom as each line prints,
  **Save PNG…** writes it out a pixel a dot, **Tear off** starts again, and a
  switch puts it on the ZX Printer's silver paper or faded thermal. `COPY`,
  `LPRINT` and `LLIST` all print.

Joysticks are in the **Input** window: Kempston, the two Sinclair sticks,
Cursor and Fuller, worked from the keyboard or from a gamepad. Either can be
mapped onto the stick, onto keys of the machine's own keyboard, or onto the
buttons of a fitted mouse. The DK'Tronics Programmable interface is taught its
keys there too, with its slider at 2, as its manual says.

What's plugged in, and into which machine, is remembered between launches.
[`docs/peripherals.md`](docs/peripherals.md) has the detail on each.

## ROMs, Snapshots and Recordings

### Where ROMs Are Looked For

A shipped app can't rely on the working directory. A double-clicked macOS
`.app` runs with it set to `/`, which is not where you keep your ROMs. So these
places are searched in order, each also with a `roms` subdirectory:

- the working directory, for `cargo run` or a shell launch
- beside the executable, for an unzip-and-run build
- `../Resources`, inside a macOS `.app`
- `../share/zx-rustrum`, for a Unix `bin`/`share` install
- the configuration directory, which survives replacing the app

A name outranks proximity, so a `128.rom` anywhere beats a `128k.rom` nearby.
Opening a ROM also **scans its folder for the others** and adopts the ones for
machines that have no image yet. They're recognised by size (16K, 32K, 64K),
with the file name breaking ties, so pointing at one `128.rom` usually lights
up the 48K, 128K and +3 together. A ROM you've already loaded is never
replaced by a scanned one. The folder is scanned again at the next launch.

### Snapshots and Quicksaves

**File ▸ Load snapshot…** takes `.szx`, `.sna`, `.z80` (v1/v2/v3) and `.scr`.
A snapshot carries its machine type, so loading one switches to the machine it
needs first, provided that ROM is there. **Save snapshot…** writes `.szx` (which
keeps the paging, the AY and the joystick) or `.sna`, the old 48K dump. **Save
screen…** writes the 6,912 bytes of a `.scr`.

**Quick** on the main window keeps ten quicksaves in memory. Save puts the
machine in the slot shown, Load puts it back, and the dropdown picks another
slot (filled ones are marked `•`). F1 to F9 save to slots 1 to 9 and F10 to
slot 0. A quicksave is the *whole* machine, including what's plugged in and
where the tape had got to, which a `.sna` can't hold and a `.szx` only partly
can. They last until the emulator closes.

### Recording

**⏺ RZX** records everything the machine reads from now on, starting from a
snapshot, so a run of a game can be played back exactly and read instruction
by instruction. **File ▸ Load recording…** plays one back. During playback the
keyboard belongs to the recording, so don't bother pressing anything.

The video button writes what the screen shows to a video file, CRT effects
and all, with the sound joined on when you stop. It hands the frames to
`ffmpeg`, which needs to be on your `PATH`. Without it you get a plain message
saying so, not a broken button. The picture's size can't change mid-recording,
so CRT, Overscan and the zoom are held while it runs. Composite can still be
switched, since it changes the pixels rather than how many there are.

## Taking a Game Apart

Some of this is aimed at a language model doing the work rather than a person,
but it's all usable by hand.

- `cargo run --release --bin mcp` runs the emulator as an
  [MCP](https://modelcontextprotocol.io) server over stdin and stdout, with no
  window. It loads tapes, snapshots and recordings, runs the machine under
  control, reads registers and memory, disassembles, watches which routine does
  what, and writes labels and comments against addresses.
  [`docs/mcp.md`](docs/mcp.md) lists the tools.
- `trace recordings/manic.rzx --frames 3000 > manic.jsonl` plays a recording
  and writes one JSON object per routine saying what it was *seen* to do: how
  often it ran, what it wrote, and what it drew, since the bus remembers who
  wrote each byte of the screen.
- `verify recordings/manic.rzx 8DAA --frames 600` tests a claim like "$8DAA
  draws the guardians" by playing the same frames twice, once with the routine
  patched to return at once, and seeing what changed. If the guardians vanish,
  the claim holds. If nothing changes, it doesn't, whatever the code looks like.
- `corpus` builds a training corpus from a memory image plus somebody else's
  symbol file, using the emulator's own feature extractor.

Each of those is a binary in `src/bin/`. Run them with `cargo run --release
--bin <name> -- <args>`. [`docs/agent-disassembly.md`](docs/agent-disassembly.md)
sets out what's still missing for a full disassembly done this way.

## Training a Network to Play

The **Training** window teaches a neural network to play whatever the machine
is running, with nothing extra to install. Say what it may press (a joystick
interface, or particular keys), how much of the picture it sees, and where the
game keeps its score and lives, then press Start. **Find a number** works out
where the score is kept, including scores spread across several bytes.

It learns by PPO on the graphics card, sees the screen and nothing else, and is
rewarded by a judge that reads the score out of memory. The network is kept
beside the tape, and **Let it play** hands it the controls. **Time it** tells
you how long the training will take before you commit to it.
[`docs/training.md`](docs/training.md) says what each setting does and what was
measured.

## Preferences

A preferences file is created the first time the emulator runs, in the usual
place for your platform:

| Platform | Location |
| --- | --- |
| macOS | `~/Library/Application Support/ZX Spectrum Emulator/preferences.toml` |
| Windows | `%APPDATA%\ZX Spectrum Emulator\preferences.toml` |
| Linux and friends | `$XDG_CONFIG_HOME/zx-rustrum/preferences.toml`, or `~/.config/…` |

Set `ZX_SPECTRUM_CONFIG_DIR` to put it somewhere else.

It keeps every window's position and size, the display scale, the overscan
setting, what's plugged in, and where each kind of file last came from. Open a
ROM and the ROM picker starts there next time, and tapes and snapshots are
tracked separately, so a tape doesn't send you hunting for ROMs. It's written
when the emulator closes and again a couple of seconds after anything settles,
so a crash doesn't lose your layout.

It's a plain `key = "value"` file, TOML-compatible, and meant to be edited by
hand if you like. Keys the emulator doesn't know are left alone when it saves.

## How It Looks

The interface follows `zx-ux-mockup.html`: dark case plastic, near-black
outlines and the machine's own seven colours. It's monospace throughout, on the
grounds that the thing being emulated never displayed anything else.

`src/ui/theme.rs` holds the palette, and everything drawn by hand takes its
colours from there rather than from a literal. The debugger's registers sit on
a sunken LCD panel, the display has a bevelled surround, and the status line
ends with the seven-colour flash the machine wears on its case.

Two things in the mockup didn't make it. The title bars belong to the operating
system, so there are no rivets. And the headings use the built-in monospace
font rather than Press Start 2P, which would have meant shipping a font file.

`src/svg.rs` is a small SVG renderer, just enough for the drawings in
`designs/` (groups with matrix transforms, lines and cubic curves, rectangles,
circles, solid and linear-gradient fills, strokes, and the even-odd rule that
makes the cassette's window a hole). It isn't a general implementation and
isn't meant to become one. It exists so that re-exporting a drawing changes the
emulator without anyone redrawing it in code, and because no new crate can be
added to this build (see `CLAUDE.md`).

**The name**: ZX-Rustrum, the machine it emulates by way of the language it's
written in. The mark is the Spectrum's seven-colour flash. `src/logo.rs` draws
it as pixels for the window icon, so nothing has to ship beside the binary, and
`packaging/make-icon.py` draws it larger for the application icon.

## Tests

```
cargo test --release
```

There are 114 test files in `tests/`, named for what they cover, with each test
named as a sentence saying what behaviour it checks. A cold build of all of
them is heavy, which is why CI turns LTO off for tests (see
[Building a Release](#building-a-release)). Tests that need `roms/48.rom` or
`tapes/` skip themselves when those aren't there.

The ones that hold the emulator to the real machine are the ones to know about:

- `tests/halt2int.rs` boots a real 48K ROM, types `LOAD ""`, plays HALT2INT off
  tape, and reads the results back off the screen by matching character cells
  against the ROM font. It then compares them with the photographs of real
  hardware in both timing modes.
- `tests/border.rs` loads Border Break, waits for its border routine and
  compares the rendered border with rows taken from the photograph. It also
  checks that a loop alternating the border makes stripes 38 and 62 pixels
  wide, which are the 19 and 31 T-states that loop actually takes.
- `tests/timing.rs` checks T-state counts for around 60 instructions against
  the published timings, plus the interrupt and NMI acknowledge sequences,
  `EI`'s one-instruction shadow and contention.
- `tests/tape.rs` parses every tape in `tapes/` and checks the pulse trains
  against the ROM's timings. Then it does the real thing twice: once by calling
  the 48K ROM's `LD-BYTES` at `$0556`, and once by typing `LOAD ""` and loading
  a whole game from a real TZX.
- `tests/profiler.rs` checks time attribution against hand-counted T-states
  (two leaf functions at exactly 30 and 50 per call, a nested pair splitting 35
  self and 85 inclusive).
- The `ui_*` tests drive the interface itself through `egui_kittest`, clicking
  real buttons and checking that the thing actually changed. One early tape test
  was matching the toolbar's `100%` button instead of the progress bar it meant
  to check. That's why every test here is checked against the bug it guards:
  revert the fix and watch it fail.

`tests/audio_device.rs` opens your actual sound device, and skips itself if
there isn't one.

## Building a Release

`packaging/macos-app.sh` builds `ZX-Rustrum.app` and a `.dmg` around it,
generating the `.icns` from `packaging/icon.png` with `sips` and `iconutil`.
It signs both with the Developer ID named by `MACOS_SIGN_IDENTITY` under the
hardened runtime. If you give it an App Store Connect API key
(`MACOS_NOTARY_KEY`, `MACOS_NOTARY_KEY_ID`, `MACOS_NOTARY_ISSUER`), it
notarises both and staples the ticket to each. Without them you get an unsigned
app, which is fine on the machine it was built on.

The release workflow does all this from five repository secrets, and **won't
build a release without them**:

| Secret | What it is |
| --- | --- |
| `MACOS_CERTIFICATE` | The Developer ID Application certificate and its key, exported from Keychain Access as a `.p12`, then base64-encoded |
| `MACOS_CERTIFICATE_PASSWORD` | The password the `.p12` was exported with |
| `MACOS_NOTARY_KEY` | The text of an App Store Connect API key's `.p8` file |
| `MACOS_NOTARY_KEY_ID` | That key's ID |
| `MACOS_NOTARY_ISSUER` | The issuer ID shown above the list of keys |

`.github/workflows/release.yml` runs the tests, `cargo fmt --check` and `cargo
clippy -D warnings`, then packages a `.dmg` for each of the two macOS
architectures, a `.tar.gz` for Linux and a `.zip` for Windows.

**What a push costs decides what it runs.** This is a private repository, so
Actions minutes are billed: one a minute on Linux, two on Windows, ten on
macOS. An ordinary push to main is tested on Linux and nothing else. A push
that only changes prose isn't built at all, so this README cost nothing. The
other platforms and the four packaging jobs run only when there's something to
release, and a run overtaken by another push is cancelled.
`workflow_dispatch` has a **full** switch for a complete build without a
version bump.

Tests are built without link-time optimisation (`CARGO_PROFILE_RELEASE_LTO:
"false"`). Thin LTO over seventy-odd test binaries was most of the build: cold,
on this machine, `cargo test --release --no-run` took 2,194 seconds of CPU with
it and 374 without. The packaged binaries keep the release profile as it is.
`cargo fmt` and `cargo clippy` run on the Linux job alone, since their answer
doesn't change with the platform.

**Cutting a release is bumping `version` in `Cargo.toml`.** When a commit lands
on main with a version that has no `v<version>` tag yet, the workflow tags it
and publishes a release with the four builds attached, named for the version:
`zx-rustrum-1.2.0-linux-x86_64.tar.gz`, `ZX-Rustrum-1.2.0-macos-arm64.dmg`, and
so on. Pushing anything else to main tests it and publishes nothing, so a
version never means two different sets of binaries. You can still push a `v*`
tag by hand, but it's checked against `Cargo.toml` first. A tag naming a
version the tree doesn't have stops the build, rather than shipping binaries
that report a version they aren't.

`packaging/make-icon.py` draws the icon, a `ZX` over the four Spectrum colour
bars, writing `icon.png` and a multi-size `icon.ico`. It has no dependencies,
not even Pillow. It rasterises the shapes itself and writes the PNG chunks by
hand.

Per platform:

- **macOS**: the binary links only system frameworks, so there's nothing to
  bundle beyond the app.
- **Linux**: `cpal` links `libasound.so.2` (build with `libasound2-dev`), and
  the file dialogs go through `xdg-desktop-portal`. An AppImage or Flatpak
  carries both. A bare tarball expects the host to have them.
- **Windows**: built with `windows_subsystem = "windows"`, so no console window
  opens behind it. Cross-compiling from macOS isn't worth the trouble, so let
  the CI runner do it.

No ROMs are shipped, so a packaged build starts with the machines greyed out
until ROMs are put where it looks. See
[Where ROMs Are Looked For](#where-roms-are-looked-for).

## Known Limitations

[`docs/missing.md`](docs/missing.md) lists what other emulators have that this
one doesn't, checked against the source with `grep` rather than from memory.
The big one is divMMC/divIDE with esxDOS, which is how people load things on
real hardware today. Beyond that: Interface 2 cartridges, Beta 128/TR-DOS
(`roms/trdos.rom` is sitting there with nothing to use it), +D and DISCiPLE,
the Timex machines and the Pentagon, ULAplus, conditional breakpoints and an
in-app assembler.

Smaller ones you might hit:

- Quicksaves and Save snapshot are Spectrum-only. The ZX81 can't be copied yet.
- CSW and generalised-data TZX blocks ($18/$19) are skipped, not played.
- IPF disks are read-only.
- Video recording needs `ffmpeg` on the `PATH`.
- Day to day it's built and run on macOS. Every push is tested on Linux, and
  Windows gets built and tested when there's a release. Neither is driven by
  hand as often as the Mac build, so problems there may take longer to notice.

## Layout

| Path | What |
| --- | --- |
| `src/z80/` | CPU: state, decode/execute, ALU and flag tables |
| `src/machine.rs` | Memory map, ULA timing, contention, I/O, run loop |
| `src/zx81.rs` | The ZX81: its ULA, display and memory decoding |
| `src/screen.rs`, `src/crt.rs` | Display rendering, and the tube and composite effects |
| `src/tracker.rs`, `src/detect.rs` | Access heat maps and back-buffer detection |
| `src/tape.rs`, `src/flashload.rs` | TZX/TAP parsing, pulse-level playback and Fastload |
| `src/audio.rs`, `src/audio_out.rs` | AY-3-8912, the mixer, and the cpal output device |
| `src/snapshot.rs`, `src/szx.rs`, `src/scr.rs` | Snapshot and screen formats |
| `src/rzx.rs`, `src/recorder.rs`, `src/video_out.rs` | RZX recording and playback, and video out through ffmpeg |
| `src/fdc.rs`, `src/disk.rs`, `src/ipf.rs` | The +3's µPD765A and its disk images |
| `src/if1.rs`, `src/microdrive.rs`, `src/multiface.rs`, `src/uspeech.rs`, `src/sp0256.rs`, `src/printer.rs`, `src/mouse.rs`, `src/joystick.rs` | What plugs into the back |
| `src/disasm.rs`, `src/asmexport.rs`, `src/profiler.rs`, `src/callgraph.rs`, `src/observe.rs` | Disassembler, assembly export, profiler, call graph, the observer |
| `src/training/` | The network, PPO, the judge and the score search |
| `src/prefs.rs`, `src/resources.rs` | Preferences, and where ROMs are looked for |
| `src/ui/` | The main window and every other window |
| `src/bin/` | `zextest`, `mcp`, `trace`, `verify`, `corpus` |
| `docs/` | What has been learned about the hardware and the subsystems |

## Further Reading

- [`docs/timing.md`](docs/timing.md): T-states, contention, the floating bus,
  snow, the painted frame and how recordings are paced.
- [`docs/tape-loading.md`](docs/tape-loading.md): tape formats, the ROM
  loader's contract, and the three loading speeds.
- [`docs/peripherals.md`](docs/peripherals.md): everything in the Hardware
  window, and how far each goes.
- [`docs/disks.md`](docs/disks.md): the +3's disks, DSK and IPF.
- [`docs/training.md`](docs/training.md): teaching a network to play.
- [`docs/mcp.md`](docs/mcp.md): the MCP server's tools.
- [`docs/missing.md`](docs/missing.md): what isn't here yet.

If something on screen doesn't match a real machine, that's a bug. A
photograph of the real thing doing it differently is worth more than any
description, so attach one if you can. :)
