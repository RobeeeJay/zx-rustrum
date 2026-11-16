---
name: rebuild-for-128k
description: Rebuild a disassembled 48K ZX Spectrum game as a faster 128K game with AY sound, using the 128K's spare banks, and make a .tap of it that loads through the ROM with the original's loading screen. Use whenever the task is to port, convert, speed up, optimise or "128K-ify" a 48K game, to replace its beeper with the AY, to move it into banks, to pace it for turbo, or to make a tape of a rebuilt game — even when it only says "do what we did to Alien 8" to another game. Needs a disassembly that reassembles (see decompile-spectrum-game) and a recording of the game being played.
---

# Rebuilding a 48K game for the 128K and its AY

The goal is the same game, made faster and given proper sound, on a machine with
eight times the memory: a build that plays exactly the game the original plays,
pass for pass, until a change says otherwise and why. Speed is measured, never
assumed, and every change is checked against a version of the game it must
match.

What makes it hard is not the Z80. It is that **a game is easy to change without
meaning to**: it stirs the R register into its random numbers, its timing decides
what a key press interrupts, and its memory is full of addresses that move when
anything moves. So the method is mostly about keeping a reference to compare
against at every stage, and taking it with you as the game moves away from the
recording.

Alien 8 went through every stage here; [references/alien-8.md](references/alien-8.md)
has its numbers, its scripts and what each stage was worth, as a worked example and a
check on what to expect. Its scripts are in `assemblies/` (gitignored, since
they carry the game's code) and work by swapping exact passages of its
disassembly, so **they are a pattern to copy for each game, not a tool to run on
another one**. A game with the same engine (Knight Lore shares about two thirds
of Alien 8's code) still has its own addresses, layout and quirks.

## What you need before starting

- **A disassembly that reassembles.** `disassembly/<game>.asm`, made with the
  decompile-spectrum-game skill, proven by `roundtrip.py` and by
  `tools/z80asm.py` giving back the recording's memory byte for byte.
- **A recording** (`recordings/<game>.rzx`) of somebody playing well into the
  game. It is the only witness of what the original does, it says which code
  ran, and it gives busy rooms to measure in. A first frame inside the game
  (not the menu) is easiest.
- **The original tape**, for its loading screen at the end. Ask the user for it
  rather than fetching it.
- **`roms/128.rom` and `roms/48.rom`.**

Work in `assemblies/`, a script per stage beside what it makes, one shell script
(`build.sh`, then `fast.sh`) that remakes everything from the disassembly, and a
test file per game (`tests/<game>_*.rs`) that skips itself when its inputs are
absent. Write up each stage in a plan beside the scripts (Alien 8's is
`assemblies/memory-plan.md`): what it changed, what it measured, what it cost,
and what was wrong on the way.

## How every stage is checked

These are the habits that caught nearly every mistake. Set them up first.

- **Test variants without R.** Build each stage twice more with `LD A,R` made
  `LD A,$00` (`--fixed-random`). Two builds that do the same thing then play the
  same game, and can be compared byte for byte.
- **Pass for pass.** Find the game's main loop and the end of one pass of it
  (`end_of_pass`). Feed both builds the same keys on the same pass number, and
  compare the screen and all the game's state at the end of every pass:
  `comparable()` takes $4000 to the last object, with anything that only points
  into moved code (a draw list's position, the room's data) made an offset, and
  anything written before it is read each pass blanked, with the reason in a
  comment.
- **Leave the stack out of what is compared** if it sits inside it (Knight
  Lore's is the printer buffer below $5BA0): return addresses left below SP
  differ as soon as code moves. Check no instruction names an address there
  first, and measure how deep it goes.
- **Compare from the same pass, not the same frame.** A faster build has been
  round more often by any frame. Count passes; start the comparison from a fixed
  pass number, and make sure setting up a game stops **before** it. Alien 8's
  harness ran a fixed thousand frames and a faster build was past the first
  compared pass by then.
- **Take state at the end of a pass, before it draws.** The picture you compare
  at pass n was drawn from the objects as they stood at pass n − 1.
- **Busy rooms from the recording.** Walking about never leaves the first room.
  Take the game's state from the recording at a few frames in busy rooms
  (screen, variables, objects, the draw buffer), carry only that into each build
  at `end_of_pass`, and play on. Fit the joystick the recording used: with none
  fitted the game reads the floating bus, whose value depends on timing.
- **Rooms the user saves.** When a build is seen to slow down, the user can save
  the room; turn it into the same kind of state (`keep_the_room_...`) and add it
  to the pass-for-pass test.
- **A fuzz test for anything with cases.** Random rectangles for a copy
  routine, random pairs for a depth test, random rooms of objects for a marking
  routine: each compared against the old routine, with the trials that take the
  rare paths counted and required.
- **Every test checked against the bug.** Put the fault in, watch the test fail
  with a message that says why, take it out. Do it for every change; a test that
  has never failed has not been shown to test anything.
- **Measure speed as passes in 1,000 frames**, from the same state in each build,
  in the first room walked and in each busy room. Profile by charging each
  instruction's T-states to the label it ran in, bucketed by job (drawing,
  ordering, marking, copying, sound, waiting). `SpectrumBus::stalls` counts
  contention by page. Profile before choosing what to change, and again after.
- **Report negative results.** A change measured slower is written up and not
  kept (built behind a step number, not the default). Several were.

## The stages

In order. Each produces a build that the next starts from, and a snapshot to
play (`snapshot.py --start`). Commit the tests after each; `assemblies/` itself
is not committed.

### 1. Dead code out, and an export that can move

`reach.py`: what ran in the recording and what can be reached from it.
`relocate.py`: remove what neither ran, is reached nor is read (read each piece
by hand first), turn reachable DEFB back into instructions, every address into
the code area into a label, address tables into DEFW. After this, code can
move. Check: the build plays the recording's input to the end with the same
screens.

**Watch for** anything addressed by page (`LD H,$D2`, `OR $F0`): tables and
buffers found by their high byte can't move with a label, and need their pages
written in as `name/256` when they do.

**Prove it can move** with a test variant that moves every byte of code by an
odd amount (16, not 256, so page-addressed tables show too) and plays the
recording. Two limits, both found on Knight Lore:

- **The random number may read memory.** Ultimate's end of pass adds the byte at
  the address its pass count makes, so the game reads its own memory a byte a
  pass, and once the count reaches a byte the move changed (code, or a variable
  holding a pointer into it) the game goes another way. Find that frame by
  recording the byte the count reads in both builds, and hold the variant to the
  recording only until then.
- **The recording only checks what it runs.** A stale address in code the
  recording never reaches (the menu, drawn before it starts) changes nothing it
  sees. Check a mutation in code that runs every pass fails the test, and say
  which code the recording does not cover.

A game may also have **no dead code** (Knight Lore had none): read every
unreached stretch before deciding, and say so.

### 2. The AY, in step with the beeper

`ay.py`: the beeper's `OUT ($FE)` loops become AY writes. The recording counts
opcode fetches between key reads, so the AY build must make **exactly** the
fetches the beeper build made: each routine that writes the AY gives back what
the writes cost out of the wait that follows. `ay_check.py` runs every sound
and note in both builds and compares R and every register. Also take out any
`OUT ($FD)` the keyboard read makes. It is not only the AY's port: with the key
row as the port's high byte, a row with bit 7 clear ($7EFD, $00FD) is the 128K's
**paging port**, and $7E pages bank 6 over the game's code and locks the paging.
Knight Lore, started on a 128K, made one pass and ran off the end of memory for
it. Make it a NOP (one fetch, the same as the OUT) in **every** 128K build from
the first, test variants included, and test that $7FFD is unchanged after a
game. Check: the recording still plays.

**If the 48K game is full**, there is nowhere for the driver: measure the
unused memory over the whole recording first (zero at the start, never read,
written or run). Knight Lore's longest such run was 16 bytes. Then compare the
game's sound routines with a game whose stage 2 was done; if they are the same
instruction for instruction, skip this stage and say why, make room first
(stage 6's room bank, checked pass for pass against stage 1), and put the AY
in free of the beeper from the start.

### 3. The AY, free of the beeper's waiting

`ay_free.py`: sounds go into a queue and return; an IM 2 interrupt takes a
frame's worth off the queue fifty times a second and sets the AY; the
envelope fades each sound. **The ROM's IM 1 handler can't be used**: it writes
its system variables, which a game like this keeps its objects on top of. The
recording can't follow this build; check the tunes note for note against the
beeper build instead, with the audio analysed (`tests/alien8_free_ay.rs`).

**The interrupt's table** needs a whole page of RAM at $80 or above. A page of
$FF in ROM serves on a 48K (pages $39-$3B), but **the 128K's ROM 1 has no page
of one value**. If the game has filled $8000 up, move a few of its last sprites
(data the sprite table points at) into freed contended memory and put the table
where they were, filled with a byte V whose address VV is also free, for the
handler's jump; then test every sprite type draws the same in both builds.

**Traps:** one fall of the envelope takes 256 clocks per step of its period,
not 4,096. A sprite drawer that reads with POP and SP inside the sprite must not
take an interrupt there unless the handler is written for it (stage 4). The
game runs faster while sound plays, since the sound's time is now the game's.

### 4. Cheaper drawing, measured

`fast.py`: profile, then make the hottest routines cheaper without changing what
they do. In Alien 8's engine that was the depth test's pair loop, the copy to
the screen, the marking of what to redraw, and the sound queue's running total.

**Interrupts inside sprites.** Turning interrupts off for every sprite lost 30%
of them. The handler instead switches to a stack of its own at once, takes the
return address the CPU pushed into the sprite, writes back DE (the two bytes the
last POP read, which nothing touches between POPs) and returns through a jump it
writes itself. Before a sprite's first POP, DE holds something else, so
interrupts come back on at the end of its first row.

**Profile this game before carrying another game's changes over.** On Knight
Lore, the same engine, Alien 8's copy, marking and interrupt changes went in
cleanly and bought no speed at all in the busy rooms (209 passes against 209):
the copy saved 1%, the mark scan saves only on empty slots and those rooms had
38 to 40 live ones, and taking interrupts inside sprites costs about what the
copy saves. Alien 8's gain had been mostly the pair loop, whose code differs in
Knight Lore. Keep the stage for the interrupts (701 to 965 of 1,000 frames)
and write the speed test as "no slower" if that is what was measured. When
profiling by nearest label in a banked build, leave out every label of the
banked sections, not only the renamed ones: routines moved into a bank keep
their internal labels, and they share addresses with fixed code.

### 5. Paced

`fast.py --paced`: the busy-wait that was the game's only pacing becomes a wait
in HALT until `PACE_FRAMES` frames have gone by, counted by the interrupt. The
original's quickest pass was 5.2 frames; Alien 8 used 5, then 4 once the banked
build's busy rooms were being held as well. The user decides the number; offer
`--pace=N` to try one. This is the build for the emulator's CPU turbo.

### 6. The 128K's banks

Write the plan first (Alien 8's `memory-plan.md`), with a profile of what is used
when: what runs every pass, what only when a room is built, what only at the
start or end of a game. Then `banked.py`, a step at a time, each behind
`--step N`:

- **Tools first.** `tools/z80asm.py` takes `BANK n` and writes each bank to its
  own file with `bankN` beside its labels in the map; `snapshot.py --banks`
  puts them in and `--unlocked` leaves the paging free. Snapshots that are
  locked break any game that pages.
- **What is read only to build a room** (room data and its builders) goes to a
  bank, called through stubs that page it in and out (`bank3_call`), and moved
  code reaches back through thunks (`bank0_call`). Data stays in fixed memory;
  refuse moved code that reads memory that will be paged out.
- **The interrupt's table, handler and stack** must not be in a bank that gets
  paged out. **I must never point into $40-$7F** (or a contended bank at
  $C000): the refresh address snows, slows every instruction in this emulator,
  and is known to crash a real 128K. The IM 2 table is 257 bytes of one value,
  since the floating bus can put anything on it.
- **Uncontended memory is banks 0, 2, 4 and 6.** Put the hottest code, the draw
  buffer, the tables and the stack in bank 2 ($8000-$BFFF), **by a measured
  profile, not call counts** (call counts put a fifth of a busy room's time in
  contended memory). Sprites go to bank 0 at $C000.
- **Make once what the game makes every pass.** Alien 8 mirrored sprites in
  place for every object facing the other way; a mirrored copy of every sprite
  in bank 4 removed that, and pre-shifted copies of the sprites drawn shifted
  most (counted over the recording) filled banks 1, 6 and 7.
- **Then the algorithms**, each measured, each its own step: copy the changed
  rectangles a character cell at a time; let the drawing order step back after
  each draw instead of starting again; stop the depth test at the first axis
  that settles it (exact); clip each sprite to the rows that will be copied to
  the screen (exact for the screen, since the buffer outside those rows is never
  shown — check every full-buffer copy comes after a clear).
- **Not exact, and the user's call:** skipping the depth test for sprites apart
  on the screen changed which way round some unordered pairs were drawn (a few
  bytes in 5 passes of 300), and a valve's passing check depended on which pairs
  were tested. Ask before keeping anything that changes the picture, and give
  the test an explicit rule for what may differ.
- **Tried and slower:** cutting columns as well as rows (the POP-driven loop can
  only skip by reading); remembering pair tests. Write these up.
- **Sound on the 128K's other channels.** Tunes on A; the player's sounds on B,
  everything else's on C, each with its own small queue, a set volume with an
  attack and a release instead of the envelope. Check the volumes a frame at a
  time through the AY's registers, and say they haven't been listened to.

**Memory runs out.** Keep a running account of what is free where. When a step
doesn't fit, move code that runs only at a room change, a new life or a key
press to the room bank (stubs make it cost a trampoline a call), and find the
largest free run (the tail of a sprite bank was where the last table went).

### 7. A tape that loads through the ROM

`tap.py`: a BASIC program that starts itself, sets paper and ink black (so the
"Bytes:" a header prints can't be seen), loads the loading screen with
`LOAD ""SCREEN$`, and calls machine code in its first line, a REM. Then the
banks and the main image, headerless, read by the ROM's own `LD-BYTES` at
$0556 with ROM 1 paged.

- **Two stages.** The last block overwrites everything from $5B00, BASIC and its
  system variables included, and the bank at $C000 changes under each bank's
  load. Stage 1 copies itself out of BASIC (RANDOMIZE USR leaves its address in
  BC) and loads the banks; stage 2 lives in the free tail of the last bank
  loaded, loads the main image, silences the AY and starts the game exactly as
  the start snapshot does.
- **Each stage has its own IM 2 table and an EI/RET handler.** LD-BYTES turns
  interrupts on as it returns, and the ROM's handler would write into the game's
  variables.
- **Load each bank only as far as the assembler wrote** (run the assembler and
  read its banks' addresses; trailing zeros may be the game's).
- **It needs a 128K's Tape Loader.** 48 BASIC locks the paging.
- **The loading screen** is a standard block only if the original's loader is
  the ROM's. A protected loader (Speedlock and the like) has to be run: load the
  original on a 48K at the tape's speed, and keep the screen that stood still
  while its last blocks went in, as it was the frame before the game first drew
  over it — after the screen's own blocks, or you get the BASIC screen.
- **Check** by loading the tape on a 128K through the Tape Loader, once with
  fastload and once at the ROM's speed, stopping at the game's start: the main
  image, every bank as far as the game uses it, SP, $7FFD and a silent AY must
  match the start snapshot, the screen must be the loading screen, and the same
  menu must be drawn 400 frames later.

## What cost time

- **Numbers typed instead of taken.** `$A500` for a length that was `$6500`;
  take lengths and addresses from the files and maps.
- **A harness that measured the wrong thing**: comparing builds at the same frame
  instead of pass, judging a picture by the objects after it was drawn, a set-up
  that ran past the first compared pass, a build-a-room test that compared a
  buffer that may now differ.
- **The emulator, not the game.** Fastload handed two headerless blocks asked for
  back to back as the same block; a paused window repainted at full rate. Fix
  the emulator with a test of its own, then carry on.
- **Relative jumps out of range** whenever code is inserted between a JR and its
  target: make the swap use JP, or put the new code where nothing jumps across.
- **A register the next routine needs**, clobbered by code dropped in front of
  it (the shifted sprite loop keeps its table's page in H).
- **CI's clippy is newer than the local one**; `needless_range_loop` and the like
  appear only there. A map written in text mode on Windows ends lines with
  `\r\n`.

## Reporting

Say per stage what was measured (passes in 1,000 frames, from which state),
what was tested and how each test was shown to fail, what was not checked (real
hardware, listening), and what the user has to decide. Commit locally; push only
with a version bump the user asks for.
