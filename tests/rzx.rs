//! Playing back RZX recordings.
//!
//! The recordings themselves are somebody's play-through of a copyrighted
//! game, so like `roms/` and `tapes/` they are not in the repository: these
//! tests skip themselves when there is nothing to read.

use zx_rustrum::machine::Spectrum;
use zx_rustrum::notes::Notes;
use zx_rustrum::ui::{App, Roms};

fn recording(name: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(format!("recordings/{name}.rzx"));
    path.exists().then_some(path)
}

fn app() -> App {
    let roms = Roms {
        rom48: std::fs::read("roms/48.rom").ok(),
        rom128: std::fs::read("roms/128.rom").ok(),
        rom_plus3: std::fs::read("roms/plus3.rom").ok(),
        rom_zx81: None,
    };
    let mut app = App::with_roms(Spectrum::new(), String::new(), roms, None);
    app.show_ram_map = false;
    app.show_debugger = false;
    app.show_back_buffer = false;
    app.show_tape = false;
    app
}

/// A recording is a header, a snapshot and a long list of frames.
#[test]
fn a_recording_reads_as_a_snapshot_and_a_list_of_frames() {
    let Some(path) = recording("manic") else {
        return;
    };
    let recording = zx_rustrum::rzx::parse(&std::fs::read(path).unwrap()).unwrap();

    assert!(!recording.creator.is_empty(), "nobody made it?");
    let snapshot = recording
        .snapshot
        .clone()
        .expect("no snapshot to start from");
    assert_eq!(snapshot.extension, "z80");
    assert!(snapshot.data.len() > 1000, "the snapshot was not unpacked");
    assert!(
        recording.len() > 1000,
        "only {} frames: the frames were not unpacked",
        recording.len()
    );
    assert!(
        recording.frames.iter().any(|f| !f.inputs.is_empty()),
        "nothing was ever read from a port, which cannot be right"
    );
}

/// Something that is not a recording is turned away rather than half read.
#[test]
fn something_that_is_not_a_recording_is_refused() {
    let error = zx_rustrum::rzx::parse(b"not an RZX file at all").unwrap_err();
    assert!(error.contains("not an RZX"), "{error}");
}

/// What it ran, not what it was asked for.
///
/// Instructions are run whole, so asking for one fetch and getting a prefixed
/// instruction runs two. A caller playing a recorded frame in several goes
/// takes the count off what is left of the frame, and a count that stops at
/// what was asked for loses the overshoot every time — so the frame is thought
/// to have further to run than it has, and runs on into input that was never
/// recorded.
#[test]
fn a_fetch_count_says_what_was_run() {
    let mut spec = Spectrum::new();
    spec.bus.poke(0x8000, 0xED); // LD A,R: the prefix and the opcode
    spec.bus.poke(0x8001, 0x5F);
    spec.cpu.pc = 0x8000;

    let (_, ran) = spec.run_fetches(1);
    assert_eq!(
        ran, 2,
        "one fetch was asked for and a two-fetch instruction was run, which is          two fetches however many were wanted"
    );
}

/// Playing one back runs the machine along the path it took when it was
/// recorded, which is what makes it worth having.
#[test]
fn a_recording_plays_back_without_coming_adrift() {
    let Some(path) = recording("manic") else {
        return;
    };
    let mut app = app();
    app.load_path(&path);
    assert!(app.rzx.is_some(), "it did not start: {}", app.status);

    for _ in 0..2000 {
        app.advance(1.0 / 50.08);
    }

    let played = app.rzx.as_ref().unwrap().frame;
    assert!(played > 1500, "only got to frame {played}");
    let short = app.spec.bus.playback.as_ref().unwrap().short;
    assert_eq!(
        short, 0,
        "the program read from the ports {short} times more than the \
         recording holds, so it is no longer following the recorded path"
    );
    let drawn = (0x4000..0x5800u32)
        .filter(|a| app.peek(*a as u16) != 0)
        .count();
    assert!(drawn > 500, "only {drawn} bytes of screen written");
}

/// A frame is measured in opcode fetches, not in instructions: a prefixed
/// instruction is two or more fetches, and counting whole instructions runs
/// past the end of every frame and reads input that was never recorded.
#[test]
fn a_frame_is_counted_in_opcode_fetches() {
    let mut spec = Spectrum::new();
    // LD A,R is two fetches: the ED prefix and the opcode.
    spec.bus.poke(0x8000, 0xED);
    spec.bus.poke(0x8001, 0x5F);
    spec.bus.poke(0x8002, 0x00); // NOP, one fetch
    spec.cpu.pc = 0x8000;

    let before = spec.bus.fetches;
    let (_, ran) = spec.run_fetches(2);
    assert_eq!(
        spec.bus.fetches - before,
        2,
        "LD A,R should have used up the whole two-fetch budget"
    );
    assert_eq!(ran, 2);
    assert_eq!(
        spec.cpu.pc, 0x8002,
        "and it should have run one instruction"
    );
}

/// The keyboard belongs to the recording while one is playing: what the user
/// types would fight with what was recorded.
#[test]
fn the_keyboard_is_the_recordings_while_it_plays() {
    let source = include_str!("../src/ui/mod.rs");
    let read_keyboard = source
        .split("fn read_keyboard")
        .nth(1)
        .expect("read_keyboard has gone");
    let guard = read_keyboard
        .split("ctx.input")
        .next()
        .expect("it reads no input at all now");
    assert!(
        guard.contains("self.rzx.is_some()"),
        "the host keyboard is still read while a recording plays"
    );
}

/// Notes go beside the recording: it is what is being read.
#[test]
fn notes_are_kept_beside_the_recording() {
    let Some(path) = recording("manic") else {
        return;
    };
    let mut app = app();
    app.tape_path = Some("tapes/somethingelse.tap".into());
    app.load_path(&path);

    assert_eq!(
        app.notes.file(),
        Some(Notes::sidecar(&path).as_path()),
        "the notes should sit beside the recording, not the tape"
    );

    // And they go back to the tape's when the recording is stopped.
    app.stop_playback();
    assert_eq!(
        app.notes.file(),
        Some(Notes::sidecar(std::path::Path::new("tapes/somethingelse.tap")).as_path())
    );
}

/// AutoDoc reads where the recording actually went. Static reading has to
/// guess which bytes are code; a recording says where the program really got
/// to, past the loader and the protection.
#[test]
fn a_recording_tells_autodoc_where_the_program_goes() {
    let Some(path) = recording("manic") else {
        return;
    };
    let mut app = app();
    app.load_path(&path);
    for _ in 0..600 {
        app.advance(1.0 / 50.08);
    }

    let visited = &app.rzx.as_ref().unwrap().visited;
    assert!(
        visited.len() > 20,
        "only {} places reached in 600 frames",
        visited.len()
    );

    app.spec.bus.observer.enabled = true;
    let doc = {
        let entries: Vec<u16> = visited.iter().copied().collect();
        let peek = |a: u16| app.peek(a);
        zx_rustrum::autodoc::analyse(&peek, &entries)
    };
    assert!(
        doc.labels.len() > 5,
        "the recording's own path through the game yielded only {} routines",
        doc.labels.len()
    );
}

/// The file picker does not filter recordings by extension.
///
/// rfd's macOS backend hands the extension list to the panel through an API
/// that wants types the system knows about, and nothing on a Mac claims
/// `.rzx`: with a filter set, the recordings show up greyed out and cannot be
/// chosen at all. The file is checked when it is opened instead.
#[test]
fn the_picker_does_not_filter_recordings_out_of_existence() {
    let source = include_str!("../src/ui/mod.rs");
    let picker = source
        .split("pub fn pick_file")
        .nth(1)
        .expect("pick_file has gone");
    let arm = picker
        .split("Some(FileKind::Recording) =>")
        .nth(1)
        .expect("the picker no longer knows about recordings");
    let arm = arm.split(',').next().unwrap_or_default();
    assert!(
        !arm.contains("add_filter"),
        "a filter on .rzx makes the recordings unselectable on macOS: {arm}"
    );
}

/// Anything that is not a recording still gets turned away when it is opened.
#[test]
fn opening_something_that_is_not_a_recording_says_so() {
    let dir = std::env::temp_dir().join(format!("zxrs-rzx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("notreally.rzx");
    std::fs::write(&path, b"this is not a recording").unwrap();

    let mut app = app();
    app.load_path(&path);

    assert!(app.rzx.is_none(), "it should not have started playing");
    assert!(
        app.status.contains("not an RZX"),
        "and it should say why: {}",
        app.status
    );
}

/// The main window says a recording is playing, how far through it is, and
/// offers the choice between the speed it was played at and as fast as this
/// machine will go.
#[test]
fn the_main_window_says_a_recording_is_playing() {
    use egui_kittest::kittest::Queryable;

    let Some(path) = recording("manic") else {
        return;
    };
    let mut app = app();
    app.load_path(&path);

    let mut harness = egui_kittest::Harness::builder()
        .with_size([1600.0, 900.0])
        .build_ui_state(|ui, app: &mut App| app.draw(ui), app);
    harness.run_steps(3);

    let text = every_string(&harness);
    assert!(
        text.iter().any(|t| t.contains("REPLAY")),
        "nothing says a recording is playing: {text:?}"
    );
    assert!(
        text.iter().any(|t| t.contains('%')),
        "and nothing says how far through it is"
    );

    // The toggle is off to start with: a recording plays at the speed it was
    // played at unless asked otherwise.
    assert!(!harness.state().rzx.as_ref().unwrap().max_speed);
    harness.get_by_label("Max speed").click();
    harness.run_steps(2);
    assert!(
        harness.state().rzx.as_ref().unwrap().max_speed,
        "the toggle did nothing"
    );
}

/// And at maximum speed it really does get through more of the recording.
#[test]
fn maximum_speed_plays_more_of_the_recording() {
    let Some(path) = recording("manic") else {
        return;
    };

    let mut realtime = app();
    realtime.load_path(&path);
    for _ in 0..40 {
        realtime.advance(1.0 / 50.08);
    }
    let played_realtime = realtime.rzx.as_ref().unwrap().frame;

    let mut flat_out = app();
    flat_out.load_path(&path);
    flat_out.rzx.as_mut().unwrap().max_speed = true;
    for _ in 0..40 {
        flat_out.advance(1.0 / 50.08);
    }
    let played_fast = flat_out.rzx.as_ref().unwrap().frame;

    assert!(
        played_fast > played_realtime * 4,
        "maximum speed managed {played_fast} frames against {played_realtime} \
         at the speed it was recorded"
    );
    // And it is still following the recording, not just running.
    let short = flat_out.spec.bus.playback.as_ref().unwrap().short;
    assert_eq!(short, 0, "it came adrift when hurried along");
}

fn every_string(h: &egui_kittest::Harness<'_, App>) -> Vec<String> {
    use egui_kittest::kittest::NodeT;
    fn walk(node: &egui_kittest::Node<'_>, out: &mut Vec<String>) {
        for text in [node.accesskit_node().label(), node.accesskit_node().value()]
            .into_iter()
            .flatten()
        {
            out.push(text.to_string());
        }
        for child in node.children() {
            walk(&child, out);
        }
    }
    let mut found = Vec::new();
    walk(&h.root(), &mut found);
    found
}

/// Opening a recording remembers where it came from, so the next one opens
/// there. Nothing did, so the picker fell back to wherever a snapshot or a
/// tape was last opened — which for most people is not where they keep
/// recordings.
#[test]
fn loading_a_recording_remembers_its_directory() {
    use zx_rustrum::prefs::FileKind;

    let path = std::path::PathBuf::from("recordings/manic.rzx");
    if !path.exists() {
        return;
    }
    let mut app = app();
    assert_ne!(
        app.prefs.dir_for(FileKind::Recording).map(|d| d.as_path()),
        Some(std::path::Path::new("recordings")),
        "the test would prove nothing if it were already set"
    );

    app.load_path(&path);
    assert!(app.rzx.is_some(), "the recording should have loaded");
    assert_eq!(
        app.prefs.dir_for(FileKind::Recording).map(|d| d.as_path()),
        Some(std::path::Path::new("recordings")),
        "and the next Load recording should open where this one came from"
    );
}

/// The interrupt a recording asks for is taken, and one the machine misses on
/// its own is dropped.
///
/// The ULA holds the interrupt line down for a few dozen T-states and then
/// lets it go, so a program with interrupts disabled across the top of a frame
/// misses that one. That window is measured against the frame while the
/// machine runs on its own — but a recording's frames are counted in opcode
/// fetches and wander away from the T-state frame, so there it is the moment
/// the recording asked and no later. Measured against the frame in both worlds, the
/// interrupt a recording asked for was thrown away as missed and the machine
/// ran on without ever taking one.
#[test]
fn the_interrupt_window_is_timed_from_when_the_line_goes_down() {
    use zx_rustrum::machine::{Spectrum, IRQ_LEN};

    // EI, then a stretch of NOPs to run through.
    let mut spec = Spectrum::new();
    spec.bus.poke(0x8000, 0xFB);
    for at in 0x8001..0x8100u16 {
        spec.bus.poke(at, 0x00);
    }
    spec.cpu.pc = 0x8000;
    spec.cpu.sp = 0xFF00;
    spec.cpu.im = 1;
    spec.step_instruction(); // EI, which defers the interrupt by one
    spec.step_instruction();

    // Well past the top of the frame, where a recording's boundary can fall.
    while spec.bus.tstates < IRQ_LEN * 4 {
        spec.step_instruction();
    }
    spec.bus.playback = Some(zx_rustrum::machine::Playback::default());
    spec.bus.raise_interrupt();
    spec.run_fetches(1);
    assert_eq!(
        spec.cpu.pc, 0x0038,
        "the interrupt was asked for here and should have been taken here"
    );

    // And one the machine misses on its own is let go, as the ULA lets it go.
    let mut spec = Spectrum::new();
    for at in 0x8000..0x8100u16 {
        spec.bus.poke(at, 0x00);
    }
    spec.cpu.pc = 0x8000;
    spec.cpu.sp = 0xFF00;
    spec.cpu.im = 1;
    spec.cpu.iff1 = false;
    spec.bus.raise_interrupt();
    while spec.bus.tstates < IRQ_LEN * 2 {
        spec.run_fetches(1);
    }
    spec.cpu.iff1 = true;
    spec.run_fetches(2);
    assert!(
        spec.cpu.pc >= 0x8000,
        "the line was let go long before this, so nothing should be waiting \
         to fire into ${:04X}",
        spec.cpu.pc
    );
}

/// A recording plays back exactly: every frame the machine reads what the
/// recording holds for it, and no more. Reading more than was recorded means
/// the machine has taken a path the recording never took.
///
/// Two thousand frames, which is forty seconds of play. This comment used to
/// say that Space Harrier stayed in step for 7,891 frames and Vindicator for
/// 13,328 and then drifted "for something this does not yet explain". The
/// something was the interrupt window: a recording's boundary is where the
/// instruction count ran out, not where the line went down, and giving the
/// interrupt thirty-two T-states from there let a program that enabled
/// interrupts just after it take one the recording says was never taken.
/// Jetpac parted company that way at frame 1,444. Out Run went wrong by the
/// second frame for another reason — it carries a snapshot every few
/// thousand frames, and the last was being played from the start — and Tank
/// Command's snapshot is a .szx, which did not load at all.
#[test]
fn the_recordings_play_back_without_coming_adrift() {
    for name in [
        "manic.rzx",
        "spaceharrier.rzx",
        "vindicator.zip",
        "Jetpac.rzx",
        "outrun.rzx",
        "tankcommand.rzx",
    ] {
        let path = std::path::PathBuf::from("recordings").join(name);
        if !path.exists() {
            continue;
        }
        let mut app = app();
        // The ROM matters: a game with IM 1 spends every frame in the ROM's
        // interrupt handler, and without one it sits on $FF at $0038 forever.
        if let Some(rom) = app.roms.rom48.clone() {
            app.spec.load_rom(&rom);
        }
        app.load_path(&path);
        assert!(app.rzx.is_some(), "{name} did not load: {}", app.status);
        for _ in 0..2000 {
            app.advance(1.0 / 50.0);
        }
        let played = app.rzx.as_ref().map(|rzx| rzx.frame).unwrap_or(0);
        assert!(played > 1500, "{name} only played {played} frames");
        let playback = app.spec.bus.playback.as_ref().expect("still playing");
        // Both halves matter: reading more than was recorded means the
        // machine went somewhere the recording never went, and reading less
        // means it never got to the code that reads at all — which is what a
        // missed interrupt looks like, and which a count of overruns alone
        // would call perfect.
        assert_eq!(
            (playback.short, playback.unread),
            (0, 0),
            "{name} read {} bytes the recording did not hold and left input \
             unread in {} of {played} frames",
            playback.short,
            playback.unread
        );
    }
}

/// The interrupt a recording asks for is taken at its boundary or not at all.
///
/// The boundary is where the instruction count ran out; the line went down on
/// the recording machine before that, during or before the last instruction,
/// so by the time a program three instructions on enables interrupts the line
/// has gone. Jetpac does exactly this at $69A2 — XOR A, LD ($5DD5),A, EI —
/// and the recording says the frame after it read nothing at all.
#[test]
fn a_recorded_interrupt_is_not_taken_by_an_ei_a_few_instructions_later() {
    let mut spec = Spectrum::new();
    for at in 0x8000..0x8100u16 {
        spec.bus.poke(at, 0x00);
    }
    spec.bus.poke(0x8002, 0xFB); // EI after two NOPs
    spec.cpu.pc = 0x8000;
    spec.cpu.sp = 0xFF00;
    spec.cpu.im = 1;
    spec.cpu.iff1 = false;
    spec.bus.playback = Some(zx_rustrum::machine::Playback::default());
    spec.bus.raise_interrupt();
    spec.run_fetches(10);
    assert!(
        spec.cpu.iff1 && spec.cpu.sp == 0xFF00,
        "the interrupt asked for at the boundary was taken after the EI two \
         instructions on (PC ${:04X}, SP ${:04X})",
        spec.cpu.pc,
        spec.cpu.sp
    );
}

/// An EI holds an interrupt off for one instruction on any Z80, so one that
/// ends a recorded frame does not lose the next frame's interrupt: it is
/// taken after the instruction that follows.
#[test]
fn an_ei_ending_a_recorded_frame_puts_its_interrupt_off_one_instruction() {
    let mut spec = Spectrum::new();
    for at in 0x8000..0x8100u16 {
        spec.bus.poke(at, 0x00);
    }
    spec.bus.poke(0x8000, 0xFB);
    spec.cpu.pc = 0x8000;
    spec.cpu.sp = 0xFF00;
    spec.cpu.im = 1;
    spec.bus.playback = Some(zx_rustrum::machine::Playback::default());
    spec.run_fetches(1); // the EI, as the last instruction of a frame
    spec.bus.raise_interrupt();
    spec.run_fetches(1); // the NOP it defers the interrupt past
    spec.run_fetches(1);
    let pushed = spec.bus.peek_raw(0xFEFE) as u16 | (spec.bus.peek_raw(0xFEFF) as u16) << 8;
    assert_eq!(
        pushed, 0x8002,
        "the interrupt should have been taken after the NOP at $8001, \
         pushing $8002 (SP ${:04X}, PC ${:04X})",
        spec.cpu.sp, spec.cpu.pc
    );
}

/// The snapshots further into a recording replace the machine where they fall.
///
/// SPIN loads a tape faster than the machine could and cannot record the
/// instructions it skipped, so it writes the machine down afterwards and the
/// frames go on from there: Out Run carries one every few thousand frames.
/// Keeping only the last of them, as this once did, started the game from
/// where the player stopped and fed it the input from the beginning. The
/// first snapshot here is a .szx, as Tank Command's is, which did not load at
/// all. And what is known about which code has run is kept across the later
/// one: it is the same run, and a .sna forgets it on loading.
#[test]
fn a_snapshot_further_into_a_recording_replaces_the_machine_where_it_falls() {
    // A machine sitting in a loop of NOPs with interrupts off.
    let mut first = Spectrum::new();
    for at in 0x8000..0x8100u16 {
        first.bus.poke(at, 0x00);
    }
    first.bus.poke(0x8100, 0xC3); // JP $8000
    first.bus.poke(0x8101, 0x00);
    first.bus.poke(0x8102, 0x80);
    first.cpu.pc = 0x8000;
    first.cpu.sp = 0xFF00;
    first.cpu.iff1 = false;
    let mut later = first.clone();
    later.bus.poke(0x9000, 0xAB);
    later.bus.poke(0x9001, 0x18); // JR -2, somewhere the first never went
    later.bus.poke(0x9002, 0xFE);
    later.cpu.pc = 0x9001;

    fn block(out: &mut Vec<u8>, id: u8, body: &[u8]) {
        out.push(id);
        out.extend_from_slice(&(body.len() as u32 + 5).to_le_bytes());
        out.extend_from_slice(body);
    }
    fn snapshot(ext: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut body = 0u32.to_le_bytes().to_vec();
        body.extend_from_slice(ext);
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        body
    }
    fn frames(count: u32) -> Vec<u8> {
        let mut body = count.to_le_bytes().to_vec();
        body.push(0);
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..count {
            body.extend_from_slice(&100u16.to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes());
        }
        body
    }
    let mut rzx = b"RZX!\x00\x0d".to_vec();
    rzx.extend_from_slice(&0u32.to_le_bytes());
    block(
        &mut rzx,
        0x30,
        &snapshot(b"szx\0", &zx_rustrum::szx::save(&first)),
    );
    block(&mut rzx, 0x80, &frames(2));
    block(
        &mut rzx,
        0x30,
        &snapshot(b"sna\0", &zx_rustrum::snapshot::save_sna(&later)),
    );
    block(&mut rzx, 0x80, &frames(2));

    let recording = zx_rustrum::rzx::parse(&rzx).expect("it should read");
    assert_eq!(
        recording
            .later
            .iter()
            .map(|(at, s)| (*at, s.extension.clone()))
            .collect::<Vec<_>>(),
        vec![(2, "sna".to_string())],
        "the second snapshot should come before frame 2"
    );

    let path = std::env::temp_dir().join("zxrs-two-snapshots.rzx");
    std::fs::write(&path, &rzx).unwrap();
    let mut app = app();
    app.load_path(&path);
    assert!(app.rzx.is_some(), "it did not load: {}", app.status);
    app.rzx.as_mut().unwrap().max_speed = true;
    app.running = true;
    app.advance(1.0 / 50.0);
    let rzx = app.rzx.as_ref().unwrap();
    assert!(rzx.finished(), "only got to frame {}", rzx.frame);
    assert_eq!(
        (app.peek(0x9000), app.spec.cpu.pc),
        (0xAB, 0x9001),
        "the machine should be the later snapshot's, looping at $9001"
    );
    let ran = |addr: u16| app.spec.bus.tracker.executed[app.spec.bus.phys_index(addr)];
    assert!(
        ran(0x8000) && ran(0x9001),
        "both halves of the run should be known to have run: $8000 {}, $9001 {}",
        ran(0x8000),
        ran(0x9001)
    );
}

/// A reset puts the T-state counter back to nothing, and the interrupt window
/// has to go with it.
///
/// Timing the ordinary case from a stored moment left that number behind after
/// a reset — which is what loading a tape does — with the counter back at zero
/// and the stored moment in the future. The subtraction saturated, nothing was
/// ever missed, and every interrupt was taken wherever in the frame the
/// program happened to enable them: a game drawn against the interrupt then
/// draws against nothing, which looks like the picture tearing itself apart.
#[test]
fn a_reset_does_not_leave_the_interrupt_window_open() {
    use zx_rustrum::machine::{Spectrum, FRAME_T, IRQ_LEN};

    let mut spec = Spectrum::new();
    for at in 0x8000..0x8100u16 {
        spec.bus.poke(at, 0x00);
    }

    // Run for a while, so anything remembered about the interrupt is from a
    // frame far in the past, and then reset as loading a tape does.
    spec.cpu.pc = 0x8000;
    spec.cpu.sp = 0xFF00;
    for _ in 0..8 {
        spec.run(FRAME_T);
    }
    spec.reset();

    // A program that enables interrupts in the middle of a frame, long after
    // the ULA would have let the line go.
    for at in 0x8000..0x8100u16 {
        spec.bus.poke(at, 0x00);
    }
    spec.cpu.pc = 0x8000;
    spec.cpu.sp = 0xFF00;
    spec.cpu.im = 1;
    spec.cpu.iff1 = true;
    // Well past the top of the frame first: an interrupt asked for up there is
    // taken, and rightly.
    while spec.bus.tstates < IRQ_LEN * 8 {
        spec.run_fetches(1);
    }
    spec.bus.irq_pending = true;
    spec.run_fetches(2);

    assert_ne!(
        spec.cpu.pc, 0x0038,
        "the line was let go thousands of T-states ago and this interrupt \
         should have gone with it"
    );
}

/// Playing back a recording of a real machine puts nothing on the screen that
/// comes and goes.
///
/// A byte that alternates between two values frame after frame is text or a
/// sprite being drawn and rubbed out again, which is what flickering looks
/// like from the inside. SPIN's recording of Space Harrier on real hardware
/// has none of it, so neither should our replay of it.
#[test]
fn a_recording_of_real_hardware_does_not_flicker_on_replay() {
    let path = std::path::PathBuf::from("recordings/spaceharrier.rzx");
    if !path.exists() {
        return;
    }
    let mut app = app();
    if let Some(rom) = app.roms.rom48.clone() {
        app.spec.load_rom(&rom);
    }
    app.load_path(&path);
    if app.rzx.is_none() {
        return;
    }

    // Well inside the stretch that replays exactly, and in the game rather
    // than on a menu.
    while app.rzx.as_ref().map(|rzx| rzx.frame).unwrap_or(0) < 5000 {
        app.advance(1.0 / 50.0);
    }

    let mut frames: Vec<Vec<u8>> = Vec::new();
    for _ in 0..10 {
        app.advance(1.0 / 50.0);
        frames.push((0x4000..0x5B00u32).map(|at| app.peek(at as u16)).collect());
    }
    let flickering = (0..frames[0].len())
        .filter(|at| {
            let values: Vec<u8> = frames.iter().map(|frame| frame[*at]).collect();
            values
                .windows(3)
                .all(|three| three[0] == three[2] && three[0] != three[1])
        })
        .count();

    assert_eq!(
        flickering, 0,
        "{flickering} bytes of the screen went back and forth every frame, and \
         on the machine this was recorded from none did"
    );
}

/// A machine that was paused stays paused when a recording is loaded into it.
///
/// Somebody who stopped the machine to look at something has not asked for a
/// recording to start running the moment it arrives — and the frame it starts
/// on is worth looking at. A machine that was running plays it straight away,
/// as before.
#[test]
fn a_paused_machine_stays_paused_when_a_recording_is_loaded() {
    let path = std::path::PathBuf::from("recordings/manic.rzx");
    if !path.exists() {
        return;
    }

    let mut paused = app();
    paused.running = false;
    paused.load_path(&path);
    assert!(paused.rzx.is_some(), "the recording should have loaded");
    assert!(
        !paused.running,
        "it was paused before, so it should still be paused"
    );
    assert!(
        paused.status.contains("paused"),
        "and say so: {:?}",
        paused.status
    );

    // The machine is ready: the snapshot is in and the first frame is waiting.
    assert_eq!(
        paused.rzx.as_ref().map(|rzx| rzx.frame),
        Some(0),
        "at the first frame of the recording"
    );
    assert!(
        paused.spec.bus.playback.is_some(),
        "with the recording's input ready to be handed out"
    );

    // And pressing Run plays it.
    paused.running = true;
    for _ in 0..10 {
        paused.advance(1.0 / 50.0);
    }
    assert!(
        paused.rzx.as_ref().is_some_and(|rzx| rzx.frame > 0),
        "once started, it should play"
    );

    // A machine that was running does not stop to ask.
    let mut running = app();
    running.running = true;
    running.load_path(&path);
    assert!(running.running, "it was running, so it plays straight away");
}

/// While a recording plays, its frames are the machine's frames.
///
/// A recording counts a frame in opcode fetches, and on the machine it was made
/// on that boundary was the start of a video frame — that is where the ULA's
/// interrupt came from. Letting the T-state frame run on beside it lets the two
/// drift apart, and then everything timed against the picture happens at the
/// wrong height: the interrupt was going off with the beam two hundred lines
/// down, part-way through the screen it was meant to be starting.
#[test]
fn the_recordings_frame_is_the_video_frame() {
    let path = std::path::PathBuf::from("recordings/manic.rzx");
    if !path.exists() {
        return;
    }
    let mut app = app();
    if let Some(rom) = app.roms.rom48.clone() {
        app.spec.load_rom(&rom);
    }
    app.load_path(&path);
    if app.rzx.is_none() {
        return;
    }
    app.running = true;

    // A frame of the recording per call, so each one ends on a boundary the
    // recording set. The video frame should have started again there.
    let view = app.view();
    for n in 0..40 {
        app.advance(1.0 / 50.0);
        let t = app.spec.bus.tstates;
        assert!(
            t < app.spec.bus.model.t_per_line(),
            "frame {n} of the recording ended at T {t}, which is {} lines into \
             a video frame that should have just begun",
            t / app.spec.bus.model.t_per_line()
        );
        let (_, y) = zx_rustrum::screen::pixel_at_t(
            view,
            app.spec.bus.first_pixel_t(),
            app.spec.bus.model.t_per_line(),
            t,
        );
        let line = y - view.border_top as i64;
        assert!(
            line < -50,
            "and the beam should be up in the border above the picture, not on \
             line {line}"
        );
    }
}
