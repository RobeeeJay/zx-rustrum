//! The Scope window: an oscilloscope and a spectrum analyser of the sound.
//!
//! The probe is meant to be on what the speaker is given, so these check it
//! against the sound card's own queue, and the analyser against notes whose
//! pitch and loudness are known.

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::{Arc, Mutex};

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zx_rustrum::audio::{Audio, SharedQueue};
use zx_rustrum::machine::Spectrum;
use zx_rustrum::scope::{self, Scope, FFT};
use zx_rustrum::ui::scopewin::{note_name, sweep_start, Trigger};
use zx_rustrum::ui::App;

const RATE: f64 = 48_000.0;

/// A mixer with the probe fitted, feeding a queue that stands in for the card.
fn mixer() -> (Audio, SharedQueue) {
    let mut audio = Audio::new(3_500_000.0);
    let q: SharedQueue = Arc::new(Mutex::new(VecDeque::new()));
    audio.attach(q.clone(), RATE);
    audio.volume = 1.0;
    audio.scope = Some(Scope::default());
    (audio, q)
}

/// Click the beeper as a square wave of `hz` for `seconds`.
fn beep(audio: &mut Audio, hz: f64, seconds: f64) {
    let half = (3_500_000.0 / hz / 2.0) as u64;
    let mut t = 0;
    let mut level = false;
    while t < (seconds * 3_500_000.0) as u64 {
        audio.advance_to(t);
        level = !level;
        audio.beeper = if level { 0.5 } else { 0.0 };
        t += half;
    }
    audio.flush();
}

fn sine(hz: f32, amplitude: f32, phase: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amplitude * (2.0 * PI * hz * i as f32 / RATE as f32 + phase).sin())
        .collect()
}

// "True" means the trace is the signal the speaker gets, volume, DC blocker
// and all, not a cleaner idea of what the machine meant to play.
#[test]
fn the_probe_holds_exactly_what_was_sent_to_the_sound_card() {
    let (mut audio, q) = mixer();
    audio.volume = 0.3;
    beep(&mut audio, 440.0, 0.2);
    let sent: Vec<f32> = q.lock().unwrap().iter().copied().collect();
    let probed = audio.scope.as_ref().unwrap().latest(sent.len());
    assert!(sent.len() > 9_000, "only {} samples were sent", sent.len());
    let differ = sent.iter().zip(&probed).filter(|(a, b)| a != b).count();
    assert_eq!(differ, 0, "{differ} of {} samples differ", sent.len());
}

// The probe costs a write per sample on the busiest path in the mixer, so a
// machine nobody is watching must not be carrying one.
#[test]
fn a_mixer_has_no_probe_until_the_window_asks_for_one() {
    assert!(Audio::new(3_500_000.0).scope.is_none());
}

// A restored quicksave takes the sound card over from the machine it replaces;
// the scope watching that card has to come with it, or the trace stops dead.
#[test]
fn a_restored_machine_keeps_the_probe() {
    let (mut live, _q) = mixer();
    let mut restored = Audio::new(3_500_000.0);
    restored.take_output_from(&mut live);
    assert!(restored.scope.is_some() && live.scope.is_none());
}

// A steady note has to stand still on the screen: every sweep starts at the
// same point of the wave, wherever in the wave the newest sample happens to be.
#[test]
fn a_rising_trigger_starts_every_sweep_at_the_same_point_of_the_wave() {
    for phase in [0.0, 0.7, 1.9, 3.3, 5.1] {
        let samples = sine(700.0, 0.4, phase, 2_000);
        let (at, triggered) = sweep_start(&samples, 480, Trigger::Rising);
        assert!(triggered, "no trigger at phase {phase}");
        assert!(
            samples[at - 1] < 0.0 && samples[at] >= 0.0,
            "at phase {phase} the sweep starts at {at}, on {} after {}",
            samples[at],
            samples[at - 1]
        );
        assert!(at + 480 <= samples.len(), "a whole sweep fits after {at}");
        let (down, _) = sweep_start(&samples, 480, Trigger::Falling);
        assert!(
            samples[down - 1] > 0.0 && samples[down] <= 0.0,
            "falling at phase {phase} starts on {} after {}",
            samples[down],
            samples[down - 1]
        );
    }
}

// Hiss wobbling around the level would trigger on every wobble and the trace
// would never settle; it has to fall clearly below before a rise counts.
#[test]
fn noise_smaller_than_the_hysteresis_does_not_trigger() {
    let samples: Vec<f32> = (0..2_000)
        .map(|i| if i % 2 == 0 { 0.005 } else { -0.005 })
        .collect();
    let (_, triggered) = sweep_start(&samples, 480, Trigger::Rising);
    assert!(!triggered);
}

// Full scale is the reference: a sine from -0.5 to +0.5 is 6dB down.
#[test]
fn the_analyser_reads_a_sines_pitch_and_level() {
    let db = scope::spectrum(&sine(1_000.0, 0.5, 0.3, FFT));
    let (hz, level) = scope::loudest(&db, RATE).unwrap();
    assert!((hz - 1_000.0).abs() < 3.0, "read {hz:.1}Hz");
    assert!((level + 6.02).abs() < 1.5, "read {level:.2}dB");
}

// The beeper is a square wave, so its fundamental is the loudest thing in it
// and its odd harmonics stand above everything between them.
#[test]
fn the_analyser_finds_the_note_the_beeper_is_playing() {
    let (mut audio, _q) = mixer();
    beep(&mut audio, 440.0, 0.3);
    let db = scope::spectrum(&audio.scope.as_ref().unwrap().latest(FFT));
    let (hz, _) = scope::loudest(&db, RATE).unwrap();
    assert!((hz - 440.0).abs() < 3.0, "read {hz:.1}Hz");
    let bin = |f: f64| db[(f * FFT as f64 / RATE).round() as usize];
    assert!(
        bin(1_320.0) > bin(880.0) + 20.0,
        "third harmonic {:.1}dB, second {:.1}dB",
        bin(1_320.0),
        bin(880.0)
    );
    assert_eq!(note_name(hz).split(' ').next(), Some("A4"), "{hz:.1}Hz");
}

#[test]
fn notes_are_named_as_a_piano_names_them() {
    assert_eq!(note_name(440.0), "A4 +0¢");
    assert_eq!(note_name(261.63), "C4 +0¢");
    assert_eq!(note_name(446.4), "A4 +25¢");
}

// The window is opened from the main window's Windows row, and opening it is
// what fits the probe; closing it takes the probe off again.
#[test]
fn the_scope_button_opens_the_window_and_fits_the_probe() {
    let mut app = App::with_roms(Spectrum::new(), String::new(), Default::default(), None);
    app.show_ram_map = false;
    app.show_debugger = false;
    app.show_back_buffer = false;
    app.show_tape = false;
    app.running = false;
    let mut h: Harness<'_, App> = Harness::builder()
        .with_size([1500.0, 1000.0])
        .build_ui_state(|ui, app: &mut App| app.draw(ui), app);
    h.run_steps(3);
    assert!(!h.state().show_scope && h.state_mut().audio().scope.is_none());

    h.get_by_label("Scope").click();
    h.run_steps(3);
    assert!(h.state().show_scope, "the toggle opens it");
    assert!(h.state_mut().audio().scope.is_some(), "and fits the probe");
    assert!(
        h.query_by_label("Free run").is_some() && h.query_by_label("Hold").is_some(),
        "the window's controls are drawn"
    );

    // Closed however it is closed: the window's own name is a second "Scope"
    // once it is open, so the toolbar's button is not the only one to find.
    h.state_mut().show_scope = false;
    h.run_steps(3);
    assert!(
        h.state_mut().audio().scope.is_none(),
        "closing takes it off"
    );
}
