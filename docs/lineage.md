# Game Lineage

Games by the same people tend to share code. A programmer takes the sprite
routine, the sound driver or the whole engine from one game into the next, and
a team that changes publisher takes its code along with it. So if you
fingerprint the code of every game you take apart, you can ask which earlier
games hold each new one's code, how much of it they hold, and whose code that
was. Line the answers up by year and you have a family tree.

That's what `fingerprint`, `lineage` and `export_lineage` do in the MCP server.
This page says what they measure, how far that was checked, and what went wrong
along the way. If you just want the calls, skip to [Using It](#using-it).

## Basic Design

**Compare code that ran, in short fragments, with the numbers taken out.**

Comparing memory would compare graphics, level data and whatever the loader
left lying about, so a fingerprint only covers addresses the tracker saw an
opcode fetched from. Each unbroken run of instructions is cut where control
can't fall through (`RET`, `RETI`, `RETN`, or a `JP` with no condition). A
`JR` doesn't end a run, since it mostly hops a few bytes inside the routine
it's in.

Every instruction is reduced to its shape, with each number replaced by `n`,
so `LD H,$B7` becomes `LD H,n`. The run is then cut into overlapping fragments
of five instructions, and each fragment is hashed. Two games holding the same
fragment hold the same five instructions in the same order. Two games holding
hundreds of them hold the same code.

Why five? It's long enough that two people writing the same thing don't often
produce it instruction for instruction, and short enough that a routine of a
dozen instructions still gives you something to match. It's a judgement, not a
measurement. Nobody has yet tried other lengths and compared them.

Three things are taken out, because each would make games look related when
they aren't:

- **The ROM's own fragments.** The ROM is read straight through from $0000 and
  anything it holds is dropped. Otherwise every game that copied `CLS` into RAM
  would be related to every other one that did.
- **The tape loader.** A loader belongs to whoever protected the tape, not to
  whoever wrote the game. Any run containing one of the sampling loops in
  `flashload::CORES` is dropped. So is every run packed close to it, working
  outward for as long as each run is within $40 bytes of the last. Which loader
  it was is kept as a technique, since that's still worth knowing.
- **Anything you say isn't the game**, through `exclude`.

### Weighting

A fragment two games share says something about those two. A fragment half the
collection holds says much less about any pair of them: it's an idiom, or
something everybody copied from the same magazine. So a fragment found in `d`
games counts for `2/d`, and one found in two or fewer counts in full.

The number on a link is the **share**: the weighted fraction of the later
game's fragments that are also in the earlier game. It's measured from the
later game's side because the question being asked is "how much of this game
came from that one?"

### The Tree

Games are put in order by year, then by title. A game hangs off the earlier
game holding the biggest share of it, as long as that game holds at least 5% of
it and at least twelve fragments. Below that it's a root with no known parent.
Other earlier games that pass the same bar are drawn as dashed lines.

The time order is what makes it a tree. Since a parent always comes first, no
cycle is possible. But within a single year the order is only by title, so a
link between two games of the same year is marked as such. Which of the two
came first isn't known.

### Where a Routine Came From

For each run of a game's code, the tools find the earlier game holding the most
of its fragments. If that's at least half of them, the run is said to be
**found in** that game. Ties go to the earlier one.

That's the earliest game *in this collection* holding it, which isn't always
the game it was first written for. The collection only knows what you've
fingerprinted.

The **suspected author** is the person who may have coded that earlier game and
isn't credited with coding this one. "May have coded" means credited with Code,
or credited with nothing in particular, which is how ZXDB lists the one
programmer of many early games. It's a guess, it's marked as a guess (`@` in the
text, italics on the page), and when the coders are the same people the tools
say "the same coders" rather than inventing anybody.

### Techniques

Alongside the fragments, a fingerprint notes the tells it can read from the
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
gives on your machine. It reads the dump a line at a time rather than loading
it into MySQL, takes about a second and a half, and writes one line per title
(44,516 of them from the October 2026 dump). It's somebody else's work, so it's
generated into the preferences directory and not committed.

Then for each game, load it, play it, and fingerprint it:

```jsonc
{"name":"load_tape","arguments":{"path":"tapes/Jetpac (1983)(Ultimate Play The Game)[16K].tzx"}}
{"name":"run_frames","arguments":{"frames":500}}
// ... press_keys until the game is being played, not just sitting on its menu ...
{"name":"fingerprint","arguments":{}}
```

Playing it matters. The fingerprint holds only what ran, so a game left on its
title screen is fingerprinted as a title screen.

The title, year and publisher come from a TOSEC-style file name and are then
looked up in ZXDB. ZXDB holds many titles more than once (remakes, budget
re-releases, type-ins with the same name), so the year and publisher choose
between them. If nothing can choose, you're told so rather than given a guess.
A subtitle after " - " is dropped when the full title isn't found, because
ZXDB has "Saboteur II" where TOSEC has "Saboteur II - Avenging Angel".

Anything can be given outright instead: `title`, `year`, `publisher` and
`authors` (as `Name (Code, Music); Name`). The fingerprint is written beside the
tape as `<name>.lineage.txt`. It's plain text, so you can correct the game's
details in an editor; everything below them was measured.

`lineage` draws the tree as text from every fingerprint in a directory (the
loaded file's by default). Pass `game` to see one game's relatives in both
directions, the techniques they share, and its routines found in earlier games
with who is suspected of writing them. `export_lineage` writes the same thing as
one self-contained HTML page with nothing loaded from anywhere, so it can be
passed around.

## What Was Measured

Twenty-seven tapes were each loaded in the MCP server and given exactly the
same input, so that no game was played better than another. That was 500 frames
of attract mode, the keys 1, 0, 5, SPACE, ENTER, S and 1 again, then six rounds
of O, P, Q, A, M, SPACE, Z and X. Each one was then fingerprinted. The two
Sidewize dumps wrote to the same file, so 26 fingerprints came out (see below).

A second sample was taken with a different key order, twelve rounds instead of
six, and 900 frames of attract mode. Both columns below are measured:

| Earlier game | Later game | Share, sample 1 | Share, sample 2 | Why they should be related |
|---|---|---|---|---|
| Head over Heels (one dump) | Head over Heels (another) | 100% | 100% | the same game: a control |
| Daley Thompson's Decathlon, day 1 | day 2 | 97% | 97% | the same engine, two loads |
| Jetpac | Pssst | 39% | 44% | Chris Stamper, 1983 |
| Starglider | Starstrike II | 21% | 21% | Ian Oliver and Graeme Baird on both |
| 3D Starstrike | Starstrike II | 15% | 14% | Realtime, the same coders |
| Saboteur! | Saboteur II | 15% | 14% | Clive Townsend |
| Atic Atac | Sabre Wulf | 10% | 9% | Chris Stamper |

Starglider is the interesting one. It's a Rainbird game and Starstrike II is a
Realtime one, so the tree followed the authors across publishers without being
told who they were. Jetpac and Pssst went up between the samples because more
of each game ran, which is what you'd expect when the input is longer.

Between games nobody has any reason to think are related, the biggest share was
2–3%, on 10 fragments at most. The exception is RoboCop: R-Type holds 18% of
it, but RoboCop has only 44 fragments, so that 18% is 8 of them. That's why a
branch needs twelve fragments as well as 5%. A percentage of almost nothing is
almost nothing.

### What Was Not Checked

- **Saboteur II never started**, as far as can be told. Only 454 instructions
  ran on the 128K, against 1,707 for the first game, so its 15% is from very
  little code.
- **RoboCop mostly didn't run.** Of its 2,711 instructions, all but 77 were
  packed loader code. Its game is a separate load the input never got to.
- **Several runs ended with PC in the ROM**, at $0296–$02AE. That's the ROM's
  KEY-SCAN under the IM 1 interrupt, which is a game reading the keyboard
  rather than one that has crashed, but nobody looked at the screens to make
  sure.
- **Nothing has been compared with a real reference for the 128K.** Only what
  is paged in when `fingerprint` is called is read, so code in a bank that's
  paged out isn't in the fingerprint.
- **The phone layout of the HTML page.** Headless Chrome won't lay out
  narrower than 500 pixels, so the page was only checked at 500 and above.

## Things That Went Wrong

### Masking Only the Addresses

The first version blanked only the sixteen-bit operands and kept the eight-bit
ones, on the theory that `AND $07` and `AND $18` are different code. Then
Saboteur came out only 8% in its sequel, and Skool Daze turned out to keep its
character buffers by page, with `LD H,$B7`. A routine moved along with its
tables changes that byte too.

Blanking every number was tried on the same 26 tapes. Every link between games
known to share code went up: Saboteur 8% to 15%, Jetpac to Pssst 34% to 39%,
3D Starstrike to Starstrike II 12% to 15%. The unrelated pairs stayed at 2% or
so. So every number is blanked now, and
`a_routine_moved_to_another_address_with_its_tables_still_matches` holds it
there.

### Taking the Loader as 256 Bytes Either Side

The first loader rule dropped runs within $100 bytes of a sampling loop. That
left Head over Heels holding 17% of RoboCop, which would be news to Jon Ritman
and Mike Lamb. The routines that matched were at $FE25–$FEC3, the end of
Speedlock, $145 bytes above its sampler. Both tapes are Speedlocked. Growing
the loader outward from the sampler for as long as its runs are packed together
removed the link. RoboCop's own code then turned out to be almost entirely
loader, which is the finding above.

### Skool Daze and Contact Sam Cruise

These two were supposed to be the known-good pair. Both are by David Reidy at
Microsphere, and they're often said to share an engine. They came out under 1%
both ways. With every number blanked, it was still under 1% of Contact Sam
Cruise and 2% of Skool Daze, on 22 fragments.

So was the tool missing them? To find out without the emulator's coverage
getting in the way, the two programs were compared whole from Richard Dymond's
SkoolKit disassemblies ([`skoolkid/skooldaze`](https://github.com/skoolkid/skooldaze),
[`skoolkid/contactsamcruise`](https://github.com/skoolkid/contactsamcruise)).
That was every instruction in each, run or not, cut the same way. They share
78 fragments: 1.7% of Contact Sam Cruise's.

That check needed a known-good of its own. The same script on Dymond's Manic
Miner and Jet Set Willy, where JSW is built on Manic Miner's code, gives 509
fragments: 30.4% of JSW's.

So the assumption was wrong, not the tool. Whatever Contact Sam Cruise took
from Skool Daze, it took as a design and wrote out again. Code that's been
rewritten is exactly what fragments can't see.

### Two Dumps of One Game Share a File Name

`Sidewize (1987)(Firebird Software).tzx` and its `.zip` both fingerprint to
`Sidewize (1987)(Firebird Software).lineage.txt`, so the second overwrote the
first. The notes beside a tape have the same property. Two different games
can't collide this way, since their titles differ. Pass `path` if you want both
dumps kept.

## Future Work

- **Code that does the same thing differently.** Comparing the shape of each
  routine's flow, rather than its instructions, might catch Skool Daze and
  Contact Sam Cruise. It isn't done because there's no reference yet to say
  whether a structural match is real or two people solving the same problem
  the same way.
- **Telling an author by style.** Register habits and idioms, scored against
  each author's known games, would put a name to code nobody else has. It needs
  many more fingerprinted games per author than there are here before it can be
  checked against anything.
- **Banks that are paged out**, for 128K games. The tracker keeps them; the
  fingerprint reads through the CPU's view of memory.
- **The fragment length** is untested. Five was chosen, not measured.
