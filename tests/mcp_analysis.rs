//! Reading a program through the MCP server: the disassembly, what the machine
//! was watched doing, and the notes written about it.

use zx_rustrum::mcp::json::{parse, Json};
use zx_rustrum::mcp::Server;

fn call(server: &mut Server, tool: &str, args: Json) -> Result<String, String> {
    let line = Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", Json::num(1)),
        ("method", Json::str("tools/call")),
        (
            "params",
            Json::obj([("name", Json::str(tool)), ("arguments", args)]),
        ),
    ])
    .to_string();
    let reply = parse(&server.handle(&line).unwrap()).unwrap();
    let result = reply.get("result").unwrap().clone();
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|i| i.first())
        .and_then(|i| i.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    match result.get("isError").and_then(|e| e.as_bool()) {
        Some(true) => Err(text),
        _ => Ok(text),
    }
}

/// A small program that calls a routine which fills part of the screen, so
/// there is something for the observer to have an opinion about.
///
/// The outer loop is a routine of its own rather than a bare loop, because an
/// edge in the call graph is a call made from inside something: the observer
/// works the stack out from what the CPU did, and code that was never called
/// has no frame for a call to come from.
fn drawing_program(server: &mut Server) {
    let program: &[(u16, u8)] = &[
        // $7FF0: CALL $8000 : JP $7FF0 — the stub that keeps it going.
        (0x7FF0, 0xCD),
        (0x7FF1, 0x00),
        (0x7FF2, 0x80),
        (0x7FF3, 0xC3),
        (0x7FF4, 0xF0),
        (0x7FF5, 0x7F),
        // $8000: CALL $9000 : RET — the main loop.
        (0x8000, 0xCD),
        (0x8001, 0x00),
        (0x8002, 0x90),
        (0x8003, 0xC9),
        // $9000: LD HL,$4000 : LD B,32 : LD (HL),$FF : INC HL : DJNZ : RET
        (0x9000, 0x21),
        (0x9001, 0x00),
        (0x9002, 0x40),
        (0x9003, 0x06),
        (0x9004, 0x20),
        (0x9005, 0x36),
        (0x9006, 0xFF),
        (0x9007, 0x23),
        (0x9008, 0x10),
        (0x9009, 0xFB),
        (0x900A, 0xC9),
    ];
    for (at, byte) in program {
        server.session.spec.bus.poke(*at, *byte);
    }
    server.session.spec.cpu.pc = 0x7FF0;
    server.session.spec.cpu.sp = 0xFF00;
}

/// The disassembly reads as the debugger's does, and carries whatever has been
/// written against the addresses.
#[test]
fn the_disassembly_carries_the_notes_written_against_it() {
    let mut server = Server::new();
    drawing_program(&mut server);

    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$9000")),
            ("label", Json::str("fill_top_line")),
            (
                "comment",
                Json::str("32 bytes of $FF into the display file"),
            ),
        ]),
    )
    .unwrap();

    let text = call(
        &mut server,
        "disassemble",
        Json::obj([("address", Json::str("$9000")), ("count", Json::num(4))]),
    )
    .unwrap();
    assert!(text.contains("fill_top_line:"), "{text}");
    assert!(text.contains("LD HL,$4000"), "{text}");
    assert!(text.contains("21 00 40"), "the bytes are there too: {text}");
    assert!(text.contains("32 bytes of $FF"), "and the comment: {text}");
    assert!(text.contains("(next address $9008)"), "{text}");
}

/// Without the observer there is nothing to report, and the tools say so
/// rather than answering with an empty table that reads like an answer.
#[test]
fn the_analysis_tools_say_when_nothing_has_been_watched() {
    let mut server = Server::new();
    for tool in ["routines", "call_graph", "code_map"] {
        let answer = call(&mut server, tool, Json::obj([]));
        let why = answer.expect_err(&format!("{tool} should refuse"));
        assert!(
            why.contains("watch_routines"),
            "{tool} should say what to do first: {why}"
        );
    }
}

/// What the observer is for: which routine did the drawing, how often it was
/// called, and how much it wrote.
#[test]
fn a_watched_routine_is_reported_with_what_it_wrote() {
    let mut server = Server::new();
    drawing_program(&mut server);
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    assert!(server.session.spec.bus.observer.enabled);

    // Long enough for the loop to come round a few times.
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(20_000))]),
    )
    .unwrap();

    let listing = call(&mut server, "routines", Json::obj([])).unwrap();
    assert!(
        listing.contains("$9000"),
        "the routine is listed: {listing}"
    );

    let detail = call(
        &mut server,
        "routine",
        Json::obj([("address", Json::str("$9000"))]),
    )
    .unwrap();
    assert!(detail.contains("called"), "{detail}");
    assert!(
        detail.contains("display file"),
        "it should say what it wrote to: {detail}"
    );
    assert!(
        detail.contains("loops") || detail.contains("reaches"),
        "and what it looks like: {detail}"
    );

    let graph = call(&mut server, "call_graph", Json::obj([])).unwrap();
    assert!(
        graph.contains("$9000"),
        "the call from $8000 should be in the graph: {graph}"
    );

    // What ran is code; the screen it wrote to is not, and was never read.
    let map = call(&mut server, "code_map", Json::obj([])).unwrap();
    assert!(map.contains("code:"), "{map}");
    assert!(map.contains("$9000"), "{map}");
}

/// Turning the observer off leaves what it saw, unless asked to forget it.
#[test]
fn the_observer_can_be_switched_off_without_losing_what_it_saw() {
    let mut server = Server::new();
    drawing_program(&mut server);
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(20_000))]),
    )
    .unwrap();
    let seen = server.session.spec.bus.observer.routines.len();
    assert!(seen > 0, "something should have been seen");

    call(
        &mut server,
        "watch_routines",
        Json::obj([("enabled", Json::Bool(false))]),
    )
    .unwrap();
    assert!(!server.session.spec.bus.observer.enabled);
    assert_eq!(
        server.session.spec.bus.observer.routines.len(),
        seen,
        "what was seen is kept"
    );

    call(
        &mut server,
        "watch_routines",
        Json::obj([("enabled", Json::Bool(false)), ("clear", Json::Bool(true))]),
    )
    .unwrap();
    assert_eq!(
        server.session.spec.bus.observer.routines.len(),
        0,
        "and forgotten when asked"
    );
}

/// Comments and labels are the point of the exercise: they go in, they come
/// back, and what a person typed is marked apart from what was guessed.
#[test]
fn comments_go_in_and_come_back_with_the_guesses_marked() {
    let mut server = Server::new();
    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$8000")),
            ("label", Json::str("main_loop")),
            (
                "comment",
                Json::str("calls the drawing routine every frame"),
            ),
        ]),
    )
    .unwrap();
    // A guess, as AutoDoc would write one.
    server
        .session
        .notes
        .suggest(0x9000, "maybe_draw", "possibly a drawing routine");

    let all = call(&mut server, "comments", Json::obj([])).unwrap();
    assert!(all.contains("$8000  main_loop"), "{all}");
    assert!(all.contains("calls the drawing routine"), "{all}");
    assert!(
        all.contains("maybe_draw (a guess)"),
        "a guess is marked as one: {all}"
    );

    // And a range asks for a range.
    let narrow = call(
        &mut server,
        "comments",
        Json::obj([("from", Json::str("$8100")), ("to", Json::str("$FFFF"))]),
    )
    .unwrap();
    assert!(!narrow.contains("main_loop"), "{narrow}");
    assert!(narrow.contains("maybe_draw"), "{narrow}");
}

/// Typing over a guess makes it yours, which is the rule everywhere else in
/// this emulator and has to hold here too.
#[test]
fn writing_over_a_guess_makes_it_a_persons_note() {
    let mut server = Server::new();
    server
        .session
        .notes
        .suggest(0x9000, "maybe_draw", "possibly a drawing routine");
    assert!(server.session.notes.label_is_auto(0x9000));

    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$9000")),
            ("label", Json::str("draw_row")),
        ]),
    )
    .unwrap();
    assert!(!server.session.notes.label_is_auto(0x9000));
    assert_eq!(server.session.notes.label(0x9000), "draw_row");

    let text = call(&mut server, "comments", Json::obj([])).unwrap();
    assert!(text.contains("draw_row"), "{text}");
    assert!(!text.contains("draw_row (a guess)"), "{text}");
}

/// Notes with nowhere to go say so instead of being lost quietly.
#[test]
fn saving_notes_with_no_file_says_where_they_would_have_gone() {
    let mut server = Server::new();
    call(
        &mut server,
        "set_comment",
        Json::obj([("address", Json::num(0x8000)), ("label", Json::str("x"))]),
    )
    .unwrap();
    let why = call(&mut server, "save_comments", Json::obj([])).unwrap_err();
    assert!(why.contains("not attached to a file"), "{why}");
}

/// AutoDoc guesses, and is made to say that it is guessing.
#[test]
fn autodoc_offers_guesses_and_calls_them_guesses() {
    let mut server = Server::new();
    drawing_program(&mut server);
    let text = call(
        &mut server,
        "autodoc",
        Json::obj([("entries", Json::arr(vec![Json::str("$9000")]))]),
    )
    .unwrap();
    assert!(
        text.to_lowercase().contains("guess"),
        "it should not be read as fact: {text}"
    );
}

/// The whole thing against a real game: load the tape, watch, run, and see
/// what the machine was doing. Skips itself where the ROM or the tapes are
/// not on the machine, as the rest of the suite does.
#[test]
fn a_real_game_can_be_loaded_watched_and_read() {
    let tape = "tapes/Jetpac (1983)(Ultimate Play The Game)[16K].tzx";
    if std::fs::read("roms/48.rom").is_err() || std::fs::read(tape).is_err() {
        eprintln!("need roms/48.rom and {tape}; skipping");
        return;
    }
    let mut server = Server::new();
    let loaded = call(
        &mut server,
        "load_tape",
        Json::obj([("path", Json::str(tape))]),
    )
    .expect("the tape should load");
    assert!(loaded.contains("blocks"), "{loaded}");

    // A game that has loaded is out of the ROM's loader and drawing something.
    let drawn = (0x4000..0x5800u16)
        .filter(|a| server.session.spec.bus.peek_raw(*a) != 0)
        .count();
    assert!(drawn > 500, "there should be a picture: {drawn} bytes");

    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    call(
        &mut server,
        "run_frames",
        Json::obj([("frames", Json::num(50))]),
    )
    .unwrap();

    let routines = call(&mut server, "routines", Json::obj([])).unwrap();
    assert!(
        server.session.spec.bus.observer.routines.len() > 3,
        "a running game calls things: {routines}"
    );
    assert!(routines.contains("writes"), "{routines}");

    // Whatever the busiest routine is, it can be read as code and asked about.
    let busiest = server
        .session
        .spec
        .bus
        .observer
        .routines
        .values()
        .max_by_key(|r| r.instructions)
        .map(|r| r.entry)
        .unwrap();
    let detail = call(
        &mut server,
        "routine",
        Json::obj([("address", Json::num(busiest as f64))]),
    )
    .unwrap();
    assert!(detail.contains(&format!("${busiest:04X}")), "{detail}");
    let listing = call(
        &mut server,
        "disassemble",
        Json::obj([
            ("address", Json::num(busiest as f64)),
            ("count", Json::num(8)),
        ]),
    )
    .unwrap();
    assert!(listing.contains(&format!("${busiest:04X}")), "{listing}");

    // The machine can be put down and picked up again exactly.
    call(
        &mut server,
        "save_state",
        Json::obj([("name", Json::str("mid-game"))]),
    )
    .unwrap();
    let pc = server.session.spec.cpu.pc;
    call(
        &mut server,
        "run_frames",
        Json::obj([("frames", Json::num(10))]),
    )
    .unwrap();
    call(
        &mut server,
        "restore_state",
        Json::obj([("name", Json::str("mid-game"))]),
    )
    .unwrap();
    assert_eq!(server.session.spec.cpu.pc, pc, "back where it was");
}

/// Cross-references, which is the question asked when naming anything: who
/// calls this, who writes to it, and what in memory names it.
#[test]
fn cross_references_separate_what_was_watched_from_what_was_searched_for() {
    let mut server = Server::new();
    drawing_program(&mut server);
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(20_000))]),
    )
    .unwrap();

    let text = call(
        &mut server,
        "xrefs",
        Json::obj([("address", Json::str("$9000"))]),
    )
    .unwrap();

    // The call from $8000 was watched happening.
    assert!(text.contains("called by (watched)"), "{text}");
    assert!(text.contains("$8000"), "{text}");
    // And the CALL instruction itself is in memory, found by searching.
    assert!(
        text.contains("CALL $9000"),
        "the instruction that names it: {text}"
    );
    assert!(
        text.contains("some of these will be data"),
        "a search is not a fact, and should not read as one: {text}"
    );
}

/// An address nothing refers to says so, rather than an empty heading that
/// reads like an answer.
#[test]
fn an_address_with_no_references_says_so() {
    let mut server = Server::new();
    let text = call(
        &mut server,
        "xrefs",
        Json::obj([("address", Json::str("$BEEF"))]),
    )
    .unwrap();
    assert!(text.contains("nothing in"), "{text}");
}

/// Names out of a symbol file are what turn "CALL $0D6B" into something worth
/// reading, and they fill in where nothing has been typed rather than over it.
#[test]
fn symbols_from_a_file_name_the_calls() {
    let dir = std::env::temp_dir().join("zxrs-mcp-symbols");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("symbols.txt");
    std::fs::write(
        &path,
        "# a couple of names\n\
         9000 fill_screen ; fills the top line\n\
         0D6B rom_cls ; clears the screen\n",
    )
    .unwrap();

    let mut server = Server::new();
    drawing_program(&mut server);
    let loaded = call(
        &mut server,
        "load_symbols",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    )
    .unwrap();
    assert!(loaded.contains('2'), "two names should be read: {loaded}");

    // The call at $8000 names $9000, which now has a name.
    let listing = call(
        &mut server,
        "disassemble",
        Json::obj([("address", Json::str("$8000")), ("count", Json::num(2))]),
    )
    .unwrap();
    assert!(
        listing.contains("-> fill_screen"),
        "the call should say what it calls: {listing}"
    );

    // And at the routine itself the name is used as its label.
    let listing = call(
        &mut server,
        "disassemble",
        Json::obj([("address", Json::str("$9000")), ("count", Json::num(1))]),
    )
    .unwrap();
    assert!(listing.contains("fill_screen:"), "{listing}");

    // Something typed here wins over the file.
    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$9000")),
            ("label", Json::str("draw_row")),
        ]),
    )
    .unwrap();
    let listing = call(
        &mut server,
        "disassemble",
        Json::obj([("address", Json::str("$9000")), ("count", Json::num(1))]),
    )
    .unwrap();
    assert!(listing.contains("draw_row:"), "{listing}");
    assert!(!listing.contains("fill_screen:"), "{listing}");

    let listed = call(&mut server, "symbols", Json::obj([])).unwrap();
    assert!(listed.contains("rom_cls"), "{listed}");
    let _ = std::fs::remove_file(&path);
}

/// A routine copied out of the ROM is recognised where it landed, because the
/// bytes hash the same wherever they are.
#[test]
fn a_rom_routine_copied_into_ram_is_recognised() {
    let Ok(rom) = std::fs::read("roms/48.rom") else {
        eprintln!("need roms/48.rom; skipping");
        return;
    };
    let mut server = Server::new();
    server.session.spec.load_rom(&rom);
    server.session.rom_loaded = true;
    call(&mut server, "load_symbols", Json::obj([])).unwrap();

    // Copy the ROM's own CLS into RAM, as a game would.
    for i in 0..32u16 {
        let byte = server.session.spec.bus.peek_raw(0x0D6B + i);
        server.session.spec.bus.poke(0x9000 + i, byte);
    }
    let text = call(
        &mut server,
        "identify",
        Json::obj([("address", Json::str("$9000"))]),
    )
    .unwrap();
    assert!(
        text.contains("copied here"),
        "it should say it is a copy, and of what: {text}"
    );

    // And where nothing matches, it says that rather than inventing one.
    let text = call(
        &mut server,
        "identify",
        Json::obj([("address", Json::str("$A000"))]),
    )
    .unwrap();
    assert!(text.contains("does not match"), "{text}");
}

/// The access map: which pages a program reads, which it writes, and which it
/// runs — the shape of a program's memory before a byte of it is disassembled.
#[test]
fn the_access_map_says_what_each_page_is_for() {
    let mut server = Server::new();
    drawing_program(&mut server);
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(30_000))]),
    )
    .unwrap();

    let text = call(&mut server, "memory_activity", Json::obj([])).unwrap();
    // The routine's own page was executed; the screen was written.
    assert!(text.contains("$9000xx"), "the code's page: {text}");
    assert!(
        text.contains("code"),
        "and it should be called code: {text}"
    );
    assert!(text.contains("$4000xx"), "the screen it wrote: {text}");
    assert!(
        text.contains("the display file"),
        "and the machine's own furniture should be named: {text}"
    );
    // And it says what it does not know rather than implying it found nothing.
    assert!(
        text.contains("No back buffer detected") || text.contains("Back buffer:"),
        "{text}"
    );
}

/// The tape's own block list, which is the memory map before there is one: a
/// standard block's header says what it loads and where.
#[test]
fn the_tape_says_what_it_holds_and_where_it_loads() {
    let tape = "tapes/Jetpac (1983)(Ultimate Play The Game)[16K].tzx";
    if std::fs::read("roms/48.rom").is_err() || std::fs::read(tape).is_err() {
        eprintln!("need roms/48.rom and {tape}; skipping");
        return;
    }
    let mut server = Server::new();
    // In the deck without starting it, so the block list is the tape as it
    // was rather than as it ended.
    call(
        &mut server,
        "load_tape",
        Json::obj([("path", Json::str(tape)), ("autoload", Json::Bool(false))]),
    )
    .unwrap();

    let text = call(&mut server, "tape_blocks", Json::obj([])).unwrap();
    assert!(text.contains("blocks"), "{text}");
    assert!(
        text.contains("Program") || text.contains("Bytes"),
        "a header should be decoded: {text}"
    );
    assert!(
        text.contains("loading at $") || text.contains("autostart line"),
        "and say where it loads: {text}"
    );

    // With no tape at all it says so rather than answering emptily.
    let mut empty = Server::new();
    let why = call(&mut empty, "tape_blocks", Json::obj([])).unwrap_err();
    assert!(why.contains("no tape"), "{why}");
}

/// Which loader is reading the tape, told from the loop the CPU is in.
#[test]
fn the_loader_is_named_from_the_loop_it_is_counting_pulses_in() {
    let mut server = Server::new();
    // The ROM's own sampling loop, which is the one every standard block is
    // read by: LD-EDGE-1 as it is written at $05ED.
    let core: [u8; 12] = [
        0x04, 0xC8, 0x3E, 0x7F, 0xDB, 0xFE, 0x1F, 0xA9, 0xE6, 0x20, 0x28, 0xF4,
    ];
    for (i, byte) in core.iter().enumerate() {
        server.session.spec.bus.poke(0x9000 + i as u16, *byte);
    }
    server.session.spec.cpu.pc = 0x9000;
    let text = call(&mut server, "loader", Json::obj([])).unwrap();
    assert!(
        text.contains("sampling loop"),
        "it should recognise the ROM's own loop: {text}"
    );

    server.session.spec.cpu.pc = 0x8000;
    let text = call(&mut server, "loader", Json::obj([])).unwrap();
    assert!(text.contains("Not in a loader"), "{text}");
}

/// When in the frame a routine ran, which on this machine is half the
/// question: a write above the beam is seen now, one below it next frame.
#[test]
fn a_routine_is_placed_in_the_frame_it_ran_in() {
    let mut server = Server::new();
    drawing_program(&mut server);
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    call(
        &mut server,
        "run_frames",
        Json::obj([("frames", Json::num(3))]),
    )
    .unwrap();

    // The frame itself, described in the machine's own terms.
    let text = call(&mut server, "frame_timing", Json::obj([])).unwrap();
    assert!(text.contains("69888"), "a 48K frame: {text}");
    // The machine's own number, not one from memory: the emulator anchors the
    // first pixel at 14335 and the ULA's fetch cycle a T-state later, and a
    // test that hard-codes the other one would be arguing with docs/timing.md.
    let first = server.session.spec.bus.first_pixel_t();
    assert!(
        text.contains(&first.to_string()),
        "and where the first pixel is (T {first}): {text}"
    );
    assert!(
        text.contains("display line") || text.contains("border"),
        "and where the beam is: {text}"
    );

    // And the drawing routine, placed against it.
    let text = call(
        &mut server,
        "frame_timing",
        Json::obj([("address", Json::str("$9000"))]),
    )
    .unwrap();
    assert!(text.contains("$9000"), "{text}");
    assert!(text.contains("was entered between"), "{text}");
    assert!(
        text.contains("display file"),
        "it writes to the screen, so it should say what that means: {text}"
    );
}

/// What the machine is playing, and with what. A 48K has only the beeper; a
/// 128K has a chip whose registers say what is sounding.
#[test]
fn the_sound_state_is_reported_for_both_ways_of_making_a_noise() {
    let mut server = Server::new();
    let text = call(&mut server, "sound_state", Json::obj([])).unwrap();
    assert!(text.contains("Beeper"), "{text}");
    assert!(
        text.contains("no sound chip"),
        "a 48K has none, and should say so: {text}"
    );

    // On a 128K the registers are read back and turned into something worth
    // reading: a middle A on channel A, tone on, volume 15.
    let Ok(rom) = std::fs::read("roms/128.rom") else {
        eprintln!("need roms/128.rom for the rest; skipping");
        return;
    };
    server
        .session
        .spec
        .set_model(zx_rustrum::machine::Model::Spectrum128, &rom);
    server.session.rom_loaded = true;
    let ay = &mut server.session.spec.bus.audio.ay;
    ay.regs[0] = 0xFD; // period 253, about 440Hz
    ay.regs[1] = 0x00;
    ay.regs[7] = 0b0011_1110; // tone on A only, no noise
    ay.regs[8] = 15;

    let text = call(&mut server, "sound_state", Json::obj([])).unwrap();
    assert!(text.contains("AY-3-8912"), "{text}");
    assert!(text.contains("channel A: tone"), "{text}");
    assert!(text.contains("volume 15"), "{text}");
    assert!(
        text.contains("437 Hz") || text.contains("438 Hz") || text.contains("440 Hz"),
        "the period should be turned into a pitch: {text}"
    );
    assert!(text.contains("channel B: no tone"), "{text}");
}

/// The deliverable: the whole listing, with what was learned against it, and
/// the parts that never ran left as bytes.
#[test]
fn the_listing_can_be_written_out_with_the_comments_on_it() {
    let mut server = Server::new();
    drawing_program(&mut server);
    // A table of data after the routine, which nothing executes.
    for i in 0..16u16 {
        server.session.spec.bus.poke(0x9010 + i, 0xAA);
    }
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(20_000))]),
    )
    .unwrap();
    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$9000")),
            ("label", Json::str("draw_row")),
            ("comment", Json::str("fills the top line of the screen")),
        ]),
    )
    .unwrap();

    let path = std::env::temp_dir().join("zxrs-listing.asm");
    let report = call(
        &mut server,
        "export_listing",
        Json::obj([
            ("path", Json::str(path.display().to_string())),
            ("from", Json::str("$9000")),
            ("to", Json::str("$9020")),
        ]),
    )
    .unwrap();
    assert!(report.contains("instructions"), "{report}");

    let listing = std::fs::read_to_string(&path).unwrap();
    assert!(listing.contains("draw_row:"), "{listing}");
    assert!(
        listing.contains("; fills the top line"),
        "the comment should be on the line: {listing}"
    );
    assert!(listing.contains("LD HL,$4000"), "{listing}");
    assert!(
        listing.contains("DEFB $AA"),
        "the table never ran, so it should be bytes: {listing}"
    );
    assert!(
        listing.contains("may be data"),
        "and the header should say what \"run\" means: {listing}"
    );
    let _ = std::fs::remove_file(&path);
}

/// Blocks: what is code and what is data, kept in the notes so a listing keeps
/// its shape between sessions.
#[test]
fn blocks_can_be_worked_out_and_written_down() {
    let mut server = Server::new();
    drawing_program(&mut server);

    let text = call(&mut server, "blocks", Json::obj([])).unwrap();
    assert!(text.contains("No blocks"), "{text}");

    // Marked by hand.
    call(
        &mut server,
        "blocks",
        Json::obj([
            ("from", Json::str("$9010")),
            ("to", Json::str("$901F")),
            ("kind", Json::str("data")),
        ]),
    )
    .unwrap();
    let text = call(&mut server, "blocks", Json::obj([])).unwrap();
    assert!(text.contains("$9010-$901F  DATA"), "{text}");

    // And worked out from what ran, keeping what was written down.
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(20_000))]),
    )
    .unwrap();
    call(
        &mut server,
        "blocks",
        Json::obj([("work_out", Json::Bool(true))]),
    )
    .unwrap();
    let text = call(&mut server, "blocks", Json::obj([])).unwrap();
    assert!(text.contains("CODE"), "the code that ran: {text}");
    assert!(
        text.contains("$9010-$901F  DATA"),
        "and what was written down is kept: {text}"
    );
}

/// Where the time goes, which is not the same as what is called most often.
#[test]
fn the_profiler_says_where_the_time_went() {
    let mut server = Server::new();
    drawing_program(&mut server);
    let text = call(
        &mut server,
        "profile",
        Json::obj([("frames", Json::num(2))]),
    )
    .unwrap();
    assert!(text.contains("Where the time went"), "{text}");
    assert!(text.contains("$9000"), "the routine doing the work: {text}");
    assert!(text.contains('%'), "with its share of the time: {text}");
}

/// A recording is somebody else's session: it plays back by fetch count, and
/// it can be wound to a frame — forwards by playing on, backwards by starting
/// again, since nothing can be un-executed.
#[test]
fn a_recording_can_be_played_and_sought() {
    let dir = std::path::Path::new("recordings");
    let Some(path) = std::fs::read_dir(dir).ok().and_then(|entries| {
        let mut found: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "rzx"))
            .collect();
        found.sort();
        found.into_iter().next()
    }) else {
        eprintln!("no recordings/*.rzx here; skipping");
        return;
    };
    if std::fs::read("roms/48.rom").is_err() {
        return;
    }

    let mut server = Server::new();
    let loaded = call(
        &mut server,
        "load_recording",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    );
    let Ok(loaded) = loaded else {
        eprintln!("{}: {}", path.display(), loaded.unwrap_err());
        return;
    };
    assert!(loaded.contains("frames"), "{loaded}");

    let info = call(&mut server, "recording_info", Json::obj([])).unwrap();
    assert!(info.contains("opcode fetches"), "{info}");
    assert!(info.contains("At frame 0"), "{info}");

    call(
        &mut server,
        "play_recording",
        Json::obj([("frames", Json::num(20))]),
    )
    .unwrap();
    assert_eq!(server.session.rzx.as_ref().unwrap().frame, 20);

    // Back to five, which means starting again and playing forward.
    let text = call(
        &mut server,
        "seek_recording",
        Json::obj([("frame", Json::num(5))]),
    )
    .unwrap();
    assert!(text.contains("Sought to frame 5"), "{text}");
    assert_eq!(server.session.rzx.as_ref().unwrap().frame, 5);

    // And forwards from there is just playing on.
    call(
        &mut server,
        "seek_recording",
        Json::obj([("frame", Json::num(12))]),
    )
    .unwrap();
    assert_eq!(server.session.rzx.as_ref().unwrap().frame, 12);
}

/// The +3's drive through the server: what goes in, what is on it, and what
/// the machine has been reading.
#[test]
fn a_disk_can_be_mounted_read_and_catalogued() {
    let Ok(rom) = std::fs::read("roms/plus3.rom") else {
        eprintln!("need roms/plus3.rom; skipping");
        return;
    };
    let dir = std::env::temp_dir().join("zxrs-mcp-disk");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.dsk");

    let mut server = Server::new();
    server
        .session
        .spec
        .set_model(zx_rustrum::machine::Model::Plus3, &rom);
    server.session.rom_loaded = true;

    // A blank disk, written where it was asked for.
    let text = call(
        &mut server,
        "new_disk",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    )
    .unwrap();
    assert!(text.contains("40 tracks"), "{text}");
    assert!(path.exists());

    // What is on it, and what it says about itself.
    let cat = call(&mut server, "disk_catalogue", Json::obj([])).unwrap();
    assert!(cat.contains("No files"), "{cat}");
    assert!(cat.contains("free"), "{cat}");
    let info = call(&mut server, "disk_info", Json::obj([])).unwrap();
    assert!(info.contains("data format"), "{info}");
    assert!(info.contains("writable"), "{info}");

    // Mounting the same file again read-only is the default, and says so.
    let text = call(
        &mut server,
        "mount_disk",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    )
    .unwrap();
    assert!(
        text.contains("Read-only"),
        "a disk is not made writable without being asked: {text}"
    );

    // A sector, by the number in its address mark.
    let sector = call(
        &mut server,
        "read_sector",
        Json::obj([("track", Json::num(0)), ("sector", Json::str("$C1"))]),
    )
    .unwrap();
    assert!(sector.contains("sector $C1"), "{sector}");
    assert!(
        sector.contains("E5 E5 E5"),
        "a blank disk is full of $E5: {sector}"
    );

    // A sector that is not there says what is.
    let why = call(
        &mut server,
        "read_sector",
        Json::obj([("track", Json::num(0)), ("sector", Json::str("$77"))]),
    )
    .unwrap_err();
    assert!(why.contains("$C1"), "it should list what is there: {why}");

    // The speeds.
    let text = call(
        &mut server,
        "disk_speed",
        Json::obj([("speed", Json::str("normal"))]),
    )
    .unwrap();
    assert!(text.contains("Normal"), "{text}");
    assert_eq!(
        server.session.spec.bus.fdc.speed,
        zx_rustrum::fdc::Speed::Normal
    );

    // Nothing has been read yet, and it says so rather than showing an empty
    // table that reads like an answer.
    let activity = call(&mut server, "disk_activity", Json::obj([])).unwrap();
    assert!(activity.contains("Nothing has been read"), "{activity}");

    let _ = std::fs::remove_file(&path);
}

/// A machine with no drive says so rather than pretending to have one.
#[test]
fn a_machine_without_a_drive_refuses_a_disk() {
    let mut server = Server::new();
    let why = call(&mut server, "new_disk", Json::obj([])).unwrap_err();
    assert!(why.contains("no disk drive"), "{why}");
    assert!(why.contains("set_machine"), "and how to get one: {why}");
}

/// A disk inside a zip, through the server: mounted like any other, and never
/// written back into the archive.
#[test]
fn a_disk_in_a_zip_can_be_mounted_from_the_server() {
    let Ok(rom) = std::fs::read("roms/plus3.rom") else {
        eprintln!("need roms/plus3.rom; skipping");
        return;
    };
    let dir = std::env::temp_dir().join("zxrs-mcp-disk-zip");
    std::fs::create_dir_all(&dir).unwrap();
    let zip = dir.join("game.zip");
    let copy = dir.join("copy.dsk");
    let _ = std::fs::remove_file(&copy);

    // A zip with a disk in it, built by hand.
    let disk = zx_rustrum::disk::Disk::blank("zipped");
    let bytes = disk.to_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut directory: Vec<u8> = Vec::new();
    let name = "GAME.DSK";
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&[20, 0, 0, 0]);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&bytes);
    directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    directory.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
    directory.extend_from_slice(&0u16.to_le_bytes());
    directory.extend_from_slice(&[0; 4]);
    directory.extend_from_slice(&0u32.to_le_bytes());
    directory.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    directory.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
    directory.extend_from_slice(&[0; 8]);
    directory.extend_from_slice(&[0; 4]);
    directory.extend_from_slice(&0u32.to_le_bytes());
    directory.extend_from_slice(name.as_bytes());
    let directory_at = out.len();
    out.extend_from_slice(&directory);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    out.extend_from_slice(&(directory_at as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    std::fs::write(&zip, &out).unwrap();

    let mut server = Server::new();
    server
        .session
        .spec
        .set_model(zx_rustrum::machine::Model::Plus3, &rom);
    server.session.rom_loaded = true;

    let text = call(
        &mut server,
        "mount_disk",
        Json::obj([("path", Json::str(zip.display().to_string()))]),
    )
    .unwrap();
    assert!(
        text.contains("GAME.DSK"),
        "it says what it took out: {text}"
    );
    assert!(text.contains("Read-only"), "{text}");
    assert!(server.session.spec.bus.fdc.drives[0].is_some());

    // Writable without saying where is refused: there is nowhere in an
    // archive to put the writes.
    let why = call(
        &mut server,
        "mount_disk",
        Json::obj([
            ("path", Json::str(zip.display().to_string())),
            ("writable", Json::Bool(true)),
        ]),
    )
    .unwrap_err();
    assert!(
        why.contains("copy_to"),
        "and says what to do instead: {why}"
    );

    // With somewhere to put them, it works.
    let text = call(
        &mut server,
        "mount_disk",
        Json::obj([
            ("path", Json::str(zip.display().to_string())),
            ("copy_to", Json::str(copy.display().to_string())),
        ]),
    )
    .unwrap();
    assert!(text.contains("Writable"), "{text}");
    assert!(copy.exists());
    let _ = std::fs::remove_file(&copy);
    let _ = std::fs::remove_file(&zip);
}

/// A row of bytes stops where code starts. Sixteen at a time regardless, the
/// row ran over the start of whatever came next, and the first instructions
/// of a routine went out as data: 193 of the 1,943 that ran in Jetpac.
#[test]
fn the_listing_does_not_swallow_code_that_starts_inside_a_row_of_data() {
    let mut server = Server::new();
    drawing_program(&mut server);
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(20_000))]),
    )
    .unwrap();

    let path = std::env::temp_dir().join(format!("zxrs-row-{}.asm", std::process::id()));
    call(
        &mut server,
        "export_listing",
        Json::obj([
            ("path", Json::str(path.display().to_string())),
            // Six bytes short of the routine, so a row of sixteen would run
            // ten bytes into it.
            ("from", Json::str("$8FFA")),
            ("to", Json::str("$9020")),
        ]),
    )
    .unwrap();
    let listing = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(
        listing
            .lines()
            .any(|l| l.starts_with("$8FFA  DEFB") && l.matches('$').count() == 7),
        "six bytes of data and no more, then the code: {listing}"
    );
    assert!(
        listing
            .lines()
            .any(|l| l.starts_with("$9000") && l.contains("LD HL,$4000")),
        "the routine's first instruction is an instruction: {listing}"
    );
}

/// A dispatcher through a one-byte trampoline and a table of three handlers,
/// the way a game does it: `type` at $9100 picks one, and the third is never
/// picked. The first handler's branch always jumps over code nothing else
/// reaches. The second's is never taken, though its target runs anyway, which
/// is the case that looking at the target alone gets wrong.
fn dispatcher(server: &mut Server) {
    let spec = &mut server.session.spec;
    let code: &[(u16, &[u8])] = &[
        (0x8000, &[0x3A, 0x00, 0x91]),                   // loop: LD A,($9100)
        (0x8003, &[0x87, 0x5F, 0x16, 0x00]),             // ADD A,A : LD E,A : LD D,0
        (0x8007, &[0x21, 0x00, 0x92, 0x19]),             // LD HL,$9200 : ADD HL,DE
        (0x800B, &[0x7E, 0x23, 0x66, 0x6F]),             // LD A,(HL) : INC HL : LD H,(HL) : LD L,A
        (0x800F, &[0xC3, 0x20, 0x80]),                   // JP $8020
        (0x8020, &[0xE9]),                               // JP (HL)
        (0x8030, &[0xAF, 0x28, 0x03]),                   // handler 0: XOR A : JR Z,$8036
        (0x8033, &[0x32, 0x03, 0x91]),                   // LD ($9103),A, never reached
        (0x8036, &[0xC3, 0x00, 0x80]),                   // JP loop
        (0x8040, &[0xAF, 0x20, 0xF3]),                   // handler 1: XOR A : JR NZ,$8036
        (0x8043, &[0xC3, 0x00, 0x80]),                   // JP loop
        (0x8050, &[0x32, 0x04, 0x91, 0xC3, 0x00, 0x80]), // handler 2, never picked
    ];
    for (at, bytes) in code {
        for (i, b) in bytes.iter().enumerate() {
            spec.bus.poke(at + i as u16, *b);
        }
    }
    let table = [0x30, 0x80, 0x40, 0x80, 0x50, 0x80];
    for (i, b) in table.iter().enumerate() {
        spec.bus.poke(0x9200 + i as u16, *b);
    }
    spec.cpu.pc = 0x8000;
}

fn run_dispatcher(server: &mut Server) {
    dispatcher(server);
    call(server, "watch_routines", Json::obj([])).unwrap();
    let run = |server: &mut Server| {
        call(
            server,
            "run_tstates",
            Json::obj([("tstates", Json::num(5_000))]),
        )
        .unwrap();
    };
    server.session.spec.bus.poke(0x9100, 0);
    run(server);
    server.session.spec.bus.poke(0x9100, 1);
    run(server);
}

/// Coverage says what has run, what has not, and how each branch went as
/// recorded: a target running from elsewhere does not make a branch taken.
#[test]
fn coverage_finds_the_code_not_reached_and_the_branches_one_way() {
    let mut server = Server::new();
    run_dispatcher(&mut server);
    let args = Json::obj([("from", Json::str("$8000")), ("to", Json::str("$80FF"))]);
    let said = call(&mut server, "coverage", args).unwrap();

    assert!(
        said.contains("$8033-$8035"),
        "the far side of the branch that always jumps: {said}"
    );
    assert!(
        said.contains("$8050-$8055"),
        "the handler only the table leads to: {said}"
    );
    assert!(
        said.contains("led to by $9204"),
        "and the table's word is what leads there: {said}"
    );
    let line = |at: &str| {
        said.lines()
            .find(|l| l.trim_start().starts_with(at) && l.contains("JR"))
            .unwrap_or_else(|| panic!("no line for {at}: {said}"))
            .to_string()
    };
    assert!(line("$8031").contains("always taken"), "{said}");
    assert!(
        line("$8041").contains("never taken"),
        "its target at $8036 runs, but not because of this branch: {said}"
    );
}

/// The code does not name where a JP (HL) goes. What it did names the handlers,
/// the instruction that got to the trampoline, and the table, including the
/// entry nothing has jumped to.
#[test]
fn indirect_jumps_finds_the_handlers_and_the_table_behind_them() {
    let mut server = Server::new();
    run_dispatcher(&mut server);
    let said = call(&mut server, "indirect_jumps", Json::obj([])).unwrap();

    assert!(said.contains("$8020, 2 targets"), "{said}");
    assert!(
        said.contains("reached from $800F"),
        "the trampoline's caller: {said}"
    );
    assert!(said.contains("-> $8030"), "{said}");
    assert!(said.contains("-> $8040"), "{said}");
    assert!(
        said.contains("$9200  3 words, 2 jumped to so far"),
        "the table: {said}"
    );
    assert!(
        said.lines()
            .any(|l| l.contains("[ 2] $8050") && l.contains("not jumped to yet")),
        "with the handler never picked: {said}"
    );
}

/// Nothing watched is said, rather than reported as nothing having run.
#[test]
fn coverage_says_when_nothing_has_been_watched() {
    let mut server = Server::new();
    let err = call(&mut server, "coverage", Json::obj([])).unwrap_err();
    assert!(err.contains("watch_routines"), "{err}");
    let err = call(&mut server, "indirect_jumps", Json::obj([])).unwrap_err();
    assert!(err.contains("watch_routines"), "{err}");
}

/// A loader moving the program reads every byte of its code, and that must not
/// make the code look like data: in Jetpac it made all 8K "read, never run", and
/// coverage reported nothing left to reach.
#[test]
fn code_a_loader_has_moved_can_still_be_reached() {
    let mut server = Server::new();
    dispatcher(&mut server);
    // LD HL,$8050 : LD DE,$A000 : LD BC,6 : LDIR : JP $8000, reading the handler
    // nothing dispatches to.
    let copier = [
        0x21, 0x50, 0x80, 0x11, 0x00, 0xA0, 0x01, 0x06, 0x00, 0xED, 0xB0, 0xC3, 0x00, 0x80,
    ];
    for (i, b) in copier.iter().enumerate() {
        server.session.spec.bus.poke(0x8100 + i as u16, *b);
    }
    server.session.spec.cpu.pc = 0x8100;
    server.session.spec.bus.poke(0x9100, 0);
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(5_000))]),
    )
    .unwrap();
    server.session.spec.bus.poke(0x9100, 1);
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(5_000))]),
    )
    .unwrap();
    let args = Json::obj([("from", Json::str("$8000")), ("to", Json::str("$80FF"))]);
    let said = call(&mut server, "coverage", args).unwrap();
    assert!(
        said.contains("$8050-$8055"),
        "read by the copy, and still reachable through the table: {said}"
    );
}

/// A table is two different targets side by side, near each other and for one
/// jump. A repeated word is a row of a sprite, and a far-off word is not part of
/// it: $5CB0 sat in front of Jetpac's table as the operand of the JP before it.
#[test]
fn a_table_is_not_a_repeated_word_or_a_word_from_far_away() {
    let mut server = Server::new();
    dispatcher(&mut server);
    {
        let bus = &mut server.session.spec.bus;
        // The operand of something in front of the table, far from any handler.
        bus.poke(0x91FE, 0xB0);
        bus.poke(0x91FF, 0x5C);
        // A handler's address twice over, somewhere else.
        for (i, b) in [0x30, 0x80, 0x30, 0x80].iter().enumerate() {
            bus.poke(0x9300 + i as u16, *b);
        }
        bus.poke(0x9304, 0xFF);
        bus.poke(0x9305, 0xFF);
    }
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    for handler in [0, 1] {
        server.session.spec.bus.poke(0x9100, handler);
        call(
            &mut server,
            "run_tstates",
            Json::obj([("tstates", Json::num(5_000))]),
        )
        .unwrap();
    }
    let said = call(&mut server, "indirect_jumps", Json::obj([])).unwrap();
    assert!(
        said.contains("$9200  3 words"),
        "starts at the real table: {said}"
    );
    assert!(
        !said.contains("$91FE"),
        "not at the operand before it: {said}"
    );
    assert!(
        !said.contains("$9300"),
        "and a repeated word is not one: {said}"
    );
}
