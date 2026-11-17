# Worked example: Trap Door (Piranha, 1986), from a recording, with a picture language

`recordings/trapdoor.rzx`, 189,333 frames, played through in about seventy
seconds with the observer on, without coming adrift. The disassembly is
`disassembly/Trap Door, The (1986)(Piranha).asm`, `$72A0-$FFFF`. Everything that
made it is in the `notes` folder beside it, and `replay.py` then `export.py`
make it again.

- 5,262 instructions; every routine described and every jump target named.
  `roundtrip.py`: 0 mismatches, 0 EQUs. `tools/z80asm.py` rebuilds all 36,192
  bytes the same.
- 11,732 bytes ran (32.4%), all from the recording. Forcing the 69 branches that
  lead to unrun code, from states taken every 20,000 frames, reached 13 of them
  and 137 more bytes. The other 56 were not reached within 600 frames of any
  state: left alone, the game waits on a player. Started from the menu, a fork
  reaches nothing, since the menu waits for a key; the first run tried it
  first, and stopped there because the script did not recognise the tool's
  "did not get to".
- 215 picture programs decoded and drawn, covering every one of the 6,146
  bytes the machine read in `$9208-$AAAA`.

## What Was Different

- **Interrupts are off for the whole game.** Nothing paces it: 18,997 passes in
  189,333 frames, 9.97 frames a pass, measured.
- **The tape cannot be compared.** Block 5 is not anywhere in memory as it is
  on the tape; the loader in block 3 decodes it. The comparison was with the
  machine saved the moment loading ended. The game's first code also copies its
  sound code from `$6F04`, under a picture buffer, up to `$FC6C`; the copy left
  behind showed four tune variables as loaded, before the menu tune changed them.
- **Text after a `CALL`.** `print_inline` pops its return address, prints to a
  terminator and returns past the text. `coverage` counts those 250 bytes as
  code that could be reached.
- **A text claim over a whole region swallowed code.** Three
  `LD HL,text : JP print_text` stubs sat among the strings, and had never run, so
  `set_data` could not warn. The texts are typed string by string from the bytes,
  cut at every label.
- **The pictures are a language.** A cell is a character code, an attribute
  (unless a fixed one is set) and a step; commands `$EE-$FF` repeat, call, jump,
  cycle, choose at random and move the origin. The interpreter rewrites a `JP`
  to dispatch, so the call graph draws the commands as if `draw_objects` called
  them.

## What Decoding the Pictures Took

1. **Read each command's operands off its handler,** including what a cell
   skips when it is off the screen: it steps over its attribute without reading
   it, which is why 3 decoded bytes were never read.
2. **Find the starts:** the object records' `+$0A`, `LD BC/DE/HL` constants in
   the listing's instruction lines (scanning bytes found an `LD A,$01` operand and
   took it for `LD BC`), and words in the data the game read, which is where the
   animation scripts are.
3. **Check against `code_map` for the region, both ways:** every byte read
   should be decoded, and every decoded byte read unless it was never shown. The
   map only listed its widest sixty runs, silently; that is fixed (it counts what
   it leaves out and takes a range), and the decoder went from "1,983 bytes
   missing" to 0.
4. **Words found in data are noise until proven.** A misaligned word in a byte
   script, or a table read at an odd address, decoded cleanly and landed inside
   real programs. A start is kept only if its cells and commands line up with
   every program it overlaps, both ways round, and a data start inside another
   program is dropped as unproven.
5. **Carry the fixed-attribute mode across jumps and calls.** A program jumped
   to after `$F2` has two-byte cells. Walked in the wrong mode it ran on through
   the next program.
6. **Draw them.** Berk's frames, the pans, the cauldron and the rooms' arches
   came out recognisable, which is the check that the step and attribute rules
   are read the right way round.

## Corrected Along the Way

- **`$7305` is Berk's level, not his size;** levels count up towards the front,
  so two tests named `_behind` were `_in_front`.
- **`events` bit 7 is "a creature is out of the trap door",** not "the creature
  has gone".
- **A step of `$20` moves down a row,** not to the start of the next row.
- **Bit 7 of a step makes a cell use its own PAPER;** the first docstring had it
  the other way round.
- **The decoder's own bug:** `$FF` given two bytes made every program swallow
  the next one's first byte. The alignment check found it.
