//! A paused machine's window stops repainting once nothing is happening.

use egui_kittest::Harness;
use zx_rustrum::machine::Spectrum;
use zx_rustrum::ui::{App, Roms};

fn harness<'a>(running: bool, steps: u64) -> Harness<'a, App> {
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
    app.running = running;
    // Each step is a quarter of a second of egui's clock.
    Harness::builder()
        .with_size([1200.0, 900.0])
        .with_max_steps(steps)
        .build_ui_state(|ui, app: &mut App| app.draw(ui), app)
}

// The window asked to be repainted at the end of every frame whether the
// machine was running or not, so a paused machine was redrawn at the
// display's rate for nothing: 12% to 19% of a core. Paused, it has to settle,
// a second or so after the last thing that happened.
#[test]
fn a_paused_machine_stops_asking_to_be_repainted() {
    let mut h = harness(false, 20);
    let steps = h.try_run();
    assert!(
        steps.is_ok(),
        "still asking for a frame after 20 steps of a quarter of a second: {steps:?}"
    );
}

// Running, the machine is the picture, and every frame is wanted.
#[test]
fn a_running_machine_goes_on_asking_for_frames() {
    let mut h = harness(true, 20);
    assert!(
        h.try_run().is_err(),
        "a running machine settled as if it were paused"
    );
}

// The RAM map's heat fades a frame at a time, by as little as 1 a frame if it
// is set so, and stopped where it was if the window settled first. So a
// paused window goes on until the last of it has gone.
#[test]
fn a_paused_window_repaints_until_the_heat_has_faded() {
    let mut h = harness(false, 400);
    {
        let tracker = &mut h.state_mut().spec.bus.tracker;
        tracker.fade_read = 1;
        tracker.read_heat[0] = 255;
    }
    let steps = h.try_run().expect("it settled once the heat had gone");
    assert!(
        steps >= 254,
        "it settled after {steps} frames, with heat that takes 255 to fade"
    );
    assert_eq!(
        h.state().spec.bus.tracker.read_heat[0],
        0,
        "the heat left when it settled"
    );
}
