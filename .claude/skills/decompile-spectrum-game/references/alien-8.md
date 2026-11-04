# Worked example: Alien 8 (Ultimate, 1985), from a recording and Knight Lore

`recordings/Alien8.rzx`, 342,604 frames, played through in about a minute with the
observer on: 8,834 bytes of code ran. The replay leaves the recorded path at frame
132,195, which was not noted at the time; what ran after it is the game's code played to the
wrong input. The disassembly is
`disassembly/Alien 8 (1985)(Ultimate Play The Game).asm`, $5B00-$FFFF, as the machine
stood at the recording's first frame.

- 4,360 instructions, 691 labels: every routine called and every place jumped to is
  named, and each of the 131 object types' handlers. 77 sprites pictured.
  `roundtrip.py`: 0 mismatches, 0 EQUs. No tape and no real assembler.
- About 300 names came from Knight Lore by `port_names.py`, the engine almost whole;
  27 line comments carried where the code matched up to them.
- What is Alien 8's own and read: the light-years countdown that ends the game, the
  valves and their sockets, the conveyors and their switches, lifts, patrols and
  chasers. About a dozen handlers are named only by their type number, marked as
  read but not matched to anything seen on the screen. Descriptions are thinner
  than Knight Lore's.

The same engine with the variables $A0 lower, 56 objects where Knight Lore has 40,
the buffer at $D200 and the stack at $F100.
