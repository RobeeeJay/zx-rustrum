//! Writing a recording out, and reading it back.

use zx_rustrum::machine::Spectrum;
use zx_rustrum::{rzx, snapshot};

/// A snapshot written and read back is the same machine. A 48K .sna has no
/// field for the program counter — it resumes by executing a RET — so PC is
/// pushed on the machine's own stack, which is the part worth checking.
#[test]
fn a_snapshot_round_trips_through_sna() {
    let mut spec = Spectrum::new();
    spec.cpu.pc = 0x8123;
    spec.cpu.sp = 0xFF00;
    spec.cpu.a = 0x5A;
    spec.cpu.set_hl(0x1234);
    spec.cpu.set_de(0x5678);
    spec.cpu.i = 0x3F;
    spec.cpu.im = 2;
    spec.cpu.iff1 = true;
    spec.cpu.iff2 = true;
    spec.bus.border = 3;
    spec.bus.poke(0x9000, 0xAB);

    let sna = snapshot::save_sna(&spec);
    assert_eq!(sna.len(), 49179, "a 48K snapshot is a header and 48K");

    let mut back = Spectrum::new();
    snapshot::load_sna(&mut back, &sna).expect("it should read back");

    assert_eq!(back.cpu.pc, 0x8123, "the PC comes off the stack");
    assert_eq!(back.cpu.sp, 0xFF00, "and the stack is where it was");
    assert_eq!(back.cpu.a, 0x5A);
    assert_eq!(back.cpu.hl(), 0x1234);
    assert_eq!(back.cpu.de(), 0x5678);
    assert_eq!(back.cpu.i, 0x3F);
    assert_eq!(back.cpu.im, 2);
    assert!(back.cpu.iff1 && back.cpu.iff2, "interrupts were enabled");
    assert_eq!(back.bus.border, 3);
    assert_eq!(back.bus.peek_raw(0x9000), 0xAB, "and the memory with it");
}

/// A recording written out is one this can read: the writer and the reader are
/// the two halves of the same format, and a file that only one of them
/// understands is no use to anybody.
#[test]
fn a_recording_round_trips_through_rzx() {
    let recording = rzx::Recording {
        creator: "ZX-Rustrum".to_string(),
        snapshot: Some(rzx::Snapshot {
            extension: "sna".to_string(),
            data: vec![0x42; 49179],
        }),
        later: Vec::new(),
        frames: vec![
            rzx::Frame {
                fetches: 1234,
                inputs: vec![0xFF, 0xBF, 0x7F],
            },
            rzx::Frame {
                fetches: 1000,
                inputs: Vec::new(),
            },
        ],
        start_t: 0,
    };

    let bytes = rzx::write(&recording);
    assert_eq!(&bytes[0..4], b"RZX!", "it should be an RZX file");

    let back = rzx::parse(&bytes).expect("and one this can read");
    assert_eq!(back.creator, "ZX-Rustrum");
    assert_eq!(back.frames.len(), 2);
    assert_eq!(back.frames[0].fetches, 1234);
    assert_eq!(back.frames[0].inputs, vec![0xFF, 0xBF, 0x7F]);
    assert_eq!(
        back.frames[1].inputs,
        Vec::<u8>::new(),
        "a frame that read nothing should read nothing back"
    );
    let snapshot = back.snapshot.expect("the machine to start from");
    assert_eq!(snapshot.extension, "sna");
    assert_eq!(snapshot.data.len(), 49179);
}

/// A recording made here plays back here: the machine it starts from, the
/// instructions it counts and the bytes it hands out are the same ones on the
/// way in and on the way out. Anything less and the file is a souvenir.
#[test]
fn a_recording_made_by_the_emulator_plays_back() {
    use zx_rustrum::ui::{App, Roms};

    // A program that reads the keyboard and counts how many times, so the
    // playback has something to get wrong: IN A,($FE) : INC HL : JR back.
    let mut spec = Spectrum::new();
    for (offset, byte) in [0xDB, 0xFE, 0x23, 0x18, 0xFB].iter().enumerate() {
        spec.bus.poke(0x8000 + offset as u16, *byte);
    }
    spec.cpu.pc = 0x8000;
    spec.cpu.sp = 0xFF00;

    let mut app = App::with_roms(spec, String::new(), Roms::default(), None);
    app.show_ram_map = false;
    app.show_debugger = false;
    app.show_back_buffer = false;
    app.show_tape = false;
    app.running = true;

    app.start_recording();
    for _ in 0..20 {
        app.advance(1.0 / 50.0);
    }
    let frames = app.recorded_frames().expect("it should be recording");
    assert!(frames >= 10, "twenty frames of running recorded {frames}");

    let bytes = app.stop_recording().expect("there is something to write");
    assert!(
        app.recorded_frames().is_none(),
        "and it should have stopped recording"
    );

    let recording = rzx::parse(&bytes).expect("what was written should read back");
    assert_eq!(recording.frames.len(), frames);
    assert!(
        recording
            .frames
            .iter()
            .any(|frame| !frame.inputs.is_empty()),
        "the program reads a port every few instructions, so the frames should \
         carry what it read"
    );
    assert!(
        recording.frames.iter().all(|frame| frame.fetches > 0),
        "and every frame should have run some instructions"
    );

    // The snapshot it carries is the machine as it was when recording started.
    let snapshot = recording.snapshot.expect("a machine to start from");
    let mut back = Spectrum::new();
    snapshot::load_sna(&mut back, &snapshot.data).expect("it should load");
    assert_eq!(back.cpu.pc, 0x8000, "which is where the program was");
    assert_eq!(back.bus.peek_raw(0x8000), 0xDB, "with its code in place");
}

/// A recording goes beside the tape it is of, under the same name. Somebody
/// looking for a recording of a game looks where the game is.
#[test]
fn a_recording_is_named_after_the_tape_in_the_deck() {
    use zx_rustrum::ui::{App, Roms};

    let mut app = App::with_roms(Spectrum::new(), String::new(), Roms::default(), None);
    assert_eq!(
        app.recording_path(),
        std::path::PathBuf::from("recording.rzx"),
        "with nothing loaded there is nothing to be named after"
    );

    app.tape_path = Some(std::path::PathBuf::from("/games/manic miner.tap"));
    assert_eq!(
        app.recording_path(),
        std::path::PathBuf::from("/games/manic miner.rzx"),
        "and with a tape in the deck, beside it under the same name"
    );
}

/// A snapshot saved from a running machine loads back into the same machine.
///
/// The whole point of one: stop where you are, come back to it later. What it
/// does not carry is where the tape had reached or what the sound was doing —
/// the format has no room for them, and a snapshot that claimed to hold them
/// would be lying about what comes back.
#[test]
fn a_snapshot_saved_from_a_running_machine_comes_back() {
    use zx_rustrum::ui::{App, Roms};

    // A program that counts in memory, so the machine has state worth keeping.
    let mut spec = Spectrum::new();
    for (offset, byte) in [0x21u8, 0x00, 0x90, 0x34, 0x18, 0xFC].iter().enumerate() {
        spec.bus.poke(0x8000 + offset as u16, *byte);
    }
    spec.cpu.pc = 0x8000;
    spec.cpu.sp = 0xFF00;

    let mut app = App::with_roms(spec, String::new(), Roms::default(), None);
    app.show_ram_map = false;
    app.show_debugger = false;
    app.show_back_buffer = false;
    app.show_tape = false;
    app.running = true;
    for _ in 0..5 {
        app.advance(1.0 / 50.0);
    }

    let counted = app.peek(0x9000);
    assert!(counted > 0, "the program should have counted something");
    let (pc, sp, hl) = (app.spec.cpu.pc, app.spec.cpu.sp, app.spec.cpu.hl());
    let bytes = snapshot::save_sna(&app.spec);

    let mut back = Spectrum::new();
    snapshot::load_sna(&mut back, &bytes).expect("it should load");
    assert_eq!(back.cpu.pc, pc, "the program counter comes back");
    assert_eq!(back.cpu.sp, sp, "and the stack pointer");
    assert_eq!(back.cpu.hl(), hl);
    assert_eq!(
        back.bus.peek_raw(0x9000),
        counted,
        "and what it had counted so far"
    );
    assert_eq!(
        back.bus.peek_raw(0x8000),
        0x21,
        "with the program still in place"
    );
}

/// A snapshot goes beside the tape it is of, under the same name — where
/// somebody looking for a saved game of it would look.
#[test]
fn a_snapshot_is_named_after_the_tape_in_the_deck() {
    use zx_rustrum::ui::{App, Roms};

    let mut app = App::with_roms(Spectrum::new(), String::new(), Roms::default(), None);
    assert_eq!(
        app.snapshot_path(),
        std::path::PathBuf::from("snapshot.sna"),
        "with nothing loaded there is nothing to be named after"
    );

    app.tape_path = Some(std::path::PathBuf::from("/games/jetpac.tzx"));
    assert_eq!(
        app.snapshot_path(),
        std::path::PathBuf::from("/games/jetpac.sna"),
        "and with a tape in the deck, beside it under the same name"
    );
}
