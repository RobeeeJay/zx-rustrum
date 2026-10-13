# A faster CPU against the same ULA

**Done, and on main.** This started as a plan, and the plan is kept below as
it was decided, because the reasoning is the useful part. Where what was built
differs from what was planned, it says so in place. What got built is the
**CPU** dropdown on the main window: up to 8× (28MHz on a 48K), held at 1×
while a tape or a recording is playing, with a line beside it saying which.
The two clocks themselves are written up in [`timing.md`](timing.md).

*Earlier versions of this file still opened with "A plan, not a change.
Nothing in this file is implemented yet" after all four stages below had been
merged. That was out of date from 19 August.*

Before this, the Clock dropdown (hidden then, behind `ui::SHOW_CLOCK`)
multiplied the whole machine: at 7MHz the frame was still 69,888 T-states and
those T-states went by twice as fast, so the interrupt came a hundred times a
second and a game reading the frame counter ran fast rather than smoothly.
That is a Spectrum with its crystal changed. What is wanted instead is an accelerator: the CPU
gets more cycles inside a frame of the ULA's own time, the picture is still
drawn fifty times a second, and a game's timing loops finish early rather than
its whole world speeding up.

## Why it is not a multiplier

Everything here counts one clock. `SpectrumBus::tstates` is the ULA's position
in the frame *and* the CPU's cost of the instruction it is running, and a dozen
things read it as one or the other:

| reads `tstates` as | what for |
| --- | --- |
| the ULA's position | the contention table, the floating bus, snow, the painted frame, the beam |
| elapsed real time | the tape's pulses, the mixer's samples, the observer's timestamps |
| the CPU's cost | `Z80` instruction timing, `run(budget)`, the profiler |

An accelerator splits the third from the first two. The CPU's cycles get
cheaper in ULA time; everything the ULA does, and everything measured in real
seconds, stays where it is.

## The model

Two clocks, one of them derived:

- **The ULA's clock** stays `tstates`: 69,888 to a 48K frame, 3.5MHz, and every
  table indexed by it keeps its meaning.
- **The CPU's clock** is `turbo` times that. An instruction that costs four
  CPU cycles costs `4 / turbo` ULA T-states.

Only whole multiples — 1, 2, 4, 8 — as the hardware offered, so the division is
exact with a remainder carried between instructions rather than a float:

```rust
/// CPU cycles that have not yet been paid for in ULA time.
cpu_debt: u32,

fn cpu_cycles(&mut self, cycles: u32) {
    self.cpu_debt += cycles;
    self.tstates += self.cpu_debt / self.turbo;
    self.cpu_debt %= self.turbo;
}
```

**There is no contention above 1×.** Decided rather than derived: an
accelerated machine is not sharing the ULA's bus on the ULA's terms any more,
and the switch exists to get work done rather than to reproduce a stall. At 1×
every delay is exactly what it always was — that is the machine, and the tests
that pin it down are the ones that matter. Above 1× the delay tables are not
consulted at all.

That makes the change smaller than it looked: `access` and `contend_addr`
already add the delay and the cycles as separate statements, so the delay
becomes conditional and the cycles go through `cpu_cycles`.

## What changes, by name

Everything is in `src/machine.rs` unless it says otherwise.

1. **`SpectrumBus`** gains `pub turbo: u32` (1 by default) and `cpu_debt: u32`,
   and a `fn cpu_cycles(&mut self, cycles: u32)` as above.
2. **`contended_addr(addr)`** answers no above 1×, which switches off memory
   contention everywhere it is asked — `access`, `contend_addr` and the M1
   fetch — from one place rather than three.
3. **`access(addr, t)`** and **`contend_addr(addr, times)`** — the cycles go
   through `cpu_cycles`; the delays are already behind the test above.
4. **`io_stall()`** returns without adding anything above 1×, which is the same
   switch for the four I/O patterns; the 1, 3 and 4 T-state costs in
   `contend_io` go through `cpu_cycles`.
   One thing to know: `sampled` — the T-state the ULA puts its byte on the bus,
   which the floating bus and the EAR line read — is taken part way through the
   pattern, after a cycle that is now fractional. The moment it names is
   therefore quantised to whole ULA T-states. That is right, since the ULA can
   only present a byte at its own rate whatever is asking, but it means two
   `IN`s in a row at 8× can read the same floating-bus byte where at 1× they
   read consecutive ones. Worth a test rather than a surprise.

5. **`set_model`** keeps `turbo` across a model change, as it keeps
   `tape_boost` and the rest.
6. **`Spectrum::run(budget)`** needs nothing: the budget is in ULA T-states, so
   a faster CPU does more instructions in the same budget and the emulator's
   pacing against the host clock is unchanged.
7. **`ui::App`** — `clock_mult` becomes `turbo`, pushed to the bus rather than
   multiplied into the work budget; `clock_hz` reports the CPU's clock for the
   dropdown's label; `apply_clock` stops touching the mixer, since the mixer
   counts ULA T-states and those still pass at 3.5MHz.
8. **`ui::SHOW_CLOCK`** comes out, and the dropdown says what it now is.
   *As built:* the dropdown says CPU, but the constant didn't come out. It's
   still there, set to `true`, and the comment above it still describes the
   old whole-machine multiplier as the reason it's hidden. That comment is
   wrong now and wants rewriting or removing along with the constant.

## Three places it has to be held at 1×

- **While a recording plays.** An RZX frame is a number of opcode fetches, and
  the recording's frame boundary is the video frame; a CPU getting through
  those fetches in a quarter of the ULA time would put four frames' worth of
  input into one frame of picture. `rzx.is_some()` should force 1× and say so.
- **While a tape is playing.** Measured, not assumed: at 4× Head over Heels
  loads nothing at all and the machine is left in the ROM's edge loop at $05ED
  when the tape has run out. Every loader — the ROM's own as much as a game's —
  counts turns of its own loop against pulses that are in ULA time, so at 4× it
  counts four times as many turns for the same pulse and every length it knows
  is wrong. That is what an accelerated machine does, which is why they had a
  switch; here the switch throws itself, and the CPU is held at 1× while the
  deck is playing.
- **Race the Beam and Cursor Beam.** Both replay a frame from its interrupt to
  a T-state; that still works, but "what the machine had done by this point in
  the frame" means something different when the CPU is running four times as
  fast, so the drawing wants a word saying which clock the cursor is in.

*As built:* the first two are held (`App::turbo_held_because`, "a recording
is playing" and "a tape is loading"). The third is not. The beam drawings run
at whatever the CPU is set to, and the word on the drawing hasn't been written.
It's under **Still open**.

## What it does to the parts that were not touched

- **Sound** comes out right without being told anything. The beeper is a level
  the program writes; at 7MHz a toggling loop comes round in half the ULA
  T-states, so the note is an octave up — which is what an accelerated machine
  sounded like — and the mixer, which counts ULA T-states, hears it happen.
- **The tape** loads at the same rate in seconds, since its pulses are in ULA
  time, and a loader counting them in CPU cycles will miscount exactly as it
  would on the real thing. Loaders are why accelerators had a switch.
- **The floating bus and snow** are read at the ULA's position, which is still
  the ULA's position. Whether a real accelerator snows the same way is not
  known here and would want a note rather than a guess.
- **The observer, the profiler and the call-flow timeline** timestamp in ULA
  T-states, so a turn of the loop still lands against the frame it ran in —
  which is the point of that drawing. A routine's instruction count is its own
  and does not change; its cost in T-states does, which is the thing being
  bought.
- **The interrupt** is raised at the frame boundary and held for 32 ULA
  T-states. At 8× the CPU can get through sixty-odd instructions inside that
  window rather than eight, which is what lets an accelerated machine answer an
  interrupt it would otherwise have missed. Nothing to change; something to
  check.

## Contention: decided

**None above 1×.** The three ways it could have gone were: stall as the ULA
always did; contend in CPU cycles, which is not a thing any hardware does; or
switch it off, which is what accelerated machines that only ran fast outside
the contended range amount to from the program's point of view. The last one is
what this will do, everywhere, above 1×.

What that means, said plainly:

- **A program that uses contention to keep time will not keep it.** The
  multicolour effects that count on a contended `LD` costing a known number of
  T-states are exactly the programs a turbo switch breaks, on a real machine as
  much as here. That is the trade the switch is for.
- **1× is untouched.** Every delay table, every I/O pattern, every number in
  `tests/reference_48k.rs` still applies to the machine as built, and the
  switch has no effect on any of it.
- **It is simpler and quicker.** The delay lookup on every contended access
  goes away above 1×, which is most of what the accelerated path costs.

## How it is kept honest

At 1× nothing may move: `tests/timing.rs`, `tests/reference_48k.rs`,
`tests/halt2int.rs` and the Border Break comparison are the ones that would
catch a mistake in the arithmetic, and they all run at 1×. That is the first
milestone — the mechanism in place, every existing test unchanged.

New tests, once it multiplies:

- A frame is still 69,888 T-states and the interrupt still comes 50.08 times a
  second at 2×, 4× and 8×.
- Twice the instructions in a frame at 2×; eight times at 8×, within the
  rounding the debt carries.
- A contended access costs nothing extra above 1×: `LD A,(HL)` on $4000 at the
  worst T-state of the frame costs the same as the same instruction on $8000,
  where at 1× it costs six T-states more.
- The four I/O patterns are the plain 4 T-states above 1×, and exactly what the
  reference says at 1×.
- The debt never loses a cycle: run a million instructions at 8× and the ULA
  time is the CPU time divided by eight, to the T-state.
- A tape still loads: Head over Heels at 4× reaches the same place, since the
  pulses are in ULA time.

## Staging

All four done, on `cpu-turbo`, and since merged into main:

1. **`cpu_cycles` and `turbo`, fixed at 1** (`7e6415e`). Nothing moved, which
   the timing tests, HALT2INT and the reference numbers decided.
2. **The multipliers, and seven tests** (`a5e813e`). The frame is still 69,888
   T-states at 8×; twice the instructions fit in one at 2×; a million
   instructions at 8× cost exactly an eighth of the time; a contended read
   costs what an uncontended one does above 1×.
3. **The dropdown** (`7090ab5`), saying CPU rather than Clock, with the two
   rules that hold it at 1× and a line in the row saying which is holding it.
4. **`docs/timing.md`** carries what the two clocks mean; this file is the
   record of how it was decided.

## What was learned doing it

**No tape loads with the CPU accelerated.** Expected in the abstract — the plan
said a loader counting pulses in CPU cycles would miscount — but the measurement
is worth having: at 4× Head over Heels reaches nothing at all and is left in the
ROM's edge loop at $05ED when the tape has run out. It is not only the game's
loader; the ROM's own counts turns of its loop the same way. Hence the rule
rather than a warning.

**The remainder matters more than it looks.** `LD A,(HL)` is seven cycles, and
seven does not divide by two, four or eight: without carrying the remainder a
machine at 8× would lose a fraction of a T-state on most instructions and run
perceptibly slow. The test that catches it runs a million instructions and
compares to the T-state.

## Still open

- **Snow at turbo.** The M1's last T-state falls in a different place in the
  ULA's eight-T-state cycle when the CPU is running faster, so a program that
  snows at 1× may not at 4×. What a real accelerated machine does is not known
  here, and guessing would be worse than saying so.
- **The beam drawings.** Race the Beam and Cursor Beam still replay a frame to
  a T-state, which is right; what "the machine had done by this point" means
  when the CPU is four times faster is a sentence somebody should write on the
  drawing.
