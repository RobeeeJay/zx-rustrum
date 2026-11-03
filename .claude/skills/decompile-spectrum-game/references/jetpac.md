# Worked example: Jetpac (Ultimate, 1983, 16K)

What doing this on one real game produced, as a check on scale and on what to
expect. The game's own files are in `tapes/` and not in the repository.

## What came out

- `tapes/Jetpac (1983)(Ultimate Play The Game)[16K].asm`: $5B80–$7FFF, 2,620
  instructions, 470 labels (every routine and all 233 jump targets named), 50
  sprites drawn as pictures, the measured lines, and 0 EQUs inside the range.
- `scripts/roundtrip.py` against the export state: 0 mismatches. `tape_blocks.py`:
  $6000–$7FFB equals the tape's 8K block less its first four bytes, and the stub at
  $5B80 equals its block. Not assembled with a real assembler, because none was
  installed.
- Coverage of $6000–$7FFF: about 65% run, of which 73 bytes only under
  force_branch. The rest is data (sprites, tables, text) and about 220 bytes that
  could be reached and never were, some of them the overwritten bootstrap at $6000
  decoded as the nonsense it now is.

## The shape of the game

- **Loading.** BASIC runs four bytes at $6000 that jump to a 15-byte stub at $5B80,
  which moves the 8K down by four bytes and enters at $61E5. The entry checks
  FRAMES' high byte is $83, which only the tape's last block sets.
- **The loop.** Thirty 8-byte objects at $5D00 (player, four lasers, a sound, the
  rocket and the two things falling towards it, six enemies, explosions). Type
  picks a handler from an 18-word table at $6374, through a `JP (HL)` loaded from
  tape at $5CB0.
- **Timing.** The loop runs flat out. When FRAMES changes, the loop patches its own
  end marker and exit (`$69B5`, `$69D3`) so it runs just the player, lasers and
  sound, then patches them back: those run once a frame, and everything else gets
  what is left. An empty slot burns about 5,000 T-states so a quiet screen is not
  much faster than a busy one. `self_modifying` found the patches by itself.
- **Enemies.** Six behaviours over eight levels, each with its own sprite: the
  sprite belongs to the level, not the handler, so the same handler is a meteor on
  level 0 and a fighter on level 6.
- **Sprites** are drawn up the screen a row at a time, erasing the old position and
  drawing the new in step, which is why they are stored upside down.

## What went wrong, and the fix

- **Coverage was stuck near 45%.** Random play does not build a rocket. Poking the
  rocket finished and the player onto it flew through the levels.
- **The loader looked like data.** The observer was switched on after the tape had
  loaded and run the stub; watch first.
- **Pictures upside down.** Found by reading the FUEL sprite; hence `bottom_up`.
- **A table 8 words too long** typed over the start of the next routine. The bytes
  still matched, so only an unexpected EQU gave it away; `set_data` now warns.
- **Descriptions overwritten** by one-line comments on the same address.
- **Blank and junk pictures.** The pre-shifted sprite workspace at $5E00–$5FFF was
  typed as bitmaps while looking at a game in progress, where it held the level's
  enemy. Exported as loaded it holds nothing yet, and eight pictures came out blank
  or noise. It is now bytes, described as workspace.
- **The 128K "autoload bug"** was the game crashing: the 128K ROM decrements $5B89
  every other frame, the first byte of the stub's LDIR. It runs from 48 BASIC.
- **Two bytes differed from the tape** at the menu: the menu sets the flash bit on
  the chosen options' colours. They were put back before export and the notes say so.
- **Names written in a hurry** were wrong in a handful of places (a jet's "drift
  up" that goes down, a "1UP" that is "whichever player is playing"). Reading the
  instruction at each target before naming it caught them.
