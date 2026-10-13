# Building ZX-Rustrum

Most people want the [prebuilt release](../README.md#quick-start), and that's
the quicker way in. This page is for everyone else: building it yourself,
running the tests, cutting a release, and why the build is the shape it is.

## Building From Source

You need a stable Rust toolchain. It's built day to day with 1.97.1, and CI
uses whatever stable is current (1.98.1 at the time of writing). Then:

```
cargo run --release             # 48K
cargo run --release -- --128    # 128K
cargo run --release -- --plus2a # +2A
cargo run --release -- --plus3  # +3
cargo run --release -- --zx81   # ZX81 with 16K (--zx81-1k for the bare machine)
```

The first build takes a while, since the release profile uses thin LTO and the
training code brings a fair bit of `burn` with it. After that an edit rebuilds
in under a minute: 42 seconds here for a one-crate change.

Run from the project, the working directory is the first place ROMs are looked
for, so put the images in `roms/` at the top of the checkout and they're found
without asking. [Where ROMs Are Looked For](../README.md#where-roms-are-looked-for)
has the full list.

You can also name a file on the command line, and it's dispatched by extension:

```
cargo run --release -- "tapes/Jetpac (1983)(Ultimate Play The Game)[16K].tzx"
```

`.rom` replaces the ROM, `.tzx` and `.tap` load a tape, and `.sna`, `.z80` and
`.szx` load a snapshot. A `.p` brings up a ZX81 on its own, since there's
nothing else it could be for.

The tools under `src/bin/` (`zextest`, `mcp`, `trace`, `verify` and `corpus`)
aren't in the release archives, which carry only the emulator. Run them from a
checkout with `cargo run --release --bin <name> -- <args>`.

### Linux

`cpal` links `libasound.so.2`, so building needs `libasound2-dev`, and `gilrs`
finds gamepads through udev, so it needs `libudev-dev` too. On Debian or
Ubuntu:

```
sudo apt-get install libasound2-dev libudev-dev pkg-config
```

That's the same list CI installs. Without `libudev-dev` the gamepad crate's
build script can't find the library and nothing builds at all, which is a
confusing way to find out you wanted a joystick.

## Why wgpu and Not OpenGL

The renderer is `eframe`/`egui` on **wgpu**: Metal on macOS, Vulkan or DX12
elsewhere. The OpenGL path was tried first and dropped. On macOS it goes
through glutin, whose Cocoa backend panics with *"context to have a current
view"* when one of the extra windows loses its view. With a dozen extra
windows, this emulator found that out quickly.

If you're building somewhere wgpu can't find a backend, swap the `wgpu`
feature for `glow` in `Cargo.toml` and you're back on OpenGL. Just don't do it
on a Mac.

The training code uses wgpu too, through `burn`, and it asks for the same
wgpu 29 that `eframe` does, so there's still only one wgpu in the build.
[`training.md`](training.md) has why the graphics card was chosen over the
processor, with the measurements.

## How It Looks

The interface follows `zx-ux-mockup.html`: dark case plastic, near-black
outlines and the machine's own seven colours. It's monospace throughout, on the
grounds that the thing being emulated never displayed anything else.

`src/ui/theme.rs` holds the palette, and everything drawn by hand takes its
colours from there rather than from a literal. The debugger's registers sit on
a sunken LCD panel, the display has a bevelled surround, and the status line
ends with the seven-colour flash the machine wears on its case.

Two things in the mockup didn't make it. The title bars belong to the operating
system, so there are no rivets. And the headings use the built-in monospace
font rather than Press Start 2P, which would have meant shipping a font file.

`src/svg.rs` is a small SVG renderer, just enough for the drawings in
`designs/` (groups with matrix transforms, lines and cubic curves, rectangles,
circles, solid and linear-gradient fills, strokes, and the even-odd rule that
makes the cassette's window a hole). It isn't a general implementation and
isn't meant to become one. It exists so that re-exporting a drawing changes the
emulator without anyone redrawing it in code, and because no new crate can be
added to this build without being asked for (see
[`CLAUDE.md`](../CLAUDE.md)).

The window icon is the same reasoning again. `src/logo.rs` draws the
seven-colour flash as pixels, so nothing has to ship beside the binary, and
`packaging/make-icon.py` draws it larger for the application icon: a `ZX` over
the four Spectrum colour bars, written out as `icon.png` and a multi-size
`icon.ico`. It has no dependencies, not even Pillow. It rasterises the shapes
itself and writes the PNG chunks by hand.

## Tests

```
cargo test --release
```

There are 114 test files in `tests/`, named for what they cover, with each test
named as a sentence saying what behaviour it checks. A cold build of all of
them is heavy, which is why CI turns LTO off for tests (see
[Building a Release](#building-a-release)). Tests that need `roms/48.rom` or
`tapes/` skip themselves when those aren't there.

The ones that hold the emulator to the real machine are the ones to know about:

- `tests/halt2int.rs` boots a real 48K ROM, types `LOAD ""`, plays HALT2INT off
  tape, and reads the results back off the screen by matching character cells
  against the ROM font. It then compares them with the photographs of real
  hardware in both timing modes.
- `tests/border.rs` loads Border Break, waits for its border routine and
  compares the rendered border with rows taken from the photograph. It also
  checks that a loop alternating the border makes stripes 38 and 62 pixels
  wide, which are the 19 and 31 T-states that loop actually takes.
- `tests/timing.rs` checks T-state counts for around 60 instructions against
  the published timings, plus the interrupt and NMI acknowledge sequences,
  `EI`'s one-instruction shadow and contention.
- `tests/tape.rs` parses every tape in `tapes/` and checks the pulse trains
  against the ROM's timings. Then it does the real thing twice: once by calling
  the 48K ROM's `LD-BYTES` at `$0556`, and once by typing `LOAD ""` and loading
  a whole game from a real TZX.
- `tests/profiler.rs` checks time attribution against hand-counted T-states
  (two leaf functions at exactly 30 and 50 per call, a nested pair splitting 35
  self and 85 inclusive).
- The `ui_*` tests drive the interface itself through `egui_kittest`, clicking
  real buttons and checking that the thing actually changed. One early tape test
  was matching the toolbar's `100%` button instead of the progress bar it meant
  to check. That's why every test here is checked against the bug it guards:
  revert the fix and watch it fail.

`tests/audio_device.rs` opens your actual sound device, and skips itself if
there isn't one.

The dev profile builds with no debug information at all. On macOS a debug
build leaves its debug information in the object files rather than the binary,
so Cargo keeps every object of every test program it has ever built, and a few
weeks of test runs left `target/` at 47 GB. The comment in `Cargo.toml` has the
measurements. A failing test still tells you the file and line it failed at.

## Building a Release

`packaging/macos-app.sh` builds `ZX-Rustrum.app` and a `.dmg` around it,
generating the `.icns` from `packaging/icon.png` with `sips` and `iconutil`.
It signs both with the Developer ID named by `MACOS_SIGN_IDENTITY` under the
hardened runtime. If you give it an App Store Connect API key
(`MACOS_NOTARY_KEY`, `MACOS_NOTARY_KEY_ID`, `MACOS_NOTARY_ISSUER`), it
notarises both and staples the ticket to each. Without them you get an unsigned
app, which is fine on the machine it was built on.

The release workflow does all this from five repository secrets, and **won't
build a release without them**:

| Secret | What it is |
| --- | --- |
| `MACOS_CERTIFICATE` | The Developer ID Application certificate and its key, exported from Keychain Access as a `.p12`, then base64-encoded |
| `MACOS_CERTIFICATE_PASSWORD` | The password the `.p12` was exported with |
| `MACOS_NOTARY_KEY` | The text of an App Store Connect API key's `.p8` file |
| `MACOS_NOTARY_KEY_ID` | That key's ID |
| `MACOS_NOTARY_ISSUER` | The issuer ID shown above the list of keys |

It has to be a *Developer ID Application* certificate. An Apple Distribution
one is for the App Store, and Gatekeeper turns away a download signed with it.

`.github/workflows/release.yml` runs the tests, `cargo fmt --check` and `cargo
clippy -D warnings`, then packages a `.dmg` for each of the two macOS
architectures, a `.tar.gz` for Linux and a `.zip` for Windows.

**Not every push builds everything.** An ordinary push to main is tested on
Linux and nothing else. A push that only changes prose isn't built at all, so
editing this page costs nothing. Windows, macOS and the four packaging jobs run
only when there's something to release, or when you ask: `workflow_dispatch`
has a **full** switch for a complete build without a version bump. A full run
is seven jobs, three of them on macOS, and the 1.2.0 release took 29 minutes
from push to published.

On a pull request, a run overtaken by a newer push is cancelled. On main it
never is. Runs there are created minutes after the push that asks for them,
sometimes out of order, and cancelling in favour of whatever was queued last
once killed a nearly finished run of the newest commit for a run of an older
one. (An earlier version of the README said any overtaken run was cancelled.
Only pull requests are.)

Tests are built without link-time optimisation (`CARGO_PROFILE_RELEASE_LTO:
"false"`). Thin LTO over seventy-odd test binaries was most of the build: cold,
on this machine, `cargo test --release --no-run` took 2,194 seconds of CPU with
it and 374 without. The packaged binaries keep the release profile as it is.
`cargo fmt` and `cargo clippy` run on the Linux job alone, since their answer
doesn't change with the platform.

**Cutting a release is bumping `version` in `Cargo.toml`.** When a commit lands
on main with a version that has no `v<version>` tag yet, the workflow tags it
and publishes a release with the four builds attached, named for the version:
`zx-rustrum-1.2.0-linux-x86_64.tar.gz`, `ZX-Rustrum-1.2.0-macos-arm64.dmg`, and
so on. Pushing anything else to main tests it and publishes nothing, so a
version never means two different sets of binaries. You can still push a `v*`
tag by hand, but it's checked against `Cargo.toml` first. A tag naming a
version the tree doesn't have stops the build, rather than shipping binaries
that report a version they aren't.

Per platform:

- **macOS**: the binary links only system frameworks, so there's nothing to
  bundle beyond the app.
- **Linux**: `cpal` links `libasound.so.2`, and the file dialogs go through
  `xdg-desktop-portal`. An AppImage or Flatpak would carry both. The tarball
  expects the host to have them.
- **Windows**: built with `windows_subsystem = "windows"`, so no console window
  opens behind it. Cross-compiling from macOS isn't worth the trouble, so let
  the CI runner do it. The Windows build isn't signed.

No ROMs are shipped, so a packaged build starts with the machines greyed out
until ROMs are put where it looks.

## Source Layout

| Path | What |
| --- | --- |
| `src/z80/` | CPU: state, decode/execute, ALU and flag tables |
| `src/machine.rs` | Memory map, ULA timing, contention, I/O, run loop |
| `src/zx81.rs` | The ZX81: its ULA, display and memory decoding |
| `src/screen.rs`, `src/crt.rs` | Display rendering, and the tube and composite effects |
| `src/tracker.rs`, `src/detect.rs` | Access heat maps and back-buffer detection |
| `src/tape.rs`, `src/flashload.rs` | TZX/TAP parsing, pulse-level playback and Fastload |
| `src/audio.rs`, `src/audio_out.rs` | AY-3-8912, the mixer, and the cpal output device |
| `src/snapshot.rs`, `src/szx.rs`, `src/scr.rs` | Snapshot and screen formats |
| `src/rzx.rs`, `src/recorder.rs`, `src/video_out.rs` | RZX recording and playback, and video out through ffmpeg |
| `src/fdc.rs`, `src/disk.rs`, `src/ipf.rs` | The +3's µPD765A and its disk images |
| `src/if1.rs`, `src/microdrive.rs`, `src/multiface.rs`, `src/uspeech.rs`, `src/sp0256.rs`, `src/printer.rs`, `src/mouse.rs`, `src/joystick.rs` | What plugs into the back |
| `src/disasm.rs`, `src/asmexport.rs`, `src/profiler.rs`, `src/callgraph.rs`, `src/observe.rs` | Disassembler, assembly export, profiler, call graph, the observer |
| `src/training/` | The network, PPO, the judge and the score search |
| `src/prefs.rs`, `src/resources.rs` | Preferences, and where ROMs are looked for |
| `src/ui/` | The main window and every other window |
| `src/bin/` | `zextest`, `mcp`, `trace`, `verify`, `corpus` |
| `packaging/` | The macOS app script and the icon |
| `docs/` | What has been learned about the hardware and the subsystems |
