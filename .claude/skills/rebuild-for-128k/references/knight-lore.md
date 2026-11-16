# Worked example: Knight Lore (Ultimate, 1984)

The same engine as Alien 8, and none of its free memory: the longest unused run
in the 48K game was 16 bytes. Rebuilt from
`disassembly/Knight Lore (1984)(Ultimate Play The Game).asm` and
`recordings/KnightLore.rzx` (262,259 frames). Everything made is in
`assemblies/knight-lore/`, which git ignores: `plan.md` there has every stage
with its measurements and its mistakes, and `build.sh` remakes all of it in
under a minute (47 seconds, most of it replaying the recording). The tests are `tests/knightlore_usage.rs` and
`tests/knightlore_fast.rs`.

## The scripts, in the order build.sh runs them

| Script | Makes |
|---|---|
| Alien 8's `reach.py`, then `relocate.py` | `Knight Lore.asm`, movable, no dead code |
| `banked.py` | the room data and its builder in bank 3 |
| `ay_free.py` (imports Alien 8's) | the free-running AY |
| `fast.py` (imports Alien 8's) | copy, mark scan, running total, interrupts inside sprites |
| `drawing.py` | the pair test stopping early; only the rows that show |
| `effects.py` (Alien 8's `step13`, lifted out with `ast`) | effects on channels B and C |
| `memory.py` | the exchange, mirrored copies, pre-shifted copies, the drawing order going back |
| `paced.py` | four frames a pass |
| `tap.py` (imports Alien 8's) | `Knight Lore 128.tap` |

## What each step was worth

Passes in 1,000 frames standing still, keys left alone (more is faster):

| Build | Busy 9,000 | Busy 25,000 | Busy 91,000 | First room |
|---|---|---|---|---|
| AY free | 209 | 179 | 207 | |
| Faster (fast.py) | 209 | 180 | 207 | |
| Pair test stops early | 228 | 190 | 215 | |
| Only the rows that show | 240 | 195 | 229 | 439 |
| Effects on their own channels | 238 | 194 | 227 | 437 |
| Exchanged and mirrored | 239 | 194 | 260 | 437 |
| Pre-shifted copies | 244 | 198 | 262 | 444 |
| Drawing order goes back | 265 | 203 | 274 | 444 |
| Paced, for play | 248 | 203 | 248 | 251 |

The room the played build was saved slowing down in: 179 before the drawing
order went back, 193 after, 188 paced (174 in the build it was saved from).
The AY freed from the beeper made 4.5% more passes walking (276 against 264 in
1,500 frames); Alien 8 had 12%.

## What was different from Alien 8

- **No room for stage 2** (the AY in step with the beeper): skipped, with the
  reason written down. The room bank came first, since its 3.5K was all there
  was.
- **The keyboard is read with `OUT ($FD),A`**, which a 128K takes as the paging
  port: the original crashes there. A NOP in every 128K build.
- **No ROM page for the IM 2 table.** The 48K ROM has three pages of $FF; the
  128K's ROM 1 has none of one value. The last three sprites moved and the table
  went in their place.
- **fast.py bought no speed** (209 against 209). Alien 8's gain had been the pair
  loop, whose code is not Knight Lore's; the mark scan saves only on empty
  slots; taking interrupts inside sprites cost about what the copy saved. Kept
  for the interrupts: 701 of 1,000 frames became 965.
- **Mirroring needed the exchange.** The sprites were in memory that can't be
  paged and the drawer and its buffer at $C000, so everything at $C000 came down
  into the 15.7K the sprites left, and the sprites went up into bank 0.
- **Buffer addresses written as numbers**, five of them, taken for print
  positions: the status line came out wrong until they were `buffer+n`.
- **The pre-shift directory is two levels** (a byte by type to a slot of words by
  shift): banks 0 and 4 had 779 bytes free, too few for one by type and shift.
  The copies were worth 1.3-1.6%, as on Alien 8.
- **The interrupt's stack** was 16 bytes until the effects went under it at 18.
- **The tape's second stage** enters LD-BYTES at $0562 so as to need no
  interrupt table, which freed bank 6 for copies, and made the emulator's
  Fastload learn that entry.

## Layout at the end

- Fixed, contended: variables and objects at $5B00; the AY driver, its queues,
  the effects and the interrupt's 48-byte stack; the cooler code from $728A.
- Fixed, uncontended: hot code at $8000, the shift tables $8200-$8FFF, the
  buffer at $9000, the hottest code round the interrupt's jump at $ADAD and its
  table at $AE00, to $C000.
- Bank 0: the sprites, then a pre-shift directory at $FD00. Bank 4 the same,
  mirrored. Bank 3: the rooms and their builder (and the tape's stage 2 at
  $F000 while loading). Banks 6, 1 and 7: pre-shifted copies, full.
- Free at the end: a few bytes in each region of fixed memory, plus room kept
  for pacing. Anything more needs cold code moved into bank 3 first, which has
  not been done.
