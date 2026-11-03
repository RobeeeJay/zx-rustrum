//! What a program does to itself, who changes a value, and what the other side
//! of a branch would have done.

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

fn start(program: &[(u16, &[u8])]) -> Server {
    let mut server = Server::new();
    for (at, bytes) in program {
        for (i, b) in bytes.iter().enumerate() {
            server.session.spec.bus.poke(at + i as u16, *b);
        }
    }
    server.session.spec.cpu.pc = program[0].0;
    server.session.spec.cpu.sp = 0xFF00;
    server.session.spec.cpu.iff1 = false;
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    server
}

fn run(server: &mut Server, tstates: i32) {
    call(
        server,
        "run_tstates",
        Json::obj([("tstates", Json::num(tstates))]),
    )
    .unwrap();
}

fn at(address: &str) -> Json {
    Json::obj([("address", Json::str(address))])
}

// Self-modifying code is why a listing can lie: what's printed is one moment's
// bytes, and the operand there is not always that value.
#[test]
fn rewriting_an_operand_that_has_run_is_reported_with_who_did_it() {
    // $8000: LD A,1 : INC A : LD ($8001),A : JR $8000
    let mut server = start(&[(0x8000, &[0x3E, 0x01, 0x3C, 0x32, 0x01, 0x80, 0x18, 0xF8])]);
    run(&mut server, 2_000);
    let report = call(&mut server, "self_modifying", Json::obj([])).unwrap();
    assert!(report.contains("$8001-$8001"), "{report}");
    assert!(report.contains("by $8003"), "who wrote it: {report}");
    assert!(report.contains("$02,$03,$04"), "the values: {report}");
}

// A loader writing a program before it runs, or a variable beside the code, is
// not self-modification. Reporting those would bury the ones that are.
#[test]
fn writes_to_code_not_yet_run_and_writes_of_the_same_value_do_not_count() {
    // $8000: LD A,$3E : LD ($8000),A : LD ($9000),A : JR $8000. The opcode at
    // $8000 is $3E already, so that write changes nothing, and $9000 never runs.
    let mut server = start(&[(
        0x8000,
        &[0x3E, 0x3E, 0x32, 0x00, 0x80, 0x32, 0x00, 0x90, 0x18, 0xF6],
    )]);
    run(&mut server, 2_000);
    let report = call(&mut server, "self_modifying", Json::obj([])).unwrap();
    assert!(report.contains("Nothing has rewritten"), "{report}");
}

// A bare count of writes cannot say "the score went up here", and stopping the
// machine at each one can't be done over a game's worth of frames.
#[test]
fn a_watched_value_logs_each_change_and_what_made_it() {
    // $8000: LD HL,$9000 : INC (HL) : LD A,(HL) : LD (HL),A : JR $8003
    let mut server = start(&[(0x8000, &[0x21, 0x00, 0x90, 0x34, 0x7E, 0x77, 0x18, 0xFB])]);
    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$8003")),
            ("label", Json::str("bump")),
        ]),
    )
    .unwrap();
    call(&mut server, "watch_value", at("$9000")).unwrap();
    run(&mut server, 300);
    let log = call(&mut server, "value_log", Json::obj([])).unwrap();
    assert!(log.contains("$00 -> $01"), "the first change: {log}");
    assert!(log.contains("$01 -> $02"), "the second change: {log}");
    assert!(log.contains("by $8003 bump"), "who did it: {log}");
    assert!(
        !log.contains("$8005"),
        "LD (HL),A writes the same value back, which is no change: {log}"
    );
}

// Every byte kept is one a value changing every frame can flood, so the log has
// to stay bounded and say how much it let go.
#[test]
fn a_watch_keeps_the_latest_changes_and_says_how_many_it_let_go() {
    let mut server = start(&[(0x8000, &[0x21, 0x00, 0x90, 0x34, 0x18, 0xFD])]);
    call(&mut server, "watch_value", at("$9000")).unwrap();
    run(&mut server, 200_000);
    let log = call(
        &mut server,
        "value_log",
        Json::obj([("limit", Json::num(1))]),
    )
    .unwrap();
    assert!(log.contains("4096 changes kept"), "{log}");
    assert!(log.contains("older ones let go"), "{log}");
}

/// A branch that never goes one way, with code behind it that only that side
/// reaches: $8003 JR Z,$8009 with A always 1.
fn one_sided() -> Server {
    start(&[(
        0x8000,
        &[
            0x3E, 0x01, // LD A,1
            0xA7, // AND A
            0x28, 0x04, // JR Z,$8009
            0xC3, 0x00, 0x80, // JP $8000
            0x00, // NOP
            0x3E, 0x07, // $8009: LD A,7
            0x32, 0x00, 0x90, // LD ($9000),A
            0xC3, 0x09, 0x80, // JP $8009
        ],
    )])
}

// The way to read code a game won't go near in a test run. A fork, because
// forcing a flag on the real machine leaves it somewhere it could never have
// got to, and nothing afterwards could be trusted.
#[test]
fn forcing_a_branch_reports_the_new_code_and_leaves_the_machine_as_it_was() {
    let mut server = one_sided();
    run(&mut server, 500);
    let pc = server.session.spec.cpu.pc;
    let frame = server.session.spec.bus.frame;
    let report = call(&mut server, "force_branch", at("$8003")).unwrap();
    assert!(report.contains("JR Z,$8009 taken"), "{report}");
    assert!(report.contains("$8009-$8010"), "the new code: {report}");
    assert_eq!(server.session.spec.cpu.pc, pc, "PC moved");
    assert_eq!(server.session.spec.bus.frame, frame, "the clock moved");
    assert_eq!(
        server.session.spec.bus.peek_raw(0x9000),
        0,
        "the fork's write reached the real machine"
    );
}

// What was forced is not what the program does. If it counted as the branch
// having been seen, coverage would report as covered something that never
// happened, and a reader would stop looking.
#[test]
fn a_forced_branch_still_counts_as_one_sided_and_its_code_is_marked_as_forced() {
    let mut server = one_sided();
    run(&mut server, 500);
    call(&mut server, "force_branch", at("$8003")).unwrap();
    let report = call(
        &mut server,
        "coverage",
        Json::obj([("from", Json::str("$8000")), ("to", Json::str("$8020"))]),
    )
    .unwrap();
    assert!(report.contains("never taken, fell through"), "{report}");
    assert!(
        report.contains("only because a branch was forced"),
        "{report}"
    );
    assert!(server.session.spec.bus.observer.was_executed(0x8009));
    assert!(server.session.spec.bus.observer.was_forced(0x8009));
    assert!(!server.session.spec.bus.observer.was_forced(0x8000));
}

#[test]
fn forcing_says_so_when_there_is_nothing_to_force_or_the_machine_never_gets_there() {
    let mut server = one_sided();
    run(&mut server, 500);
    let err = call(&mut server, "force_branch", at("$8000")).unwrap_err();
    assert!(err.contains("nothing to force"), "{err}");
    let err = call(
        &mut server,
        "force_branch",
        Json::obj([
            ("address", Json::str("$9020")),
            ("side", Json::str("taken")),
            ("reach_frames", Json::num(1)),
        ]),
    )
    .unwrap_err();
    assert!(
        err.contains("nothing to force") || err.contains("did not get"),
        "{err}"
    );
    let pc = server.session.spec.cpu.pc;
    server.session.spec.bus.poke(0x9020, 0x28);
    let err = call(
        &mut server,
        "force_branch",
        Json::obj([
            ("address", Json::str("$9020")),
            ("reach_frames", Json::num(1)),
        ]),
    )
    .unwrap_err();
    assert!(err.contains("did not get to $9020"), "{err}");
    assert_eq!(
        server.session.spec.cpu.pc, pc,
        "a failed fork moved the machine"
    );
}

// The table of which flag each condition tests has to agree with the CPU that
// will act on it, and there are sixteen ways for it not to. The CPU is the
// reference: set the flag as the table says and see which way it goes.
#[test]
fn the_flag_forced_makes_every_condition_go_the_way_that_was_asked() {
    let conditionals: Vec<u8> = (0..8u8)
        .flat_map(|cc| [0xC2 | (cc << 3), 0xC4 | (cc << 3), 0xC0 | (cc << 3)])
        .chain([0x20, 0x28, 0x30, 0x38])
        .collect();
    for opcode in conditionals {
        for take in [true, false] {
            let mut server = Server::new();
            let spec = &mut server.session.spec;
            for (i, b) in [opcode, 0x10, 0x90, 0x00].iter().enumerate() {
                spec.bus.poke(0x8000 + i as u16, *b);
            }
            spec.bus.poke(0xFF00, 0x00);
            spec.bus.poke(0xFF01, 0xA0);
            spec.cpu.pc = 0x8000;
            spec.cpu.sp = 0xFF00;
            let (flag, wanted) = zx_rustrum::coverage::condition(opcode).unwrap();
            spec.cpu.f = if wanted == take { flag } else { 0 };
            let length = zx_rustrum::disasm::disasm(&|a| spec.bus.peek_raw(a), 0x8000).len;
            call(&mut server, "step", Json::obj([])).unwrap();
            let fell_through = server.session.spec.cpu.pc == 0x8000 + length as u16;
            assert_eq!(
                !fell_through, take,
                "opcode ${opcode:02X} asked to be {take} ended at ${:04X}",
                server.session.spec.cpu.pc
            );
        }
    }
}

// DJNZ has no flag, so it is the one conditional forced through a register.
#[test]
fn a_djnz_is_forced_through_b() {
    // $8000: LD B,200 : DJNZ $8000, reloading B each time so that it never falls
    // through by itself, and code past it.
    let mut server = start(&[(
        0x8000,
        &[0x06, 0xC8, 0x10, 0xFC, 0x3E, 0x07, 0xC3, 0x04, 0x80],
    )]);
    run(&mut server, 100);
    let report = call(&mut server, "force_branch", at("$8002")).unwrap();
    assert!(report.contains("not taken"), "{report}");
    assert!(report.contains("$8004-$8008"), "past the loop: {report}");
}

#[test]
fn forcing_can_start_from_a_saved_state() {
    let mut server = one_sided();
    call(
        &mut server,
        "save_state",
        Json::obj([("name", Json::str("start"))]),
    )
    .unwrap();
    run(&mut server, 500);
    let report = call(
        &mut server,
        "force_branch",
        Json::obj([
            ("address", Json::str("$8003")),
            ("state", Json::str("start")),
        ]),
    )
    .unwrap();
    assert!(report.contains("$8009-$8010"), "{report}");
    let err = call(
        &mut server,
        "force_branch",
        Json::obj([
            ("address", Json::str("$8003")),
            ("state", Json::str("nope")),
            ("side", Json::str("taken")),
        ]),
    )
    .unwrap_err();
    assert!(err.contains("no state called"), "{err}");
}

fn listing(server: &mut Server, from: &str, to: &str, facts: bool) -> String {
    // Tests run side by side, and two asking for the same range must not share
    // a file.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let file = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("zxrs-watching-{}-{file}.lst", std::process::id()));
    call(
        server,
        "export_listing",
        Json::obj([
            ("path", Json::str(path.display().to_string())),
            ("from", Json::str(from)),
            ("to", Json::str(to)),
            ("facts", Json::Bool(facts)),
        ]),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    text
}

fn label(server: &mut Server, address: &str, name: &str) {
    call(
        server,
        "set_comment",
        Json::obj([("address", Json::str(address)), ("label", Json::str(name))]),
    )
    .unwrap();
}

// A listing can only say what somebody wrote, unless what was measured goes in
// too. Who uses a routine is the first thing a reader asks and can't be wrong.
#[test]
fn the_listing_says_what_uses_a_label_and_what_it_was_seen_doing() {
    let mut server = start(&[
        // $8000: main: CALL $9000 : JR $8000
        (0x8000, &[0xCD, 0x00, 0x90, 0x18, 0xFB]),
        // $9000: draw: SCF : LD ($4000),A : JP NC,$9000 : RET, which never jumps but
        // names draw from inside draw.
        (0x9000, &[0x37, 0x32, 0x00, 0x40, 0xD2, 0x00, 0x90, 0xC9]),
    ]);
    label(&mut server, "$8000", "main");
    label(&mut server, "$9000", "draw");
    run(&mut server, 2_000);
    let text = listing(&mut server, "$8000", "$9005", true);
    assert!(text.contains("CALL draw"), "{text}");
    assert!(text.contains("; used by: main"), "{text}");
    assert!(text.contains("; seen: called"), "{text}");
    assert!(
        !text.contains("; used by: draw"),
        "a routine does not use itself: {text}"
    );
}

// The comment says what somebody thinks a branch is for. Whether it has ever
// gone that way is a different kind of fact, and they should be told apart.
#[test]
fn the_listing_marks_what_was_measured_about_an_instruction() {
    let mut server = one_sided();
    run(&mut server, 500);
    call(&mut server, "force_branch", at("$8003")).unwrap();
    let text = listing(&mut server, "$8000", "$8010", true);
    assert!(
        text.lines()
            .any(|l| l.starts_with("$8003") && l.contains("[never taken]")),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.starts_with("$8009") && l.contains("[forced]")),
        "{text}"
    );
    assert!(
        !text
            .lines()
            .any(|l| l.starts_with("$8000") && l.contains("[")),
        "nothing to say about $8000: {text}"
    );
    let plain = listing(&mut server, "$8000", "$8010", false);
    assert!(
        !plain.contains("[never taken]"),
        "asked for none of it: {plain}"
    );
}

#[test]
fn the_listing_marks_code_that_was_rewritten_after_it_ran() {
    let mut server = start(&[(0x8000, &[0x3E, 0x01, 0x3C, 0x32, 0x01, 0x80, 0x18, 0xF8])]);
    label(&mut server, "$8000", "patcher");
    run(&mut server, 500);
    let text = listing(&mut server, "$8000", "$8007", true);
    assert!(
        text.lines()
            .any(|l| l.starts_with("$8000") && l.contains("[rewritten by patcher with $02")),
        "{text}"
    );
}

// What makes "the comments look sparse" something to act on: the biggest hole
// is what to read next, and it should shrink as it gets filled.
#[test]
fn unexplained_puts_the_biggest_gap_first_and_counts_what_gets_written() {
    let mut server = start(&[
        (0x8000, &[0xCD, 0x00, 0x90, 0xCD, 0x10, 0x90, 0x18, 0xF8]),
        (0x9000, &[0x00, 0xC9]),
        (0x9010, &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC9]),
    ]);
    label(&mut server, "$8000", "main");
    label(&mut server, "$9000", "small");
    label(&mut server, "$9010", "big");
    run(&mut server, 2_000);

    let range = Json::obj([("from", Json::str("$8000")), ("to", Json::str("$9020"))]);
    let report = call(&mut server, "unexplained", range.clone()).unwrap();
    assert!(report.contains("0 of 3 routines"), "{report}");
    let big = report.find("big").expect(&report);
    let small = report.find("small").expect(&report);
    assert!(big < small, "the larger gap comes first: {report}");

    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$9010")),
            ("comment", Json::str("waits")),
        ]),
    )
    .unwrap();
    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$9011")),
            ("comment", Json::str("second")),
        ]),
    )
    .unwrap();
    let report = call(&mut server, "unexplained", range).unwrap();
    assert!(report.contains("1 of 3 routines"), "{report}");
    assert!(
        report.contains("1 of 12 instructions"),
        "the description is not a comment on an instruction: {report}"
    );
}

// A conditional jump whose target is a bare number in the export is the reader
// having to work out what that place is for. Every one should get a name.
#[test]
fn unexplained_lists_the_places_jumped_to_that_have_no_name() {
    let mut server = one_sided();
    run(&mut server, 500);
    let range = Json::obj([("from", Json::str("$8000")), ("to", Json::str("$8020"))]);
    let report = call(&mut server, "unexplained", range.clone()).unwrap();
    assert!(
        report
            .contains("Jumped to but not named: 2 places: $8000 (from $8005), $8009 (from $8003)"),
        "{report}"
    );
    label(&mut server, "$8009", "seven");
    let report = call(&mut server, "unexplained", range).unwrap();
    assert!(report.contains("1 places: $8000"), "{report}");
}

// The file that is meant to be the disassembly is the .asm, so it carries what
// the listing does, and is only the stretch asked for.
#[test]
fn the_assembly_export_covers_the_range_asked_for_with_what_was_measured() {
    let mut server = start(&[
        (0x8000, &[0xCD, 0x00, 0x90, 0x18, 0xFB]),
        (0x9000, &[0x37, 0x32, 0x00, 0x40, 0xD2, 0x00, 0x90, 0xC9]),
    ]);
    label(&mut server, "$8000", "main");
    label(&mut server, "$9000", "draw");
    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$9000")),
            ("comment", Json::str("Puts a byte in the corner.")),
        ]),
    )
    .unwrap();
    run(&mut server, 2_000);
    let path = std::env::temp_dir().join(format!("zxrs-range-{}.asm", std::process::id()));
    call(
        &mut server,
        "export_asm",
        Json::obj([
            ("path", Json::str(path.display().to_string())),
            ("from", Json::str("$9000")),
            ("to", Json::str("$9007")),
        ]),
    )
    .unwrap();
    let asm = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(asm.contains("        ORG $9000"), "{asm}");
    assert!(
        !asm.contains("CALL draw"),
        "main is outside the range: {asm}"
    );
    assert!(
        asm.contains("draw:\n; Puts a byte in the corner.\n"),
        "a label's description goes above it, however short: {asm}"
    );
    assert!(
        asm.trim_end().ends_with("RET"),
        "nothing after $9007: {asm}"
    );
    assert!(
        asm.contains("; used by: main"),
        "main is outside the file and calls it all the same: {asm}"
    );
    assert!(asm.contains("; seen: called"), "{asm}");
    assert!(asm.contains("JP NC,draw"), "{asm}");
    assert!(asm.contains("[never taken]"), "{asm}");
    assert!(
        !asm.contains("main EQU"),
        "nothing in the file names main, so it needs no EQU: {asm}"
    );
}
