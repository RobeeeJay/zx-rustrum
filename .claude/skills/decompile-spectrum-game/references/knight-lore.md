# Worked example: Knight Lore (Ultimate, 1984), from a recording

No tape, only `recordings/KnightLore.rzx`: 262,259 frames of somebody playing to day
35, 88% of the quest and 14 charms collected. The disassembly is
`disassembly/Knight Lore (1984)(Ultimate Play The Game).asm`, $5B00–$FFFF, as the
machine stood at the recording's first frame.

- 4,159 instructions, 707 labels (every routine and all 360 jump targets named), 103
  sprites pictured from their picture bytes, 0 EQUs inside the range.
- `roundtrip.py`: 0 mismatches. No tape to compare with, and no real assembler.
- Coverage $5B00–$FFFF: 8,556 bytes run from the recording alone; 379 could be
  reached and were not, most of them the keyboard and rotational controls (the
  recording used a Kempston stick, directional) and the end of the game won.

## How it works, briefly

Forty 32-byte objects from $5C08 walked by IX each pass, dispatched by type through
188 words at $B096. Everything is drawn into a buffer at $D8F3 and only the changed
rectangles copied to the screen; objects are projected isometrically ($D6C9),
sorted so nothing is drawn after something in front of it ($CEBB), and drawn with
the stack pointer reading the sprite ($D718), with two of its instructions rewritten
for every sprite. Rooms are built from tables from $6248. The sun and moon cross a
window every eight passes; when night ends a day is counted, and day forty is the
end. Fourteen charms in the wizard's order make the potion.

## What cost time

- The observer was off after `load_recording`.
- `code_map` printed data blocks with their length as an end address (fixed).
- The sprite width was misread by one: the tiling check caught it.
- One label sat a byte inside an instruction, from a dump that ran data into code.
- A handler named from a guess at what it drew was something else entirely.
