# ZX-Rustrum

A ZX Spectrum emulator in Rust — 48K, 128K, +2A and +3 — with a cycle-accurate
Z80 core, beeper and AY sound, and a set of detachable debugging windows, built
on `eframe`/`egui` with the **wgpu** renderer (Metal on macOS, Vulkan or DX12
elsewhere).

The OpenGL renderer is deliberately not used: on macOS it goes through glutin,
whose Cocoa backend panics with *"context to have a current view"* when one of
the extra windows loses its view — which this emulator, with four of them, runs
into. If you are building somewhere wgpu cannot find a backend, swapping the
`wgpu` feature for `glow` in `Cargo.toml` restores the OpenGL path.

```
cargo run --release             # 48K
cargo run --release -- --128    # 128K
cargo run --release -- --plus2a # +2A
cargo run --release -- --plus3  # +3
```

Each debug window is a real OS window (an egui *viewport*), so you can drag them
to a second monitor and watch them while the emulator runs.

Under the menu bar is a toolbar with everything you need in one click: the
machine selector (`48K 128K +2A +3`, greyed with "(no ROM)" when the image is
missing), a **Load…** button that takes tapes, snapshots and ROMs, **Reset**, and
toggles for the four debug windows. The window title always names the machine in
use, and failures — a missing ROM, an unreadable tape — appear in red in the
status bar rather than being swallowed.

## What it does

**Cycle-accurate Z80.** Timing is not counted per instruction but per bus cycle:
the CPU tells the machine how many T-states each fetch, read, write and internal
cycle takes, and the ULA applies memory contention at the exact T-state the
access happens. That includes the 48K contention pattern `6,5,4,3,2,1,0,0` over
the 128 T-states of each pixel line, the four I/O contention cases, and the
correct extra idle cycles for `INC (HL)`, `(IX+d)` addressing, `ADD HL,rr`, the
block instructions, conditional returns, and so on.

The core implements the full documented instruction set plus the undocumented
ones (`IXH`/`IXL`, `SLL`, the `DD CB d op` register copies), MEMPTR/WZ, the Q
register that drives `SCF`/`CCF`'s F3/F5 behaviour, and interrupt modes 0/1/2
with NMI.

**Verified against real hardware.** HALT2INT (Mark Woodmass) measures the R
register at the moment an interrupt is taken after a HALT reached at exact
T-states, which pins down interrupt timing, memory contention, the halt state
and the floating bus at once. The emulator reproduces the output of a real 48K
exactly, in both timing variants — the reference photographs are in `tapes/`,
and `tests/halt2int.rs` runs the tape and compares all thirty values plus the
test's own Early/Late verdicts.

Two things that test caught, both now fixed: the halt state performs its M1
cycles at PC — the byte *after* the HALT — so a HALT at `$7FFF` refreshes from
the uncontended `$8000` while one at `$4000` is contended on every cycle; and
the floating bus is sampled at the start of the IORQ cycle and reads back `$FF`
for the four idle T-states in each group of eight.

**Early and late timing.** Real 48K machines came in two variants, one running
the display a T-state later relative to the interrupt. **Late**, beside the machine dropdown on a 48K,
switches between them; the emulator matches the reference photograph for each.

**Race the beam.** With this on, hovering the picture shows the frame
half-drawn: everything up to the cursor is what the ULA has put out so far, and
the rest of that raster line and the lines below it are the *previous* frame,
drawn a third darker. The cursor position is converted to a T-state with the
same arithmetic the renderer uses, so the split lands exactly where the beam
would be. It works while paused, which makes it a way to read a frame's timing
by hand: park the emulator on a breakpoint and sweep the cursor to see what had
been drawn at any point in the frame.

**Overscan.** The toolbar's **Overscan** toggle chooses how much border to
draw: on (the default) shows the whole 384x304 area the ULA puts out, which is
where border-art demos work; off crops it to 304x240, roughly what a television
showed. Cropping only changes how much border surrounds the picture — every
pixel stays exactly where it was.

**Border art.** The border is rasterised by T-state rather than by scanline —
the ULA puts out two pixels per T-state, so a program writing port `$FE` in a
tight loop can draw in it at that resolution. Border Break (introspec/gonzy)
renders **pixel for pixel identically** to a real 48K: all 116,736 pixels of
`tapes/bb.png` match, border included.

Getting there needed three things beyond per-line sampling: the ULA fetches two
T-states ahead of the pixels it is emitting, so a border write lands on screen
slightly before the fetch clock that contention counts; a frame that has only
been drawn part-way still shows the *previous* frame below the point the ULA has
reached, rather than going black; and the border log has to be big enough for
the hundreds of writes per frame that this kind of code makes.

**Verified against zexdoc and zexall.** Both exercisers pass every test,
including the undocumented flag tests in zexall:

```
cargo run --release --bin zextest -- zexall.com
```

(`zexall.com` / `zexdoc.com` are not included; they are widely available, e.g.
in the `anotherlin/z80emu` repository under `testfiles/`.)

## Debug windows

### RAM access map

One pixel per byte, 256 bytes per row, updated every frame. Two views:

* **Address space** — the 64K the CPU sees right now, with each 16K slot
  labelled with what is paged into it (`$C000 RAM7`), video RAM and any
  detected back buffer outlined, and a white line on the row PC is in.
* **All memory** — every RAM bank and ROM page the machine has, stacked as 16K
  blocks. Banks that are *not* currently paged in are still there, greyed and
  marked "paged out"; the ones that are get a bright outline saying which
  address they answer to (`RAM7 → $C000`), and the bank the ULA is displaying
  is marked "(screen)" with its display file outlined. On a 48K only the three
  reachable banks are shown.

Heat is recorded per physical location rather than per address, so a bank keeps
its history while it is paged out, and two banks that take turns at `$C000` do
not smear into each other.

* **Green** — reads
* **Red** — writes
* **Blue** — instruction fetches

Each of the three can be switched off on its own, so you can look at writes
alone, or separate code from data by hiding reads. Every access sets its pixel to
full intensity and fades from there, with a separate fade rate per channel and an
overall gain. Video RAM and
any detected back buffer are outlined, and a white line marks the row PC is in.
Hovering reports the bank, the offset within it, the address it answers to (or
"not paged in"), the byte's value and its lifetime read/write counts; clicking
jumps the debugger to it.

### Debugger

* Live disassembly around PC, aligned to real opcode boundaries (it searches
  backwards for a start address that tiles exactly onto PC, so the lines above
  the current instruction are not garbage).
* All registers including the shadow set, `IX`/`IY`, `I`, `R`, `IM`, `IFF1/2`,
  `WZ`, and the individual flag bits.
* **Pause / Run** (F5), **Step into** (F7), **Step over** (F8), **Step out**.
  Step over runs `CALL`, `RST` and the repeating block instructions to
  completion; step out runs until SP rises above its current value.
* Breakpoints: click a disassembly line to toggle one, or type an address.
* **Disassemble**, beside Follow PC, disassembles only where code has been seen
  to run since the last reset or snapshot, and shows every other byte as data
  (`DEFB`) — so tables, graphics and text stop reading as nonsense code.
  **Memory**, beside Go to's PC, moves the listing to where the memory dump is.
* **Export…**, beside Disassemble, saves the program in RAM ($4000-$FFFF) as
  assembly source that assembles back into the same bytes: instructions where
  code has run, `DEFB` elsewhere, with every label and comment from the notes.
  The ROMs it ran against are named in the header by CRC32 and left out; calls
  into them keep their addresses, with the ROM's names as `EQU`s where known.
  Undocumented instructions, and ones with more than one encoding, are written
  as `DEFB` with the instruction beside them, so any assembler rebuilds the
  same bytes. Checked with sjasmplus against a real program, byte for byte.
* Speed presets from 1% to 2000%, plus a logarithmic slider.
* A hex/ASCII memory dump with shortcuts to follow HL or SP.

### Back buffer

Detection, preview and slow-motion controls for screens that are built outside
video RAM.

### Tape

Block list, transport and an oscilloscope for the loaded tape — see below.

### Profiler

**Start** and **Stop** record a run. Each run appears in a list with the local
date and time it started, and gains its length (wall-clock, plus the emulated
time covered) when stopped; runs are kept so you can compare them.

Next to the list is a bar graph of where the time went, one bar per function,
longest first. Each bar is labelled with the function's entry point in hex and
shows its share of the run, its time and how many times it was called; hovering
adds whether it is in ROM or a RAM bank, both time figures, and how deeply it
nested. **Clicking an entry point opens the disassembly of that function** in
the debugger.

Calls are spotted from what the CPU did rather than by decoding opcodes: a call
is any instruction that leaves SP two lower with the address of the following
instruction on the stack, which catches `CALL`, `RST` *and* interrupt
acceptance, so interrupt handlers are profiled like anything else. A return is
any instruction that pops the address it jumps to. Each function gets both
**self** time (excluding its callees) and **inclusive** time (entry to return),
switchable with **Rank by**; anything still on the stack when you press Stop
keeps the time it had spent, and time outside any call is reported separately.
While recording, the window shows the live call depth and which function the CPU
is in.

## Watching the screen being drawn

**Slow draw** parks the CPU once it has written a set number of bytes to the
watched area during the current host frame, so a screen that normally appears
instantly assembles over several seconds. Set it to 8 writes/frame and you can
watch a routine fill the display byte by byte; the emulator still renders 60
times a second, so the picture updates smoothly as it fills.

What is "watched" is configurable:

* **video RAM** — `$4000`–`$5AFF`, the normal display file.
* **back buffer** — the region found by the detector, or one you set by hand.

### Back-buffer detection

Plenty of programs assemble a frame somewhere in ordinary RAM and then blit it
into `$4000`, which means the interesting drawing never touches video memory.
The detector looks for that:

* It counts writes per 256-byte page over a 25-frame window and looks for runs
  of at least 24 pages (6144 bytes — one bitmap's worth) outside video RAM.
* Whenever a write into video RAM immediately follows a read from somewhere
  else — the signature of `LDIR`, a stack blit or unrolled `LD` copies — the
  source page's score goes up. A copy is worth 32 plain writes when ranking
  candidates, so a buffer that is actually flipped to the screen beats one that
  is merely written to.
* The result is reported with a confidence figure, and a strong copy signal on
  its own is enough even without a long contiguous run.

The back-buffer window renders the detected region as if it were video RAM, so
you can watch the *off-screen* image being built — and with slow draw watching
the back buffer, you can watch it being built a few bytes at a time. Set an
address manually if the heuristic guesses wrong.

Running the emulator with no ROM demonstrates all of this: the built-in demo ROM
(`src/demo_rom.rs`) builds an animated 6912-byte screen at `$8000` and `LDIR`s
it into `$4000`, and the detector finds it within a couple of seconds.

## Tapes

**File ▸ Load tape…** opens `.tzx` and `.tap` files, and the ZX81's `.p`, `.81`
and `.p81`. TZX is a pulse-level
format, so the player does not decode bytes: it turns blocks into a stream of
pulse lengths in T-states and drives the EAR bit through port `$FE`, exactly
like a real tape feeding the ULA. Ordinary ROM loading, turbo loaders and
custom pulse schemes therefore all work through the same path — nothing is
trapped or faked.

Load a game the way you would have in 1983: type `LOAD ""`, press ENTER, then
press Play — the tape does not start on its own. **Boost speed while playing** (on by default) runs the CPU at 8x
while the tape moves, so a 48K game arrives in seconds rather than minutes.

Supported TZX blocks: standard speed ($10), turbo ($11), pure tone ($12), pulse
sequence ($13), pure data ($14), direct recording ($15), pause and stop-the-tape
($20), group start/end ($21/$22), jump ($23), loops ($24/$25), call sequence and
return ($26/$27), stop-if-48K ($2A), set signal level ($2B), and the
informational blocks ($30–$35, $5A). CSW and generalized-data blocks ($18/$19)
are skipped over rather than played.

### ZX81 tapes

`.p`, `.81` and `.p81` files play as the pulse train a real ZX81 would have
saved, so they load through the ROM's own loader rather than being poked into
memory. Bits go out most significant first: a burst of four pulses for a 0 and
nine for a 1, each pulse a 150 µs high half and a 150 µs low half, with a
1300 µs gap closing every bit. Those come from the ROM's SAVE routine at
`$031E`, which works out the burst length with `AND $05 / ADD A,$04`.

A `.p` carries no file name, so one is made from the host file name in ZX81
character codes with bit 7 marking the last; a `.p81` brings its own. Opening
one while a Spectrum is running switches to a ZX81 first. Then it is the 1982
ritual: type `LOAD ""`, press NEWLINE, press Play. At roughly 50 bytes a second
a 16K game is several minutes, so leave the speed boost on.

Both machines have their own deck, because a tape is timed in the T-states of
the machine playing it and the ZX81's 3.25 MHz clock is not the Spectrum's
3.5 MHz. The tape window follows whichever machine is running.

### Tape window

* **Deck** — **Load…** puts a tape in; **Blank** puts in an empty one and
  records onto it, so `SAVE "name"` on the machine writes a block to the tape
  for each block the ROM sends, listed as it arrives; **Save…** writes whatever
  is in the deck out as `.tzx` (pauses kept) or `.tap`. Loading a real tape
  stops the recording, so a SAVE never lands on the end of somebody's game.
  Only the ROM's own timings are read back — a program with a saver of its own
  is counted as not read rather than written down wrongly.
* **Block list** — every block with its type, size and, for standard blocks, the
  decoded ZX header (`Program "JETPAC"`, `Bytes "JPSP"`, …). Click any block to
  move the tape straight to it. The block being played is highlighted and kept in
  view as the tape advances; **Follow playing block** turns that off if you want
  to browse the list while it runs.
* **Progress** — one bar for the tape as a whole and another for the block
  being played, which names it, shows the percentage through it and how much
  playing time is left. The position comes from where the pulse generator has
  got to rather than from the clock, so it stays right after a seek or a pause.
* **Transport** — Start, Rewind, Play/Pause, Stop, Fast forward. Rewind and fast
  forward step backwards and forwards through the tape's sections, skipping the
  informational blocks that make no sound. A tape loads **stopped**, like a real
  one: press Play when the loader is waiting. (**Play on load** turns that off if
  you would rather it started immediately.)
* **Oscilloscope** — the EAR waveform as it plays, drawn from the player's edge
  log so it is exact rather than sampled. The sweep is edge-triggered (rising by
  default, selectable, or free-running) so the trace stands still instead of
  scrolling; the trigger point is marked at the left edge and the threshold is
  drawn across the middle. The sweep width goes from 50 µs — individual pulses
  of a turbo loader — up to 40 ms.

## ZX81

**ZX81 1K** and **ZX81 16K** in the machine dropdown switch to a ZX81, which needs an
8K ROM at `roms/zx81.rom`.

The ZX81 has no video hardware to speak of: the picture is produced by the CPU
walking the display file while the ULA watches the bus. An opcode fetched from
an address with A15 set whose byte has bit 6 clear is fed to the CPU as a NOP,
while the ULA takes that byte as a character code, fetches its bitmap from
`(I<<8) | (char&0x3F)<<3 | LCNT` during the refresh half of the same M1 cycle,
and shifts out eight pixels — two per T-state, the same as a Spectrum's border.
A byte with bit 6 set is a `HALT`, which the CPU really executes, ending the
line.

Emulating it at that level rather than drawing a character grid is what makes it
cycle exact, and it means programs that abuse the mechanism for high-resolution
graphics need no special handling: they get whatever their fetches put on the
screen.

Around that sit the rest of the ULA's jobs: the three-bit line counter that
picks the row within a character and is held in reset while the vertical sync is
low, the sync itself (started by reading port `$FE`, ended by any `OUT`), the
NMI
generator that times the borders in SLOW mode (`$FE` on, `$FD` off, firing once
a line), and the interrupt, which comes from bit 6 of the refresh register
falling — which is how the ROM counts out a character row.

A line's T-states are counted from the interrupt that starts it, which is a
little before the visible part of the line begins, so the picture is shifted
back by the difference when it is drawn: the 256x192 picture then sits in the
middle of the 414x312 raster with 79 pixels of border either side, where a
television shows it. The cropped view is centred on it in turn.

What is on the screen is what a television would show, rather than a tidy
picture assembled from whatever the program drew. Each line is painted as the
beam reaches it and the rest of the screen is left alone, so a display that
keeps restarting does not flash a fragment of a picture over an empty screen.
The beam is blanked while the sync is low, and a television's line oscillator
only locks to a sync that turns up when a line is due — one arriving far too
early is not a line at all, so the beam stays where it is and is merely
blanked. That is what puts the ZX81's loading pattern on the screen: with the
display off in FAST mode, the ROM's tape loader pulses the sync hundreds of
times a frame, leaving black bars that shift with the data. With no sync at all
— a FAST computation with nothing driving the display — the beam sweeps a blank
white screen, which is what the machine does.

The sync is treated the way a television treats it. Releasing it puts the beam
at a fixed point in the line, because the ULA holds its counters in reset while
the sync is low; and only a sync held for at least a line's worth of time pulls
the picture back to the top. That fixed point is not the left edge — the sync
pulse and the back porch after it take up the start of a raster line — and it is
what makes a program that paces itself land where the ROM's display does. That matters for the hi-res games, which pace
themselves by raising and dropping the sync once per row rather than leaving it
to the ROM: taking each of those pulses for a vertical sync restarts the picture
hundreds of times a second and draws nothing but the first row, and ignoring
them entirely lets every row land wherever in the line the code happened to
reach, so the picture skews and slides about from frame to frame.

Memory follows the machine's sparse decoding: an 8K ROM appears twice in the
bottom page, 1K of RAM repeats sixteen times through its own page, and the whole
lot is mirrored above `$8000`, which is what lets the display routine execute the
display file with A15 set. A 16K ROM image fills the bottom page instead of
mirroring. `.p` files can also be loaded straight in as an image at `$4009`,
and one too large for 1K is refused rather than silently truncated.

Tape input arrives on bit 7 of port `$FE`, which the ROM's loader tests with
`RLA` at `$035B`; bit 6 is the 50/60 Hz jumper.

### On a ZX81

The debugger and the RAM map work the same way on a ZX81 as on a Spectrum: both
machines are a Z80 with memory and breakpoints behind them. The panels a ZX81
has no use for — the paging latch and the AY registers — are not drawn, the
memory line reads `$0000:ROM  $4000:RAM  $8000:mirror of $0000-$7FFF`, and the
RAM map's overlays name the ROM, the RAM and the mirror, with the display file
outlined where D_FILE currently points rather than at a fixed address.

Tape sound works too. The ZX81 has no sound hardware of its own, so what you
hear is the monitor from the recorder while a tape loads, mixed at the T-state
of each edge like everything else.

The toolbar is the only place the machine, the windows and the sound are
chosen: the menus that duplicated them are gone, leaving File. The 48K's late
timing switch sits beside the machine dropdown, where it applies, and the sound
device, buffer and auto-mute are with the volume under the screen.

Speed, machine and zoom are dropdowns rather than rows of buttons, which keeps
the toolbar to one line; the machines whose ROM is missing stay in the list,
saying so, rather than disappearing.

Each debug window opens where it was last left, including after being toggled
off and on again. The geometry in the viewport
builder is not always honoured when a window is created — on macOS the window
manager centres a default-sized one instead — so it is sent again from inside
the window, and nothing is recorded until it has had a moment to move.

## The artwork

The cassette in the tape window is the artwork in `designs/`, rendered as it
is. `src/svg.rs` is a small SVG renderer — groups with matrix transforms,
paths of lines and cubic curves, rectangles, circles, solid and linear-gradient
fills, strokes, and the even-odd rule that makes the shell's window a hole. It
is not a general implementation and is not meant to become one; it covers what
the drawings use, so that re-exporting them changes the emulator without
anyone having to redraw anything in code.

The shell is rasterised once per size and kept as a texture. The reels are the
flat discs the files say they are, drawn directly so they can wind on. The cogs
are rasterised once and then turned by rotating the quad they sit on, so a
spinning tape costs nothing per frame.

The hubs turn at the speed a real deck's do: a compact cassette runs at 1⅞
inches a second, and a C60's tape winds out to about 25.7 mm from the hub, so a
full pack comes round about eighteen times a minute and a nearly empty one
about thirty. Hurrying the tape along turns them half again as fast.

## The name

ZX-Rustrum: the machine it emulates, by way of the language it is written in.
The mark is the Spectrum's own seven-colour flash — `src/logo.rs` draws it as
pixels for the window icon, so nothing has to be shipped beside the binary, and
`packaging/make-icon.py` draws the same thing larger for the application icon.

## How it looks

The interface follows `zx-ux-mockup.html`: dark case plastic, near-black
outlines and the machine's own seven colours. It is monospace throughout, on
the grounds that the thing being emulated displayed nothing else.

`src/ui/theme.rs` holds the palette and applies it to egui's visuals, and
everything drawn by hand takes its colours from there rather than from a
literal — the oscilloscope's phosphor green, the RAM map's read/write/execute
key, the profiler's yellow-to-red bars, the flags that light red when set. The
debugger's registers sit on a sunken LCD panel, the display has a bevelled
surround instead of bare black, and the status line ends with the seven-colour
flash the machine wears on its case.

Two things in the mockup are not reproduced: the title bars are the operating
system's, so they have no rivets, and the headings use the built-in monospace
font rather than Press Start 2P, which would mean shipping a font file.

## Preferences

A preferences file is created the first time the emulator runs, in the usual
place for the platform:

| Platform | Location |
| --- | --- |
| macOS | `~/Library/Application Support/ZX Spectrum Emulator/preferences.toml` |
| Windows | `%APPDATA%\ZX Spectrum Emulator\preferences.toml` |
| Linux and friends | `$XDG_CONFIG_HOME/zx-rustrum/preferences.toml`, or `~/.config/…` |

It also keeps the position and size of every window, the display scale and the
overscan setting, written when the emulator closes and again a couple of
seconds after anything settles, so a crash does not lose the layout.

It is a TOML-compatible `key = "value"` file, meant to be readable and editable;
keys the emulator does not know about are left alone when it saves. Set
`ZX_SPECTRUM_CONFIG_DIR` to put it somewhere else.

What it remembers so far is where files came from: **open a ROM and the ROM
picker starts there next time**, the same for tapes and snapshots, each tracked
separately so a tape does not send you looking for ROMs.

Opening a ROM also **scans that directory for other ROMs** and adopts the ones
for machines you have no image for, recognised by size (16K, 32K, 64K) with the
file name breaking ties — so pointing at one `128.rom` typically lights up the
48K, 128K and +3 entries at once, and says which files it found. A ROM you have
already loaded is never replaced by a scanned one. That directory is scanned
again at the next launch, so the machines stay available between sessions.

## ROMs and snapshots

The 48K ROM is copyrighted and not included. Drop one at `roms/48.rom` (or use
**File ▸ Load ROM…**) and it is picked up at startup; without one, the built-in
demo ROM above runs instead. The names looked for are `48.rom` (16K), `128.rom`
(32K), `plus3.rom` (64K) and `zx81.rom` (8K).

A shipped app cannot rely on the working directory — a double-clicked macOS
`.app` runs with it set to `/` — so ROMs are looked for in several places, in
order, each also with a `roms` subdirectory:

* the working directory, for `cargo run` or a shell launch
* beside the executable, for an unzip-and-run build
* `../Resources`, inside a macOS `.app`
* `../share/zx-rustrum`, for a Unix `bin`/`share` install
* the configuration directory, which survives replacing the app

A name outranks proximity, so a `128.rom` anywhere beats a `128k.rom` nearby.
The directory a ROM was last opened from is scanned too.

**File ▸ Load snapshot…** loads `.sna` and `.z80` (v1/v2/v3) for 48K, 128K and
+3. Snapshots carry their machine type, so loading one switches the emulator to
the machine it needs first (provided that ROM is there); `.z80` v3 also restores
the `$1FFD` latch on a +3.

Files can also be named on the command line, dispatched by extension:

```
cargo run --release -- "tapes/Jetpac (1983)(Ultimate Play The Game)[16K].tzx"
```

`--48`, `--128`, `--plus2a`, `--plus3`, `--zx81` (or `--zx81-16k`) and
`--zx81-1k` choose the machine; a `.p` on the command line brings up a ZX81 on
its own.

## Sound

Both machines are audible. Sound is generated on the emulated clock, not on a
timer: the mixer integrates the output level over each sample period, so a
beeper edge lands in the right sample even when it happens mid-instruction.

* **Beeper** — bit 4 of port `$FE`. Bit 3 (MIC) and the EAR input are mixed in
  quietly as well, which is why you can hear a tape loading.
* **AY-3-8912** (128K) — three square-wave channels with 12-bit periods, the
  17-bit noise generator, all sixteen envelope shapes, and the logarithmic
  volume table. It runs at half the CPU clock and is averaged over each output
  sample rather than point-sampled, which keeps high tones from aliasing.

The tape's EAR line is mixed in at the T-state each edge happens, not sampled
whenever the CPU last looked at the port — otherwise a burst of pilot tone
collapses into noise. The output is DC-blocked, as the real machine's is AC
coupled, so a held beeper level decays to silence instead of thumping, and gain
changes are ramped so muting never clicks.

The emulator paces itself against the sound device: each host frame it runs
slightly more or less emulation to hold the buffer near its target depth
(60 ms by default, adjustable under **Sound ▸**), which is what stops the queue
drifting into underruns or overflows — both of which crackle.

**Sound ▸** has an on/off switch, a volume slider (also in the status bar), the
buffer depth and the device details. By default sound mutes itself when the emulator is not
running at roughly normal speed, since fast-forwarding and tape boost otherwise
produce a shriek; turn that off if you want to hear it anyway. Samples go to the
host device through a quarter-second queue: run slow and it underruns quietly,
run fast and the oldest samples are dropped rather than letting latency grow.

## The 128K, +2A and +3

The toolbar's machine dropdown, or **Machine ▸**, switches between the four models
at any time; each needs its ROM
in `roms/` (`48.rom`, `128.rom` — 32K, and `plus3.rom` — 64K, shared by the +2A
and +3). The same choices exist as `--48`, `--128`, `--plus2a`, `--plus3`.

What the 128K changes over the 48K:

* **Memory** — eight 16K RAM banks with `$7FFD` paging: bank at `$C000`,
  shadow-screen select, ROM select and the paging lock bit. Banks 5 and 2 stay
  at `$4000` and `$8000`.
* **Timing** — 3.5469 MHz, 70908 T-states per frame, 228 per line, first pixel
  at 14361. All of it is per-model, so contention lands correctly on both.
* **Contention** — follows the *bank*, not the address: any odd-numbered bank is
  contended wherever it is paged, so `$C000` is contended only when bank 1, 3, 5
  or 7 is there.
* **Display** — the ULA draws from bank 5 or bank 7 depending on the
  shadow-screen bit, and the main window follows it.
* **AY** — `$FFFD` selects a register (and reads it back), `$BFFD` writes it.

And what the +2A/+3 change again:

* **Four ROMs** instead of two. The ROM number is two bits: the low one from
  `$7FFD` bit 4, the high one from `$1FFD` bit 2.
* **Port `$1FFD`** — the second paging latch. Bit 0 selects *special* paging, in
  which there is no ROM at all and four RAM banks fill the address space, in one
  of four fixed layouts chosen by bits 1-2: `0,1,2,3` / `4,5,6,7` / `4,5,6,3` /
  `4,7,6,3`. Bit 3 is the disk motor.
* **Contention follows different banks** — the top four (4-7) rather than the
  odd ones, *and* uses a different delay sequence: `1,0,7,6,5,4,3,2` where the
  earlier machines use `6,5,4,3,2,1,0,0`.
* **Stricter port decoding** — `$7FFD` needs A15 low and A14 high, so unlike a
  128K a write to `$3FFD` does not disturb paging.
* **No floating bus** — the gate array drives the bus high, so unattached ports
  read as `$FF`.

The +3's disk controller is not emulated, so disk images will not load; the ROM
boots to its menu and reports "Drive M: available" (the RAM disk, which needs no
hardware), and tapes and snapshots work normally.

The debugger shows the live memory map (`$0000:ROM0 $4000:RAM5 $8000:RAM2
$C000:RAM7`), the `$7FFD` and `$1FFD` latches (including which all-RAM
configuration is active), which bank is being displayed, and an expandable AY
panel with the raw registers plus decoded periods, frequencies in
Hz, volumes, mixer routing and envelope settings.

## Emulated hardware

16K/32K/64K of ROM plus 48K or 128K of RAM, ULA display with FLASH and
per-scanline border capture (so raster bars show up), keyboard, beeper,
AY-3-8912 on the 128K and later, cassette input, and an approximate floating bus
on the machines that have one. The +3's disk controller, the Interface 1 with
its microdrives, the three Multifaces, the Currah µSpeech with its speech chip,
the Fuller Audio Box and the SpecDrum are emulated as well —
[`docs/peripherals.md`](docs/peripherals.md) says how far each goes.

Four joystick interfaces — Kempston, the two Sinclair sticks and the Cursor
one, plus the Fuller — are worked from the keys of the desk or from a gamepad
through the **Input** window, which maps either onto the stick, onto keys of
the machine's own keyboard, or onto the buttons of whichever mouse is fitted.
The Kempston, DK'Tronics and DK'Tronics Programmable joystick interfaces are
also in the **Hardware** window — the programmable one is taught its keys in
the Input window with its slider at 2, as its manual says — which lists everything by section — Mice, Multiface, Printers, Audio,
Joysticks and Drives — and the two windows agree about which the stick is in.

The **Keyboard** window is the machine's keyboard, pressable and lit by the
real one. A box at its top right finds words on the keys — `print`, `merge`,
`cat`, `not`, `beep` — and rings the key each is on in yellow and the shifts it
takes in cyan, with a line saying how to type it: "BEEP: extended mode, then
SYMBOL SHIFT with Z". Commas look for several at once.

The **Graphics** window reads memory as pictures, in cells (eight bytes a
character, as the font is kept) or in rows (a row of pixels at a time, as most
games keep their sprites), mirrored or inverted. **Find…** pauses the machine
and rings the 8×8 block under the pointer on the main window; clicking one
looks for its eight bytes everywhere in the 64K the machine can see — as a
character, as a column of a sprite up to 64 bytes a row (32 with a mask beside
each byte), mirrored and inverted — and **Show** puts any find in the viewer. A **Kempston mouse** or an **AMX mouse** — one or the
other, since both answer at $DF — fitted in the Hardware window, takes the
host's mouse when the screen is clicked: the pointer is
hidden and held, and its movement and both buttons go to the machine, in the
machine's pixels. **Esc** gives it back, and so does switching away from the
main window. A **ZX Printer** or an **Alphacom
32** — one or the other, since they share a port — adds a **Printer** window
the width of the tape window: the paper comes out of the bottom as each line
is printed and grows upwards, **Save PNG…** writes it out a pixel a dot, **Tear
off** starts again, and a switch at the top puts the printout on the ZX
Printer's silver paper or on faded thermal paper. `COPY`, `LPRINT` and `LLIST`
all print. Snapshots are `.szx`, `.sna` and `.z80`, and screens are `.scr`
both ways.

**Quick**, beside Record on the main window, keeps ten quicksaves in memory:
Save puts the machine in the slot shown on the button beside it, Load puts it
back, and the button drops down to pick another slot. F1 to F9 save to slots 1
to 9 and F10 to slot 0 from anywhere in the main window, and each selects its
slot. A quicksave is the whole machine — what is plugged in, where the tape had
got to — and it lasts until the emulator is closed.

Not emulated, and what other emulators have that this does not — saving to
tape, a gamepad, the printer, the other disk systems, the other machines — is
written down in [`docs/missing.md`](docs/missing.md), checked against the
source rather than remembered.

## Tests

```
cargo test --release
```

`tests/prefs.rs` checks the configuration directory for each platform's
convention (including `XDG_CONFIG_HOME` and the no-home case), that the file and
its directory are created on launch, that settings round-trip while hand-added
keys survive, that ROM scanning recognises images by size and prefers a name
that mentions the machine, and that opening a ROM or a tape remembers the right
directory without replacing ROMs already loaded.

`tests/zx81.rs` covers the ZX81: the mirroring of ROM and of 1K and 16K RAM, a
fetch above `$8000` drawing a character and running as a NOP in four T-states,
inverse video, the line counter picking the row, `HALT` passing through, the
sync and NMI ports, the interrupt from the refresh register, and — driven by a
display routine in the same shape as the ROM's — a whole row of 32 characters
landing contiguously as 256 pixels in the same place on every line.

`tests/race_the_beam.rs` checks that the beam splits the picture between the
two frames at the right place — including part-way along a single line — that
the older part is dimmed to exactly two thirds, that the border is split and
dimmed with it, and that moving the beam changes the picture with nothing
running.

`tests/border.rs` loads Border Break, waits for its border routine to start and
compares the rendered border against rows taken from the photograph of real
hardware, run-length encoded. It also checks that cropping the border shows
the identical picture with less around it, that alternating the border in a
tight loop produces stripes 38 and 62 pixels wide — the 19 and 31 T-states that
loop actually takes — and that a half-drawn frame keeps the rest of the previous
one.

`tests/halt2int.rs` boots a real 48K ROM, types `LOAD ""`, plays HALT2INT off
tape and reads the results back off the screen by matching character cells
against the ROM font, then compares them with the photographs of real hardware
in both timing modes. It also unit-tests the halt state's refresh address and
the return address pushed when an interrupt wakes it.

`tests/timing.rs` checks T-state counts for around 60 instructions against the
published timings, the interrupt/NMI acknowledge sequences, `EI`'s one
instruction interrupt shadow, and the contention behaviour at frame positions
where it should and should not apply. `tests/debug_features.rs` drives the demo
ROM and checks the heat maps, the fade, back-buffer detection, the slow-draw
allowance, breakpoints and the disassembler end to end.

`tests/sound_and_128k.rs` also covers the +2A/+3: four-ROM selection across both
paging ports, all four all-RAM configurations (including that `$0000` becomes
writable and stops being when ROM returns), bank-4-to-7 contention with the
`1,0,7,6,5,4,3,2` sequence, the stricter `$7FFD` decoding, the absent floating
bus, and that the real +3 ROM boots to its menu. It checks AY tone frequencies against `clock / (16 ×
period)`, the volume table, mixer routing, envelope ramps and holds, and
register masking; that the mixer emits exactly one sample period's worth of
audio per sample and that a 1 kHz beeper square wave comes out at 1 kHz; and, on
the 128K, `$7FFD` paging, the shadow screen, the paging lock, bank-following
contention, the AY ports, and that the real 128K ROM boots to its menu.
`tests/ui_menu.rs` drives the interface itself through `egui_kittest`: it clicks
the machine selector and every Machine menu entry and checks the model actually
changed, that a missing ROM reports a visible error instead of doing nothing,
that a switch leaves the new machine running, and that the toolbar's window
toggles and Reset work.

`tests/audio_quality.rs` guards the things that make sound go wrong: that a tape
tone keeps its shape (and pitch) even when the CPU never reads port $FE, that a
constant level decays to silence rather than sitting as DC, that muting ramps
instead of stepping, that an AY tone comes out at the frequency its registers
ask for with no sample-to-sample discontinuities, that the buffer pacing pushes
back in the right direction, and that the mixer clock never runs backwards.

`tests/ui_tape.rs` checks that a block's length adds up from its pilot, sync,
data and pause, that progress through it rises from nothing to everything
without going backwards, that a block making no sound reports none, that seeking
resets it, that a loaded tape stays stopped and emits no pulses
until Play, that the opt-in auto-play still works, and that the block list
scrolls the playing block into view when playback moves on but leaves the list
alone when following is off. `tests/ui_ram_map.rs` covers the read/write/execute
toggles and their fade sliders, that heat follows the bank rather than the
address (including that a paged-out bank keeps its history), that ROM pages are
tracked separately, the layout of the all-memory view for both a 128K and a 48K,
that each block reports the slot it is paged into, and what hovering returns for
a bank that is not paged in.

`tests/profiler.rs` checks time attribution against hand-counted T-states: two
leaf functions come out at exactly 30 and 50 T-states per call, a nested pair
splits 35 self / 85 inclusive correctly, interrupt handlers are counted once per
frame at 14 T-states each, a function that never returns still gets its time,
nothing accumulates while stopped, and the window's Start/Stop, run list and
bar clicks do what they should.

`tests/audio_device.rs` is a smoke test that opens the host audio device and
checks the callback drains samples; it skips itself when there is no device.

`tests/tape.rs` parses every tape in `tapes/`, checks the generated pulse trains
against the ROM's published timings (pilot, sync, bit lengths, turbo overrides,
loops and stop blocks), and then does the real thing twice over: it calls the
48K ROM's `LD-BYTES` routine at `$0556` and verifies the header it reads back
matches the tape byte for byte, and it types `LOAD ""` on the emulated keyboard
and loads a whole game from a real TZX. Tests that need `roms/48.rom` or
`tapes/` skip themselves when those are absent.

## What plugs into it

A **Hardware** window lists the peripherals and says how far each is emulated:
the Interface 1 with up to eight microdrives, the Multiface One, 128 and 3 with
their red button, the Fuller Audio Box and Cheetah's SpecDrum all do something;
the Currah µSpeech talks, with the SP0256-AL2 emulated as the chip it is —
a microsequencer driving a twelve-pole filter, not a bank of samples; the RAM
Music Machine is a switch with nothing behind it yet, and the window says so rather than looking as though they work. A **Microdrive** window puts `.mdr`
cartridges — loose or zipped — in the drives, read-only, writing to a copy, or
writing in place. With Sinclair's own 8K at `roms/if1.rom` the microdrives run:
`FORMAT`, `SAVE *`, `LOAD *` and `CAT` all work, and a test drives that whole
round trip from the emulated keyboard.
[`docs/peripherals.md`](docs/peripherals.md) says what is emulated and what is
not.

## Disks

The +3 has a working disk interface: a µPD765A controller and `.dsk` images,
read-only or writable. Inserting a writable disk asks first whether writes
should go to a copy, so a game cannot quietly rewrite the image it came from,
and **New disk…** makes a blank one formatted as the machine's own FORMAT
formats it. [`docs/disks.md`](docs/disks.md) says what is emulated and what is
not.

## Training a network to play

The **Training** window teaches a neural network to play whatever the machine
is running, with nothing to install: say what it may press — a joystick
interface, or particular keys — how much of the picture it sees, and where the
game keeps its score and its lives, and press Start. It learns by PPO on the
graphics card, sees the screen and nothing else, and is rewarded by a judge
that reads the score out of memory; **Find a number** finds where the score is
kept. The network is kept beside the tape, and **Let it play** hands it the
machine's controls. [`docs/training.md`](docs/training.md) says what each
setting does and what was measured.

## Driving it from a program

`cargo run --release --bin mcp` runs the emulator as an
[MCP](https://modelcontextprotocol.io) server over stdin and stdout: load a
tape, snapshot or recording, run the machine under control, read its registers
and memory, disassemble, watch which routine does what, and write labels and
comments against addresses. It is the emulator without a window, meant for a
language model taking a game apart. [`docs/mcp.md`](docs/mcp.md) lists the
tools and what each is for.

## Building a release

`packaging/macos-app.sh` builds `ZX-Rustrum.app` and a `.dmg` around it,
generating the `.icns` from `packaging/icon.png` with `sips` and `iconutil`.
It signs both with the Developer ID named by `MACOS_SIGN_IDENTITY`, under the
hardened runtime, and when an App Store Connect API key is given
(`MACOS_NOTARY_KEY`, `MACOS_NOTARY_KEY_ID`, `MACOS_NOTARY_ISSUER`) notarises
both and staples the ticket to each. Without them it builds an unsigned app,
which is fine for the machine it was built on.

The release workflow does this from five repository secrets, and will not
build a release without them:

| Secret | What it is |
| --- | --- |
| `MACOS_CERTIFICATE` | The Developer ID Application certificate and its key, exported from Keychain Access as a `.p12`, then base64-encoded |
| `MACOS_CERTIFICATE_PASSWORD` | The password the `.p12` was exported with |
| `MACOS_NOTARY_KEY` | The text of an App Store Connect API key's `.p8` file |
| `MACOS_NOTARY_KEY_ID` | That key's ID |
| `MACOS_NOTARY_ISSUER` | The issuer ID shown above the list of keys |

`.github/workflows/release.yml` runs the tests, `cargo fmt --check` and
`cargo clippy -D warnings`, then packages a `.dmg` for each of the two macOS
architectures, a `.tar.gz` for Linux and a `.zip` for Windows.

**What each push costs decides what it runs.** This is a private repository,
so Actions minutes are billed — one a minute on Linux, two on Windows, ten on
macOS. An ordinary push to main is tested on Linux and nothing else; a push
that only changes prose is not built at all; and the other two platforms and
the four packaging jobs run when there is something to release. A run that has
been overtaken by another push is cancelled. `workflow_dispatch` takes a
**full** switch for a complete build without a version bump.

The tests are built without link-time optimisation (`CARGO_PROFILE_RELEASE_LTO:
"false"`), which is thin LTO over seventy-odd binaries and most of the build:
cold, on this machine, `cargo test --release --no-run` takes 2,194 seconds of
CPU with it and 374 without. The packaged binaries keep the release profile as
it stands. `cargo fmt` and `cargo clippy` run on the Linux job alone, since
their answer does not change with the platform.

**Cutting a release is bumping `version` in `Cargo.toml`.** When a commit
lands on main with a version that has no `v<version>` tag yet, the workflow
tags that commit and publishes a release with the four builds attached, named
for the version — `zx-rustrum-0.2.0-linux-x86_64.tar.gz`,
`ZX-Rustrum-0.2.0-macos-arm64.dmg`, and so on. Pushing anything else to main
tests it and publishes nothing, so a version never means two different sets of
binaries. Pushing a `v*` tag by hand still releases, and the
tag is checked against `Cargo.toml` first: a tag that names a version the tree
does not stops the build rather than shipping binaries that report a version
they are not.

`packaging/make-icon.py` draws the icon — a `ZX` over the four Spectrum colour
bars — writing both `icon.png` and a multi-size `icon.ico`. It has no
dependencies, not even Pillow: it rasterises the shapes itself and writes the
PNG chunks by hand.

Things worth knowing per platform:

* **macOS** — the binary links only system frameworks, so there is nothing to
  bundle beyond the app itself.
* **Linux** — `cpal` links `libasound.so.2` (build with `libasound2-dev`), and
  the file dialogs go through `xdg-desktop-portal`. An AppImage or Flatpak
  carries both; a bare tarball expects the host to have them.
* **Windows** — the binary is built with `windows_subsystem = "windows"`, so no
  console window opens behind it. Cross-compiling from macOS is not worth the
  trouble; use the CI runner.

No ROM images are shipped: they are still under copyright. A packaged build
therefore starts with the machines disabled until ROMs are put where it looks —
see **ROMs and snapshots**.

## Layout

| Path | What |
| --- | --- |
| `src/z80/` | CPU: state, decode/execute, ALU and flag tables |
| `src/machine.rs` | Memory map, ULA timing, contention, I/O, run loop |
| `src/tracker.rs` | Access heat maps and back-buffer detection |
| `src/screen.rs` | Display rendering |
| `src/tape.rs` | TZX/TAP parsing and pulse-level playback |
| `src/prefs.rs` | Preferences file and its platform-appropriate location |
| `src/profiler.rs` | Call profiler: call/return detection and time attribution |
| `src/audio.rs` | AY-3-8912 and the beeper/AY mixer |
| `src/audio_out.rs` | cpal output device |
| `src/disasm.rs` | Disassembler |
| `src/ui/` | Main window, RAM map, debugger, back-buffer, tape and profiler windows |
| `src/bin/zextest.rs` | CP/M harness for zexdoc/zexall |
