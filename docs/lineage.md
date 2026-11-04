# Game Lineage

Games by the same people tend to share code. A programmer takes the sprite
routine, the sound driver or the whole engine from one game into the next, and
a team that changes publisher takes its code along with it. So once you've
taken a few games apart, you can ask which earlier games hold each one's code,
how much of it they hold, and whose code that was. Line the answers up by year
and you have a family tree.

That's what `lineage` and `export_lineage` do in the MCP server. They read
every disassembly in `disassembly/` and draw the tree from those. This page
says what they compare, how far that was checked, and what went wrong along the
way. If you just want the calls, skip to [Using It](#using-it).

An earlier version of this page described a `fingerprint` tool that hashed
whatever a running machine had fetched. That was replaced. The reasons, with
the measurements, are in
[Fingerprinting What Ran](#fingerprinting-what-ran) below.

## Basic Design

**Compare each game's disassembly, in short fragments, with the numbers and
names taken out.**

A game goes into the tree once it has been disassembled, and not before. The
tools read the `.asm` that `export_asm` writes. That file has code where code
was seen to run, `DEFB` everywhere else, and the names somebody gave the
routines. Only the instructions are read. Graphics, level data and tables are
left alone, since they would match on nothing in particular.

Each unbroken run of instructions is cut where control can't fall through
(`RET`, `RETI`, `RETN`, or a `JP` with no condition), or where data comes in
between. A `JR` doesn't end a run, because it mostly hops a few bytes inside
the routine it's in.

Every instruction is reduced to its shape: `LD H,$B7`, `LD H,183` and
`LD H,page` all become `LD H,n`. A bit number, an interrupt mode and a restart
are kept, because they choose the opcode rather than being its operand. Each
run is then cut into overlapping fragments of five instructions, and each
fragment is hashed. Two games holding the same fragment hold the same five
instructions in the same order. Two games holding hundreds of them hold the
same code.

Why five? It's long enough that two people writing the same thing don't often
produce it instruction for instruction, and short enough that a routine of a
dozen instructions still gives you something to match. It's a judgement, not a
measurement. Nobody has tried other lengths and compared them yet.

### Where Each Instruction Is

An `.asm` doesn't say where each instruction is, only where labels are. So the
address is counted up from the `ORG`. Data directives add their sizes:
`DEFB`, `DEFM` and `DEFW` count their items, and `DEFS` counts the size it
names. Each instruction adds its length, which comes from a table built by
running the emulator's own decoder over every opcode there is.

An instruction whose length can't be told stops the reading, and the line it's
on is named. Guessing would put everything after it in the wrong place, which
is worse than not reading the file. Every label in the three disassemblies here
(1,908 of them) lands at the address the notes give it.

### What Is Left Out

Two things are taken out, because each would make games look related when they
aren't:

- **The ROM's own fragments.** The 48K ROM is read straight through from $0000,
  and any fragment it holds is dropped. Otherwise every game that copied `CLS`
  into RAM would be related to every other one that did. The tools refuse to
  run without `roms/48.rom` rather than quietly leave the ROM in.
- **The tape loader.** A loader belongs to whoever protected the tape, not to
  whoever wrote the game. Any run containing one of the sampling loops the fast
  loader knows is dropped, found by the shapes of their instructions
  (`lineage::LOADERS`), and so is every run packed close to it. The tools work
  outward from the sampler for as long as each run is within $40 bytes of the
  last. Which loader it was is kept as a technique, since that's still worth
  knowing. A test holds each shape to the bytes in `flashload`, so the two
  can't drift apart.

### Weighting

A fragment two games share says something about those two. A fragment half the
collection holds says much less about any pair of them: it's an idiom, or
something everybody copied out of the same magazine. So a fragment found in `d`
games counts for `2/d`, and one found in two or fewer counts in full.

The number on a link is the **share**: the weighted fraction of the later
game's fragments that are also in the earlier game. It's measured from the
later game's side because the question being asked is "how much of this game
came from that one?"

### The Tree

Games are put in order by year, then by title. A game hangs off the earlier
game holding the biggest share of it, as long as that game holds at least 5% of
it and at least twelve fragments. Below that, the game is a root with no known
parent. Other earlier games that pass the same bar are drawn as dashed lines.

The time order is what makes it a tree. Since a parent always comes first, no
cycle is possible. But within a single year the order is only by title, so a
link between two games of the same year is marked as such. Which of the two
came first isn't known.

### Where a Routine Came From

For each run of a game's code, the tools find the earlier game holding the most
of its fragments. If that's at least half of them, the routine is said to be
**found in** that game, and it's listed under the name the disassembly gives
it. Ties go to the earlier game.

That's the earliest game *in this collection* holding it, which isn't always
the game it was first written for. The collection only knows what has been
disassembled.

The **suspected author** is the person who may have coded that earlier game and
isn't credited with coding this one. "May have coded" means credited with Code,
or credited with nothing in particular, which is how ZXDB lists the one
programmer of many early games. It's a guess, and it's marked as one (`@` in the
text, italics on the page). When the coders are the same people, the tools say
"the same coders" rather than inventing anybody.

### Techniques

Alongside the fragments, the tools note the tells they can read from the
instructions alone: interrupt mode 2, reading R, drawing by XOR, OR or through
a mask, the pixel-address calculation, `INC H` down a character, an unrolled
`LDI` copy, writing through the stack, the beeper, the AY, a Kempston read,
`HALT`, records reached through IX or IY, and `RST $10`. The list is
`lineage::TECHNIQUES`.

These say how two games do things, not whether they share code, so they're
listed beside a link and never added into the share. A rule like "contains
`HALT`" is true of a great many games, so read them as colour, not evidence.

## Using It

First, once, get the author list. ZXDB is Spectrum Computing's database of
every title and who made it. [Download](https://github.com/zxdb/ZXDB) its
`ZXDB_mysql.sql.zip`, then convert it:

```sh
python3 tools/zxdb-games.py ZXDB_mysql.sql.zip \
    > ~/Library/Application\ Support/ZX\ Spectrum\ Emulator/zxdb-games.txt
```

That's the macOS path; use whichever preferences directory `prefs::config_dir`
gives on your machine. The script reads the dump a line at a time rather than
loading it into MySQL, takes about a second and a half, and writes one line per
title (44,516 of them from the October 2026 dump). It's somebody else's work,
so it's generated into the preferences directory and not committed.

Then disassemble each game, and export it into `disassembly/` under the tape's
name:

```jsonc
{"name":"export_asm","arguments":{"path":"disassembly/Knight Lore (1984)(Ultimate Play The Game).asm"}}
```

The name matters. Title, year and publisher come from the TOSEC-style file
name, and are then looked up in ZXDB. ZXDB holds many titles more than once
(remakes, budget re-releases, type-ins with the same name), so the year and
publisher choose between them. If nothing can choose, the tree says the authors
aren't known, and why. A subtitle after " - " is dropped when the full title
isn't found, because ZXDB has "Saboteur II" where TOSEC has "Saboteur II -
Avenging Angel".

Then ask for the tree:

```jsonc
{"name":"lineage","arguments":{}}
{"name":"lineage","arguments":{"game":"Alien 8"}}
{"name":"export_lineage","arguments":{"path":"out/lineage.html"}}
```

You should see:

```text
3 games disassembled in disassembly, oldest first. ...

1983 Jetpac — Ultimate Play The Game — code: Chris Stamper
1984 Knight Lore — Ultimate Play The Game — code: Chris Stamper
└─ 1985 Alien 8 — 66% of its code is in Knight Lore
```

With `game`, you also get that game's relatives in both directions, the
techniques they share, and its routines found in earlier games, under their own
names: Alien 8's `place_valves` has 83 of its 90 fragments in Knight Lore, for
instance. `export_lineage` writes the same thing as one self-contained HTML
page with nothing loaded from anywhere, so it can be passed around. A
disassembly that can't be read is named in the reply, along with the line that
stopped it. A game missing from the tree would otherwise read as a game with no
relatives.

## What Was Measured

### Disassembly Against Recording

The question was whether a disassembly or a run of the machine gives the more
accurate comparison. So the same three games were fingerprinted both ways:
once from their disassemblies, and once by playing each RZX recording from its
snapshot to its last frame and taking every instruction fetched.

| Game | Disassembly | Recording, played to the end | Recording's share of the disassembly's fragments |
|---|---|---|---|
| Jetpac | 2,620 instructions | 2,062 | 77.6% |
| Knight Lore | 4,159 | 4,159 | 100% |
| Alien 8 | 4,360 | 4,360 | 99.9% |

And the links:

| Pair | From the disassemblies | From the recordings |
|---|---|---|
| Knight Lore holds of Alien 8 | 65.7% | 65.7% |
| Jetpac holds of Knight Lore | 1.2% | 1.2% |
| Jetpac holds of Alien 8 | 1.1% | 1.0% |

Knight Lore and Alien 8 agree because their disassemblies were exported from
these very recordings, so the two sources hold the same code. Jetpac's
disassembly holds more than its recording, because it was built up from more
than one session.

**The Jetpac row is wrong, and I've left it there so it can be seen to be.**
When it was measured the player gave a recorded interrupt thirty-two
T-states from the frame boundary, and Jetpac enables interrupts just after one
at frame 1,444. From there the machine was not following the recording: it
read seventeen bytes in a frame recorded as reading none, and played the
other 52,000 frames to input meant for somewhere else. So 2,062 is what a
broken replay of Jetpac reached, not what the recording reaches. Alien 8's
replay parts company too, at frame 132,195 of 342,604, for a reason not yet
found, so its disassembly is of what ran on a replay that was right for the
first 132,195 frames. (Knight Lore's is in step the whole way.)
[`timing.md`](timing.md) has the fault and the fix. None of this touches the
conclusion: the disassembly still holds at least as much as any replay, and
gives the same answer every time.

The run-time side only got this close because the recordings were played all
the way through. Stopped at 200,000 frames, Knight Lore's share of Alien 8 came
out at 61.4% instead of 65.7%. My earlier scripted runs from the tapes, a
minute or so of attract mode and key-mashing, got only 1,824 of Jetpac's
instructions. So the disassembly is never worse, often holds more, and gives
the same answer every time it's read. That's why it's the prerequisite.

Three games is not much of a test, mind. What it does show:
- the known engine reuse (Alien 8 is built on Knight Lore's Filmation engine)
  comes out strongly;
- the game that shouldn't be related, Jetpac, comes out at about 1%;
- one reading of a file is the same as the next.

### Across Twenty-Six Tapes, From the Running Machine

Before the switch, the run-time version was checked on 26 tapes, each given the
same scripted input. These numbers came from running machines, not
disassemblies, and they were taken before the change to shapes that also
blank names. They aren't reproducible with the current tools until those games
are disassembled. They're kept because they're the evidence that the method
finds families:

| Earlier game | Later game | Share | Why they should be related |
|---|---|---|---|
| Head over Heels (one dump) | Head over Heels (another) | 100% | the same game: a control |
| Daley Thompson's Decathlon, day 1 | day 2 | 97% | the same engine, two loads |
| Jetpac | Pssst | 39–44% | Chris Stamper, 1983 |
| Starglider | Starstrike II | 21% | Ian Oliver and Graeme Baird on both |
| 3D Starstrike | Starstrike II | 14–15% | Realtime, the same coders |
| Saboteur! | Saboteur II | 14–15% | Clive Townsend |
| Atic Atac | Sabre Wulf | 9–10% | Chris Stamper |

Starglider is a Rainbird game and Starstrike II a Realtime one, so that link
followed the authors across publishers without being told who they were.
Between games nobody has any reason to think are related, the biggest share was
2–3%.

### What Was Not Checked

- **More than three disassemblies.** The tree has only been drawn from the
  three in `disassembly/`, all Ultimate. A family by different people, or a
  link across publishers, hasn't yet been seen from disassemblies.
- **Loaders in a disassembly.** None of the three carries a protected loader,
  so the loader rule has only been tested on synthetic code and on running
  machines.
- **128K games.** An export is of the memory the CPU sees, so a bank that was
  paged out when it was exported isn't in it.
- **The phone layout of the HTML page.** Headless Chrome won't lay out
  narrower than 500 pixels, so the page was only checked at 500 and above.

## Things That Went Wrong

### Fingerprinting What Ran

The first version fingerprinted whatever the machine had fetched since a game
was loaded. That made every number depend on how well the game had been
played. A game left on its title screen was fingerprinted as a title screen.
RoboCop's 2,711 instructions turned out to be almost all loader, because its
game is a separate load the input never got to. And Saboteur II ran 454
instructions and probably never started. Playing an RZX recording to the end
fixed the loader problem and made the result repeatable. But it still held no
more than what the recording reached (see the table above), so the
disassembly replaced both.

### Masking Only the Addresses

The first version blanked only the sixteen-bit operands and kept the eight-bit
ones, on the theory that `AND $07` and `AND $18` are different code. Then
Saboteur came out only 8% in its sequel. Skool Daze turned out to keep its
character buffers by page, with `LD H,$B7`, and a routine moved along with its
tables changes that byte too.

Blanking every number raised every link between games known to share code:
Saboteur from 8% to 15%, Jetpac to Pssst from 34% to 39%, 3D Starstrike to
Starstrike II from 12% to 15%. The unrelated pairs stayed at 2% or so.
Disassemblies add names on top of numbers (`LD H,page`), and those are blanked
the same way.

### Taking the Loader as 256 Bytes Either Side

The first loader rule dropped runs within $100 bytes of a sampling loop. That
left Head over Heels holding 17% of RoboCop, which would be news to Jon Ritman
and Mike Lamb. The routines that matched were at $FE25–$FEC3, the end of
Speedlock, $145 bytes above its sampler. Both tapes are Speedlocked. Growing
the loader outward from the sampler for as long as its runs are packed together
removed the link.

### A Loader With No Shape

When loaders moved from being matched by bytes to being matched by shapes, one
of the seven was missed: the ROM's sampler with a byte of filler in it, which is
Microsphere's and Bleepload's. A disassembly of Skool Daze or Bubble Bobble
would have kept its loader in. The test that holds the shapes to `flashload`'s
bytes found it. The filler varies (`AND A` in one, `NOP` in the other), so that
shape has a wildcard in it.

### Skool Daze and Contact Sam Cruise

These two were supposed to be the known-good pair. Both are by David Reidy at
Microsphere, and they're often said to share an engine. From the running
machines they came out under 1%.

So was the tool missing them? To find out, the two programs were compared
whole from Richard Dymond's SkoolKit disassemblies
([`skoolkid/skooldaze`](https://github.com/skoolkid/skooldaze),
[`skoolkid/contactsamcruise`](https://github.com/skoolkid/contactsamcruise)),
with every instruction in each, cut the same way. They share 78 fragments: 1.7%
of Contact Sam Cruise's.

That check needed a known-good of its own. The same script on Dymond's Manic
Miner and Jet Set Willy, where JSW is built on Manic Miner's code, gives 509
fragments: 30.4% of JSW's.

So the assumption was wrong, not the tool. Whatever Contact Sam Cruise took
from Skool Daze, it took as a design and wrote out again. Rewritten code is
exactly what fragments can't see. That comparison was a one-off script; the
tools themselves read only `disassembly/`.

## Future Work

- **More disassemblies.** The tree is as good as what's in the folder, and with
  three games it can only show one family. The twenty-six tapes above are the
  obvious next ones, since what they should show is already known.
- **Code that does the same thing differently.** Comparing the shape of each
  routine's flow, rather than its instructions, might catch Skool Daze and
  Contact Sam Cruise. It isn't done because there's no reference yet to say
  whether a structural match is real or two people solving the same problem
  the same way.
- **Telling an author by style.** Register habits and idioms, scored against
  each author's known games, would put a name to code nobody else has. It needs
  many more disassembled games per author before it can be checked against
  anything.
- **Correcting a game's details.** The title, year and publisher come only from
  the file name and ZXDB. A game ZXDB knows by another name (its Freddy Hardest
  in South Manhattan is "Freddy Hardest en Manhattan Sur") has no authors in the
  tree until the file is renamed.
- **The fragment length** is untested. Five was chosen, not measured.
