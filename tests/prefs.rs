//! Preferences: where the file goes, that it is created on launch, and that
//! the directories files were opened from are remembered.

use std::path::{Path, PathBuf};

use zx_rustrum::machine::{Model, Spectrum};
use zx_rustrum::prefs::{config_dir_from, FileKind, Platform, Prefs, FILE_NAME};
use zx_rustrum::ui::{App, Roms};

/// A scratch directory that cleans up after itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "zx-prefs-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn env_of(pairs: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    let owned: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |key: &str| owned.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

#[test]
fn the_config_directory_follows_each_platforms_convention() {
    let unix = env_of(&[("HOME", "/home/someone")]);
    assert_eq!(
        config_dir_from(Platform::Unix, &unix).unwrap(),
        PathBuf::from("/home/someone/.config/zx-rustrum")
    );

    let xdg = env_of(&[("HOME", "/home/someone"), ("XDG_CONFIG_HOME", "/cfg")]);
    assert_eq!(
        config_dir_from(Platform::Unix, &xdg).unwrap(),
        PathBuf::from("/cfg/zx-rustrum"),
        "XDG_CONFIG_HOME wins on Unix"
    );

    let mac = env_of(&[("HOME", "/Users/someone")]);
    assert_eq!(
        config_dir_from(Platform::MacOs, &mac).unwrap(),
        PathBuf::from("/Users/someone/Library/Application Support/ZX Spectrum Emulator")
    );

    let win = env_of(&[("APPDATA", r"C:\Users\someone\AppData\Roaming")]);
    assert_eq!(
        config_dir_from(Platform::Windows, &win).unwrap(),
        // Built by joining, so the separator matches whatever host runs this.
        PathBuf::from(r"C:\Users\someone\AppData\Roaming").join("ZX Spectrum Emulator")
    );

    // With nothing to go on, there is no directory rather than a guess.
    let empty = env_of(&[]);
    assert!(config_dir_from(Platform::Unix, &empty).is_none());
}

#[test]
fn launching_creates_the_file_if_it_is_not_there() {
    let dir = TempDir::new("create");
    let path = dir.path().join(FILE_NAME);
    assert!(!path.exists());

    let prefs = Prefs::load_or_create_in(dir.path());
    assert!(path.exists(), "the file should be created on launch");
    assert_eq!(prefs.path.as_deref(), Some(path.as_path()));

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("rom_dir"), "with its keys present: {text}");
    assert!(text.starts_with('#'), "and a comment explaining itself");
}

#[test]
fn it_creates_the_directory_too() {
    let dir = TempDir::new("mkdir");
    let nested = dir.path().join("a").join("b");
    let prefs = Prefs::load_or_create_in(&nested);
    assert!(nested.join(FILE_NAME).exists());
    assert!(prefs.path.is_some());
}

#[test]
fn settings_survive_a_round_trip_and_hand_edits_are_kept() {
    let dir = TempDir::new("roundtrip");
    let mut prefs = Prefs::load_or_create_in(dir.path());
    prefs.rom_dir = Some(PathBuf::from("/roms"));
    prefs.tape_dir = Some(PathBuf::from("/tapes"));
    prefs.save();

    // Someone adds a key of their own.
    let path = dir.path().join(FILE_NAME);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}my_own_setting = \"42\"\n")).unwrap();

    let mut reloaded = Prefs::load_or_create_in(dir.path());
    assert_eq!(reloaded.rom_dir, Some(PathBuf::from("/roms")));
    assert_eq!(reloaded.tape_dir, Some(PathBuf::from("/tapes")));
    assert_eq!(reloaded.snapshot_dir, None);

    reloaded.snapshot_dir = Some(PathBuf::from("/snaps"));
    reloaded.save();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("my_own_setting = \"42\""),
        "unknown keys must survive a save: {text}"
    );
    assert!(text.contains("snapshot_dir = \"/snaps\""));
}

#[test]
fn remembering_a_file_stores_its_directory_and_saves() {
    let dir = TempDir::new("remember");
    let mut prefs = Prefs::load_or_create_in(dir.path());
    prefs.remember_file(FileKind::Rom, Path::new("/somewhere/roms/48.rom"));
    prefs.remember_file(FileKind::Tape, Path::new("/elsewhere/tapes/game.tzx"));

    assert_eq!(prefs.rom_dir, Some(PathBuf::from("/somewhere/roms")));
    assert_eq!(prefs.tape_dir, Some(PathBuf::from("/elsewhere/tapes")));

    // And it went to disk without being asked again.
    let reloaded = Prefs::load_or_create_in(dir.path());
    assert_eq!(reloaded.rom_dir, Some(PathBuf::from("/somewhere/roms")));
    assert_eq!(reloaded.tape_dir, Some(PathBuf::from("/elsewhere/tapes")));
}

#[test]
fn file_kinds_are_recognised_by_extension() {
    let cases = [
        ("a.rom", Some(FileKind::Rom)),
        ("a.BIN", Some(FileKind::Rom)),
        ("a.tzx", Some(FileKind::Tape)),
        ("a.tap", Some(FileKind::Tape)),
        ("a.z80", Some(FileKind::Snapshot)),
        ("a.sna", Some(FileKind::Snapshot)),
        ("a.szx", Some(FileKind::Snapshot)),
        ("a.txt", None),
        ("a", None),
    ];
    for (name, want) in cases {
        assert_eq!(FileKind::of_path(Path::new(name)), want, "{name}");
    }
}

// ---------------------------------------------------------------------------
// scanning a directory of ROMs
// ---------------------------------------------------------------------------

fn write_rom(dir: &Path, name: &str, len: usize) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, vec![0u8; len]).unwrap();
    path
}

#[test]
fn scanning_recognises_roms_by_size() {
    let dir = TempDir::new("scan");
    write_rom(dir.path(), "48.rom", 0x4000);
    write_rom(dir.path(), "128.rom", 0x8000);
    write_rom(dir.path(), "plus3.rom", 0x10000);
    write_rom(dir.path(), "notes.txt", 10);
    write_rom(dir.path(), "odd.rom", 0x1234); // not a ROM size

    let found = Roms::scan_directory(dir.path());
    let mut names: Vec<&str> = found.iter().map(|(_, m)| m.name()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["+3", "128K", "48K"], "found {found:?}");
    // The odd-sized file and the text file are ignored.
    assert_eq!(found.len(), 3);
}

#[test]
fn a_named_rom_beats_an_anonymous_one_of_the_same_size() {
    let dir = TempDir::new("names");
    write_rom(dir.path(), "aaa-mystery.rom", 0x8000);
    write_rom(dir.path(), "zx128.rom", 0x8000);

    let found = Roms::scan_directory(dir.path());
    assert_eq!(found.len(), 1, "one ROM per machine");
    assert_eq!(found[0].1, Model::Spectrum128);
    assert!(
        found[0].0.ends_with("zx128.rom"),
        "the name that mentions the machine should win, got {:?}",
        found[0].0
    );
}

#[test]
fn loading_a_rom_remembers_its_directory_and_adopts_its_neighbours() {
    let dir = TempDir::new("adopt");
    let cfg = TempDir::new("adopt-cfg");
    let rom48 = write_rom(dir.path(), "48.rom", 0x4000);
    write_rom(dir.path(), "128.rom", 0x8000);
    write_rom(dir.path(), "plus3.rom", 0x10000);

    let mut app = App::with_roms(Spectrum::new(), String::new(), Roms::default(), None);
    app.prefs = Prefs::load_or_create_in(cfg.path());
    assert!(app.roms.rom128.is_none(), "nothing to start with");

    app.load_path(&rom48);

    assert_eq!(app.spec.bus.model, Model::Spectrum48);
    assert_eq!(
        app.prefs.rom_dir.as_deref(),
        Some(dir.path()),
        "the directory should be remembered"
    );
    assert!(
        app.roms.rom128.is_some() && app.roms.rom_plus3.is_some(),
        "the other machines' ROMs should have been picked up"
    );
    assert!(
        app.status.contains("128.rom") && app.status.contains("plus3.rom"),
        "and mentioned: {}",
        app.status
    );

    // The other machines can now be selected.
    app.switch_model(Model::Plus3);
    assert_eq!(app.spec.bus.model, Model::Plus3);

    // And it was written to the preferences file.
    let reloaded = Prefs::load_or_create_in(cfg.path());
    assert_eq!(reloaded.rom_dir.as_deref(), Some(dir.path()));
}

#[test]
fn adopting_never_replaces_a_rom_that_is_already_loaded() {
    let dir = TempDir::new("no-clobber");
    let cfg = TempDir::new("no-clobber-cfg");
    let rom48 = write_rom(dir.path(), "48.rom", 0x4000);
    std::fs::write(dir.path().join("128.rom"), vec![0xaa; 0x8000]).unwrap();

    let mut app = App::with_roms(Spectrum::new(), String::new(), Roms::default(), None);
    app.prefs = Prefs::load_or_create_in(cfg.path());
    // A 128K ROM the user already chose.
    app.roms.rom128 = Some(vec![0x55; 0x8000]);

    app.load_path(&rom48);
    assert_eq!(
        app.roms.rom128.as_ref().unwrap()[0],
        0x55,
        "the loaded ROM should be left alone"
    );
}

#[test]
fn loading_a_tape_remembers_its_directory() {
    let Some(tape) = std::fs::read_dir("tapes").ok().and_then(|e| {
        let mut v: Vec<PathBuf> = e
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                matches!(
                    p.extension().and_then(|x| x.to_str()),
                    Some("tzx") | Some("tap")
                )
            })
            .collect();
        v.sort();
        v.into_iter().next()
    }) else {
        eprintln!("no tapes/; skipping");
        return;
    };

    let cfg = TempDir::new("tape-dir");
    let mut app = App::with_roms(Spectrum::new(), String::new(), Roms::default(), None);
    app.prefs = Prefs::load_or_create_in(cfg.path());

    app.load_path(&tape);

    assert_eq!(
        app.prefs.tape_dir.as_deref(),
        tape.parent(),
        "the tape's directory should be remembered"
    );
    assert!(app.prefs.rom_dir.is_none(), "and not confused with ROMs");
    let reloaded = Prefs::load_or_create_in(cfg.path());
    assert_eq!(reloaded.tape_dir.as_deref(), tape.parent());
}

// ---------------------------------------------------------------------------
// window geometry and display settings
// ---------------------------------------------------------------------------

use zx_rustrum::prefs::WindowRect;
use zx_rustrum::screen;

#[test]
fn window_geometry_round_trips() {
    let dir = TempDir::new("windows");
    let mut prefs = Prefs::load_or_create_in(dir.path());
    prefs.set_window(
        "main",
        WindowRect {
            x: 100.0,
            y: 50.0,
            w: 800.0,
            h: 600.0,
        },
    );
    prefs.set_window(
        "debugger",
        WindowRect {
            x: -20.0,
            y: 0.0,
            w: 640.0,
            h: 480.0,
        },
    );
    prefs.display_scale = Some(1.5);
    prefs.overscan = Some(false);
    prefs.save();

    let back = Prefs::load_or_create_in(dir.path());
    assert_eq!(
        back.window("main"),
        Some(WindowRect {
            x: 100.0,
            y: 50.0,
            w: 800.0,
            h: 600.0
        })
    );
    assert_eq!(
        back.window("debugger").map(|r| (r.x, r.w)),
        Some((-20.0, 640.0)),
        "a window off the left edge is still remembered"
    );
    assert_eq!(back.display_scale, Some(1.5));
    assert_eq!(back.overscan, Some(false));
    assert_eq!(back.window("nothing"), None);
}

#[test]
fn nonsense_window_entries_are_ignored() {
    let dir = TempDir::new("bad-windows");
    let path = dir.path().join(FILE_NAME);
    std::fs::write(
        &path,
        "window.main = \"1,2,0,0\"\n\
         window.debugger = \"not,a,rectangle,at all\"\n\
         window.tape = \"5,6\"\n\
         window.profiler = \"7,8,300,200\"\n",
    )
    .unwrap();
    let prefs = Prefs::load_or_create_in(dir.path());
    assert_eq!(prefs.window("main"), None, "a zero-sized window is no use");
    assert_eq!(prefs.window("debugger"), None);
    assert_eq!(prefs.window("tape"), None, "too few numbers");
    assert!(prefs.window("profiler").is_some(), "this one is fine");
}

#[test]
fn the_display_settings_are_taken_from_the_file_on_launch() {
    let dir = TempDir::new("apply");
    let mut prefs = Prefs::load_or_create_in(dir.path());
    prefs.display_scale = Some(3.0);
    prefs.overscan = Some(false);
    prefs.save();

    let mut app = App::with_roms(Spectrum::new(), String::new(), Roms::default(), None);
    app.prefs = Prefs::load_or_create_in(dir.path());
    app.apply_prefs();

    assert_eq!(app.scale, 3.0);
    assert!(!app.overscan);
    assert_eq!(app.view(), screen::View::CROPPED);
}

#[test]
fn closing_writes_the_layout_out() {
    let dir = TempDir::new("on-close");
    let mut app = App::with_roms(Spectrum::new(), String::new(), Roms::default(), None);
    app.prefs = Prefs::load_or_create_in(dir.path());
    app.scale = 1.5;
    app.overscan = false;
    app.prefs.set_window(
        "main",
        WindowRect {
            x: 10.0,
            y: 20.0,
            w: 700.0,
            h: 500.0,
        },
    );

    app.save_window_state();

    let back = Prefs::load_or_create_in(dir.path());
    assert_eq!(back.display_scale, Some(1.5));
    assert_eq!(back.overscan, Some(false));
    assert_eq!(
        back.window("main").map(|r| (r.w, r.h)),
        Some((700.0, 500.0))
    );
}

#[test]
fn which_windows_were_open_survives_a_restart() {
    let mut prefs = Prefs::default();
    prefs.open_windows = Some(vec!["debugger".into(), "tape".into()]);
    let text = prefs.to_text();
    assert!(
        text.contains("open_windows = \"debugger,tape\""),
        "expected the open windows in the file: {text}"
    );

    let read = Prefs::parse(&text);
    assert_eq!(
        read.open_windows,
        Some(vec!["debugger".to_string(), "tape".to_string()])
    );
}

#[test]
fn a_file_with_no_windows_open_is_not_the_same_as_one_that_never_said() {
    // An empty list means every window was closed, and they should stay
    // closed; a file with no entry at all leaves the defaults alone.
    let mut prefs = Prefs::default();
    prefs.open_windows = Some(Vec::new());
    let read = Prefs::parse(&prefs.to_text());
    assert_eq!(read.open_windows, Some(Vec::new()));

    let silent = Prefs::parse("rom_dir = \"\"\n");
    assert_eq!(silent.open_windows, None);
}

/// Recordings remember where they were opened from, and not where a snapshot
/// was. They live with the games they are of; sending somebody back to
/// wherever they last opened a snapshot is sending them somewhere else.
#[test]
fn recordings_remember_their_own_directory() {
    let mut prefs = Prefs::default();
    prefs.remember_file(FileKind::Snapshot, std::path::Path::new("/snaps/game.z80"));
    prefs.remember_file(
        FileKind::Recording,
        std::path::Path::new("/games/manic/manic.rzx"),
    );

    assert_eq!(
        prefs.dir_for(FileKind::Recording),
        Some(&std::path::PathBuf::from("/games/manic")),
        "the recording's own directory, not the snapshot's"
    );
    assert_eq!(
        prefs.dir_for(FileKind::Snapshot),
        Some(&std::path::PathBuf::from("/snaps")),
        "and the snapshot's is left where it was"
    );

    // It survives being written out and read back.
    let text = prefs.to_text();
    let read = Prefs::parse(&text);
    assert_eq!(
        read.recording_dir,
        Some(std::path::PathBuf::from("/games/manic"))
    );
}

/// Before a recording has ever been opened, the tapes are the best guess
/// there is: a recording is of something, and that something came off a tape.
#[test]
fn recordings_start_where_the_tapes_are() {
    let mut prefs = Prefs::default();
    assert_eq!(prefs.dir_for(FileKind::Recording), None);

    prefs.remember_file(FileKind::Tape, std::path::Path::new("/games/manic.tap"));
    assert_eq!(
        prefs.dir_for(FileKind::Recording),
        Some(&std::path::PathBuf::from("/games")),
        "wherever the games are"
    );

    prefs.remember_file(FileKind::Recording, std::path::Path::new("/rzx/manic.rzx"));
    assert_eq!(
        prefs.dir_for(FileKind::Recording),
        Some(&std::path::PathBuf::from("/rzx")),
        "and after that, wherever the recordings are"
    );
}

/// Everything the tape window is set to comes back with the emulator.
///
/// A deck somebody has dialled in — a head out of square by a particular
/// amount, a hiss at a particular level, the scope put away to make room for
/// the block list — is tedious to find again, and none of it survived a
/// restart.
#[test]
fn the_tape_windows_settings_survive_a_restart() {
    use zx_rustrum::tape::Quality;
    use zx_rustrum::ui::tape::Trigger;
    use zx_rustrum::ui::Hurry;

    let dir = TempDir::new("tape-settings");

    let mut app = App::with_roms(Spectrum::new(), String::new(), Roms::default(), None);
    app.prefs = Prefs::load_or_create_in(dir.path());
    app.tape.show_scope = false;
    app.tape.show_quality = true;
    app.tape.window_us = 750.0;
    app.tape.trigger = Trigger::Falling;
    app.set_hurry(Hurry::Fastload);
    app.quality = Quality {
        wobble: true,
        wow: 0.02,
        flutter: 0.01,
        alignment: true,
        alignment_offset: 0.4,
        alignment_wobble: 0.2,
        noise: true,
        noise_level: 0.35,
    };
    app.save_window_state();

    let mut next = App::with_roms(Spectrum::new(), String::new(), Roms::default(), None);
    next.prefs = Prefs::load_or_create_in(dir.path());
    next.apply_prefs();

    assert!(!next.tape.show_scope, "the scope was put away");
    assert!(
        next.tape.show_quality,
        "and the deck's failings were on show"
    );
    assert_eq!(next.tape.window_us, 750.0, "sweep");
    assert_eq!(next.tape.trigger, Trigger::Falling, "trigger");
    assert!(
        next.tape_flash() && next.tape_boost(),
        "the tape was being got through on Fastload"
    );
    assert_eq!(
        next.quality, app.quality,
        "and the deck's failings should be exactly what they were"
    );
}
