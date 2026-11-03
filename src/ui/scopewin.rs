//! The Scope window: an oscilloscope of the sound the machine is making, and a
//! spectrum analyser of the same signal under it.
//!
//! Both are drawn from the samples sent to the sound card (`crate::scope`), so
//! what is on the screen is what is coming out of the speaker: the volume
//! knob changes the height of the trace, and a muted machine draws a flat line.

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Vec2};

use crate::scope::{self, Scope, FFT};
use crate::ui::{theme, App};

/// Milliseconds a division across the scope, of which there are ten.
const TIMEBASES: [(f64, &str); 6] = [
    (0.1, "0.1 ms"),
    (0.2, "0.2 ms"),
    (0.5, "0.5 ms"),
    (1.0, "1 ms"),
    (2.0, "2 ms"),
    (5.0, "5 ms"),
];
const GAINS: [f32; 4] = [1.0, 2.0, 4.0, 8.0];
/// The analyser's floor: below this a sixteen-bit sound card has nothing.
const FLOOR_DB: f32 = -96.0;
/// How fast a peak falls back on the analyser, so a note that has stopped
/// fades rather than vanishing between one frame and the next.
const FALL_DB_PER_S: f32 = 60.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Trigger {
    Rising,
    Falling,
    Off,
}

pub struct View {
    timebase: usize,
    gain: usize,
    trigger: Trigger,
    /// A copy of the probe taken when Hold was pressed, drawn instead of the
    /// live signal until it is let go.
    held: Option<Scope>,
    /// The analyser as last drawn, falling back towards what it reads now.
    shown: Vec<f32>,
}

impl Default for View {
    fn default() -> Self {
        View {
            timebase: 3,
            gain: 0,
            trigger: Trigger::Rising,
            held: None,
            shown: Vec::new(),
        }
    }
}

pub fn ui(app: &mut App, ui: &mut egui::Ui) {
    let rate = app.audio().sample_rate;
    controls(app, ui);
    let probe = match &app.scope_view.held {
        Some(held) => held.clone(),
        None => app.audio().scope.clone().unwrap_or_default(),
    };
    let height = ((ui.available_height() - 40.0) / 2.0).max(120.0);
    oscilloscope(&app.scope_view, &probe, rate, ui, height);
    ui.add_space(6.0);
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    analyser(&mut app.scope_view, &probe, rate, dt, ui, height);
}

fn controls(app: &mut App, ui: &mut egui::Ui) {
    let view = &mut app.scope_view;
    ui.horizontal_wrapped(|ui| {
        theme::group_label(ui, "Time/div");
        for (i, (_, name)) in TIMEBASES.iter().enumerate() {
            if theme::selectable(ui, view.timebase == i, name).clicked() {
                view.timebase = i;
            }
        }
        ui.separator();
        theme::group_label(ui, "Gain");
        for (i, gain) in GAINS.iter().enumerate() {
            if theme::selectable(ui, view.gain == i, &format!("×{gain}")).clicked() {
                view.gain = i;
            }
        }
    });
    let mut hold = view.held.is_some();
    ui.horizontal_wrapped(|ui| {
        theme::group_label(ui, "Trigger");
        for (kind, name) in [
            (Trigger::Rising, "Rising"),
            (Trigger::Falling, "Falling"),
            (Trigger::Off, "Free run"),
        ] {
            if theme::selectable(ui, view.trigger == kind, name).clicked() {
                view.trigger = kind;
            }
        }
        ui.separator();
        theme::toggle(ui, &mut hold, "Hold")
            .on_hover_text("Freeze both pictures on what was playing when it was pressed");
    });
    if hold != view.held.is_some() {
        app.scope_view.held = if hold {
            Some(app.audio().scope.clone().unwrap_or_default())
        } else {
            None
        };
    }
}

/// Where the trace starts in `samples`, and whether a trigger put it there.
pub fn sweep_start(samples: &[f32], width: usize, trigger: Trigger) -> (usize, bool) {
    let found = match trigger {
        Trigger::Rising => scope::trigger(samples, width, 0.0),
        Trigger::Falling => {
            let flipped: Vec<f32> = samples.iter().map(|s| -s).collect();
            scope::trigger(&flipped, width, 0.0)
        }
        Trigger::Off => None,
    };
    match found {
        Some(at) => (at, true),
        None => (samples.len().saturating_sub(width), false),
    }
}

fn oscilloscope(view: &View, probe: &Scope, rate: f64, ui: &mut egui::Ui, height: f32) {
    let (ms, name) = TIMEBASES[view.timebase];
    let width = ((ms * 10.0 / 1000.0 * rate).round() as usize).max(2);
    // Twice the sweep to look for a trigger in, so a note as slow as the
    // sweep is wide still has one.
    let samples = probe.latest((width * 2).min(scope::KEPT));
    let (start, triggered) = sweep_start(&samples, width, view.trigger);
    let gain = GAINS[view.gain];

    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::LCD_BG);
    let grid = Stroke::new(1.0, theme::LCD_GRID);
    for i in 0..=10 {
        let x = rect.left() + rect.width() * i as f32 / 10.0;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            grid,
        );
    }
    for i in 0..=8 {
        let y = rect.top() + rect.height() * i as f32 / 8.0;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            grid,
        );
    }
    let mid = rect.center().y;
    painter.line_segment(
        [Pos2::new(rect.left(), mid), Pos2::new(rect.right(), mid)],
        Stroke::new(1.0, Color32::from_rgb(0x5a, 0x46, 0x14)),
    );

    // One point a sample when they are fewer than the pixels, and otherwise
    // the highest and lowest of each pixel's samples, which is what a
    // phosphor would show of a signal faster than the sweep can draw.
    let trace = &samples[start..(start + width).min(samples.len())];
    let y_of = |s: f32| mid - (s * gain).clamp(-1.0, 1.0) * rect.height() * 0.5;
    let columns = rect.width().max(2.0) as usize;
    let mut shape = Vec::with_capacity(columns * 2);
    if trace.len() <= columns {
        for (i, &s) in trace.iter().enumerate() {
            let x = rect.left() + rect.width() * i as f32 / (trace.len() - 1).max(1) as f32;
            shape.push(Pos2::new(x, y_of(s)));
        }
    } else {
        for c in 0..columns {
            let a = c * trace.len() / columns;
            let b = ((c + 1) * trace.len() / columns).max(a + 1);
            let part = &trace[a..b];
            let high = part.iter().fold(f32::MIN, |m, s| m.max(*s));
            let low = part.iter().fold(f32::MAX, |m, s| m.min(*s));
            let x = rect.left() + c as f32;
            shape.push(Pos2::new(x, y_of(high)));
            shape.push(Pos2::new(x, y_of(low)));
        }
    }
    painter.add(egui::Shape::line(shape, Stroke::new(1.5, theme::LCD_FG)));

    let small = egui::FontId::monospace(11.0);
    let said = match (triggered, view.trigger) {
        (_, Trigger::Off) => "free run",
        (true, _) => "triggered",
        (false, _) => "no trigger",
    };
    painter.text(
        Pos2::new(rect.left() + 6.0, rect.top() + 4.0),
        egui::Align2::LEFT_TOP,
        format!("{name}/div   ×{gain}   {said}"),
        small,
        if triggered { theme::AMBER } else { theme::DIM },
    );
}

/// The frequency `x` of the way across a log axis from `low` to `high`.
fn frequency_at(x: f32, low: f64, high: f64) -> f64 {
    low * (high / low).powf(x as f64)
}

/// A note's name and octave, as a piano would call it, for a frequency.
pub fn note_name(hz: f64) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round() as i32;
    let cents = (1200.0 * (hz / (440.0 * 2f64.powf((midi - 69) as f64 / 12.0))).log2()).round();
    format!(
        "{}{} {:+}¢",
        NAMES[midi.rem_euclid(12) as usize],
        midi.div_euclid(12) - 1,
        cents
    )
}

fn analyser(view: &mut View, probe: &Scope, rate: f64, dt: f32, ui: &mut egui::Ui, height: f32) {
    let db = scope::spectrum(&probe.latest(FFT));
    if view.shown.len() != db.len() || view.held.is_some() {
        view.shown = db.clone();
    } else {
        let fall = FALL_DB_PER_S * dt;
        for (shown, now) in view.shown.iter_mut().zip(&db) {
            *shown = now.max(*shown - fall);
        }
    }

    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme::LCD_BG);
    let grid = Stroke::new(1.0, theme::LCD_GRID);
    let small = egui::FontId::monospace(10.0);
    let low = 20.0;
    let high = (rate / 2.0).min(20_000.0);
    let x_of = |hz: f64| rect.left() + rect.width() * ((hz / low).ln() / (high / low).ln()) as f32;
    let y_of = |db: f32| rect.top() + rect.height() * (db / FLOOR_DB).clamp(0.0, 1.0);
    for decade in [100.0, 1_000.0, 10_000.0] {
        for step in 1..10 {
            let hz = decade / 10.0 * step as f64;
            if hz < low || hz > high {
                continue;
            }
            let x = x_of(hz);
            let stroke = if step == 1 {
                Stroke::new(1.0, theme::DIM.gamma_multiply(0.4))
            } else {
                grid
            };
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                stroke,
            );
        }
        if decade <= high {
            let label = if decade >= 1000.0 {
                format!("{}k", decade / 1000.0)
            } else {
                format!("{decade}")
            };
            painter.text(
                Pos2::new(x_of(decade) + 3.0, rect.bottom() - 2.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                small.clone(),
                theme::DIM,
            );
        }
    }
    for i in 1..8 {
        let level = FLOOR_DB * i as f32 / 8.0;
        let y = y_of(level);
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            grid,
        );
        painter.text(
            Pos2::new(rect.right() - 3.0, y - 1.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{level:.0} dB"),
            small.clone(),
            theme::DIM,
        );
    }

    // Each pixel across is the loudest bin under it; at the bottom, where a
    // pixel is narrower than a bin, the bins are joined by straight lines.
    let bin_hz = rate / FFT as f64;
    let columns = rect.width().max(2.0) as usize;
    let mut top = Vec::with_capacity(columns);
    for c in 0..columns {
        let f0 = frequency_at(c as f32 / columns as f32, low, high);
        let f1 = frequency_at((c + 1) as f32 / columns as f32, low, high);
        let (a, b) = ((f0 / bin_hz) as usize, (f1 / bin_hz) as usize);
        let level = if b > a {
            view.shown[a.min(view.shown.len() - 1)..=b.min(view.shown.len() - 1)]
                .iter()
                .fold(FLOOR_DB, |m, d| m.max(*d))
        } else {
            let at = (f0 / bin_hz).min((view.shown.len() - 2) as f64);
            let (k, frac) = (at as usize, (at.fract()) as f32);
            view.shown[k] * (1.0 - frac) + view.shown[k + 1] * frac
        };
        top.push(Pos2::new(rect.left() + c as f32, y_of(level)));
    }
    let fill = theme::LCD_FG.gamma_multiply(0.25);
    for p in &top {
        painter.line_segment([*p, Pos2::new(p.x, rect.bottom())], Stroke::new(1.0, fill));
    }
    painter.add(egui::Shape::line(top, Stroke::new(1.5, theme::LCD_FG)));

    let said = match scope::loudest(&db, rate) {
        Some((hz, level)) => format!("loudest {hz:.0} Hz ({}), {level:.0} dB", note_name(hz)),
        None => "silence".to_string(),
    };
    painter.text(
        Pos2::new(rect.left() + 6.0, rect.top() + 4.0),
        egui::Align2::LEFT_TOP,
        said,
        egui::FontId::monospace(11.0),
        theme::AMBER,
    );
}
