# Worked example: Alien 8 (Ultimate, 1985)

Rebuilt from `disassembly/Alien 8 (1985)(Ultimate Play The Game).asm` and
`recordings/Alien8.rzx` (342,604 frames). Everything made is in `assemblies/alien-8/`,
which git ignores: `README.md` there describes every build, and `memory-plan.md`
every banking step, with its measurements and its mistakes. The tests are
`tests/alien8_usage.rs`, `tests/alien8_free_ay.rs` and `tests/alien8_fast.rs`.

## The scripts, stage by stage

| Stage | Script | Makes |
|---|---|---|
| Reach, dead code out | `reach.py`, `relocate.py` (`build.sh`) | `Alien 8.asm` |
| AY in step with the beeper | `ay.py`, checked by `ay_check.py` | `Alien 8 AY.asm` |
| AY free | `ay_free.py` | `Alien 8 AY free.asm` |
| Cheaper drawing, paced | `fast.py`, `fast.py --paced` (`fast.sh`) | `Alien 8 AY fast.asm`, `... paced.asm` |
| Banks | `banked.py --step N` (`fast.sh`) | `Alien 8 AY banked.asm` |
| Snapshot | `snapshot.py --start --unlocked --banks` | `... start.szx` |
| Tape | `tap.py`, with `keep_the_original_loading_screen` | `Alien 8 AY banked.tap` |
| Busy rooms | `busy_states.py` | `out/busy45000.sna` and the others |

## What each stage was worth

Passes of the main loop in 1,000 frames (more is faster), from the same state:

| Build | First room | Busy 90,000 | Busy 50,000 | Saved slow room |
|---|---|---|---|---|
| AY free (walking, per 3,000 frames: 520) | | | | |
| Faster (fast.py) | 192 | 112 | 156 | 91 |
| Banked, Step 7 (mirrored copies) | | 134 | | |
| Banked, Step 13 | 186 | 146 | 180 | 132 |
| Step 14, depth test stops early (exact) | 190 | 156 | 192 | 145 |
| Step 15, sprites apart not tested | 190 | 160 | 187 | 149 |
| Step 16, rows clipped (exact on screen) | 192 | 186 | 198 | 180 |
| Step 17, columns too: not kept | 193 | 185 | 196 | 178 |

Paced to 4 frames a pass, the first room walked makes 214 and the quietest busy
room 250 (the cap).

## Where the time went, the busiest room, after Step 15

Drawing sprites 43-49%, the drawing order 24-26%, object handlers 8-12%,
marking what to redraw 5-8%, copying to the screen 5%, sound 1-5%, waiting on
contended memory 1.1-1.7%. Of the bytes drawn, only 35-56% were inside the
places cleared that pass, which is what Step 16 went after.

## Layout at the end

- Fixed: $5B00 variables and objects; stubs; the AY driver, its queues, the
  effects' blocks and `ay_int` at $6666; cold code; the valve list at $76E3 and
  the sprite table, fixed by the game.
- Bank 2 (uncontended): IM 2 table at $8000, hot code, the draw buffer at
  $9800, the stack's page at $B000 (with `depth_bounds` and the clipping table),
  the shift tables from $B100.
- Bank 0: sprites, the rectangle table at $EE00, the pre-shift directory at
  $F000, the tape loader's second stage at $F840. Bank 4 the same, mirrored.
- Bank 3: room data and every routine run only at a room change, a new life, the
  menu, the end of a game or a key press.
- Banks 1, 6 and 7: pre-shifted copies, full.
- Free at the end: about 530 bytes in fixed memory (190 uncontended), 9.9K in
  bank 3, 2.5K each in banks 0 and 4.

## The tape

123,053 bytes; 122,801 loaded, eleven minutes and twenty-two seconds at the ROM's
speed (frame 34,083). The original's loading screen comes from loading the
original (Speedlock 1) at the tape's speed: complete at frame 8,559, still for
5,682 frames.
