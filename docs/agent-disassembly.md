# What an agent would need to disassemble a game

The question: given a tape or a snapshot of a game, what would a language
model driving this emulator need to produce a full disassembly, with comments
saying how it works — the game logic, how the screen is updated, how sprites
are drawn, collision detection, the menus?

Written on 14 September 2026 against the emulator as it stands, and like
[`missing.md`](missing.md) it is a description of the gap rather than a plan.
Nothing in it is promised.

## What is already there

Most of what is needed to watch a program and write down what it does exists.
The MCP server (`src/mcp/`, and [`mcp.md`](mcp.md)) has loading, running,
single-stepping and stepping back, breakpoints and watches on kinds of event,
routines and the call graph, the observer's measurements per routine (what
each writes, reads and calls, and how often), AutoDoc's hedged guesses, memory
search and change tracking, the screen as a picture, labels and comments in a
notes file, and `export_asm`, which writes RAM as source that assembles back
into the same bytes.

What is missing falls into three kinds: reaching all of the code, tracing
where data comes from, and keeping what has been worked out in a form richer
than a label and a comment.

## 1. Reaching all of the code

Code is what has run — the tracker marks every opcode fetch — so a level never
reached, a death never died or a menu option never chosen stays DEFB.

- **Code that can be reached but has not run.** `coverage` follows jumps, calls
  and jump tables out from what has run and lists it. The export and the listing
  don't tell the three apart yet.
- **Branches never taken.** `coverage` lists the conditionals that have gone one
  way. What is missing is a way to fork from a saved state with a flag or a
  register forced, so the other side can be explored. `save_state` and
  `restore_state` are there already.
- **Playing the game.** MCP has `press_keys`, `type_text` and `mouse`, and no
  joystick. An agent also wants "hold these inputs for this many frames" and a
  way to know when the picture has settled, so it can drive menus and play a
  level without guessing at timing.
- **A coverage report.** `coverage` gives how much of a range has run and what
  nothing has touched. It doesn't say which routines yet.

## 2. Where data comes from

- **Who drew this.** For a character cell of the screen, which instruction
  wrote it, when, and where the bytes it wrote were read from. That is most of
  "how sprites are drawn". The Graphics window's Find does the last part by
  searching, but it is not an MCP tool yet.
- **Watching a value.** A watch on an address with a condition on its value,
  logging PC, frame, and old and new value. `docs/missing.md` already lists
  conditional breakpoints as missing. It is how lives, score and coordinates
  are found.
- **What an input changes.** Press left and say which bytes changed and which
  routine changed them. `changed_since` does half of this, without tying the
  change to the input or to the code.
- **A trace to a file.** An instruction trace, filtered to a range or a
  routine, with registers — also listed as missing.
- **Two runs side by side.** Record which routines and which branches ran in
  two runs from the same saved state — the player dying, and not — and report
  what only one of them did. The difference is the collision and death logic.

## 3. How the screen is updated

- **When writes land against the beam.** Race the Beam and the timeline are
  not exposed over MCP ([`mcp.md`](mcp.md) says so). Without them, "draws above
  the beam" and why a sprite flickers cannot be explained.
- **The interrupt's share.** Find the IM 2 vector table and the handler, and
  say what runs from the interrupt and what from the main loop.
  `frame_timing` and `watch_events` give a flatter version.
- **Back buffers.** The RAM map already detects them; saying what is copied to
  the screen, when and by what would explain double buffering.

## 4. What the data is

- **Typed data in the notes.** They know labels, comments and CODE/DATA blocks.
  They would need pointer tables, text, graphics with their width and masking,
  level maps and structures — and `export_asm` would then write DEFW and DEFM
  where it now writes DEFB for everything.
- **Detectors** for jump tables, strings (in the game's own font as well as
  ASCII) and sprite sheets.
- **Unpacking and self-modifying code.** Flag writes into code that has run,
  and a routine copying or decrypting a block that later runs. Many games
  unpack themselves after loading; `loader` knows about tape loaders only.

## 5. Keeping what has been worked out

- **Facts with their evidence.** A label and a comment cannot hold "lives are
  at $5F30, decremented by the routine at $8A12, seen when the player died,
 confident". A store of claims, each with its evidence and how sure, that can
  be asked questions later, would let a long analysis build instead of going
  over the same ground.
- **Paged answers** for the big ones: routines, traces, coverage.
- **Every bank of a 128K.** The export, the coverage and the searches see only
  what is paged in.

## 6. Checking the result

- **Assembling it back.** `export_asm` output assembled and compared with
  memory as a tool, when an assembler is to hand — `tests/asm_export.rs` does
  this with sjasmplus already.
- **Behaving the same.** Load the rebuilt binary into a fresh machine, replay
  a recorded run of inputs, and compare the screens — or RZX recordings — frame
  by frame. Same bytes is necessary; same behaviour is the proof.

## Where to start

A joystick tool, the input-effect probe and value watches first; then
coverage and the branches never taken, with forking from a saved state; then
"who drew this". Between them an agent could play the game, reach its code,
and attach "this draws the player" and "this checks collisions" to evidence
rather than to guesses.
