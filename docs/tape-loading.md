# Tapes and their loaders

What has been learned about getting a tape into the machine: the formats, the
deck, the ROM's loader, and the three ways of making it quicker.

## Tapes are played as pulses, never decoded

The deck makes edges and the machine reads them through port $FE bit 6. It
never looks at a block and decides what the bytes are, which is why turbo
loaders, speedlock variants and the ZX81's own format all work through the same
path without a special case for any of them.

**Each machine has its own deck.** A tape is timed in the T-states of the
machine playing it, and a 48K's clock is not a 128K's, so the deck belongs to
the machine rather than to the emulator.

Edges go to the sound mixer with the T-state they happened at, rather than the
level being sampled whenever the CPU happens to look — otherwise loading noise
comes out as aliased mush.

## Formats

**TAP** is nothing but standard blocks: a length, then a flag byte, the data
and a checksum. Everything in a .tap can be handed to the ROM.

**TZX** is a container of block types, of which the ones that matter are the
standard block ($10), the turbo block ($11), pure tone ($12), pulse lists
($13), pure data ($14), pauses and the flow-control blocks. Two traps found
the hard way:

- **The custom info block ($35) names itself in sixteen bytes, not ten.**
  Reading ten took the length from the last four characters of the name —
  spaces, $20202020 — and the parser reported "block claims 538976288 bytes but
  only 54052 remain". Everything behind that block was unreadable, which for
  Sidewize is the whole tape.
- A block that cannot be understood should be stepped over whole rather than
  guessed at, and a badly formed one should be reported with the number that
  went wrong in the message. Both of those are how the $35 bug was found in a
  minute rather than an afternoon.

**Zip archives** are read directly: the first file inside with a matching
extension is loaded, and if there is none, nothing happens. `src/zip.rs` is a
small reader written here rather than a dependency, because the build stays
offline-reproducible against the crates already in the lock file.

## The pause at the end of a block

A block ends with a pause, and the pause is part of the block rather than a
gap between blocks. `in_block_pause` says the deck is in one;
`pause_ends_the_tape` says the pause is the last thing on the tape or the block
behind it stops the tape.

That distinction is what Max speed uses: it returns to normal speed only for
the pause at the end of the *last* block, or one whose next block stops the
tape — a zero-length pause block or a "stop if 48K".
Returning to normal speed at every in-block pause makes a multi-load tape crawl
for a second between every part.

## The ROM's loader

LD-BYTES lives at **$0556** in the 48K ROM. Its first eight bytes are
`INC D / EX AF,AF' / DEC D / DI / LD A,$0F / OUT ($FE),A` — `14 08 15 F3 3E 0F
D3 FE` — and checking those is the only safe way to know the routine is really
there. A 128K is running its own ROM at that address until a game pages the
other one back, and a program may put anything it likes there in RAM.

Its contract:

| Register | On entry                       | On exit                    |
| -------- | ------------------------------ | -------------------------- |
| A        | flag byte wanted (0 header, $FF data) | —                   |
| F carry  | set to load, clear to verify   | set on success             |
| IX       | where the bytes go             | past the bytes loaded      |
| DE       | how many bytes are wanted      | what is left, 0 on success |
| H        | —                              | running parity, 0 if right |

The checksum byte at the end of a block makes flag ⊕ data ⊕ checksum = 0.

The header a program loads first is seventeen bytes, and it says what the data
block behind it is and where it goes. A program looking for its data steps over
any headers in between: a flag byte that is not the one asked for means "not
this block", not "error".

## Three speeds

**Normal**, which is neither switch on. The pulses are played and the ROM counts them. 1,500 baud, so a
full tape is minutes.

**Max speed.** The CPU is run as fast as the emulator will go while the tape
moves. This shortens the wait; it cannot remove it, because the pulses still
take as long as they take in machine time.

**Fastload** (`src/flashload.rs`). The call to LD-BYTES is answered
rather than run: the next block with the flag byte asked for is copied into
memory, the registers are left as the routine would have left them, and the
return is taken there and then. Border Break loads in two emulated frames
instead of 2,605, ending in the same place, running, with the same 975 bytes on
screen.

Three things it has to get right, all of them found by loading a real tape:

- **Hand over the next block with the flag asked for, not the block under the
  head.** Handing a program a header when it asked for data makes BASIC report
  a loading error.
- **Seeking past the last block must stop the deck, not play it.** Playing from
  the end rewinds, so the tape started loading all over again and the run took
  *longer* than without any of this.
- **Report failure the way the ROM does.** A block shorter than the program
  asked for is the "R Tape loading error" every mistyped POKE ends in, and a
  loader that reports success for it leaves the machine somewhere it could
  never have got to.

It only helps where the ROM is doing the loading. A game with a loader of its
own is reading the port and counting its own pulses; nothing here can help it,
which is why switching Fastload on switches Max speed on with it. Measured:
Border Break 2,605 emulated frames → 2; Anabasis, which loads a small BASIC
stub and then reads the rest itself, 18,722 → 18,149.

**The silence behind a block belongs to the block**, and is left on the tape
when the bytes are handed over. That pause is what the program does its work in
before the next block starts — loading its own loader, say — and taking it away
breaks games that need it. Cobra's Alkatraz loader has under two seconds of
pilot to catch and never caught it; with the pause left in it finds all 256 of
the pilot pulses it looks for, exactly as it does when the tape is played. So a
flash-loaded tape is not instant: it is the pauses plus a memcpy, about a second
for Border Break against fifty-two.

**A stopped deck is a stopped deck.** Blocks are only handed over while the
tape is running: `LOAD ""` with the deck paused waits for Play, as it should.
Without that check the whole tape ran through the moment the ROM asked for its
first block, leaving the machine part way into a tape nobody had started.

**There are two ways in.** A loader that needs interrupts kept off can't
let LD-BYTES return through SA/LD-RET at $053F, which ends in `EI`, so it does
the routine's first steps itself (`INC D / EX AF,AF' / DEC D / DI / LD A,$0F /
OUT ($FE),A`), pushes its own return in place of SA/LD-RET, and jumps to
**$0562**, past the push. Fastload answers there too, checking the four bytes
of `LD HL,$053F / PUSH HL` as well as the first eight. From there the flag and
the load/verify carry are in the alternate AF, where the caller's `EX AF,AF'`
put them, and the return leaves interrupts off, as the ROM would. Knight Lore's
rebuilt 128K tape loads its last block this way, and until Fastload answered
$0562 that block was read from the pulses: 7,055 frames instead of a few
hundred, and never at all with the CPU turbo on, which leaves the ROM unable to
time a pulse. Its tape test had passed regardless, since it checked where the
load ended and not how; it now checks the frames, and loads under the turbo
too.

The first block of a session sometimes still loads at tape speed. The ROM sits
*inside* LD-BYTES waiting for a pilot tone, and the answer is only given on
entry — so if the machine is already in there when Play is pressed, that block
is played. Start the deck before typing `LOAD ""` and every block is handed
over.

## Loading that is not the ROM's

Games with their own loaders are the majority of anything past 1984. Nearly all
of them are built round the same handful of
[loading routine cores](https://sinclair.wiki.zxnet.co.uk/wiki/Loading_routine_%22cores%22),
of which the commonest is the ROM's own edge sampler with the BREAK check taken
out:

```text
LD-SAMPLE  INC B          04
           RET Z          C8
           LD A,$7F       3E 7F
           IN A,($FE)     DB FE
           RRA            1F
           XOR C          A9
           AND $20        E6 20
           JR Z,LD-SAMPLE 28 xx
```

Speedlock uses exactly that. Found in Head over Heels at $FD30, byte for byte,
with the jump back reading `28 F4` rather than the ROM's `F6` because its loop
starts two bytes earlier — which is why the first search for it, using the
ROM's displacement, found nothing at all. `flashload::sampler` ignores the
displacement, and the immediate loaded into A before the `IN`: that is the
port's high byte and games differ on it — $7F for most, $FF for Astro Marine
Corps, $00 for City Slicker — without it changing what the loop is.

### The loops, as found on the tapes

Seven of them, read off the tapes rather than off a list: run the game, take
the bytes at the address the machine spends its time at while the tape runs.
They are all the same idea and differ in how the EAR bit is got at and in what
happens when B comes round.

| loop | bytes from the `INC B` | seen in |
| --- | --- | --- |
| the ROM's sampler | `04 C8 3E ?? DB FE 1F A9 E6 20 28` | Speedlock: Head over Heels, Daley Thompson's Decathlon |
| the same with a byte of filler | `04 C8 3E ?? DB FE 1F ?? A9 E6 20 28` | Bleepload (`00`), Microsphere (`A7`), Paul Owens |
| one that answers BREAK | `04 C8 3E ?? DB FE 1F D0 A9 E6 20 28` | Dinamic: Astro Marine Corps, Freddy Hardest; the Search loader: Blood Brothers |
| one masking the EAR bit where it lies | `04 C8 3E ?? DB FE A9 E6 40 28` | the Search loader's variant: Lotus Esprit Turbo Challenge, Space Crusade |
| the same answering the carry | `04 C8 3E ?? DB FE A9 E6 40 D8` | Hewson: City Slicker |
| Alkatraz's | `04 20 03 C9 ?? ?? DB FE 1F C8 A9 E6 ?? 28` | Cobra, 720 Degrees |
| Digital Integration's | `05 C8 DB FE A9 E6 40 CA` | ATF, Tomahawk |

The `D0` is `RET NC`, which is the ROM's own BREAK check left in rather than
taken out. Masking with $40 instead of $20 is the same bit looked at without
the `RRA` first. Digital Integration's counts B *down*, does not load the
port's high byte at all — whatever was last on the bus will do — and closes
with an absolute jump.

Knowing which loop it is does not make a tape load: nothing about a game's own
loader can be answered the way the ROM's can, for the reason below. It was on
the tape window for a while as a caption — "Digital Integration's sampler is
reading" — and that came straight back out again, because a loader is inside
its sampler for part of every byte and somewhere else for the rest, so the line
flashed several times a second. `flashload::sampler` still knows, and the tests
below are what ask it.

### What cannot be done, and why

A Speedlock block cannot be handed over the way a ROM block can, because the
bytes on the tape are not the bytes that reach memory. Its byte loop, from Head
over Heels:

```text
$FE04  LD A,$12          ; both immediates are rewritten as it goes
$FE06  XOR L             ; L is the byte assembled from the tape
$FE07  ADD A,$86
$FE09  LD (IX+$00),A
$FE0C  INC IX
$FE0E  DEC DE
$FE1A  CALL $FD2C        ; the next bit
```

The obvious shortcut is not to hand over blocks but to skip the *waiting*: when
the machine is sitting in the sampler, the deck already knows when the next
edge is due, so move the clock there and give B the turns it would have
counted. That was tried and **it does not work**. The port the loop reads is
$7FFE, whose high byte is in the contended range, so the ULA stalls the read by
an amount that depends on where the beam is: measured over a load of Head over
Heels, a turn costs 54 T-states most of the time but 56, 58 or 60 often enough
to matter. Divide the wait by a fixed 54 and B comes out wrong, the loader
mis-measures its pulses, and both test tapes failed to load. Charging each
skipped turn its true contended cost would mean reproducing the I/O contention
in closed form for a saving of a few instructions a bit, which is not worth it.

### What is done instead

Fastload lifts the work cap while a tape is moving. Max speed is held to
twenty-four frames of machine time a host frame so that fast-forward cannot
lock up the window; with a tape loading, that cap is what a person is waiting
on. Fastload keeps working until fifty milliseconds of the host's own time
have gone, then stops to draw, so the window still answers twenty times a
second.

Measured on Daley Thompson's Decathlon, from `LOAD ""` to the tape stopping:

| | host frames | what that is at 60 Hz |
| --- | --- | --- |
| Max speed | 615 | about ten seconds |
| Fastload | 11 | about a fifth of a second |

And it loads the same game: comparing all of memory at the moment the tape
stops, the loading screen's pixels and everything above the screen are
identical byte for byte. What differs is the stack below SP ($FFCE-$FFDF for
Head over Heels, $FFF3-$FFF9 for Daley) and, for Daley, 151 attribute bytes —
its loading screen cycles colours while the tape runs, so how far round it has
got depends on exactly when the tape ran out.

## The rest of what a custom loader needs

What the emulator can do for a loader it cannot answer is play the pulses
accurately and run the CPU fast. Two things make that work:

- **Pulse-level playback**, as above. A loader measuring its own edge timings
  gets the same edges a real machine would.
- **The floating bus** giving back what the ULA has on it. Sidewize will not
  even start without it: it looks for the beam by reading port $40FF and never
  enables interrupts until the byte it wants comes back.

## Every pulse ends with an edge, including the last one of a block

The silence behind a block is at the low level. A block whose last pulse left
the line low therefore used to end with no change at all: the deck went
straight from the final data pulse to holding the line low, which it already
was. A loader waiting for the edge that closes that pulse waits for ever.

The deck now finishes the last pulse first — a millisecond at the level the
pulse toggles to, and then the silence — which is the same thing the TZX
reference says when it describes the pause as beginning with a millisecond of
the current level.

**That is what stopped Cobra and 720 Degrees loading**, and it looked nothing
like a tape fault. Alkatraz reads all eight bits of the block's last byte,
waits for the closing edge, times out, and jumps to $F067: which wipes its own
code with an `LDDR`, plays 224 beeps through the ROM's beeper at $03B5, and
resets the machine. Everything before that is correct — the pilot search passes
all 256 of its measurements, the whole block is assembled byte for byte, and
both of the loader's checks on it pass — so the failure arrives long after the
mistake, wearing a copy-protection costume.

Two measurements settled it. The byte the loader wanted was on the tape: it
asked 258 T-states after finishing the previous one, with 12,316 T-states of
data still to come. And following the last read edge by edge showed all eight
bits arriving — 1135 and 1117 T-states for a one, 546 and 542 for a nought —
and then nothing at all where the sixteenth pulse should have been closed.

A ZX81 test had the old behaviour written into it, expecting fifteen bit-gaps
from sixteen bits because "the last of whose gaps is swallowed by the pause".
It gets sixteen now.

## Bleepload

Firebird's and Rainbird's, and the easiest of the four: Bubble Bobble,
Starglider and Starglider 2 all load without anything having to be done for
them. It shows on the tape as a couple of hundred small blocks — about 270
bytes each with six to thirteen milliseconds between them — rather than a few
large ones, and the pair of sync pulses swaps round from one tape to the next:
Bubble Bobble's are 735 then 667, Starglider's 667 then 735, Starglider 2's
714 twice with its blocks wrapped in groups and a four-millisecond pause and a
long tone in front.

Host frames from `LOAD ""` to the tape stopping, played against hurried:

| | played | Max speed | Fastload |
| --- | --- | --- | --- |
| Bubble Bobble | 21,475 | 1,074 | 41 |
| Starglider | 19,529 | 977 | 36 |
| Starglider 2 | 24,473 | 1,224 | 46 |

Starglider 2 was the one that did not finish, for reasons that had nothing to
do with Bleepload; see below.

Bubble Bobble is worth knowing about when reading a test: it waits at its menu
inside the ROM's keyboard scan, which is where BASIC waits too, so where the
machine is executing says nothing about whether the game loaded. What says it
is the picture — a report line is a hundred-odd bytes of screen, a menu is a
thousand.

## Microsphere

A whole game in one block, and the least ceremony of any of them. Skool Daze
is 82,109 bytes of turbo block at twice the ROM's rate — 422 and 843 T-states
a bit — behind nothing but a header and a BASIC line. Contact Sam Cruise does
it the other way about: a 244-byte turbo block carrying the loader, and then
49,465 bytes as an *ordinary standard block*, read by the game rather than by
the ROM, which is why nothing can be handed over for it.

| | played | Max speed | Fastload |
| --- | --- | --- | --- |
| Skool Daze | 14,906 | 746 | 30 |
| Contact Sam Cruise | 18,180 | 910 | 35 |

Like Bleepload, neither needs anything the other loaders needed: both load
with the closing edge taken out again and with the EAR line left dead.
Checked, not assumed.

## Paul Owens

Chase H.Q.'s, and the tidiest to read: every block at the same settings — 2,196
pilot pulses, 667 and 735 for the sync, 735 and 1,590 a bit — with the game in
four blocks and then the levels behind it, each a four-byte block followed by
its data and announced by a text block in the tape itself ("Level 1", "Level
2", and so on).

| | played | Max speed | Fastload |
| --- | --- | --- | --- |
| Chase H.Q. (48K) | over 36,000 | 2,223 | 78 |
| Chase H.Q. (128K) | — | — | 91 |

Played, it does not finish inside ten minutes of host time: the levels go on
past the game, and the game says "STOP THE TAPE" long before the tape agrees.

**It needs the closing edge**, like Alkatraz and unlike Bleepload and
Microsphere: taken out again, Chase H.Q. ends with a report line and 134 bytes
of screen. The EAR feedback it does not care about.

**It does not load with an Interface 1 fitted**, on either machine, and that is
the interface's doing rather than the tape's: its BASIC loader reaches its own
code through a deliberate report and ERR_SP, at an address the Interface 1
moves by inserting its system variables on its first call.
[`peripherals.md`](peripherals.md) has the whole of it.

## Dinamic, the Search loader and Digital Integration

Five more loaders, eight more games, and none of them needed anything the
earlier ones had not already needed: they load with the closing edge, the EAR
feedback and the pulse timings as they stand. Checked rather than assumed — the
same memory comes off the tape whether it is played or hurried, screen included
byte for byte.

| | tape | Max CPU | Fastload |
| --- | --- | --- | --- |
| Astro Marine Corps | 3,210 blocks | 1,076 | 39 |
| Freddy Hardest in South Manhattan | 7 blocks | 893 | 33 |
| Blood Brothers | 15 blocks | 696 | 26 |
| City Slicker | 3 blocks | 971 | 38 |
| Lotus Esprit Turbo Challenge | 53 blocks | 1,940 | 61 |
| Space Crusade | 43 blocks | 1,875 | 36 |
| ATF | 14 blocks | 570 | 21 |
| Tomahawk | 14 blocks | 611 | 23 |

Host frames from `LOAD ""` to the tape stopping. Fastload is worth a factor of
about thirty over Max CPU for all of them, which is what lifting the work cap
is worth when nothing can be handed over: the leading ROM blocks are handed
over, and everything after that is played to a loader running flat out.

**Astro Marine Corps** is the odd one on the shelf: 3,210 blocks, because
Dinamic wrote each of its parts as a long run of pulse blocks rather than as
data blocks. It still comes off identically either way.

**Blood Brothers** is the one that cannot be checked by its picture. It loads
the game and asks for the first module straight away, so when the tape stops it
is back in its own sampling loop with 216 bytes of screen drawn in its own
font, where the others draw a title. What can be said of it is that it left the
ROM for its own code and that pressing Play again feeds it the modules in
order, which is what its test does.

**Lotus and Space Crusade** are multiloads too, and both draw their titles
before the first stop.

## Still open: Head over Heels loads or does not, depending on when Play is pressed

Reported as "Head over Heels no longer loads with Max CPU or Fastload", and it
is neither of those things. Sweeping the moment Play is pressed across two
frames of machine time, with `LOAD ""` already running and the ROM sitting in
its edge loop at $05ED-$05F8 every time, about a third of the starts end with a
black screen and the machine in Speedlock's sampler.

What has been established by tracing a good start (wait 0) against a bad one
(wait 2):

- **The two load the same bytes.** Hooking the store in Speedlock's byte loop
  at $FE09, both runs put the same 47,866 bytes through IX, in the same order,
  to the end of the tape. Nothing is misread.
- **The two do the same things.** Comparing the loader's control flow — every
  jump target in $FC00-$FFFF, runs of the same target collapsed — they are
  identical for 2,934,480 transfers, differing only in how many turns a
  counting loop took: 159 against 164, 13 against 14, which is the contention
  jitter and not a decision going the other way.
- **The bad ones end waiting.** They sit at $FD24-$FD2A, the sampler's wait
  loop, after the tape has run out: the loader wants more pulses and there are
  none. Two hundred emulated seconds later they are still there.
- **Contention is what decides it.** With the ULA's delays taken out
  altogether, all twenty-four starts load. Shifting the contention table by up
  to two T-states either way changes nothing — the pattern of failures stays
  10 of 24 — so it is the jitter itself rather than where the table is
  anchored.
- **Speed has nothing to do with it.** The same sweep with the boost off gives
  the same pattern bar two marginal starts, and it fails identically at
  `bbe284f`, before this session's work on the deck.

One real inaccuracy was found on the way and fixed (see below): the EAR bit was
being sampled at the end of the `IN` rather than at its IORQ cycle. That moved
the failures from 10 of 24 to 8, and did not cure them.

What is left is the loader losing one pulse somewhere near the end of the tape
and never getting back in step. Whether our contended `IN` costs what a real
one does, T-state for T-state, is the thing to measure next; the tables match
the reference, but the reference is a table and not a machine.

### Where the EAR bit is sampled

`IN A,($FE)` on a contended port is stalled by the ULA and then takes its four
T-states. The byte is on the bus at the IORQ cycle, three T-states before the
instruction is over — and the tape was being read at the end instead, which is
late by the trailing T-states *plus whatever the ULA stalled that particular
read by*. That is up to seven T-states of lateness that moves about with the
beam: jitter of our own making, added to the jitter a loader is measuring
against.

The tape is now read at the IORQ cycle, which is what `sampled` in
`contend_io` has always been for — the floating bus used it and the EAR line
did not. `tests/reference_48k.rs` holds it.

## The block list follows the tape, until somebody else has hold of it

The list scrolls to keep the block being played in view, and stops doing that
the moment the reader scrolls it themselves: they are reading something, and
having it dragged away every time the tape moves on is the window arguing with
them. It starts again when what is playing comes back into view of its own
accord — the tape has caught them up, and following it is what the list is for.

Telling the reader's scrolling from the list's own is the awkward part. egui
animates its own scroll over several frames, so watching the position settle
does not separate them. Where they differ is where they *end*: the list only
ever scrolls to put the block being played in view, so a movement that leaves
it off the screen — with the pointer over the list — was not the list's doing.

## Starglider 2, and the edge that was never sent

The 48K side loaded its screen, then about two hundred of its 267-byte turbo
blocks, and then sat in the ROM's edge routine at $05F6 with the loading screen
up and the tape run out. Pressing Play again changed nothing.

It was passing as a game that loads. The check was "the machine is not in a
loader, and there is something on the screen": a loading screen is something on
the screen, and the ROM is not the address range the check called a loader. The
check now also refuses the ROM's tape routines $0530-$0620, and every other
game here passes it.

**What it was waiting for.** Traced by watching every entry into the ROM's
loading routines rather than only the calls at $0556 — this loader jumps in
part way, which is why the first look found nothing:

```
IN $0556 block   3  flag $FF len   183 to $5CCB   from $0805
IN $0556 block   3  flag $00 len    17 to $5D99   from $0771
IN $0556 block   4  flag $FF len   512 to $CD00   from $0805
IN $0562 block 206  flag $0F len  6912 to $4000   from $5DAD
IN $0562 block 212  flag $0F len  6912 to $4000   from $5DAD   (tape stopped)
```

Three ROM loads succeed; then, with the tape at block 206, the game enters
LD-BYTES **at $0562** — past the header handling — asking for 6,912 bytes at
$4000. That is the loading screen, and block 209 is exactly it. Counting the
bytes as they arrived showed the loader reading the whole of it: IX reaching
$5B00 and DE reaching zero, the entire screen in memory. It then hung on the
last byte.

**The last bit of a block was never closed.** A pulse ends where the next one
begins, so a block with no pause of its own is closed by the first pulse of the
block behind it — free, and why this went unnoticed for so long. Block 209 has
no pause and behind it is block 210, "Stop the tape", which divides side A from
the 128K version. Nothing followed, the line simply stopped at the level it was
holding, and the loader waited for ever for the edge that ends its last byte.
The second request at block 212 is that loader having been sent round again
with the deck now stopped, and that is the wait that looked like the freeze.

`Tape::after_data` emits the closing edge when the block behind is one that
stops the tape. The end of the tape is deliberately not the same case: there is
no loader left waiting by then, and an extra edge there changes what the last
block's own loader sees — City Slicker's screen came out 75 bytes different
between playing and hurrying when the first version of the fix sent one.

Three things were tried first and are worth writing down as not being it:

- **The EAR line hearing the loudspeaker while the tape runs.** A real EAR
  input sums the tape's signal with what the machine's own speaker feeds back,
  and in a gap the feedback is all there is; ours takes the tape while it is
  playing and the feedback only when it is stopped. Letting the feedback
  through during a gap breaks Head over Heels and both Dinamic games, whose
  loaders flash the border in the gaps and would then be reading their own
  border writes. A comparator summing a large signal with a small one is not
  an either/or, and modelling it as one is where that ends.
- **A hiss on the tape.** Entering at $0562 means the loader reads the EAR line
  once to decide which way round the edges are (`IN A,($FE) / RRA / AND $20 /
  OR $02 / LD C,A`), so a gap sitting at a level a real one would not could
  invert every edge after it. Switching the deck's noise on changed nothing:
  same place, same 1,945 bytes of screen.
- **Fastload.** Off made no difference, which was the clue that it was the deck
  rather than the hand-over: the same two entries at $0562, the same block.

## A deck that is not quite right

The tape window's Quality row makes the deck behave the way a real one did.
Two switches, four sliders, and nothing random about any of it: the wobbles are
sine waves read off the T-state clock and the grain and hiss are hashed from
it, so a given moment always gets the same treatment and a load can be
repeated.

**Wobble** is the motor, and wow and flutter scale every pulse. They have a
slider each because a deck can have either. Wow is the reel turning out of true — 0.037 Hz with a
slower drift at 0.012 Hz under it, so a period of tens of seconds — and flutter
is the capstan and the tape's own stiffness, at 1.4 Hz with a little at 6.3 Hz
over it. Each goes to five per cent of the right speed, which is already a deck
nobody would keep.

**Alignment** is a head out of square with the tape. It reads the top of the
track a moment before the bottom, and the two cancel each other the shorter the
wavelength gets: a low-pass whose corner comes down the further out of square
it is, from 10 kHz at one end of the slider to 250 Hz at the other. The top end
sits just above the quickest loaders on purpose — at 30 kHz the first half of
the slider was a dead run, since nothing on any tape is anywhere near that. The second slider
wanders that corner up and down, slowly — a head creeps, it does not shake.

It is the filter itself rather than a rule about it. A one-pole corner charges
towards the level the tape is holding, `y = u + (y₀ - u)e^(-t/τ)`, and the
reader flips when that gets past its threshold. So the roll-off is gradual in
the way a real one is:

| corner | short pulses (400 T) | long ones (800 T) | edges lost |
| --- | --- | --- | --- |
| square | 400 | 800 | none |
| 3.8 kHz | 395 | 805 | none |
| 2.4 kHz | 385 | 815 | none |
| 1.7 kHz | 375 | 825 | none |
| 1.3 kHz | — | 1200 | every short one |

The edges creep first — short pulses squeezed, long ones stretched, because the
filter holds a short pulse up more than a long one — and only when the swing no
longer reaches the reader's threshold do they start going missing. A reader
needs that threshold or it would chatter on noise, and it is what decides that
a rolled-off signal has become too small to read.

What that does to real tapes is take the quick loaders first, which is what a
misaligned deck did to a shelf of them:

| corner | Skool Daze (bits at 422 T, 4.1 kHz) | Chase H.Q. (bits at 735 T, 2.4 kHz) |
| --- | --- | --- |
| 4.8 kHz | loads | loads |
| 2.3 kHz | loads | loads |
| 1.6 kHz | **fails** | loads |

### The tape's own grain

A perfect signal fails all at once. Every pulse of a given length is the same
pulse, so at one slider position they all clear the threshold and one notch
along none of them does: a 400 T tone went from 875 edges to 1 between 0.54 and
0.56 of the slider, and what the user saw was a filter notched to particular
frequencies rather than a control that swept.

What was missing is the tape. It is oxide on plastic and its output is never
quite the same twice, so every pulse now carries a deterministic ±8 per cent of
its own — hashed from the T-state it starts at, so it has no period for a pulse
rate to beat against and a load still repeats exactly. The same sweep now goes
875, 847, 593, 289, 91, 9: a signal that fades over a stretch of the slider,
with the weakest pulses going first. That is also why the Chase H.Q. row above
moved from 1.1 kHz to 1.6 kHz — with grain in, a corner at 1.1 kHz takes both
loaders.

### Hiss

**Noise** sits to the right of them: tape hiss, two grains at different rates
so it is not a tone, scaled by its own slider. It is white noise laid over whatever the
line is holding — under a block's pulses, through the silence between blocks,
and on a tape held still. A real deck hisses from the moment the head touches
the tape, not from the moment there is something to read, so in a gap the hiss
is all there is and a loader waiting through one hears it.

It is heard as well as read. The reader only notices the hiss when it crosses
its threshold, so a quiet one made no sound at all where the tape was silent —
which is not what a tape sounds like with the volume up. The mixer is given the
hiss as a level and makes the noise itself, a sample at a time, so it is there
under the loading tone and through the gaps alike.

The hiss is not recorded, it is added where it is wanted. The deck keeps the
signal's corners and nothing else, because that is all the shape there is;
white noise has a value at every instant, so `Tape::scope_samples` resamples
the corners across the window the scope is showing — one sample a pixel — and
puts the hiss on at that resolution. Storing it instead would mean keeping
thousands of samples a block and then drawing straight lines between them,
which is a smoother and quieter hiss than the one the machine is hearing.

Which is why Pause and Stop are no longer the same call. Pause holds the tape
still with the head down, so the hiss carries on and the scope has something to
draw; Stop lifts the head, and then there is nothing at all. Turned up past the
reader's threshold the machine hears the hiss as edges, which is what a tape
played too loud does to a loader waiting through a gap.

The scope shows it. The deck keeps the signal's own shape as well as the
reader's edges — both ends of every pulse, and samples along the charging curve
when the head has rolled it off — so the trace is a square wave when nothing
has been done to it and rounds off as the corner comes down, with the reader's
squares faintly behind. When the swing stops reaching the threshold, the place
where an edge went missing is there to see.

A silence is drawn as nothing volts rather than as the level held low. The deck
holds the line low through a gap because that is what the reader is to make of
it, but there is no signal there at all, so the scope drew the gap between
blocks along the floor and put the hiss on the floor with it — the line only
came back to the middle if the tape was paused. A pulse the deck makes for
itself is marked as silent and the trace sits at nothing while it runs.

A deck standing still draws a flat line across the middle, or the hiss if the
tape is only paused. It used to hold the last block's reader level right across
the screen — a line at the top or the bottom that meant nothing, since a deck
that is not moving is not reading anything.

The tape window's default height went up with all this, to 900: the block list
is what is left after the cassette, the rows of controls and the scope, and at
780 the quality rows had taken the last of it. In the test harness that showed
up as the list's clip rectangle coming out inside out, and no row could be
hovered at all.

Both ends of every pulse matter: with one sample apiece the corners have
nothing joining them and a line drawn through them is a triangle wave, which is
what the scope drew until it was noticed.

And the motor, measured on Skool Daze: it loads with the speed wavering 2, 5
and 10 per cent, and fails at 20. What breaks a loader there is the total
length of a pulse pair moving, since that is what it times.

## The EAR line is never dead

Reading port $FE bit 6 with no tape playing does not give zero: the machine
hears its own loudspeaker. On an issue 3 board the bit follows bit 4 of the
last write to $FE; on an issue 2 it follows the MIC bit, bit 3, as well. The
emulator models both and has the MIC feedback on by default, with a switch —
**I2** — beside **Late** on a 48K's toolbar.

**Head over Heels does not load without it**, and the way it fails is worth
knowing because nothing about it looks like a tape problem. Every block of that
tape decodes byte for byte — all 48K of it, checked against the file — and then
the loader does this:

```text
$FECC  LD HL,$9000
$FECF  LD B,$FF
$FED1  PUSH BC
$FED2  CALL $FEDE     ; listen to the EAR line
$FED5  LD (HL),E      ; and write down what it heard
$FED6  INC HL
$FED8  DJNZ $FED1
```

`$FEDE` reads $7FFE two hundred and fifty-five times looking for bit 6 to
change. On a dead line it never does, the table at $9000 comes out all zeros,
and the loader calls $FD20 — which walks IY through the whole address space
writing zeros. A black screen and a machine that never comes back, which is
exactly what it is meant to look like when somebody has taken the tape away.

On a real machine the line is alive at that moment for either of two reasons:
the tape is still rolling long after its last block, which no tape file has
anything to say about, and the loudspeaker feeds back into the input besides.
Feeding a trailing tone into the deck by hand fixes it too, and gives the same
table — $9000 reads `77 04 C3 B7 92 DD 7E 0A` either way — so the check is
asking whether the line is alive rather than measuring what is on it.

## Three speeds, one at a time

Normal, Max CPU and Fastload, as one control rather than a pair of toggles.
They were never independent: handing blocks over means running the machine flat
out for the ones that cannot be handed over, so a pair left "Fastload without
Max CPU" to be explained away. Fastload is not offered on a ZX81, whose ROM has
no LD-BYTES to answer.

The top of the window is two labelled rows — **Playback**, which is start,
rewind, play, stop and forward, and **Speed**, which is the three above — and
five when the **Quality** switch on the speed row is on. Beside it is
**Oscilloscope**, which shows and hides the scope: it is on by default, since
what the deck is putting out is the point of watching a tape load, but it is a
third of the window's height and somebody working down the block list wants
that height for the list. The quality rows are: **Wobble** with the wow and
flutter sliders, **Alignment** with where the corner sits and how far it
wanders, and **Noise** with how loud the hiss is. A row each rather than
sharing: the window is a fixed width, and a row that wraps puts a slider under
the switch it has nothing to do with. They are put away by default because they
are three rows of a window the block list is at the bottom of, and most of the
time a deck that behaves is what is wanted.

Everything the window is set to is written to the preferences file and comes
back with the emulator: the two switches above, the sweep and trigger, which of
the three speeds is on, and every one of the deck's failings. A head out of
square by a particular amount and a hiss at a particular level are tedious to
dial in twice. The writing is left to the settling save, which only writes when
something has changed and only after two seconds of quiet, so dragging a slider
does not write the file forty times a second. The keys are `tape_*` in
`zx-rustrum.conf` and can be hand-edited like the rest of it; nonsense in one
is ignored rather than stopping the emulator starting.

## The silence at the end counts too

Max speed comes back to normal speed for the pause a tape ends on, so that a
loader finishing sounds and looks as it should. That left Fastload
crawling through it, because the budget it was capping came from the speed
setting, and with the boost off that budget is a fraction of a frame: Out Run
Europa ends with twenty-two seconds of silence, and 1,264 of its 1,341 host
frames were doing less than one frame of work each. In a hurry the budget is a
whole slice of work rather than whatever the speed setting asked for, and the
hurry lasts as long as the deck is running rather than as long as something is
being loaded. Out Run Europa: 1,341 host frames to 82.

## Stopping the deck where the tape does not

A tape stops where its author put a stop block, which is where the loader they
wrote wanted the deck to stop. Somebody taking a game apart wants it to stop
somewhere else, so hovering a row in the block list offers **⏸ Pause before**
at the right-hand end of it, and pressing it puts a stop-the-tape block —
`Block::Pause(0)` — in front of that block. The block being played goes on
being the block being played, whichever side of it the new one lands.

Two things about the row it sits on. The button is drawn into a child ui rather
than allocated, or every row in the list would grow by a button's height; and
whether the pointer is over the row is worked out from where the pointer is,
because the list is inside a scroll area whose layer is not the one
`rect_contains_pointer` reckons against — it says no over every row. The row's
own clip rectangle is checked too, so a row scrolled out of sight is not
offered.

A stop block that came from the list can go back out of it: hovering one offers
**✖ Delete** instead. The stops are the only blocks the list makes, so they are
the only ones it takes away — everything else is what the tape holds.

## A tape that stops itself

A multi-load carries a block that tells the deck to stop — TZX's pause block
with a length of zero, or a stop-if-48K. The program takes over there and asks
for the next part when it wants it. Gauntlet III does it half way through its
first side: the first part loads, the tape stops, the title and menu come up,
and the rest of the tape is for later.

A deck that stops on its own half way through a tape looks exactly like a load
that has gone wrong, so the window now says which it is. It is said again each
time rather than once a session.

Gauntlet III is also a 128K release, and it fails on a 48K the way it would on
a real one. It loads on a 128K at every speed: 12,072 host frames played, 711
at Max speed, 140 at Fastload. Shadow Dancer does the same thing and asks for
side B — 13,385 host frames played, 670 at Max, 26 at Fastload — and Out Run
Europa asks to have the tape stopped, which is what its twenty-two seconds of
trailing silence are for.

## Saving onto a blank tape

**Blank** in the tape window puts an empty tape in the deck and arms
`src/recorder.rs`, which keeps the T-state of every change of the MIC bit —
bit 3 of port $FE. When MIC has been still for half a second of the machine's
time the pulses are read back: a run of at least 256 pilot pulses between
1,800 and 2,600 T, two sync pulses under 1,000, then bits as pairs whose total
is either side of 2,565 (the ROM's pairs are 1,710 and 3,420). What comes out
is a standard block, flag and checksum included, and it is added to the tape
there and then. The pause in front of it is the silence that was in front of
it, so a `.tzx` written from it keeps the ROM's second between header and
data.

Only the ROM's timings are read. A saver with its own is counted in `unread`
and dropped, because a wrong block is worse than none. Reading back the
pulses rather than trapping SA-BYTES is deliberate: the same path works for a
program that calls the ROM from a page of its own, and nothing about the
machine is short-circuited.

One trap cost time. The frame's housekeeping first ran before the frame count
was moved on, where `tstates` has already had a frame taken off it — so the
clock read a frame early, the recorder took that for the clock going
backwards (which a reset or a snapshot does, and which drops what was half
heard), and every block vanished without a trace. `tests/tape_save.rs`, which
has the real ROM save a program and load it back, is what found it; the
pulse-train tests in `tests/recorder.rs` could not have.

Loading a tape switches the recorder off, so the next SAVE is not written onto
the end of a game.

## The ZX81

A ZX81 tape is a different format and the ZX81's ROM has no LD-BYTES, so
Fastload is a Spectrum switch only and is disabled while a ZX81 is
selected. The ZX81 loads at about fifty bytes a second, which makes Max speed
matter more there than anywhere else.
