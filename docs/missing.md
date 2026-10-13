# What other emulators have that this one does not

An audit taken on 10 September 2026, against Fuse, ZEsarUX, Spectaculator,
CSpect and the wider 8-bit emulator crowd. Every absence below was checked
against this source tree rather than remembered — `grep` for the port, the
format, the file extension — so the list says what is not there rather than
what nobody has got round to mentioning.

It is a description of the gap, not a plan. Nothing here is promised, and
anything picked up should be taken on its own merits: several of these are an
afternoon and one of them is a month.

All five this list opened with have since been done: `.szx`, `.scr`, the
joystick interfaces, gamepads, and saving to tape (**Blank** in the tape
window records what the machine sends through MIC). So have the Kempston and
AMX mice and the ZX Printer and Alphacom 32, which weren't on it. What is left
is below, re-checked against the source on 13 October 2026.

**A correction.** The first version of this list said there was no video
capture. That was wrong when it was written: the video recorder
(`src/video_out.rs`, the video button under **Record**) had been there since 18
August, three weeks before the audit. A `grep` for `mp4` finds it at once, so
it wasn't searched for under the name it goes by. It's worth bearing in mind
for the other entries: an absence is only as good as the word that was looked
for.

## The ones you would feel first

**No divMMC/divIDE with esxDOS.** This is how people load things on real
hardware now: an SD card and a file browser. The biggest job on the list — an
IDE or SPI interface plus the esxDOS ROM's hooks — and the one that would make
the emulator feel current.

## Peripherals

| | |
| --- | --- |
| **Interface 2 cartridges** | `.rom` files and sixteen games. Perhaps forty lines of paging. Its two joystick ports are done (the Sinclair sticks in the Input window); the cartridge slot is not. |
| **Interface 1's RS232 and ZX Net** | Deliberately stubbed — nothing is on the other end of $F7 — but the network is what made microdrives interesting in a classroom. |
| **Beta 128 / TR-DOS** | `.trd` and `.scl`. Note that `roms/trdos.rom` is already in the ROM directory with nothing that can use it. |
| **+D and DISCiPLE** | `.mgt` images, and the snapshot button that made them popular. |
| **Opus Discovery** | The third of the disk systems, and the rarest. |

## Machines

16K, which some programs check for; the Spanish 128K; the +2B; **Timex
TC2048 and TS2068**, whose SCLD brings 512×192 and the hi-colour modes;
**Pentagon 128**, whose ROM is also already in the directory; Scorpion 256; and
the **ZX Spectrum Next**.

**ULAplus** deserves its own line: a 64-colour palette over any of those, small,
well specified, and used by a steady trickle of modern releases.

## Recording and media

RZX in and out is done, which is the hard one, and so is video: an `.mp4`
with the sound joined on, written through `ffmpeg`. What is missing is GIF
capture, WAV capture of the sound on its own, WAV and CSW tape *input* — real
tape audio, as opposed to the decoded formats — and PZX.

On the ZX81 side: ZonX AY sound, Chroma 81 colour, and the 64-column hack.

## The developer side

This is where the emulator is ahead of most: call flow, AutoDoc, the observer,
step-back, and the MCP server have no equivalent in Fuse or Spectaculator. What
the others still have that this does not:

- an **in-app assembler and monitor** (ZEsarUX and CSpect both);
- **conditional breakpoints** — the watches here are on kinds of event, plus one
  address range;
- **trace to a file**, meaning every instruction as it runs. The `trace` binary
  is not that: it plays a recording and writes down what each *routine* was
  seen to do, one line per routine rather than per instruction;
- **cheats**: `.pok` files, and a RAM search in the Debugger window. Half of
  that exists already but only over MCP, where `find_bytes` and `changed_since`
  are exactly the two halves of a cheat search.

What a language model would need to take a game apart through the MCP server
is a list of its own, in [`agent-disassembly.md`](agent-disassembly.md).

## Where to start

divMMC. It is the biggest job here and the one that would change how the
emulator is used most.
