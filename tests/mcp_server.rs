//! The MCP server: the handshake, the tool list, and the tools themselves.
//!
//! The tools are driven through `Server::handle`, which is the same path a
//! client takes, so what these tests exercise is the thing that ships rather
//! than the functions underneath it.

use zx_rustrum::mcp::json::{parse, Json};
use zx_rustrum::mcp::{Server, PROTOCOL};

fn request(id: i64, method: &str, params: Json) -> String {
    Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", Json::num(id as f64)),
        ("method", Json::str(method)),
        ("params", params),
    ])
    .to_string()
}

fn call(server: &mut Server, tool: &str, args: Json) -> Result<String, String> {
    let line = request(
        1,
        "tools/call",
        Json::obj([("name", Json::str(tool)), ("arguments", args)]),
    );
    let reply = parse(&server.handle(&line).expect("a call is answered")).unwrap();
    let result = reply.get("result").expect("a result").clone();
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    match result.get("isError").and_then(|e| e.as_bool()) {
        Some(true) => Err(text),
        _ => Ok(text),
    }
}

/// A machine with the real ROM in it, or nothing when the ROM is not here —
/// the same bargain the rest of the suite makes with roms/ and tapes/.
fn with_rom() -> Option<Server> {
    let rom = std::fs::read("roms/48.rom").ok()?;
    let mut server = Server::new();
    server.session.spec.load_rom(&rom);
    server.session.spec.reset();
    Some(server)
}

/// The handshake: a client says hello and is told what it is talking to.
#[test]
fn the_handshake_says_what_this_is() {
    let mut server = Server::new();
    let reply = parse(
        &server
            .handle(&request(1, "initialize", Json::obj([])))
            .expect("initialize is answered"),
    )
    .unwrap();
    let result = reply.get("result").expect("a result");
    assert_eq!(
        result.get("protocolVersion").and_then(|v| v.as_str()),
        Some(PROTOCOL)
    );
    assert!(
        result
            .get("capabilities")
            .and_then(|c| c.get("tools"))
            .is_some(),
        "it offers tools"
    );
    assert_eq!(
        result
            .get("serverInfo")
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str()),
        Some("zx-rustrum")
    );
    // The instructions are what a model reads before it starts guessing.
    let instructions = result
        .get("instructions")
        .and_then(|i| i.as_str())
        .unwrap_or("");
    assert!(instructions.contains("watch_routines"), "{instructions}");
}

/// A notification is answered with silence, which is what JSON-RPC asks for.
#[test]
fn a_notification_gets_no_reply() {
    let mut server = Server::new();
    let line = Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("method", Json::str("notifications/initialized")),
    ])
    .to_string();
    assert!(server.handle(&line).is_none());
}

/// Every tool in the list can be called by the name the list gives, and every
/// one carries a schema. A tool that is advertised and then not there is worse
/// than one that was never offered.
#[test]
fn every_advertised_tool_exists_and_has_a_schema() {
    let mut server = Server::new();
    let reply = parse(
        &server
            .handle(&request(1, "tools/list", Json::Null))
            .unwrap(),
    )
    .unwrap();
    let tools = reply
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .expect("a list of tools")
        .to_vec();
    assert!(tools.len() >= 25, "there are {} tools", tools.len());

    for tool in &tools {
        let name = tool.get("name").and_then(|n| n.as_str()).expect("a name");
        let description = tool
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("");
        assert!(
            description.len() > 30,
            "{name} should say what it is for: {description:?}"
        );
        let schema = tool.get("inputSchema").expect("a schema");
        assert_eq!(schema.get("type").and_then(|t| t.as_str()), Some("object"));
        assert!(schema.get("properties").is_some(), "{name} has properties");

        // Called with nothing at all, a tool either works or explains itself.
        // What it must not do is disappear.
        let answer = call(&mut server, name, Json::obj([]));
        let text = match answer {
            Ok(text) | Err(text) => text,
        };
        assert!(
            !text.contains("no tool called"),
            "{name} is advertised but not implemented"
        );
    }
}

/// An unknown method is a protocol error; an unknown tool is not — the model
/// is told, and can try another one.
#[test]
fn unknown_methods_and_unknown_tools_are_told_apart() {
    let mut server = Server::new();
    let reply = parse(&server.handle(&request(1, "nonsense", Json::Null)).unwrap()).unwrap();
    assert_eq!(
        reply
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_i64()),
        Some(-32601)
    );

    let answer = call(&mut server, "not_a_tool", Json::obj([]));
    assert!(answer.is_err(), "an unknown tool is an error result");
    assert!(answer.unwrap_err().contains("no tool called"));

    // Rubbish on the wire is a parse error with the id JSON-RPC asks for.
    let reply = parse(&server.handle("{oops").unwrap()).unwrap();
    assert_eq!(
        reply
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_i64()),
        Some(-32700)
    );
}

/// Registers come back as the debugger writes them, flags and all.
#[test]
fn the_registers_are_reported_with_the_flags_spelled_out() {
    let mut server = Server::new();
    server.session.spec.cpu.pc = 0x8000;
    server.session.spec.cpu.sp = 0xFF00;
    server.session.spec.cpu.a = 0x3C;
    server.session.spec.cpu.f = 0b0100_0001; // Z and C
    let text = call(&mut server, "registers", Json::obj([])).unwrap();
    assert!(text.contains("PC=$8000"), "{text}");
    assert!(text.contains("SP=$FF00"), "{text}");
    assert!(text.contains("AF=$3C41"), "{text}");
    assert!(text.contains(".Z.....C"), "the flags, spelled out: {text}");
}

/// Memory comes back as hex with the printable bytes beside it, and an address
/// may be written as a number or as $8000 either way round.
#[test]
fn memory_is_read_as_hex_and_addresses_may_be_written_either_way() {
    let mut server = Server::new();
    for (i, b) in b"HELLO".iter().enumerate() {
        server.session.spec.bus.poke(0x8000 + i as u16, *b);
    }
    let by_string = call(
        &mut server,
        "read_memory",
        Json::obj([("address", Json::str("$8000")), ("length", Json::num(16))]),
    )
    .unwrap();
    assert!(by_string.contains("48 45 4C 4C 4F"), "{by_string}");
    assert!(by_string.contains("HELLO"), "{by_string}");

    let by_number = call(
        &mut server,
        "read_memory",
        Json::obj([("address", Json::num(32768)), ("length", Json::num(16))]),
    )
    .unwrap();
    assert_eq!(
        by_number, by_string,
        "32768 and \"$8000\" are the same address"
    );

    // A bare string of digits is decimal, deliberately: reading it as hex is
    // how a disassembly converter once found nothing at all.
    let decimal = call(
        &mut server,
        "read_memory",
        Json::obj([("address", Json::str("32768")), ("length", Json::num(16))]),
    )
    .unwrap();
    assert_eq!(decimal, by_string);
}

/// Writing memory changes what the machine sees, and says that it has.
#[test]
fn memory_can_be_written_to_test_a_theory() {
    let mut server = Server::new();
    call(
        &mut server,
        "write_memory",
        Json::obj([
            ("address", Json::str("$8000")),
            ("bytes", Json::str("3E 05 32 00 60")),
        ]),
    )
    .unwrap();
    assert_eq!(server.session.spec.bus.peek_raw(0x8000), 0x3E);
    assert_eq!(server.session.spec.bus.peek_raw(0x8004), 0x60);

    call(
        &mut server,
        "write_memory",
        Json::obj([
            ("address", Json::num(0x9000)),
            (
                "bytes",
                Json::arr(vec![Json::num(1), Json::num(2), Json::num(255)]),
            ),
        ]),
    )
    .unwrap();
    assert_eq!(server.session.spec.bus.peek_raw(0x9002), 255);
}

/// Stepping runs one instruction and shows what it was.
#[test]
fn stepping_runs_one_instruction_and_says_which() {
    let mut server = Server::new();
    // LD A,$05 / INC A
    for (at, byte) in [(0x8000u16, 0x3Eu8), (0x8001, 0x05), (0x8002, 0x3C)] {
        server.session.spec.bus.poke(at, byte);
    }
    server.session.spec.cpu.pc = 0x8000;

    let text = call(&mut server, "step", Json::obj([])).unwrap();
    assert!(text.contains("$8000  LD A,$05"), "{text}");
    assert_eq!(server.session.spec.cpu.pc, 0x8002);
    assert_eq!(server.session.spec.cpu.a, 5);

    let text = call(&mut server, "step", Json::obj([("count", Json::num(1))])).unwrap();
    assert!(text.contains("INC A"), "{text}");
    assert_eq!(server.session.spec.cpu.a, 6);
}

/// A frame is a frame of the machine's own time, and the ULA is what says how
/// long that is.
#[test]
fn running_a_frame_advances_the_machine_by_one_frame() {
    let Some(mut server) = with_rom() else {
        eprintln!("need roms/48.rom; skipping");
        return;
    };
    let before = server.session.spec.bus.frame;
    let text = call(
        &mut server,
        "run_frames",
        Json::obj([("frames", Json::num(2))]),
    )
    .unwrap();
    assert_eq!(server.session.spec.bus.frame - before, 2, "{text}");
    assert!(text.contains("Ran 2 frames"), "{text}");

    let before = server.session.spec.bus.total_t();
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(10_000))]),
    )
    .unwrap();
    let ran = server.session.spec.bus.total_t() - before;
    assert!(
        (10_000..10_040).contains(&ran),
        "a T-state budget is spent to the instruction that overruns it: {ran}"
    );
}

/// run_until is the breakpoint: it stops where it was told and leaves the
/// machine there.
#[test]
fn run_until_stops_at_the_address_it_was_given() {
    let mut server = Server::new();
    // A loop that falls through to $8010 after five turns.
    let program: [(u16, u8); 8] = [
        (0x8000, 0x06),
        (0x8001, 0x05), // LD B,5
        (0x8002, 0x05), // DEC B
        (0x8003, 0xC2),
        (0x8004, 0x02),
        (0x8005, 0x80), // JP NZ,$8002
        (0x8006, 0xC3),
        (0x8007, 0x06), // JP $8006 — a stop
    ];
    for (at, byte) in program {
        server.session.spec.bus.poke(at, byte);
    }
    server.session.spec.bus.poke(0x8008, 0x80);
    server.session.spec.cpu.pc = 0x8000;

    let text = call(
        &mut server,
        "run_until",
        Json::obj([("address", Json::str("$8006"))]),
    )
    .unwrap();
    assert_eq!(server.session.spec.cpu.pc, 0x8006, "{text}");
    assert!(text.contains("breakpoint at $8006"), "{text}");
    assert_eq!(server.session.spec.cpu.b, 0, "the loop ran to its end");

    // And it says so rather than pretending, when the address is never reached.
    let text = call(
        &mut server,
        "run_until",
        Json::obj([
            ("address", Json::str("$C000")),
            ("max_frames", Json::num(2)),
        ]),
    )
    .unwrap();
    assert!(text.contains("the time asked for ran out"), "{text}");
}

/// The watches are the way to find a routine without knowing its address:
/// stop on the first write to the display file, and there is the drawing code.
#[test]
fn watching_for_a_screen_write_stops_at_the_instruction_that_did_it() {
    let mut server = Server::new();
    // LD HL,$4000 / LD (HL),$FF, then spin.
    for (at, byte) in [
        (0x8000u16, 0x21u8),
        (0x8001, 0x00),
        (0x8002, 0x40),
        (0x8003, 0x36),
        (0x8004, 0xFF),
        (0x8005, 0xC3),
        (0x8006, 0x05),
        (0x8007, 0x80),
    ] {
        server.session.spec.bus.poke(at, byte);
    }
    server.session.spec.cpu.pc = 0x8000;

    let text = call(
        &mut server,
        "watch_events",
        Json::obj([
            ("screen", Json::Bool(true)),
            ("run", Json::Bool(true)),
            ("max_frames", Json::num(2)),
        ]),
    )
    .unwrap();
    assert!(
        text.contains("Wrote to the screen"),
        "it says what stopped it: {text}"
    );
    assert!(text.contains("$8003"), "and where: {text}");
    assert_eq!(server.session.spec.bus.peek_raw(0x4000), 0xFF);
}

/// A snapshot is the whole machine, and putting it back undoes whatever the
/// experiment did. This is what makes poking things safe.
#[test]
fn a_saved_machine_can_be_put_back_exactly() {
    let mut server = Server::new();
    server.session.spec.cpu.pc = 0x8000;
    server.session.spec.cpu.sp = 0xFF00;
    server.session.spec.bus.poke(0x9000, 0x42);

    call(
        &mut server,
        "save_state",
        Json::obj([("name", Json::str("before"))]),
    )
    .unwrap();

    server.session.spec.bus.poke(0x9000, 0x99);
    server.session.spec.cpu.pc = 0xC000;
    assert_eq!(server.session.spec.bus.peek_raw(0x9000), 0x99);

    let text = call(
        &mut server,
        "restore_state",
        Json::obj([("name", Json::str("before"))]),
    )
    .unwrap();
    assert_eq!(server.session.spec.bus.peek_raw(0x9000), 0x42, "{text}");
    assert_eq!(server.session.spec.cpu.pc, 0x8000);
    assert_eq!(server.session.spec.cpu.sp, 0xFF00);

    // And a name nobody saved says what there is rather than failing silently.
    let missing = call(
        &mut server,
        "restore_state",
        Json::obj([("name", Json::str("nope"))]),
    )
    .unwrap_err();
    assert!(missing.contains("before"), "{missing}");
}

/// A .sna keeps only IFF2, sets IFF1 from it, has no HALT, and pushes PC
/// into the two bytes below SP. Nightshade runs with IFF1 clear and IFF2 set
/// and draws with SP pointing into its pictures, so a state put back through
/// a .sna alone switched interrupts on in the middle of a drawing loop, and
/// the game wrote over its own code within a hundred frames. A halted machine
/// before the restore also left the restored one halted at an instruction
/// that is not a HALT.
#[test]
fn a_saved_state_comes_back_with_the_cpu_and_the_stack_as_they_were() {
    let mut server = Server::new();
    let cpu = &mut server.session.spec.cpu;
    cpu.pc = 0x8000;
    cpu.sp = 0x9002;
    cpu.iff1 = false;
    cpu.iff2 = true;
    cpu.halted = false;
    server.session.spec.bus.poke(0x9000, 0x12);
    server.session.spec.bus.poke(0x9001, 0x34);

    call(
        &mut server,
        "save_state",
        Json::obj([("name", Json::str("drawing"))]),
    )
    .unwrap();

    let cpu = &mut server.session.spec.cpu;
    cpu.iff1 = true;
    cpu.halted = true;
    cpu.pc = 0x1234;

    call(
        &mut server,
        "restore_state",
        Json::obj([("name", Json::str("drawing"))]),
    )
    .unwrap();
    let cpu = &server.session.spec.cpu;
    assert!(
        !cpu.iff1 && cpu.iff2,
        "interrupts should be as saved: IFF1 {} IFF2 {}",
        cpu.iff1,
        cpu.iff2
    );
    assert!(
        !cpu.halted,
        "the restored machine was not halted when saved"
    );
    assert_eq!((cpu.pc, cpu.sp), (0x8000, 0x9002), "PC and SP");
    let below_sp = (
        server.session.spec.bus.peek_raw(0x9000),
        server.session.spec.bus.peek_raw(0x9001),
    );
    assert_eq!(
        below_sp,
        (0x12, 0x34),
        "the bytes below SP should be the program's, not the PC a .sna pushed there"
    );
}

/// A snapshot can go to a file and come back from one, which is how a session
/// is picked up again tomorrow.
#[test]
fn a_machine_can_be_written_to_a_file_and_read_back() {
    let Some(mut server) = with_rom() else {
        eprintln!("need roms/48.rom; skipping");
        return;
    };
    let dir = std::env::temp_dir().join("zxrs-mcp-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.sna");

    server.session.spec.bus.poke(0x9000, 0x5A);
    call(
        &mut server,
        "save_state",
        Json::obj([
            ("name", Json::str("disc")),
            ("path", Json::str(path.display().to_string())),
        ]),
    )
    .unwrap();
    assert!(path.exists(), "the file should be there");

    server.session.spec.bus.poke(0x9000, 0x00);
    call(
        &mut server,
        "restore_state",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    )
    .unwrap();
    assert_eq!(server.session.spec.bus.peek_raw(0x9000), 0x5A);
    let _ = std::fs::remove_file(&path);
}

/// A machine that has just been made has $FF where its ROM should be, which
/// is RST $38 forty thousand times over. It runs, it fills the screen with
/// rubbish and it sits at $0038 — which looks from the outside exactly like a
/// game that loaded and crashed.
///
/// Loading a tape has to put a real ROM in first. Every game "loaded" against
/// the empty one and ran for thousands of frames of nothing, and the report
/// said so cheerfully.
#[test]
fn a_tape_is_loaded_into_a_machine_with_a_real_rom_in_it() {
    let tape = "tapes/Jetpac (1983)(Ultimate Play The Game)[16K].tzx";
    if std::fs::read("roms/48.rom").is_err() || std::fs::read(tape).is_err() {
        eprintln!("need roms/48.rom and {tape}; skipping");
        return;
    }
    let mut server = Server::new();
    assert!(
        server.session.spec.bus.rom.iter().all(|b| *b == 0xFF),
        "a fresh machine's ROM space is $FF, not zeros"
    );

    call(
        &mut server,
        "load_tape",
        Json::obj([("path", Json::str(tape))]),
    )
    .unwrap();

    assert!(
        server.session.spec.bus.rom[..4] != [0xFF; 4],
        "the real ROM should have been put in"
    );
    assert_ne!(
        server.session.spec.cpu.pc, 0x0038,
        "and the machine should be running the game, not RST $38"
    );
    assert_eq!(
        server.session.spec.cpu.im, 1,
        "which a machine that ran the ROM's start-up has set"
    );
}

/// The screen comes back as a picture and as something to read, because a
/// model that cannot see one still has to be able to tell whether the sprite
/// is on the screen.
#[test]
fn the_screen_can_be_looked_at() {
    let mut server = Server::new();
    // The top row of character cells filled solid, and green ink on black.
    // The display file's rows are interleaved, so a whole cell is eight
    // addresses a scan line apart rather than eight in a row.
    for row in 0..8u16 {
        for x in 0..32u16 {
            server.session.spec.bus.poke(0x4000 + row * 256 + x, 0xFF);
        }
    }
    for cell in 0..32u16 {
        server.session.spec.bus.poke(0x5800 + cell, 0x04);
    }

    let line = Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", Json::num(1)),
        ("method", Json::str("tools/call")),
        (
            "params",
            Json::obj([("name", Json::str("screen")), ("arguments", Json::obj([]))]),
        ),
    ])
    .to_string();
    let reply = parse(&server.handle(&line).unwrap()).unwrap();
    let content = reply
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .expect("content")
        .to_vec();

    // A picture block first, then the words.
    assert_eq!(
        content[0].get("type").and_then(|t| t.as_str()),
        Some("image"),
        "the screen should come back as an image"
    );
    assert_eq!(
        content[0].get("mimeType").and_then(|t| t.as_str()),
        Some("image/png")
    );
    let data = content[0].get("data").and_then(|d| d.as_str()).unwrap();
    assert!(data.len() > 100, "there should be a PNG in there");
    // The PNG's own signature, base64'd, is how a real one starts.
    assert!(
        data.starts_with("iVBORw0KGgo"),
        "{}",
        &data[..20.min(data.len())]
    );

    let text = content[1].get("text").and_then(|t| t.as_str()).unwrap();
    assert!(text.contains("green"), "the attributes are named: {text}");
    assert!(
        text.lines().any(|l| l.contains("@@@@@@@@")),
        "the top row of cells is solid, and the sketch should show it:\n{text}"
    );
    assert!(
        text.lines()
            .filter(|l| l.trim().is_empty() || l.contains("   "))
            .count()
            > 10,
        "and the rest of the screen is empty:\n{text}"
    );
}

/// Memory read as graphics: the way to find where a game keeps its sprites.
#[test]
fn memory_can_be_read_as_graphics() {
    let mut server = Server::new();
    // A capital H, as the ROM draws one.
    for (i, byte) in [0x00u8, 0x42, 0x42, 0x7E, 0x42, 0x42, 0x42, 0x00]
        .iter()
        .enumerate()
    {
        server.session.spec.bus.poke(0x9000 + i as u16, *byte);
    }
    let text = call(
        &mut server,
        "graphics",
        Json::obj([("address", Json::str("$9000")), ("count", Json::num(1))]),
    )
    .unwrap();
    assert!(text.contains(".#....#."), "the sides of the H:\n{text}");
    assert!(text.contains(".######."), "and its bar:\n{text}");
}

/// Keys can be pressed, which is how a game is driven to the part worth
/// looking at — and how the input routine is found, by pressing something and
/// seeing who reads port $FE.
#[test]
fn keys_can_be_pressed_and_the_machine_sees_them() {
    let mut server = Server::new();
    // A program that spins reading the keyboard, so what it last read is in
    // memory to look at: IN A,($FE) with B=$FE... written out as a loop that
    // stores what it read.
    let program: &[(u16, u8)] = &[
        (0x8000, 0x01),
        (0x8001, 0xFE),
        (0x8002, 0x7F), // LD BC,$7FFE — the half-row with SPACE in it
        (0x8003, 0xED),
        (0x8004, 0x78), // IN A,(C)
        (0x8005, 0x32),
        (0x8006, 0x00),
        (0x8007, 0x90), // LD ($9000),A
        (0x8008, 0xC3),
        (0x8009, 0x00),
        (0x800A, 0x80), // JP $8000
    ];
    for (at, byte) in program {
        server.session.spec.bus.poke(*at, *byte);
    }
    server.session.spec.cpu.pc = 0x8000;

    // Nothing pressed: every bit of the half-row reads high.
    call(
        &mut server,
        "run_frames",
        Json::obj([("frames", Json::num(1))]),
    )
    .unwrap();
    assert_eq!(
        server.session.spec.bus.peek_raw(0x9000) & 0x01,
        0x01,
        "with nothing held, SPACE reads high"
    );

    let text = call(
        &mut server,
        "press_keys",
        Json::obj([
            ("keys", Json::arr(vec![Json::str("SPACE")])),
            ("frames", Json::num(4)),
            ("then_frames", Json::num(0)),
        ]),
    )
    .unwrap();
    assert!(text.contains("SPACE"), "{text}");
    // What is in $9000 is the last read before the key was let go, so it is
    // the proof the machine saw it down.
    assert_eq!(
        server.session.spec.bus.peek_raw(0x9000) & 0x01,
        0x00,
        "the program read SPACE as down while it was held"
    );

    // And once it is let go and the machine runs on, high again.
    call(
        &mut server,
        "run_frames",
        Json::obj([("frames", Json::num(1))]),
    )
    .unwrap();
    assert_eq!(
        server.session.spec.bus.peek_raw(0x9000) & 0x01,
        0x01,
        "and high again afterwards"
    );

    // A key nobody has heard of is refused with the list.
    let why = call(
        &mut server,
        "press_keys",
        Json::obj([("keys", Json::arr(vec![Json::str("F1")]))]),
    )
    .unwrap_err();
    assert!(why.contains("no key called"), "{why}");
}

/// A key held is a key the machine sees down while it is held.
#[test]
fn a_held_key_reads_as_down_while_it_is_held() {
    let mut server = Server::new();
    // Read the half-row with A in it ($FDFE, bit 0) every frame and remember
    // whether it was ever low.
    let program: &[(u16, u8)] = &[
        (0x8000, 0x01),
        (0x8001, 0xFE),
        (0x8002, 0xFD), // LD BC,$FDFE
        (0x8003, 0xED),
        (0x8004, 0x78), // IN A,(C)
        (0x8005, 0xE6),
        (0x8006, 0x01), // AND 1
        (0x8007, 0x20),
        (0x8008, 0xF9), // JR NZ,$8002 — spin until A is pressed
        (0x8009, 0x3E),
        (0x800A, 0x99), // LD A,$99
        (0x800B, 0x32),
        (0x800C, 0x00),
        (0x800D, 0x90), // LD ($9000),A
        (0x800E, 0x76), // HALT
    ];
    for (at, byte) in program {
        server.session.spec.bus.poke(*at, *byte);
    }
    server.session.spec.cpu.pc = 0x8000;

    call(
        &mut server,
        "press_keys",
        Json::obj([
            ("keys", Json::str("A")),
            ("frames", Json::num(3)),
            ("then_frames", Json::num(1)),
        ]),
    )
    .unwrap();
    assert_eq!(
        server.session.spec.bus.peek_raw(0x9000),
        0x99,
        "the program should have seen A go down"
    );
}

/// Finding bytes, which is how a sprite or a string is located when you have
/// its bytes but not its address.
#[test]
fn memory_can_be_searched_for_bytes_and_text() {
    let mut server = Server::new();
    for (i, b) in b"SCORE".iter().enumerate() {
        server.session.spec.bus.poke(0x9100 + i as u16, *b);
    }
    server.session.spec.bus.poke(0xA000, 0xDD);
    server.session.spec.bus.poke(0xA001, 0x21);

    let text = call(
        &mut server,
        "find_bytes",
        Json::obj([("text", Json::str("SCORE"))]),
    )
    .unwrap();
    assert!(text.contains("$9100"), "{text}");

    let text = call(
        &mut server,
        "find_bytes",
        Json::obj([("bytes", Json::str("DD 21"))]),
    )
    .unwrap();
    assert!(text.contains("$A000"), "{text}");

    let text = call(
        &mut server,
        "find_bytes",
        Json::obj([("text", Json::str("NOWHERE"))]),
    )
    .unwrap();
    assert!(text.contains("nothing"), "{text}");
}

/// The variable hunt: save the machine, let something happen, ask what
/// changed. The display file is left out because it changes every frame and
/// would bury the answer.
#[test]
fn what_changed_since_a_saved_state_is_how_a_variable_is_found() {
    let mut server = Server::new();
    server.session.spec.bus.poke(0x9000, 3); // lives
    server.session.spec.bus.poke(0x9001, 0); // something else
    call(
        &mut server,
        "save_state",
        Json::obj([("name", Json::str("before"))]),
    )
    .unwrap();

    server.session.spec.bus.poke(0x9000, 2); // a life lost
    server.session.spec.bus.poke(0x4100, 0xFF); // and the screen redrawn

    let text = call(
        &mut server,
        "changed_since",
        Json::obj([("name", Json::str("before"))]),
    )
    .unwrap();
    assert!(text.contains("$9000  $03 -> $02"), "{text}");
    assert!(
        !text.contains("$4100"),
        "the display file should be left out: {text}"
    );

    // And it can be narrowed to what now holds a particular value.
    let text = call(
        &mut server,
        "changed_since",
        Json::obj([("name", Json::str("before")), ("value", Json::num(2))]),
    )
    .unwrap();
    assert!(text.contains("$9000"), "{text}");

    let text = call(
        &mut server,
        "changed_since",
        Json::obj([("name", Json::str("before")), ("value", Json::num(99))]),
    )
    .unwrap();
    assert!(text.contains("nothing"), "{text}");
}

/// The other half of finding a variable: once an address is known, stop the
/// machine at whatever writes to it. That instruction is the code behind it.
#[test]
fn the_machine_stops_at_whatever_writes_to_a_watched_address() {
    let mut server = Server::new();
    // LD A,$07 / LD ($9000),A, then spin.
    for (at, byte) in [
        (0x8000u16, 0x3Eu8),
        (0x8001, 0x07),
        (0x8002, 0x32),
        (0x8003, 0x00),
        (0x8004, 0x90),
        (0x8005, 0xC3),
        (0x8006, 0x05),
        (0x8007, 0x80),
    ] {
        server.session.spec.bus.poke(at, byte);
    }
    server.session.spec.cpu.pc = 0x8000;

    let text = call(
        &mut server,
        "watch_events",
        Json::obj([
            ("write_to", Json::str("$9000")),
            ("run", Json::Bool(true)),
            ("max_frames", Json::num(2)),
        ]),
    )
    .unwrap();
    assert!(
        text.contains("Wrote $07 to $9000"),
        "it should say what was written: {text}"
    );
    assert!(
        text.contains("$8002"),
        "and which instruction did it: {text}"
    );

    // A range, for a table rather than a single byte.
    server.session.spec.cpu.pc = 0x8000;
    let text = call(
        &mut server,
        "watch_events",
        Json::obj([
            ("write_to", Json::str("$8FF0")),
            ("write_to_end", Json::str("$9010")),
            ("run", Json::Bool(true)),
            ("max_frames", Json::num(2)),
        ]),
    )
    .unwrap();
    assert!(text.contains("$9000"), "{text}");

    // And it can be switched off again.
    let text = call(
        &mut server,
        "watch_events",
        Json::obj([("write_to", Json::Null)]),
    )
    .unwrap();
    assert!(
        !text.contains("writes to"),
        "the watch should be off: {text}"
    );
    assert!(server.session.spec.bus.breaks.write_range.is_none());
}

/// Stepping forward keeping the undo, and going back over it — the thing you
/// want a moment after an instruction has done something unexpected.
#[test]
fn stepped_instructions_can_be_taken_back() {
    let mut server = Server::new();
    // LD A,$05 / LD ($9000),A / INC A
    for (at, byte) in [
        (0x8000u16, 0x3Eu8),
        (0x8001, 0x05),
        (0x8002, 0x32),
        (0x8003, 0x00),
        (0x8004, 0x90),
        (0x8005, 0x3C),
    ] {
        server.session.spec.bus.poke(at, byte);
    }
    server.session.spec.bus.poke(0x9000, 0xEE);
    server.session.spec.cpu.pc = 0x8000;

    call(
        &mut server,
        "step_forward",
        Json::obj([("count", Json::num(3))]),
    )
    .unwrap();
    assert_eq!(server.session.spec.cpu.pc, 0x8006);
    assert_eq!(server.session.spec.cpu.a, 6);
    assert_eq!(server.session.spec.bus.peek_raw(0x9000), 0x05);

    let text = call(
        &mut server,
        "step_back",
        Json::obj([("count", Json::num(2))]),
    )
    .unwrap();
    assert!(text.contains("Went back 2"), "{text}");
    assert_eq!(server.session.spec.cpu.pc, 0x8002, "back to the store");
    assert_eq!(
        server.session.spec.bus.peek_raw(0x9000),
        0xEE,
        "and the write is undone"
    );

    // Running does not keep the undo, and says so rather than pretending.
    let mut server = Server::new();
    let why = call(&mut server, "step_back", Json::obj([])).unwrap_err();
    assert!(why.contains("nothing to go back over"), "{why}");
}

/// A 128K's paging, which a disassembly is meaningless without.
#[test]
fn the_paging_can_be_read_and_changed_on_a_128k() {
    let mut server = Server::new();
    let why = call(&mut server, "paging", Json::obj([])).unwrap_err();
    assert!(why.contains("no paging"), "a 48K has none: {why}");

    let Ok(rom) = std::fs::read("roms/128.rom") else {
        eprintln!("need roms/128.rom; skipping the rest");
        return;
    };
    server
        .session
        .spec
        .set_model(zx_rustrum::machine::Model::Spectrum128, &rom);
    server.session.rom_loaded = true;

    let text = call(&mut server, "paging", Json::obj([])).unwrap();
    assert!(text.contains("RAM bank"), "{text}");

    // Bank 3 at $C000, and the second ROM.
    let text = call(
        &mut server,
        "paging",
        Json::obj([("page_register", Json::num(0x13))]),
    )
    .unwrap();
    assert!(text.contains("RAM bank 3"), "{text}");
    assert!(text.contains("ROM 1"), "{text}");
    assert_eq!(server.session.spec.bus.page_reg & 7, 3);
}

/// A one-line BASIC program on tape, autostarting at line 10:
/// 10 POKE 40000,42
fn poking_tape() -> Vec<u8> {
    fn number(digits: &[u8], value: u16, out: &mut Vec<u8>) {
        out.extend_from_slice(digits);
        out.extend_from_slice(&[0x0E, 0, 0, value as u8, (value >> 8) as u8, 0]);
    }
    let mut line = vec![0xF4]; // POKE
    number(b"40000", 40000, &mut line);
    line.push(b',');
    number(b"42", 42, &mut line);
    line.push(0x0D);
    let mut program = vec![0, 10, line.len() as u8, 0];
    program.extend_from_slice(&line);

    let mut header = vec![0u8]; // a program
    header.extend_from_slice(b"POKER     ");
    let length = program.len() as u16;
    header.extend_from_slice(&length.to_le_bytes());
    header.extend_from_slice(&10u16.to_le_bytes()); // autostart
    header.extend_from_slice(&length.to_le_bytes()); // no variables
    let mut tap = Vec::new();
    for (flag, body) in [(0x00u8, &header), (0xFF, &program)] {
        let checksum = body.iter().fold(flag, |sum, b| sum ^ b);
        let size = body.len() as u16 + 2;
        tap.extend_from_slice(&size.to_le_bytes());
        tap.push(flag);
        tap.extend_from_slice(body);
        tap.push(checksum);
    }
    tap
}

// A program of our own that runs and leaves a mark, since a real game may not
// survive every machine: Jetpac's stub has its LDIR at $5B89, on a byte the 128K's
// ROM decrements, so on a 128K it loads and then crashes. That looked like autoload
// failing, and the first "fix" here changed the keys without changing anything.
#[test]
fn autoload_runs_a_tape_on_every_model_of_spectrum() {
    let path = std::env::temp_dir().join(format!("zxrs-poker-{}.tap", std::process::id()));
    std::fs::write(&path, poking_tape()).unwrap();
    for (model, rom) in [
        ("48k", "roms/48.rom"),
        ("128k", "roms/128.rom"),
        ("+2a", "roms/plus3.rom"),
        ("+3", "roms/plus3.rom"),
    ] {
        if std::fs::read(rom).is_err() {
            eprintln!("need {rom}; skipping {model}");
            continue;
        }
        let mut server = Server::new();
        call(
            &mut server,
            "set_machine",
            Json::obj([("model", Json::str(model))]),
        )
        .unwrap();
        let said = call(
            &mut server,
            "load_tape",
            Json::obj([("path", Json::str(path.display().to_string()))]),
        )
        .unwrap_or_else(|e| panic!("{model}: {e}"));
        let mark = server.session.spec.bus.peek_raw(40000);
        assert_eq!(mark, 42, "{model}: the program did not run. Told: {said}");
    }
    let _ = std::fs::remove_file(&path);
}

// Plenty of 48K games can't be loaded from a 128K's menu, because 128 BASIC keeps
// using memory they load into. 48 BASIC is how they were run on the machine, and
// the mark of having got there is the paging being locked.
#[test]
fn a_tape_can_be_started_from_48_basic_on_the_machines_with_a_menu() {
    let path = std::env::temp_dir().join(format!("zxrs-basic48-{}.tap", std::process::id()));
    std::fs::write(&path, poking_tape()).unwrap();
    for (model, rom) in [
        ("128k", "roms/128.rom"),
        ("+2a", "roms/plus3.rom"),
        ("+3", "roms/plus3.rom"),
    ] {
        if std::fs::read(rom).is_err() {
            eprintln!("need {rom}; skipping {model}");
            continue;
        }
        let mut server = Server::new();
        call(
            &mut server,
            "set_machine",
            Json::obj([("model", Json::str(model))]),
        )
        .unwrap();
        let said = call(
            &mut server,
            "load_tape",
            Json::obj([
                ("path", Json::str(path.display().to_string())),
                ("basic48", Json::Bool(true)),
            ]),
        )
        .unwrap_or_else(|e| panic!("{model}: {e}"));
        let bus = &server.session.spec.bus;
        assert!(bus.paging_locked, "{model}: not in 48 BASIC. Told: {said}");
        assert_eq!(
            bus.peek_raw(40000),
            42,
            "{model}: did not run. Told: {said}"
        );
    }
    let _ = std::fs::remove_file(&path);
}

// The case it exists for. From the 128K's Tape Loader this game loads and resets
// the machine, which wipes its stub from $5B80.
#[test]
fn jetpac_survives_on_a_128k_from_48_basic() {
    let tape = "tapes/Jetpac (1983)(Ultimate Play The Game)[16K].tzx";
    if std::fs::read("roms/128.rom").is_err() || std::fs::read(tape).is_err() {
        eprintln!("need roms/128.rom and {tape}; skipping");
        return;
    }
    let load = |basic48: bool| {
        let mut server = Server::new();
        call(
            &mut server,
            "set_machine",
            Json::obj([("model", Json::str("128k"))]),
        )
        .unwrap();
        let said = call(
            &mut server,
            "load_tape",
            Json::obj([("path", Json::str(tape)), ("basic48", Json::Bool(basic48))]),
        )
        .unwrap();
        (server, said)
    };
    let (crashed, _) = load(false);
    assert_ne!(
        crashed.session.spec.bus.peek_raw(0x5B80),
        0x21,
        "it used to crash from the menu, and if it doesn't any more this test proves nothing"
    );
    let (server, said) = load(true);
    let spec = &server.session.spec;
    assert_eq!(spec.bus.peek_raw(0x5B80), 0x21, "the stub is gone: {said}");
    assert_eq!(
        spec.bus.peek_raw(0x5C79),
        0x83,
        "FRAMES as the tape left it, which a reset would have cleared: {said}"
    );
}

// On a 48K there's no menu to go through, and asking is not a mistake worth
// stopping for: it's already 48 BASIC.
#[test]
fn asking_for_48_basic_on_a_48k_changes_nothing() {
    if std::fs::read("roms/48.rom").is_err() {
        eprintln!("need roms/48.rom; skipping");
        return;
    }
    let path = std::env::temp_dir().join(format!("zxrs-basic48k-{}.tap", std::process::id()));
    std::fs::write(&path, poking_tape()).unwrap();
    let mut server = Server::new();
    call(
        &mut server,
        "load_tape",
        Json::obj([
            ("path", Json::str(path.display().to_string())),
            ("basic48", Json::Bool(true)),
        ]),
    )
    .unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(server.session.spec.bus.peek_raw(40000), 42);
}

// Reading the tape isn't the same as the game running, and on a machine a game
// doesn't suit it ends up back at a prompt. Only the PC can say so.
#[test]
fn autoload_says_where_the_machine_ended_up() {
    if std::fs::read("roms/48.rom").is_err() {
        eprintln!("need roms/48.rom; skipping");
        return;
    }
    let path = std::env::temp_dir().join(format!("zxrs-prompt-{}.tap", std::process::id()));
    std::fs::write(&path, poking_tape()).unwrap();
    let mut server = Server::new();
    let said = call(
        &mut server,
        "load_tape",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    )
    .unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(
        said.contains("PC is in the ROM"),
        "the program has finished and BASIC is waiting: {said}"
    );
}

// A loader that never starts must not look like a game that has. $FF everywhere
// is a machine that runs and never reads a tape, whichever ROM it should have had.
#[test]
fn autoload_says_so_when_the_machine_never_starts_reading_the_tape() {
    let mut server = Server::new();
    server.session.rom_loaded = true;
    // One standard block: a TAP's length, a flag, a byte and its checksum.
    let path = std::env::temp_dir().join(format!("zxrs-autoload-{}.tap", std::process::id()));
    std::fs::write(&path, [3, 0, 0xFF, 0x01, 0xFE]).unwrap();
    let result = call(
        &mut server,
        "load_tape",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    );
    let _ = std::fs::remove_file(&path);
    let err = result.expect_err("nothing read the tape and this reported success");
    assert!(err.contains("never started reading"), "{err}");
}

/// A routine run a few times, for what the export says about it.
fn run_a_small_program(server: &mut Server) {
    // $8000: LD HL,$9000 : INC (HL) : JP $8000
    for (i, b) in [0x21u8, 0x00, 0x90, 0x34, 0xC3, 0x00, 0x80]
        .iter()
        .enumerate()
    {
        server.session.spec.bus.poke(0x8000 + i as u16, *b);
    }
    server.session.spec.cpu.pc = 0x8000;
    call(
        server,
        "run_tstates",
        Json::obj([("tstates", Json::num(500))]),
    )
    .unwrap();
}

fn exported(server: &mut Server) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let file = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("zxrs-asm-{}-{file}.asm", std::process::id()));
    call(
        server,
        "export_asm",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    text
}

// Working through a game means going back to a saved state again and again, and
// each of those wiped what was known: Jetpac's export came out with 1,409
// instructions against the listing's 2,596.
#[test]
fn going_back_to_a_saved_state_keeps_what_has_been_seen_running() {
    let mut server = Server::new();
    call(
        &mut server,
        "save_state",
        Json::obj([("name", Json::str("a"))]),
    )
    .unwrap();
    run_a_small_program(&mut server);
    assert!(
        exported(&mut server).contains("INC (HL)"),
        "ran, so it's code"
    );
    call(
        &mut server,
        "restore_state",
        Json::obj([("name", Json::str("a"))]),
    )
    .unwrap();
    // The state was saved before the program was poked, so put it back.
    for (i, b) in [0x21u8, 0x00, 0x90, 0x34, 0xC3, 0x00, 0x80]
        .iter()
        .enumerate()
    {
        server.session.spec.bus.poke(0x8000 + i as u16, *b);
    }
    let asm = exported(&mut server);
    assert!(
        asm.contains("INC (HL)"),
        "forgotten by going back: {asm:.600}"
    );
}

// The other half: what ran belongs to a program. Another program's bytes at the
// same address must not be listed as code because the last one ran there.
#[test]
fn starting_again_forgets_what_the_last_program_ran() {
    let mut server = Server::new();
    call(
        &mut server,
        "save_state",
        Json::obj([("name", Json::str("a"))]),
    )
    .unwrap();
    run_a_small_program(&mut server);
    call(
        &mut server,
        "restore_state",
        Json::obj([("name", Json::str("a"))]),
    )
    .unwrap();
    call(&mut server, "reset", Json::obj([])).unwrap();
    for (i, b) in [0x21u8, 0x00, 0x90, 0x34, 0xC3, 0x00, 0x80]
        .iter()
        .enumerate()
    {
        server.session.spec.bus.poke(0x8000 + i as u16, *b);
    }
    assert!(!exported(&mut server).contains("INC (HL)"), "after a reset");

    // And a state saved under a different program is a different program.
    let mut server = Server::new();
    call(
        &mut server,
        "save_state",
        Json::obj([("name", Json::str("a"))]),
    )
    .unwrap();
    server.session.loaded = Some("another game".to_string());
    run_a_small_program(&mut server);
    call(
        &mut server,
        "restore_state",
        Json::obj([("name", Json::str("a"))]),
    )
    .unwrap();
    for (i, b) in [0x21u8, 0x00, 0x90, 0x34, 0xC3, 0x00, 0x80]
        .iter()
        .enumerate()
    {
        server.session.spec.bus.poke(0x8000 + i as u16, *b);
    }
    assert!(
        !exported(&mut server).contains("INC (HL)"),
        "after going back to a state from before that game"
    );
}
