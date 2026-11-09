//! `.szx`: the snapshot format that is a container rather than a dump.

use zx_rustrum::joystick::Kind;
use zx_rustrum::machine::{Model, Spectrum};
use zx_rustrum::szx;

fn machine(model: Model) -> Spectrum {
    let mut spec = Spectrum::with_model(model);
    // Something in every page, so a page written to the wrong one shows up.
    for bank in 0..8usize {
        for i in 0..0x4000 {
            spec.bus.ram[bank * 0x4000 + i] = ((bank * 7 + i) % 251) as u8;
        }
    }
    spec.cpu.pc = 0x8000;
    spec.cpu.sp = 0x7FF0;
    spec.cpu.a = 0x42;
    spec.cpu.f = 0x81;
    spec.cpu.set_bc(0x1234);
    spec.cpu.set_de(0x5678);
    spec.cpu.set_hl(0x9ABC);
    spec.cpu.ix = 0xDEAD;
    spec.cpu.iy = 0xBEEF;
    spec.cpu.i = 0x3F;
    spec.cpu.im = 2;
    spec.cpu.iff1 = true;
    spec.bus.border = 5;
    spec
}

/// A machine written out and read back is the same machine — which is the
/// whole of what a snapshot is for.
#[test]
fn a_machine_written_out_comes_back_the_same() {
    for model in [Model::Spectrum48, Model::Spectrum128, Model::Plus3] {
        let spec = machine(model);
        let bytes = szx::save(&spec);
        assert!(szx::is_szx(&bytes), "{model:?}: it should start with ZXST");
        assert_eq!(szx::probe_model(&bytes), Ok(model), "and say which machine");

        let mut back = Spectrum::with_model(model);
        let note = szx::load(&mut back, &bytes).expect("it should load");
        assert!(note.contains("pages of RAM"), "{note}");

        assert_eq!(back.cpu.pc, 0x8000, "{model:?}: PC");
        assert_eq!(back.cpu.sp, 0x7FF0);
        assert_eq!((back.cpu.a, back.cpu.f), (0x42, 0x81));
        assert_eq!(back.cpu.bc(), 0x1234);
        assert_eq!(back.cpu.de(), 0x5678);
        assert_eq!(back.cpu.hl(), 0x9ABC);
        assert_eq!((back.cpu.ix, back.cpu.iy), (0xDEAD, 0xBEEF));
        assert_eq!(back.cpu.i, 0x3F);
        assert_eq!(back.cpu.im, 2);
        assert!(back.cpu.iff1);
        assert_eq!(back.bus.border, 5);

        // Every page, and each in its own place: a page written to the wrong
        // one is the mistake this catches.
        let pages: &[usize] = if model == Model::Spectrum48 {
            &[0, 2, 5]
        } else {
            &[0, 1, 2, 3, 4, 5, 6, 7]
        };
        for page in pages {
            let from = page * 0x4000;
            assert_eq!(
                &back.bus.ram[from..from + 64],
                &spec.bus.ram[from..from + 64],
                "{model:?}: page {page}"
            );
        }
    }
}

/// The pages are compressed, or a snapshot of a 128K would be 128K.
#[test]
fn the_pages_are_packed_down() {
    let mut spec = Spectrum::with_model(Model::Spectrum128);
    // Empty RAM packs to almost nothing, which is the point of packing it.
    for byte in spec.bus.ram.iter_mut() {
        *byte = 0;
    }
    let bytes = szx::save(&spec);
    assert!(
        bytes.len() < 8192,
        "eight empty pages should pack small, and this is {} bytes",
        bytes.len()
    );

    let mut back = Spectrum::with_model(Model::Spectrum128);
    back.bus.ram[0] = 0xFF;
    szx::load(&mut back, &bytes).expect("it should load");
    assert_eq!(back.bus.ram[0], 0, "and unpack to what went in");
}

/// Which joystick is plugged in goes with the machine, which is the sort of
/// thing .sna has nowhere to put.
#[test]
fn the_joystick_goes_with_the_machine() {
    let mut spec = Spectrum::with_model(Model::Spectrum48);
    spec.bus.joystick.kind = Kind::Sinclair1;
    let bytes = szx::save(&spec);

    let mut back = Spectrum::with_model(Model::Spectrum48);
    szx::load(&mut back, &bytes).expect("loaded");
    assert_eq!(back.bus.joystick.kind, Kind::Sinclair1);
}

/// The 128K's paging comes back, or the machine wakes up with the wrong bank
/// at $C000 and the wrong ROM under it.
#[test]
fn the_paging_comes_back() {
    use zx_rustrum::z80::Bus;

    let mut spec = Spectrum::with_model(Model::Spectrum128);
    // Bank 3 at $C000, the screen in bank 7, ROM 1 paged in: $1B is all
    // three at once, and each of them is a different thing to get wrong.
    spec.bus.io_write(0x7FFD, 0x1B);
    let bytes = szx::save(&spec);

    let mut back = Spectrum::with_model(Model::Spectrum128);
    szx::load(&mut back, &bytes).expect("loaded");
    assert_eq!(back.bus.page_reg, 0x1B);
    assert_eq!(back.bus.screen_bank(), 7, "the screen follows it");
    assert_eq!(back.bus.rom_in_use(), 1);
}

/// A machine whose paging was locked comes back locked. 48K software run on a
/// 128K is run with the lock on, and some of it writes to ports that the 128K
/// takes for $7FFD: Alien 8 reads its keyboard after OUT ($FD),A with $7E in
/// A, which unlocked would page bank 6 in under its stack.
#[test]
fn a_machine_with_its_paging_locked_comes_back_locked() {
    use zx_rustrum::z80::Bus;

    let mut spec = Spectrum::with_model(Model::Spectrum128);
    spec.bus.io_write(0x7FFD, 0x30);
    let bytes = szx::save(&spec);

    let mut back = Spectrum::with_model(Model::Spectrum128);
    szx::load(&mut back, &bytes).expect("loaded");
    assert!(
        back.bus.paging_locked,
        "page register ${:02X} has the lock bit, so the paging is locked",
        back.bus.page_reg
    );
    back.bus.io_write(0x7EFD, 0x7E);
    assert_eq!(
        back.bus.page_reg, 0x30,
        "a write to $7FFD after loading is ignored, as it was before saving"
    );
}

/// A block this emulator has no use for is stepped over, and named rather than
/// lost quietly: a file from another emulator loads, and what could not be
/// put back is said out loud.
#[test]
fn blocks_it_cannot_use_are_named_rather_than_dropped() {
    let spec = Spectrum::with_model(Model::Spectrum48);
    let mut bytes = szx::save(&spec);
    // A microdrive block, as another emulator would write one.
    bytes.extend_from_slice(b"MDRV");
    bytes.extend_from_slice(&8u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 8]);

    let mut back = Spectrum::with_model(Model::Spectrum48);
    let note = szx::load(&mut back, &bytes).expect("it should still load");
    assert!(
        note.contains("MDRV"),
        "it should say what it stepped over: {note}"
    );
}

/// The wrong machine is an error that says so rather than a machine full of
/// somebody else's RAM.
#[test]
fn a_snapshot_of_another_machine_says_which_it_is() {
    let spec = machine(Model::Spectrum128);
    let bytes = szx::save(&spec);
    let mut back = Spectrum::with_model(Model::Spectrum48);
    let err = szx::load(&mut back, &bytes).expect_err("wrong machine");
    assert!(err.contains("128K"), "{err}");
    assert!(err.contains("48K"), "{err}");

    assert!(szx::probe_model(b"not a snapshot").is_err());
    assert!(!szx::is_szx(b"PK\x03\x04"));
}

/// Through the MCP server: a machine saved as .szx and loaded back is the same
/// machine, which is what the format is for.
#[test]
fn the_mcp_server_writes_and_reads_it() {
    use zx_rustrum::mcp::json::Json;
    use zx_rustrum::mcp::tools::{Reply, Session};

    let call = |session: &mut Session, name: &str, args: Json| -> Result<String, String> {
        session.call(name, &args).map(|reply| match reply {
            Reply::Text(text) => text,
            Reply::Picture { text, .. } => text,
        })
    };

    let dir = std::env::temp_dir().join("zx-rustrum-szx-tests");
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let path = dir.join("state.szx");

    let mut session = Session::new();
    if std::fs::read("roms/48.rom").is_err() {
        eprintln!("need roms/48.rom; skipping");
        return;
    }
    session.spec.cpu.pc = 0x1234;
    let out = call(
        &mut session,
        "save_state",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    )
    .expect("saved");
    assert!(out.contains("bytes"), "{out}");
    assert!(
        zx_rustrum::szx::is_szx(&std::fs::read(&path).unwrap()),
        "a .szx name should get the container format"
    );

    let mut next = Session::new();
    let out = call(
        &mut next,
        "load_snapshot",
        Json::obj([("path", Json::str(path.display().to_string()))]),
    )
    .expect("loaded");
    assert!(out.contains("$1234"), "and the machine comes back: {out}");
}

// The Load snapshot… menu, the command line and the MCP server's
// restore_state all load by path through snapshot::load, which knew only .sna
// and .z80: a .szx chosen from the menu said "unsupported snapshot type".
#[test]
fn a_szx_loads_by_its_path_as_well_as_by_its_bytes() {
    let mut spec = machine(Model::Spectrum128);
    spec.cpu.pc = 0x8123;
    spec.bus.poke(0xC000, 0x5A);
    let path = std::env::temp_dir().join(format!("zxrs-szx-path-{}.szx", std::process::id()));
    std::fs::write(&path, szx::save(&spec)).unwrap();

    let mut back = machine(Model::Spectrum128);
    let loaded = zx_rustrum::snapshot::load(&mut back, &path);
    let _ = std::fs::remove_file(&path);
    assert!(loaded.is_ok(), "{loaded:?}");
    assert_eq!(back.cpu.pc, 0x8123);
    assert_eq!(back.bus.peek_raw(0xC000), 0x5A);
}
