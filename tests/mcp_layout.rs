//! Saying what a program's data is, and seeing the listing and the assembly
//! export show it as that without changing a byte of it.

use zx_rustrum::layout::{rows, Kind, Layout, Typed};
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

fn poke(server: &mut Server, at: &str, bytes: &[u8]) {
    let bytes = Json::arr(bytes.iter().map(|b| Json::num(*b as i32)).collect());
    call(
        server,
        "write_memory",
        Json::obj([("address", Json::str(at)), ("bytes", bytes)]),
    )
    .unwrap();
}

fn listing(server: &mut Server, from: &str, to: &str, tag: &str) -> String {
    let path = std::env::temp_dir().join(format!("zxrs-layout-{tag}-{}.lst", std::process::id()));
    call(
        server,
        "export_listing",
        Json::obj([
            ("path", Json::str(path.display().to_string())),
            ("from", Json::str(from)),
            ("to", Json::str(to)),
        ]),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    text
}

/// The bytes a directive stands for, read the way an assembler reads it.
fn assemble(directive: &str) -> Vec<u8> {
    let (op, rest) = directive.split_once(' ').unwrap();
    match op {
        "DEFB" => rest
            .split(',')
            .map(|b| u8::from_str_radix(b.trim_start_matches('$'), 16).unwrap())
            .collect(),
        "DEFW" => u16::from_str_radix(rest.trim_start_matches('$'), 16)
            .unwrap()
            .to_le_bytes()
            .to_vec(),
        "DEFM" => rest.trim_matches('"').bytes().collect(),
        other => panic!("not a directive: {other}"),
    }
}

// The promise that makes an export safe to build from: however a stretch is
// shown, the rows stand for exactly the bytes that were there, including the
// awkward ones that cannot go in a quoted string.
#[test]
fn every_kind_of_row_stands_for_exactly_the_bytes_it_was_made_from() {
    let memory: Vec<u8> = vec![
        b'H', b'I', b'"', b'$', b'1', b'2', b'3', b'4', b'\\', 0xC7, 0x00, 0xFF, 0x12, 0x34, 0x56,
        0x78, 0x9A,
    ];
    let length = memory.len() as u16;
    let peek = |a: u16| memory[a as usize - 0x8000];
    for kind in [
        Kind::Words,
        Kind::Text,
        Kind::Bitmap {
            width: 2,
            rows: 7,
            header: 3,
            bottom_up: true,
        },
    ] {
        let typed = Typed {
            at: 0x8000,
            kind,
            length,
        };
        let mut rebuilt = Vec::new();
        for row in rows(&typed, &peek) {
            assert_eq!(
                row.at as usize - 0x8000,
                rebuilt.len(),
                "a row out of place in {kind:?}: {row:?}"
            );
            // These would end the string, start an escape, or be turned into
            // a name by the code that names addresses.
            if let Some(quoted) = row.directive.strip_prefix("DEFM ") {
                let inside = quoted.trim_matches('"');
                assert!(
                    !inside.contains(['"', '\\', '$']),
                    "unsafe in a string: {row:?}"
                );
            }
            let bytes = assemble(&row.directive);
            assert_eq!(bytes.len() as u16, row.len, "{kind:?}: {row:?}");
            rebuilt.extend(bytes);
        }
        assert_eq!(rebuilt, memory, "{kind:?} did not give the bytes back");
    }
}

// What a sprite looks like is the fastest way to know what it is: nobody reads
// a run of hex and sees a fuel can.
#[test]
fn a_bitmap_is_listed_with_its_picture_beside_each_row() {
    let mut server = Server::new();
    poke(&mut server, "$8000", &[0, 2, 2, 0xF0, 0x0F, 0x81, 0x18]);
    call(
        &mut server,
        "set_data",
        Json::obj([
            ("address", Json::str("$8000")),
            ("kind", Json::str("bitmap")),
            ("width", Json::num(2)),
            ("rows", Json::num(2)),
            ("header", Json::num(3)),
        ]),
    )
    .unwrap();
    // On the second byte of a row, where no row starts to carry it.
    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$8004")),
            ("comment", Json::str("half way along")),
        ]),
    )
    .unwrap();
    let text = listing(&mut server, "$8000", "$8006", "bitmap");
    assert!(text.contains("half way along"), "note lost: {text}");
    assert!(
        text.contains("$8000  DEFB $00,$02,$02\n"),
        "the header has no picture: {text}"
    );
    assert!(
        text.contains("$8003  DEFB $F0,$0F  ; ####........####"),
        "first row: {text}"
    );
    assert!(
        text.contains("$8005  DEFB $81,$18  ; #......#...##..."),
        "second row: {text}"
    );
}

// Jetpac draws up the screen, so its FUEL reads upside down in memory order. The
// bytes have to stay in the order they are, so the right way up goes beside them.
#[test]
fn a_bitmap_drawn_upwards_shows_the_picture_the_right_way_up_beside_its_own_bytes() {
    let mut layout = Layout::default();
    layout.set_bitmap(0x8000, 1, 2, 0, true);
    let typed = layout.data[&0x8000];
    let peek = |a: u16| if a == 0x8000 { 0xF0 } else { 0x0F };
    let rows = rows(&typed, &peek);
    assert_eq!(rows[0].directive, "DEFB $F0");
    assert_eq!(rows[0].beside, "####.... | ....####");
    assert_eq!(rows[1].directive, "DEFB $0F");
    assert_eq!(rows[1].beside, "....#### | ####....");
}

// A table of addresses is the map of a program, and it is only one if the
// addresses read as the names of what they point at.
#[test]
fn a_table_of_words_names_what_each_one_points_at() {
    let mut server = Server::new();
    poke(&mut server, "$8000", &[0x34, 0x92, 0x00, 0x93]);
    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$9234")),
            ("label", Json::str("handler")),
        ]),
    )
    .unwrap();
    call(
        &mut server,
        "set_data",
        Json::obj([
            ("address", Json::str("$8000")),
            ("kind", Json::str("words")),
            ("length", Json::num(2)),
        ]),
    )
    .unwrap();
    let text = listing(&mut server, "$8000", "$8003", "words");
    assert!(text.contains("$8000  DEFW handler"), "{text}");
    assert!(text.contains("$8002  DEFW $9300"), "{text}");
}

// $0250 is somewhere in the ROM and a symbol file may well name it, but the
// value in LD BC,$0250 is 250 points, and calling it a routine would be a lie.
#[test]
fn a_small_immediate_is_not_taken_for_an_address_but_a_call_is() {
    let mut server = Server::new();
    poke(
        &mut server,
        "$8000",
        &[0x01, 0x50, 0x02, 0xCD, 0x50, 0x02, 0x3A, 0x50, 0x02],
    );
    call(
        &mut server,
        "set_comment",
        Json::obj([
            ("address", Json::str("$0250")),
            ("label", Json::str("in_the_rom")),
        ]),
    )
    .unwrap();
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    server.session.spec.cpu.pc = 0x8000;
    server.session.spec.cpu.sp = 0xFF00;
    call(&mut server, "step", Json::obj([("count", Json::num(1))])).unwrap();
    // The CALL has not been stepped through, only made code by being marked.
    server.session.spec.cpu.pc = 0x8003;
    call(&mut server, "step", Json::obj([("count", Json::num(1))])).unwrap();
    server.session.spec.cpu.pc = 0x8006;
    call(&mut server, "step", Json::obj([("count", Json::num(1))])).unwrap();
    let text = listing(&mut server, "$8000", "$8008", "immediate");
    assert!(text.contains("LD BC,$0250"), "a number: {text}");
    assert!(text.contains("CALL in_the_rom"), "a call: {text}");
    assert!(text.contains("LD A,(in_the_rom)"), "an address: {text}");
}

// A record's own name is what a reader is looking for. Falling back to its
// place in the table means an unnamed slot can still be told from its fellows.
#[test]
fn an_address_inside_a_record_reads_as_the_record_and_the_field() {
    let mut layout = Layout::default();
    layout.read_line("struct object 8 0=type 1=x 2=y");
    layout.read_line("array 5D00 object 3");
    let label = |a: u16| -> String {
        match a {
            0x5D00 => "objects".to_string(),
            0x5D10 => "rocket".to_string(),
            _ => String::new(),
        }
    };
    let name = |a| layout.field_name(a, &label);
    assert_eq!(name(0x5D09).as_deref(), Some("object[1].x"));
    assert_eq!(name(0x5D11).as_deref(), Some("rocket.x"));
    assert_eq!(name(0x5D01).as_deref(), Some("objects.x"));
    assert_eq!(name(0x5D0F).as_deref(), Some("object[1]+7"));
    assert_eq!(name(0x5D18), None, "past the end of the array");
}

// The same record can be read differently by different handlers: +5 is a speed
// to one and a countdown to another.
#[test]
fn the_narrowest_binding_decides_what_a_field_is_called() {
    let mut layout = Layout::default();
    layout.read_line("struct object 8 5=spare");
    layout.read_line("struct jet 8 5=dive_time");
    layout.read_line("bind IX object 6000-7FFF");
    layout.read_line("bind IX jet 63DA-6497");
    assert_eq!(
        layout
            .name_displacement(0x6500, "DEC (IX+$05)")
            .map(|(t, _)| t),
        Some("DEC (IX+spare)".to_string())
    );
    assert_eq!(
        layout
            .name_displacement(0x6423, "DEC (IX+$05)")
            .map(|(t, _)| t),
        Some("DEC (IX+dive_time)".to_string())
    );
    assert_eq!(
        layout.name_displacement(0x6423, "DEC (IY+$05)"),
        None,
        "IY is bound to nothing"
    );
}

// Kept in the same plain file as the labels, so it has to come back whole, and
// a hand-edited line that makes no sense must not take the rest with it.
#[test]
fn what_is_said_about_the_data_survives_being_written_and_read_back() {
    let mut layout = Layout::default();
    layout.set_data(0x8000, Kind::Words, 48);
    layout.set_data(0x8100, Kind::Text, 12);
    layout.set_bitmap(0x8200, 2, 11, 3, true);
    layout.set_bitmap(0x8300, 1, 8, 0, false);
    layout.read_line("struct object 8 0=type 1=x");
    layout.read_line("array 5D00 object 30");
    layout.read_line("bind IX object 6000-7FFF");

    let mut back = Layout::default();
    back.read_line("data nonsense bitmap");
    for line in layout.to_lines() {
        back.read_line(&line);
    }
    assert_eq!(back, layout);
}

// Older notes files, and the labels themselves, must not change meaning now
// these lines exist beside them.
#[test]
fn a_notes_file_keeps_its_labels_and_its_layout_apart() {
    let text = "data 8000 words 2\nstruct object 8 0=type\n8000 table ; the table\n";
    let entries = zx_rustrum::notes::parse(text);
    assert_eq!(
        entries.len(),
        1,
        "only the label line is a note: {entries:?}"
    );
    let layout = zx_rustrum::notes::parse_layout(text);
    assert_eq!(layout.data.len(), 1);
    assert_eq!(layout.structs.len(), 1);
}

// What the assembly export writes is for an assembler, so a field name must
// not go in place of the number. It is said beside the instruction instead.
#[test]
fn the_assembly_export_keeps_typed_data_and_leaves_displacements_as_numbers() {
    let mut server = Server::new();
    // DD 7E 05: LD A,(IX+5), then a word and a run of text.
    poke(&mut server, "$8000", &[0xDD, 0x7E, 0x05, 0xC9]);
    poke(&mut server, "$8010", &[0x00, 0x80, b'O', b'K', 0xA1]);
    for (tool, args) in [
        (
            "set_data",
            Json::obj([
                ("address", Json::str("$8010")),
                ("kind", Json::str("words")),
                ("length", Json::num(1)),
            ]),
        ),
        (
            "set_data",
            Json::obj([
                ("address", Json::str("$8012")),
                ("kind", Json::str("text")),
                ("length", Json::num(3)),
            ]),
        ),
        (
            "define_struct",
            Json::obj([
                ("name", Json::str("thing")),
                ("size", Json::num(8)),
                ("fields", Json::str("5=speed")),
            ]),
        ),
        (
            "bind_struct",
            Json::obj([
                ("register", Json::str("IX")),
                ("struct", Json::str("thing")),
                ("from", Json::str("$8000")),
                ("to", Json::str("$8003")),
            ]),
        ),
    ] {
        call(&mut server, tool, args).unwrap();
    }
    call(
        &mut server,
        "set_comment",
        Json::obj([("address", Json::str("$8000")), ("label", Json::str("go"))]),
    )
    .unwrap();

    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    server.session.spec.cpu.pc = 0x8000;
    call(&mut server, "step", Json::obj([("count", Json::num(1))])).unwrap();
    let code = listing(&mut server, "$8000", "$8002", "bound");
    assert!(code.contains("LD A,(IX+speed)"), "{code}");

    let text = listing(&mut server, "$8010", "$8014", "typed");
    assert!(text.contains("DEFW go"), "{text}");
    assert!(text.contains("DEFM \"OK\""), "{text}");
    assert!(
        text.contains("DEFB $A1  ; '!' with the top bit set"),
        "{text}"
    );

    let path = std::env::temp_dir().join(format!("zxrs-layout-{}.asm", std::process::id()));
    call(
        &mut server,
        "export_asm",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    )
    .unwrap();
    let asm = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(asm.contains("DEFW go"), "the word is a name: {asm:.400}");
    assert!(asm.contains("DEFM \"OK\""), "the text is text");
    assert!(
        asm.lines()
            .any(|l| l.contains("LD A,(IX+$05)") && l.contains("IX+$05 is speed")),
        "the number stays and the name is beside it: {asm}"
    );
}
