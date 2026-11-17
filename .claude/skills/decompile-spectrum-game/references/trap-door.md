# Worked example: Trap Door (Piranha, 1986), from a recording, with a picture language

`recordings/trapdoor.rzx`, 189,333 frames, played through in about seventy
seconds with the observer on, without coming adrift. The disassembly is
`disassembly/Trap Door, The (1986)(Piranha).asm`, `$72A0-$FFFF`. Everything that
made it is in the `notes` folder beside it. `replay.py`, then `force.py`, then
`export.py` make it again.

- 5,262 instructions, and 1,385 labels.
  - Every routine is described and every jump target named.
  - `roundtrip.py`: 0 mismatches, 0 EQUs.
  - `tools/z80asm.py` rebuilds all 36,192 bytes the same.
- 11,732 bytes ran (32.4%), all from the recording. Forcing the 69 branches that
  lead to unrun code reached 13 of them, and 137 more bytes of the game.
  - The states saved every 20,000 frames were tried first, and the menu last.
  - The other 56 were not reached within 600 frames of any state: left alone,
    the game waits on a player.
- 100 animation scripts decoded. All 1,115 bytes the machine read in their
  seven areas are covered.
- 345 picture programs decoded in seven areas, and 162 drawn. All 8,643 bytes
  the machine read there are covered.
- What each task's flags mean, read off the game's replies and hints.

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
- **Two languages, one inside the other.**
  - A thing's animation script is a list of words. A word under `$100` is a
    command, and any other word is the picture program to draw this pass.
  - A picture program is cells and commands. A cell is a character code, an
    attribute (unless a fixed one is set) and a step. The commands `$EE-$FF`
    repeat, call, jump, cycle, choose at random and move the origin.
  - Both interpreters dispatch through a pushed or rewritten address, so the
    call graph draws their commands as if the interpreter called them.

## What Decoding the Pictures Took

1. **Read each command's operands off its handler,** including what a cell
   skips when it is off the screen: it steps over its attribute without reading
   it.
2. **Find the starts:** the object records' `+$0A`, `LD BC/DE/HL` constants in
   the listing's instruction lines (scanning bytes found an `LD A,$01` operand and
   took it for `LD BC`), the scripts' frames, and words in the data the game read.
3. **Check against `code_map` for the area, both ways:** every byte read
   should be decoded, and every decoded byte read unless it was never shown. The
   map only listed its widest sixty runs, silently; that is fixed (it counts what
   it leaves out and takes a range), and the decoder went from "1,983 bytes
   missing" to 0.
4. **Words found in data are noise until proven.** A misaligned word in a
   script, or a table read at an odd address, decoded cleanly and landed inside
   real programs. A start is kept only if its cells and commands line up with
   every program it overlaps, both ways round, and a data start inside another
   program is dropped as unproven.
5. **Carry the fixed-attribute mode across jumps and calls.** A program jumped
   to after `$F2` has two-byte cells. Walked in the wrong mode it ran on through
   the next program.
6. **Draw them.** Berk's frames, the pans, the cauldron, the rooms' arches and
   every creature came out recognisable. That is the check that the step and
   attribute rules are read the right way round.

## What Decoding the Scripts Took

1. **Read every handler's exits.** Most commands come back to the script. Five
   start a script of their own, and one, `flyer_level`, moves the script on by
   two or four more words. The handlers that never ran were read from their
   bytes. One, `flyer_strikes`, had been taken from a note as never coming back,
   and it does.
2. **Find the starts:** `LD HL,nn` followed by `JP animate_from` or a store into
   a script variable, the variables' first values, three tables, and the places
   the scripts go on at. Four more needed reading by hand: `berk_caught` picks
   one of two by what Berk carries, and `walker_avoid` takes a `JR` first.
3. **Follow the frames.** They pointed at six picture areas beyond the one
   decoded first, all labelled as something else.
4. **Walk every word no script covers** in each area. That found four Berk
   scripts and a ghost script nothing starts, and the words after
   `flyer_strikes`.
5. **Check against what was read.** The bytes decoded and never read were
   explained: loop counts' high bytes, which the loop never reads, scripts that
   never ran, and words after a frame the game always replaced.

## What the Text Settled

Each task's reply prints a complaint for each bit of its flags that is clear,
so the replies define the bits: NO EGGS !, NOT COOKED !, NOT HOT !, NOT ENOUGH
EGGS. The hints test the records, and their words name what is in them: "GHOST !
GIVE IT A WORM", "GET IT TO LAY", "MOBILE HEATER", "CRUSHER WANTED". The drawings
agree with every one.

## Corrected Along the Way

- **The "worm" was the bird that lays task 2's eggs.** The worms are the
  walkers. Its description had said the name was a reading.
- **Three areas were not what they were labelled.** Two were picture programs.
  The third, `creature2_scripts`, was ten bytes of cooking code that cannot run.
- **The scripts are words.** Four descriptions called them byte-coded.
- **A stale label sat a byte into a script.** The round trip reported it as an
  EQU. Taking it out of the notes script had not taken it out of the notes
  file.
- **Forcing went wrong twice.** The first run tried the menu first and took
  "did not get to" for success. Its fix dropped the menu, which lost the two
  tune player branches only the menu reaches.
- **`$7305` is Berk's level, not his size.** Levels count up towards the front,
  so two tests named `_behind` were `_in_front`.
- **`events` bit 7 is "a creature is out of the trap door",** not "the creature
  has gone".
- **A step of `$20` moves down a row,** not to the start of the next row.
- **Bit 7 of a step makes a cell use its own PAPER;** the first docstring had it
  the other way round.
- **The decoder's own bug:** `$FF` given two bytes made every program swallow
  the next one's first byte. The alignment check found it.
