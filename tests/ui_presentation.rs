//! The three ways the main window shows the picture, and the keys that go
//! round them.

use eframe::egui;
use egui::{Key, Modifiers, ViewportCommand, ViewportId};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zx_rustrum::machine::Spectrum;
use zx_rustrum::ui::present::{after_report, fit_scale, Presentation};
use zx_rustrum::ui::{App, Roms};

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
    Harness::builder()
        .with_size([1500.0, 1200.0])
        .build_ui_state(|ui, app: &mut App| app.draw(ui), app)
}

/// Command+F, with Command held down the whole frame as it would be, one step
/// of the harness so that its output is the frame the key was read in.
fn command_f(h: &mut Harness<'_, App>) {
    h.input_mut().modifiers = Modifiers::MAC_CMD;
    h.event(egui::Event::Key {
        key: Key::F,
        pressed: true,
        modifiers: Modifiers::MAC_CMD,
        repeat: false,
        physical_key: None,
    });
    h.step();
}

fn let_go(h: &mut Harness<'_, App>) {
    h.input_mut().modifiers = Modifiers::NONE;
    h.key_up(Key::F);
    h.step();
}

fn asked_for(h: &Harness<'_, App>) -> Vec<bool> {
    h.output().viewport_output[&ViewportId::ROOT]
        .commands
        .iter()
        .filter_map(|c| match c {
            ViewportCommand::Fullscreen(on) => Some(*on),
            _ => None,
        })
        .collect()
}

/// Round the three and back: the controls go at the first press and come back
/// at the third, the window system is asked for full screen at the second and
/// asked to leave it at the third, and the zoom somebody chose is still theirs.
#[test]
fn command_f_goes_round_the_three_and_back_to_the_controls() {
    let mut app = test_app();
    app.scale = 2.0;
    let mut h = harness_for(app);
    h.run_steps(2);
    assert!(
        h.query_by_label("Reset").is_some(),
        "controls to begin with"
    );

    command_f(&mut h);
    assert_eq!(h.state().presentation, Presentation::Filled);
    assert_eq!(
        asked_for(&h),
        [false],
        "filling the window is not full screen"
    );
    let_go(&mut h);
    h.run_steps(2);
    assert!(h.query_by_label("Reset").is_none(), "the toolbar has gone");
    assert!(h.query_by_label("Full screen").is_none(), "with its button");

    command_f(&mut h);
    assert_eq!(h.state().presentation, Presentation::FullScreen);
    assert_eq!(asked_for(&h), [true], "the window system is asked");
    let_go(&mut h);
    h.run_steps(2);
    assert!(h.query_by_label("Reset").is_none(), "and still no toolbar");

    command_f(&mut h);
    assert_eq!(h.state().presentation, Presentation::Normal);
    assert_eq!(asked_for(&h), [false], "and asked to leave it");
    let_go(&mut h);
    h.run_steps(2);
    assert!(h.query_by_label("Reset").is_some(), "the controls are back");
    assert_eq!(h.state().scale, 2.0, "at the zoom they were left at");
}

/// F11 does the same, for the machines where Ctrl is SYMBOL SHIFT and Command
/// is not there to press; the button does it for a pointer.
#[test]
fn f11_and_the_button_do_the_same() {
    let mut h = harness_for(test_app());
    h.run_steps(2);
    h.key_press(Key::F11);
    h.run_steps(2);
    assert_eq!(h.state().presentation, Presentation::Filled, "F11");

    let mut h = harness_for(test_app());
    h.run_steps(2);
    h.get_by_label("Full screen").click();
    h.run_steps(2);
    assert_eq!(h.state().presentation, Presentation::Filled, "the button");
}

/// The F of Command+F is not typed at the Spectrum: not while Command is down,
/// and not if Command is let go first with the F still held. An F pressed on
/// its own afterwards is the machine's again.
#[test]
fn the_f_of_the_chord_is_not_typed_at_the_machine() {
    const F: (usize, u8) = (1, 3);
    let mut h = harness_for(test_app());
    h.run_steps(2);
    let f_down = |h: &Harness<'_, App>| h.state().spec.bus.keys[F.0] & (1 << F.1) == 0;

    command_f(&mut h);
    h.run_steps(2);
    assert!(!f_down(&h), "not with Command held");

    h.input_mut().modifiers = Modifiers::NONE;
    h.run_steps(2);
    assert!(!f_down(&h), "nor with Command let go and the F still down");

    h.key_up(Key::F);
    h.run_steps(2);
    h.key_down(Key::F);
    h.run_steps(2);
    assert!(f_down(&h), "an F of its own is the machine's");
}

/// Any Command chord is the host's, not only this one: Command+A selecting
/// something is not somebody typing an A at the Spectrum.
#[test]
fn a_command_chord_is_not_typed_at_the_machine() {
    const A: (usize, u8) = (1, 0);
    let mut h = harness_for(test_app());
    h.run_steps(2);
    h.input_mut().modifiers = Modifiers::MAC_CMD;
    h.key_down(Key::A);
    h.run_steps(2);
    let keys = h.state().spec.bus.keys[A.0];
    assert_eq!(
        keys & (1 << A.1),
        1 << A.1,
        "row reads {keys:08b} with Command held"
    );

    h.input_mut().modifiers = Modifiers::NONE;
    h.run_steps(2);
    let keys = h.state().spec.bus.keys[A.0];
    assert_eq!(
        keys & (1 << A.1),
        0,
        "row reads {keys:08b} once it has gone"
    );
}

/// Filled, a click lands on the pixel under the pointer at the size the
/// picture is drawn, not at the zoom left over from the normal window. Find is
/// the click that says which cell it hit.
#[test]
fn a_click_on_a_filled_picture_lands_on_the_pixel_under_it() {
    const BLOCK: [u8; 8] = [0x18, 0x3C, 0x7E, 0xDB, 0xFF, 0x24, 0x5A, 0xC1];
    let mut app = test_app();
    app.scale = 1.0;
    for y in 0..192u16 {
        for col in 0..32u16 {
            let at = zx_rustrum::machine::screen_bitmap_addr(y, col);
            app.spec.bus.poke(at, BLOCK[(y % 8) as usize]);
        }
    }
    let mut h = harness_for(app);
    h.run_steps(2);
    command_f(&mut h);
    let_go(&mut h);
    h.run_steps(2);

    // The harness draws inside an 8-point margin of its own, which a real
    // window does not have.
    let area = egui::Rect::from_min_max(egui::pos2(8.0, 8.0), egui::pos2(1492.0, 1192.0));
    let view = h.state().view();
    let fit = fit_scale(area.size(), view.width(), view.height());
    let drawn = h.state().drawn_scale();
    assert!(
        (drawn - fit).abs() < 0.01,
        "drawn at {drawn}, where the window fits {fit}"
    );
    assert_eq!(h.state().scale, 1.0, "the chosen zoom is left alone");

    let size = egui::vec2(view.width() as f32, view.height() as f32) * drawn;
    let picture = egui::Rect::from_center_size(area.center(), size);
    let (col, row) = (10.0, 5.0);
    let at = picture.min
        + egui::vec2(
            view.border_x as f32 + col * 8.0 + 4.0,
            view.border_top as f32 + row * 8.0 + 4.0,
        ) * drawn;

    h.state_mut().start_graphics_find();
    h.run_steps(2);
    h.hover_at(at);
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(2);
    }
    let found = h.state().gfx_found.as_ref().map(|p| p.cell);
    assert_eq!(found, Some((10, 5)), "the cell under the pointer");
}

/// The picture fills whichever way the window runs out first, and keeps its
/// shape.
#[test]
fn the_picture_is_fitted_by_the_tighter_side() {
    assert_eq!(fit_scale(egui::vec2(768.0, 2000.0), 384, 304), 2.0, "width");
    assert_eq!(
        fit_scale(egui::vec2(2000.0, 608.0), 384, 304),
        2.0,
        "height"
    );
}

/// Somebody leaving full screen by the window's own green button is back in the
/// normal window, or the next press would seem to do nothing. The window
/// reports that it is not full screen for a moment on the way in, which must
/// not count as leaving.
#[test]
fn leaving_full_screen_by_the_window_itself_brings_the_controls_back() {
    use Presentation::*;
    assert_eq!(
        after_report(FullScreen, false, Some(false)),
        (FullScreen, false),
        "on the way in"
    );
    assert_eq!(
        after_report(FullScreen, false, Some(true)),
        (FullScreen, true),
        "arrived"
    );
    assert_eq!(
        after_report(FullScreen, true, Some(true)),
        (FullScreen, true),
        "staying"
    );
    assert_eq!(
        after_report(FullScreen, true, Some(false)),
        (Normal, false),
        "left"
    );
    assert_eq!(
        after_report(FullScreen, true, None),
        (FullScreen, true),
        "nothing said"
    );
    assert_eq!(after_report(Filled, false, Some(false)), (Filled, false));
}

/// A window full screen is the display's size, and remembering that would bring
/// it back at that size on the next launch.
#[test]
fn the_size_of_a_full_screen_window_is_not_remembered() {
    let mut h = harness_for(test_app());
    let rect = |w, h| egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(w, h));
    let set = |h: &mut Harness<'_, App>, r: egui::Rect| {
        let info = h.input_mut().viewports.entry(ViewportId::ROOT).or_default();
        info.inner_rect = Some(r);
        info.outer_rect = Some(r);
    };
    set(&mut h, rect(720.0, 660.0));
    h.run_steps(2);
    assert_eq!(
        h.state().prefs.window("main").map(|r| r.w),
        Some(720.0),
        "as a window"
    );

    command_f(&mut h);
    let_go(&mut h);
    command_f(&mut h);
    let_go(&mut h);
    assert_eq!(h.state().presentation, Presentation::FullScreen);
    set(&mut h, rect(3000.0, 2000.0));
    h.run_steps(2);
    assert_eq!(
        h.state().prefs.window("main").map(|r| r.w),
        Some(720.0),
        "not as the display"
    );
}
