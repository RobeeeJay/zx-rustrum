//! Pictures kept beside a disassembly: the call graph and the sprites.

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

fn label(server: &mut Server, at: &str, name: &str, comment: &str) {
    call(
        server,
        "set_comment",
        Json::obj([
            ("address", Json::str(at)),
            ("label", Json::str(name)),
            ("comment", Json::str(comment)),
        ]),
    )
    .unwrap();
}

fn temp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("zxrs-export-{}-{name}", std::process::id()))
}

// A picture of the calls is how a reader finds the loop the whole game turns on
// before reading any of it, and it has to say the names the notes give.
#[test]
fn the_call_graph_is_an_svg_with_the_routines_named_and_described() {
    let mut server = Server::new();
    let program: &[(u16, &[u8])] = &[
        // $7FF0: CALL $8000 : JR $7FF0, so main is itself called.
        (0x7FF0, &[0xCD, 0x00, 0x80, 0x18, 0xFB]),
        // $8000 main: CALL $9000 : CALL $9100 : RET
        (0x8000, &[0xCD, 0x00, 0x90, 0xCD, 0x00, 0x91, 0xC9]),
        // $9000 draw: CALL $9100 : RET ; $9100 beep: RET
        (0x9000, &[0xCD, 0x00, 0x91, 0xC9]),
        (0x9100, &[0xC9]),
    ];
    for (at, bytes) in program {
        for (i, b) in bytes.iter().enumerate() {
            server.session.spec.bus.poke(at + i as u16, *b);
        }
    }
    server.session.spec.cpu.pc = 0x7FF0;
    server.session.spec.cpu.sp = 0xFF00;
    label(&mut server, "$8000", "main", "Runs the game. Every frame.");
    label(&mut server, "$9000", "draw", "Draws <everything> & more.");
    label(&mut server, "$9100", "beep", "");
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(3_000))]),
    )
    .unwrap();
    let path = temp("graph.svg");
    let said = call(
        &mut server,
        "export_call_graph",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    )
    .unwrap();
    let svg = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(
        svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""),
        "{svg:.200}"
    );
    assert!(svg.trim_end().ends_with("</svg>"));
    for name in ["main", "draw", "beep"] {
        assert!(
            svg.contains(&format!(">{name}</text>")),
            "{name} missing: {said}"
        );
    }
    assert!(
        svg.contains("<title>main: Runs the game.</title>"),
        "the description's first sentence is the tooltip"
    );
    assert!(
        svg.contains("Draws &lt;everything&gt; &amp; more."),
        "escaped"
    );
    // The loop at $7FF0 was never itself called, so the observer has no routine
    // for its call to come from: three calls, not four.
    assert_eq!(
        svg.matches("class=\"call\"").count(),
        3,
        "main to draw and beep, draw to beep: {said}"
    );
}

// A tape's loading runs the ROM's routines among the game's own; a graph of the
// game wants only the game's.
#[test]
fn a_call_graph_can_keep_to_a_range_of_addresses() {
    let mut server = Server::new();
    let program: &[(u16, &[u8])] = &[
        (0x7FF0, &[0xCD, 0x00, 0x80, 0x18, 0xFB]),
        (0x8000, &[0xCD, 0x00, 0x90, 0xCD, 0x00, 0x91, 0xC9]),
        (0x9000, &[0xCD, 0x00, 0x91, 0xC9]),
        (0x9100, &[0xC9]),
    ];
    for (at, bytes) in program {
        for (i, b) in bytes.iter().enumerate() {
            server.session.spec.bus.poke(at + i as u16, *b);
        }
    }
    server.session.spec.cpu.pc = 0x7FF0;
    server.session.spec.cpu.sp = 0xFF00;
    call(&mut server, "watch_routines", Json::obj([])).unwrap();
    call(
        &mut server,
        "run_tstates",
        Json::obj([("tstates", Json::num(3_000))]),
    )
    .unwrap();
    let path = temp("range.svg");
    call(
        &mut server,
        "export_call_graph",
        Json::obj([
            ("path", Json::str(path.display().to_string())),
            ("from", Json::str("$9000")),
        ]),
    )
    .unwrap();
    let svg = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(!svg.contains(">$8000</text>"), "outside the range: {svg}");
    assert_eq!(
        svg.matches("class=\"call\"").count(),
        1,
        "only $9000 to $9100"
    );
}

#[test]
fn a_call_graph_needs_something_watched() {
    let mut server = Server::new();
    let path = temp("empty.svg");
    let err = call(
        &mut server,
        "export_call_graph",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    )
    .unwrap_err();
    assert!(err.contains("watch_routines"), "{err}");
}

// Every sprite as a picture of its own: a masked sprite's outside is transparent,
// what the mask covers is paper, and a sprite stored upside down comes out the
// right way up.
#[test]
fn each_bitmap_is_written_as_a_png_named_by_its_label() {
    let mut server = Server::new();
    // A masked sprite, one byte wide and two rows, stored bottom row first:
    // the bottom row (first in memory) mask $FF ink $81, the top mask $F0 ink $80.
    for (i, b) in [0xFFu8, 0x81, 0xF0, 0x80].iter().enumerate() {
        server.session.spec.bus.poke(0x8000 + i as u16, *b);
    }
    call(
        &mut server,
        "set_data",
        Json::obj([
            ("address", Json::str("$8000")),
            ("kind", Json::str("bitmap")),
            ("width", Json::num(1)),
            ("rows", Json::num(2)),
            ("masked", Json::Bool(true)),
            ("bottom_up", Json::Bool(true)),
        ]),
    )
    .unwrap();
    label(&mut server, "$8000", "ghost", "");
    let dir = temp("sprites");
    call(
        &mut server,
        "export_sprites",
        Json::obj([
            ("dir", Json::str(dir.display().to_string())),
            ("scale", Json::num(1)),
        ]),
    )
    .unwrap();
    let file = dir.join("ghost.png");
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&file).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        (info.width, info.height),
        (8, 2),
        "eight pixels by two rows"
    );
    let pixel = |x: usize, y: usize| &buf[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4];
    // Top row drawn: mask $F0, ink $80.
    assert_eq!(pixel(0, 0), [0, 0, 0, 255], "ink");
    assert_eq!(
        pixel(1, 0),
        [255, 255, 255, 255],
        "covered by the mask: paper"
    );
    assert_eq!(pixel(7, 0), [0, 0, 0, 0], "outside the mask: transparent");
    // Bottom row: mask $FF, ink $81.
    assert_eq!(
        pixel(7, 1),
        [0, 0, 0, 255],
        "the bottom row is the first in memory"
    );
}
