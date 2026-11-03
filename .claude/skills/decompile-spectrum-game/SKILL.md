---
name: decompile-spectrum-game
description: Disassemble and document a ZX Spectrum game with the ZX-Rustrum MCP server, to an annotated .asm that reassembles into the same bytes. Use whenever the task is to disassemble, decompile, reverse-engineer, annotate or "work out how it works" for a Spectrum tape, snapshot or game, to make a listing or export_asm of one, to name its routines or graphics, or to carry on a disassembly started earlier — even when it only says "look at this game" or names a .tzx, .tap, .sna or .z80 file in tapes/.
---

# Decompiling a ZX Spectrum game

The goal is a single `.asm` file that a reader can learn the game from and an
assembler can turn back into the game. Every routine named and described, every
jump target named, data shown as what it is (tables, text, pictures), and the
file proven to rebuild the bytes it came from.

What makes this hard is not reading Z80. It is that **code is only known to be
code once it has run**, so most of the work is getting the machine to run the
parts nobody has played yet, and keeping what was measured apart from what was
guessed. The steps below are in the order that worked on Jetpac
([references/jetpac.md](references/jetpac.md) has that game's results, as a
worked example and a check on what "done" looks like).

## Driving the server

The MCP server is the emulator without a window. Its state (the machine, saved
states, the observer's record of what ran) lives in the process, so keep one
running for the whole job:

```sh
sh .claude/skills/decompile-spectrum-game/scripts/mcp_server.sh start
python3 .claude/skills/decompile-spectrum-game/scripts/mcp_call.py TOOL '{"json":"args"}'
```

From Python, `import mcp_call` and use `call()`: a script that pokes the game
through a level makes thousands of calls, and a process each is slow. Restart
the server after rebuilding it; its states go with it, so save anything you need
to a `.sna` with `save_state {"path": ...}` first. `tools/list` (with `--raw`)
says what every tool takes; `docs/mcp.md` says what each one is for.

Rebuild with `cargo build --release` and relaunch the app after any change to the
emulator itself, as the project's CLAUDE.md asks.

## 1. Load it, watching from the start

1. `watch_routines` **before** `load_tape`. The loader runs during the load, and
   anything that ran before watching began is invisible: on Jetpac the loader stub
   came out as bytes because of this.
2. `load_tape`. On a 128K, +2A or +3, many 48K games crash from the menu's Tape
   Loader, because 128 BASIC keeps using memory they load into. Pass `basic48: true`
   (it goes to 48 BASIC first and checks the paging locked). If the reply says PC
   is in the ROM, look at the `screen` before believing the game is running.
3. `save_state {"name": "menu"}` straight away: this is the game as loaded, before
   play has patched anything, and it is what gets exported at the end.
4. Start a game and `save_state {"name": "game-start"}`. Save more along the way
   (one per level, say): forcing a branch is only practical from a state near it.

## 2. Find the shape before reading any code

- `tape_blocks` and `scripts/tape_blocks.py TAPE` show what loads where. Small
  blocks are worth reading first: Jetpac's 1-byte block was the dispatcher's
  `JP (HL)` and its 2-byte block a check value for FRAMES.
- `routines`, `call_graph`, `memory_activity` give the main loop and the busy code.
- `indirect_jumps` finds jump tables from the `JP (HL)`s that were seen, and the
  entries not yet taken. A table of handlers is usually the spine of the game.
- Look for a table of fixed-size records walked by IX or IY. If there is one,
  `define_struct`, `struct_array` and `bind_struct` (narrow ranges per handler,
  since one handler's +5 is another's countdown) before writing comments, so the
  listing says `(IX+speed_y)` and `rocket.fuel` from then on.

## 3. Reach the code

`coverage` is the measure of progress: what has run, what could be reached from
it, what has only been read, and every conditional that has only gone one way.
Work it down:

- **Play.** Random key chords for a few hundred presses get the menu and movement.
- **Poke the game into the situation.** Most game logic waits on a situation, not
  a key: an item touching the player, a rocket full, a level number. Put the player
  on the item, set the level, keep the lives topped up. This reached far more than
  play did. Note what was poked, because it is a machine the game did not make.
- **`force_branch`** for what is left, from a saved state near it. It runs the
  other side on a copy and keeps only which bytes ran, marked `[forced]`; the branch
  stays reported as one-sided. Only one flag is changed, so treat what it reaches as
  "the code that is there", never as "what the game does".
- Name enemies and objects from what is on the `screen`, and say that is how they
  were named.

Expect a residue that never runs. Say what it is and why it was not reached.

## 4. Say what the data is

`set_data` turns bytes into what they are, for both the listing and the export:

- **words** for address tables: they then read as names.
- **text**: find the terminator (often bit 7 on the last character). A leading
  colour byte is not text; type it separately or it reads as a letter.
- **bitmap** with `width`, `rows`, `header` from the game's own sprite format
  (read it off the drawing routine). The picture is drawn above the rows at two
  characters a pixel. If the drawing routine works **up** the screen (Jetpac's
  does), the sprites are stored upside down: pass `bottom_up: true`. Check one
  picture against the screen before typing fifty.

Type what the file will hold, not what memory held when you looked. The export is
the game as loaded, so a buffer the game fills at run time (pre-shifted sprites, a
decompressed level, a back buffer) is empty or leftover junk there: typed as a
bitmap it comes out as blank or nonsense pictures. Describe it as workspace, name
the routine that fills it, and leave its bytes as bytes. Picture the art where it
is actually stored.

`set_data` warns when a claim covers code that has run. Take the warning: a table
typed eight words too long swallowed the start of a routine and nothing else
noticed, because the bytes still came out right.

## 5. Explain it

Use `self_modifying` early. Code the program patches is code whose listing shows one
moment's bytes; say so at both the patch and the patched instruction. `watch_value`
and `value_log` answer "what changes the lives and when" without stopping the
machine, and are the way to check a claim about a variable before writing it down.

Then write, using `set_comment` (a label, a comment, or both):

- **Every routine**: a name and a description of what it is for and why it is
  written that way. Descriptions go above the routine in the export.
- **Every jump target**: `unexplained` lists the places jumped to with no name;
  name each for what happens there (`homing_bounce_up`, `laser_blocked`), not
  where it is. Read the code at each one before naming it: names written from the
  routine's description alone get directions and players the wrong way round.
- **Line comments** on what is not obvious from the instruction: why a bit is
  tested, what a constant means, where a value came from. Keep them under ~50
  characters or they move above the line.
- `unexplained` also ranks routines by how many instructions have no comment. It
  counts and does not judge, so use it to pick what to read next.

Things that cost time on Jetpac:

- **One comment per address.** A routine's description and a comment on its first
  instruction share an address, so writing the second replaces the first. Fold the
  line comment into the description.
- **Measure before writing a number.** Durations, scores and speeds in comments
  were wrong whenever they were worked out in the head (a delay loop was "four
  seconds", it is two; a laser was "1 to 8 steps", it is 16 to 23).
- **Correct loudly.** When a finding turns out wrong, say so in the commit message
  and the notes, with what the wrong reasoning was. Several Jetpac "facts" were
  corrected this way, including a whole bug diagnosis.
- **Mark what is measured.** The export's `seen:` and `used by:` lines and
  `[square brackets]` are the server's; everything else is somebody's reading.

The notes file (`<tape>.zxrs.txt` beside the tape) holds all of it as plain text,
so keep a copy of what you set in a file of your own too, and replay it after a
restart. A replay script that loads, plays, pokes, forces and re-applies the notes
makes the final export reproducible.

## 6. Export and prove it

1. `restore_state {"name": "menu"}` — the game as loaded. Restoring a state of the
   same program keeps the record of what ran.
2. Compare it with the tape: `scripts/tape_blocks.py TAPE STATE.sna BLOCK ADDRESS
   [SKIP]` for each code block (SKIP for a loader that moves the code after loading).
   Any difference is something the game changed while sitting at its menu; put the
   tape's bytes back with `write_memory` and say so in the notes, so the export is
   the game as it came off the tape.
3. `save_state {"name":"export","path":".../export.sna"}`, then `export_asm` with
   `from` and `to` covering only the game, so the file is the game and not 48K of
   whatever RAM held.
4. `scripts/roundtrip.py FILE.asm export.sna FROM TO`: resolves every label and
   EQU, rebuilds all the data, and checks every instruction against the disassembly
   of the bytes at its address. It should say `0 mismatches` and `0 EQUs` inside the
   range — an EQU inside means a claim or an instruction is hiding where a label
   starts. Break a copy of the file on purpose once to see it fail.
5. Assemble it for real if an assembler is available:
   `sjasmplus --raw=game.bin FILE.asm`, and compare `game.bin` byte for byte with
   memory FROM–TO. roundtrip.py cannot catch a mnemonic that the assembler spells
   differently, only this can. If no assembler is installed, say that this step was
   not done rather than calling the file proven. Don't fetch and build one without
   asking.

## What to report

What was built and where; coverage, split into played, poked and forced; what the
round trip and the tape comparison showed; what is a reading rather than a
measurement; what was corrected along the way; and what is still unknown. Copyright
stays with the game's owners, so the outputs belong in `tapes/` (gitignored) and are
not published.
