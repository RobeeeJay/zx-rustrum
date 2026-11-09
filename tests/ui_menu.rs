//! Drives the real user interface through egui's test harness, so the machine
//! selector is covered as a UI interaction rather than only as the state it
//! calls into.
//!
//! Note on the debug windows: this harness embeds viewports instead of opening
//! real OS windows, so with them enabled they are drawn *inside* the main
//! window and can sit on top of its menu bar, which synthetic clicks then hit
//! instead of the widget underneath. That is an artefact of embedding, not of
//! the emulator, so these tests keep them closed and exercise the widgets
//! directly.

use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use zx_rustrum::machine::{Model, Spectrum};
use zx_rustrum::ui::{zoom_label, App, Roms};

/// An app with stand-in ROMs for every machine, so switching always has an
/// image to load.
fn test_app() -> App {
    let roms = Roms {
        rom48: Some(vec![0x00; 0x4000]),
        rom128: Some(vec![0x00; 0x8000]),
        rom_plus3: Some(vec![0x00; 0x10000]),
        rom_zx81: Some(vec![0x00; 0x2000]),
    };
    let mut app = App::with_roms(Spectrum::new(), String::new(), roms, None);
    app.show_ram_map = false;
    app.show_debugger = false;
    app.show_back_buffer = false;
    app.show_tape = false;
    app.running = false;
    app
}

fn harness_for<'a>(app: App) -> Harness<'a, App> {
    Harness::new_ui_state(|ui, app: &mut App| app.draw(ui), app)
}

fn harness<'a>() -> Harness<'a, App> {
    harness_for(test_app())
}

fn collect_labels(node: &egui_kittest::Node<'_>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(l) = node.accesskit_node().label() {
        out.push(l.to_string());
    }
    for c in node.children() {
        out.extend(collect_labels(&c));
    }
    out
}

/// Labels of the entries in the Machine menu, and the model each selects.
/// Labels of the entries in the always-visible machine dropdown.
const ROW_ENTRIES: [(&str, Model); 4] = [
    ("48K", Model::Spectrum48),
    ("128K", Model::Spectrum128),
    ("+2A", Model::Plus2A),
    ("+3", Model::Plus3),
];

/// Open the machine dropdown. Its trigger is a button labelled with the
/// machine it is showing, followed by the arrow, which is what tells it apart
/// from the entry of the same name inside the list.
fn open_machine_list(h: &mut Harness<'_, App>, showing: &str) {
    h.get_by_label(&format!("{showing}  \u{25be}")).click();
    h.run_steps(3);
}

#[test]
fn every_machine_dropdown_entry_works() {
    for (label, model) in ROW_ENTRIES {
        let mut h = harness();
        h.run_steps(3);
        let mut showing = "48K";
        if model == Model::Spectrum48 {
            h.state_mut().switch_model(Model::Spectrum128);
            h.run_steps(3);
            showing = "128K";
        }
        open_machine_list(&mut h, showing);
        h.get_by_label(label).click();
        h.run_steps(3);
        assert_eq!(
            h.state().spec.bus.model,
            model,
            "clicking the {label} button; status says: {}",
            h.state().status
        );
    }
}

#[test]
fn switching_resumes_the_new_machine() {
    let mut h = harness();
    h.run_steps(3);
    assert!(!h.state().running, "this test starts paused");
    open_machine_list(&mut h, "48K");
    h.get_by_label("128K").click();
    h.run_steps(3);
    assert_eq!(h.state().spec.bus.model, Model::Spectrum128);
    assert!(
        h.state().running,
        "a switch should leave the new machine running, not frozen"
    );
}

#[test]
fn a_missing_rom_reports_an_error_instead_of_failing_silently() {
    let mut app = test_app();
    app.roms.rom128 = None;
    let mut h = harness_for(app);
    h.run_steps(3);

    open_machine_list(&mut h, "48K");
    h.get_by_label("128K (no ROM)").click();
    h.run_steps(3);

    assert_eq!(
        h.state().spec.bus.model,
        Model::Spectrum48,
        "must not switch without a ROM"
    );
    assert!(h.state().status_is_error, "the failure must be flagged");
    assert!(
        h.state().status.contains("128.rom"),
        "the message should name the file: {}",
        h.state().status
    );
}

#[test]
fn the_machine_row_marks_missing_roms() {
    let mut app = test_app();
    app.roms.rom_plus3 = None;
    let mut h = harness_for(app);
    h.run_steps(3);
    open_machine_list(&mut h, "48K");
    let labels = collect_labels(&h.root());
    assert!(
        labels.iter().any(|l| l == "+3 (no ROM)"),
        "missing ROMs should be labelled: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "128K"),
        "present ROMs should not be: {labels:?}"
    );
}

#[test]
fn selecting_the_current_machine_says_so() {
    let mut h = harness();
    h.run_steps(3);
    open_machine_list(&mut h, "48K");
    h.get_by_label("48K").click();
    h.run_steps(3);
    assert_eq!(h.state().spec.bus.model, Model::Spectrum48);
    assert!(
        h.state().status.contains("Already running"),
        "status says: {}",
        h.state().status
    );
}

#[test]
fn the_toolbar_toggles_the_debug_windows() {
    let mut h = harness();
    h.run_steps(3);
    assert!(!h.state().show_tape);
    h.get_by_label("Tape").click();
    h.run_steps(3);
    assert!(
        h.state().show_tape,
        "the Tape toggle should open the window"
    );
}

#[test]
fn the_toolbar_reset_keeps_the_model() {
    let mut h = harness();
    h.run_steps(3);
    open_machine_list(&mut h, "48K");
    h.get_by_label("128K").click();
    h.run_steps(3);
    h.get_by_label("Reset").click();
    h.run_steps(3);
    assert_eq!(h.state().spec.bus.model, Model::Spectrum128);
    assert!(
        h.state().status.contains("Reset"),
        "status says: {}",
        h.state().status
    );
}

// ---------------------------------------------------------------------------
// loading ROM images
// ---------------------------------------------------------------------------

/// Write a ROM image of `len` bytes whose first byte is `marker`.
fn temp_rom(name: &str, len: usize, marker: u8) -> std::path::PathBuf {
    let mut data = vec![0u8; len];
    data[0] = marker;
    let path = std::env::temp_dir().join(format!("zx-test-{name}.rom"));
    std::fs::write(&path, &data).unwrap();
    path
}

#[test]
fn a_32k_rom_selects_the_128k() {
    let path = temp_rom("128", 0x8000, 0xaa);
    let mut h = harness();
    h.run_steps(3);
    assert_eq!(h.state().spec.bus.model, Model::Spectrum48);

    h.state_mut().load_path(&path);
    h.run_steps(3);

    let app = h.state();
    assert_eq!(
        app.spec.bus.model,
        Model::Spectrum128,
        "status says: {}",
        app.status
    );
    assert_eq!(app.spec.bus.rom.len(), 0x8000, "the whole image must fit");
    assert_eq!(app.spec.bus.rom[0], 0xaa, "and actually be loaded");
    assert!(!app.status_is_error, "status says: {}", app.status);
    assert!(
        app.status.contains("128K"),
        "the status should name the machine: {}",
        app.status
    );
}

#[test]
fn a_64k_rom_selects_a_plus3_and_a_16k_rom_a_48k() {
    let mut h = harness();
    h.run_steps(3);

    h.state_mut().load_path(&temp_rom("plus3", 0x10000, 0xbb));
    h.run_steps(3);
    assert_eq!(h.state().spec.bus.model, Model::Plus3);
    assert_eq!(h.state().spec.bus.rom.len(), 0x10000);
    assert_eq!(h.state().spec.bus.rom[0], 0xbb);

    h.state_mut().load_path(&temp_rom("48", 0x4000, 0xcc));
    h.run_steps(3);
    assert_eq!(h.state().spec.bus.model, Model::Spectrum48);
    assert_eq!(h.state().spec.bus.rom.len(), 0x4000);
    assert_eq!(h.state().spec.bus.rom[0], 0xcc);
}

#[test]
fn a_loaded_rom_is_remembered_for_later_switches() {
    let mut app = test_app();
    app.roms.rom128 = None;
    let mut h = harness_for(app);
    h.run_steps(3);

    h.state_mut()
        .load_path(&temp_rom("128-remember", 0x8000, 0xd5));
    h.run_steps(3);
    assert_eq!(h.state().spec.bus.model, Model::Spectrum128);

    // Away and back again: the image loaded from disk should still be used.
    open_machine_list(&mut h, "128K");
    h.get_by_label("48K").click();
    h.run_steps(3);
    assert_eq!(h.state().spec.bus.model, Model::Spectrum48);
    open_machine_list(&mut h, "48K");
    h.get_by_label("128K").click();
    h.run_steps(3);
    assert_eq!(
        h.state().spec.bus.model,
        Model::Spectrum128,
        "status says: {}",
        h.state().status
    );
    assert_eq!(h.state().spec.bus.rom[0], 0xd5, "the loaded ROM was kept");
}

#[test]
fn an_odd_sized_rom_is_reported_rather_than_silently_truncated() {
    let path = temp_rom("odd", 0x5000, 0xee);
    let mut h = harness();
    h.run_steps(3);
    h.state_mut().load_path(&path);
    h.run_steps(3);
    assert!(
        h.state().status_is_error,
        "status says: {}",
        h.state().status
    );
    assert_eq!(
        h.state().spec.bus.model,
        Model::Spectrum48,
        "an unrecognised size should not change machine"
    );
}

#[test]
fn the_overscan_toggle_changes_how_much_border_is_drawn() {
    use zx_rustrum::screen::View;

    let mut h = harness();
    h.run_steps(3);
    assert!(h.state().overscan, "the full border is shown by default");
    assert_eq!(h.state().view(), View::OVERSCAN);

    h.get_by_label("Overscan").click();
    h.run_steps(3);

    assert!(!h.state().overscan);
    let view = h.state().view();
    assert_eq!(view, View::CROPPED);
    assert!(
        view.width() < View::OVERSCAN.width() && view.height() < View::OVERSCAN.height(),
        "cropping should show a smaller area: {view:?}"
    );

    h.get_by_label("Overscan").click();
    h.run_steps(3);
    assert_eq!(h.state().view(), View::OVERSCAN, "and back again");
}

#[test]
fn the_display_is_centred_whatever_size_the_window_is() {
    use egui::{pos2, vec2, Rect};
    use zx_rustrum::screen::centred;

    // Room to spare: equal space on all four sides.
    let area = Rect::from_min_size(pos2(10.0, 20.0), vec2(800.0, 600.0));
    let picture = centred(area, vec2(400.0, 300.0));
    assert_eq!(picture.left() - area.left(), area.right() - picture.right());
    assert_eq!(picture.top() - area.top(), area.bottom() - picture.bottom());
    assert_eq!(picture.center(), area.center());

    // Bigger than the window: it overflows the same amount each way rather
    // than sticking to a corner.
    let big = centred(area, vec2(1200.0, 900.0));
    assert_eq!(area.left() - big.left(), big.right() - area.right());
    assert_eq!(area.top() - big.top(), big.bottom() - area.bottom());
    assert_eq!(big.center(), area.center());

    // An odd size does not drift off centre.
    let odd = centred(area, vec2(333.0, 111.0));
    assert_eq!(odd.center(), area.center());
}

#[test]
fn the_zoom_presets_set_the_display_scale() {
    use zx_rustrum::screen::SCALES;

    // A slice rather than an array, so a list of a different length is a test
    // that fails rather than one that will not compile.
    assert_eq!(SCALES.as_slice(), [1.0, 1.5, 2.0, 3.0, 3.5]);
    assert!(
        SCALES.iter().all(|s| *s >= 1.0),
        "nothing below the machine's own pixels: {SCALES:?}"
    );

    let mut h = harness();
    h.run_steps(3);

    // Half size is not offered: a Spectrum pixel is the smallest thing on the
    // screen, and half of one is a line of the picture thrown away.
    let showing = zoom_label(h.state().scale);
    h.get_by_label(&format!("{showing}  \u{25be}")).click();
    h.run_steps(3);
    assert!(
        h.query_by_label("0.5x").is_none(),
        "the list should not offer half size"
    );
    h.get_by_label(&zoom_label(h.state().scale)).click();
    h.run_steps(3);
    for scale in SCALES {
        let showing = zoom_label(h.state().scale);
        h.get_by_label(&format!("{showing}  \u{25be}")).click();
        h.run_steps(3);
        let label = zoom_label(scale);
        h.get_by_label(&label).click();
        h.run_steps(3);
        assert_eq!(h.state().scale, scale, "choosing {label}");
    }
}

#[test]
fn the_cursor_beam_is_off_until_switched_on() {
    let mut h = harness();
    h.run_steps(3);
    assert!(!h.state().cursor_beam);
    assert!(h.state().beam_t.is_none(), "no beam without the cursor");

    h.get_by_label("Cursor Beam").click();
    h.run_steps(3);
    assert!(h.state().cursor_beam);

    h.get_by_label("Cursor Beam").click();
    h.run_steps(3);
    assert!(!h.state().cursor_beam);
}

#[test]
fn the_zx81_can_be_selected_and_left_again() {
    use zx_rustrum::zx81::Ram;

    let mut h = harness();
    h.run_steps(3);
    assert!(!h.state().on_zx81(), "starts as a Spectrum");

    open_machine_list(&mut h, "48K");
    h.get_by_label("ZX81 16K").click();
    h.run_steps(3);
    assert!(h.state().on_zx81(), "status says: {}", h.state().status);
    assert_eq!(h.state().zx81_ram, Ram::K16);

    // The unexpanded machine is a separate choice.
    open_machine_list(&mut h, "ZX81 16K");
    h.get_by_label("ZX81 1K").click();
    h.run_steps(3);
    assert_eq!(h.state().zx81_ram, Ram::K1);
    assert_eq!(
        h.state().zx81.as_ref().unwrap().bus.ram.len(),
        1024,
        "1K really means 1K"
    );

    // And back to a Spectrum.
    open_machine_list(&mut h, "ZX81 1K");
    h.get_by_label("128K").click();
    h.run_steps(3);
    assert!(!h.state().on_zx81());
    assert_eq!(h.state().spec.bus.model, Model::Spectrum128);
}

#[test]
fn without_a_zx81_rom_the_list_says_so() {
    let mut app = test_app();
    app.roms.rom_zx81 = None;
    let mut h = harness_for(app);
    h.run_steps(3);

    open_machine_list(&mut h, "48K");
    h.get_by_label("ZX81 16K (no ROM)").click();
    h.run_steps(3);
    assert!(!h.state().on_zx81(), "must not switch without a ROM");
    assert!(h.state().status_is_error);
    assert!(
        h.state().status.contains("zx81.rom"),
        "and name the file: {}",
        h.state().status
    );
}

#[test]
fn closing_a_window_lets_it_be_placed_again_when_it_comes_back() {
    // A closed window loses its viewport. Reopening it makes a new one, which
    // the window system puts where it likes, so the emulator has to position
    // it again — otherwise it reappears in the middle of the screen instead of
    // where it was left.
    let mut app = test_app();
    app.show_tape = true;
    let mut h = harness_for(app);
    h.run_steps(3);
    assert!(
        h.state().window_is_placed("tape"),
        "an open window should have been positioned"
    );

    h.state_mut().show_tape = false;
    h.run_steps(3);
    assert!(
        !h.state().window_is_placed("tape"),
        "a closed window must forget where it was put, or it will not be put \
         anywhere when it returns"
    );

    h.state_mut().show_tape = true;
    h.run_steps(3);
    assert!(
        h.state().window_is_placed("tape"),
        "and reopening should position it once more"
    );
}

/// Save snapshot is in the File menu, and not offered on a ZX81 — there is no
/// snapshot format for it here.
#[test]
fn the_file_menu_offers_to_save_a_snapshot() {
    use egui_kittest::kittest::{NodeT, Queryable};

    let mut h = harness();
    h.get_by_label("File").click();
    h.run_steps(3);

    let save = h
        .get_all_by_label_contains("Save snapshot")
        .next()
        .expect("the File menu should offer to save one");
    assert!(
        !save.accesskit_node().is_disabled(),
        "and it should be live on a Spectrum"
    );
}

/// The two recorders live together in a Record section after the Machine one,
/// and say what each of them records.
#[test]
fn the_record_section_holds_both_recorders() {
    // Wide enough that the machine row does not wrap: in a narrow window it
    // does, and where the sections sit relative to each other is what is being
    // asked here.
    let mut h: Harness<'_, App> = Harness::builder()
        .with_size([1800.0, 900.0])
        .build_ui_state(|ui, app: &mut App| app.draw(ui), test_app());
    h.run_steps(3);

    let row = |label: &str| -> Option<(f32, f32)> {
        h.query_all_by_label_contains(label)
            .next()
            .and_then(|node| node.accesskit_node().bounding_box())
            .map(|b| (b.x0 as f32, b.y0 as f32))
    };
    let rzx = row("RZX").expect("the RZX button");
    let video = row("Video").expect("the video button");
    let machine = row("48K").expect("the machine dropdown");

    assert!(
        (rzx.1 - machine.1).abs() < 6.0 && (video.1 - machine.1).abs() < 6.0,
        "both recorders belong on the machine row: {rzx:?} {video:?} against \
         {machine:?}"
    );
    assert!(
        rzx.0 > machine.0 && video.0 > rzx.0,
        "after the Machine section, RZX first: {rzx:?} {video:?}"
    );

    // And the section is labelled, like the others on the row.
    let labels: Vec<String> = h
        .root()
        .children_recursive()
        .filter_map(|node| node.accesskit_node().value().map(|v| v.to_string()))
        .collect();
    assert!(
        labels.iter().any(|v| v == "RECORD"),
        "the section should be named: {labels:?}"
    );

    // Nothing is recording to start with.
    assert!(h.state().video.is_none(), "no video file open");
    assert!(
        h.query_all_by_label_contains("Stop video").next().is_none(),
        "and nothing to stop"
    );
}

/// After a reset the machine ignores the keyboard for a second or so, and the
/// status line says why rather than leaving it looking like a hang.
///
/// The 48K ROM checks every byte of RAM before it does anything else, with
/// interrupts disabled — and the keyboard is read by the interrupt. Measured
/// at 85 frames: nothing is being ignored, there is nothing to ignore it yet.
#[test]
fn a_reset_says_why_the_keyboard_is_deaf_for_a_moment() {
    let mut h = harness();
    h.run_steps(3);
    h.state_mut().speed = 1.0;
    // The toolbar's Reset, rather than the menu's: there are two of them and
    // they say the same thing.
    h.get_all_by_label("Reset").last().unwrap().click();
    h.run_steps(3);

    let status = h.state().status.clone();
    assert!(status.contains("Reset"), "{status}");
    assert!(
        status.contains("checks the RAM"),
        "it should say what the machine is doing: {status}"
    );
    assert!(
        status.contains("1.7s"),
        "and roughly how long that takes: {status}"
    );

    // At half speed it takes twice as long, and says so.
    h.state_mut().speed = 0.5;
    h.get_all_by_label("Reset").last().unwrap().click();
    h.run_steps(3);
    assert!(
        h.state().status.contains("3.4s"),
        "at half speed the wait doubles: {}",
        h.state().status
    );
}

// File → Load snapshot… now goes through load_path, which is what this runs:
// the menu used to switch the machine itself and then hand the file to a
// loader that knew only .sna and .z80, so a .szx said "unsupported snapshot
// type". The menu opens a file dialog and cannot be clicked in a test; the
// loader is tested on its own in tests/szx.rs.
#[test]
fn a_szx_snapshot_loads_into_the_machine_it_is_of() {
    let mut machine = Spectrum::with_model(Model::Spectrum128);
    machine.cpu.pc = 0x9ABC;
    machine.bus.poke(0xC123, 0x77);
    let path = std::env::temp_dir().join(format!("zxrs-ui-{}.szx", std::process::id()));
    std::fs::write(&path, zx_rustrum::szx::save(&machine)).unwrap();

    let mut h = harness();
    h.run_steps(3);
    assert_eq!(h.state().spec.bus.model, Model::Spectrum48);
    h.state_mut().load_path(&path);
    let _ = std::fs::remove_file(&path);
    let app = h.state();
    assert_eq!(
        app.spec.bus.model,
        Model::Spectrum128,
        "status: {}",
        app.status
    );
    assert_eq!(app.spec.cpu.pc, 0x9ABC, "status: {}", app.status);
    assert_eq!(app.spec.bus.peek_raw(0xC123), 0x77);
    assert!(app.status.starts_with("Loaded"), "{}", app.status);
}
