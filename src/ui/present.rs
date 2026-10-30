//! How the main window shows the picture: with its controls, filling the
//! window without them, or filling the display with the window system's own
//! full screen. Command+F or F11 goes round the three, and Esc leaves full
//! screen for the normal window.
//!
//! Ctrl is the machine's SYMBOL SHIFT, so Ctrl+F is somebody typing at a
//! Spectrum and cannot be the chord on Windows and Linux. F11 is free on all
//! of them: the quicksaves stop at F10.

use eframe::egui;
use egui::{Key, Modifiers, ViewportCommand, ViewportId};

use crate::ui::App;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Presentation {
    #[default]
    Normal,
    /// The picture fills the window, and nothing else is drawn in it.
    Filled,
    /// The same, with the window filling the display.
    FullScreen,
}

impl Presentation {
    pub fn next(self) -> Presentation {
        match self {
            Presentation::Normal => Presentation::Filled,
            Presentation::Filled => Presentation::FullScreen,
            Presentation::FullScreen => Presentation::Normal,
        }
    }

    pub fn shows_controls(self) -> bool {
        self == Presentation::Normal
    }
}

/// The largest zoom at which a picture of this many pixels fits in `area`,
/// not a whole number unless the area happens to make it one: the point of
/// this mode is that the picture fills the window.
pub fn fit_scale(area: egui::Vec2, width: usize, height: usize) -> f32 {
    (area.x / width as f32).min(area.y / height as f32)
}

/// What to do on hearing whether the window is full screen, given whether it
/// has been seen to be yet. The window takes a moment to get there, reporting
/// that it is not all the while, so only a window that has been full screen
/// and no longer is has been taken out of it by somebody: the green button,
/// say, or the system's own shortcut.
pub fn after_report(now: Presentation, seen: bool, reported: Option<bool>) -> (Presentation, bool) {
    match (now, reported) {
        (Presentation::FullScreen, Some(true)) => (now, true),
        (Presentation::FullScreen, Some(false)) if seen => (Presentation::Normal, false),
        (Presentation::FullScreen, _) => (now, seen),
        _ => (now, false),
    }
}

impl App {
    /// How many points of the display a pixel of the machine takes up as
    /// drawn. `scale` is the zoom somebody chose, and is left alone so that
    /// coming back to the normal window comes back to it.
    pub fn drawn_scale(&self) -> f32 {
        match self.presentation {
            Presentation::Normal => self.scale,
            _ => self.fill_scale,
        }
    }

    /// On to the next way of showing the picture. Not during a video: its
    /// frames are the size the picture is, and the size cannot change while
    /// one is being written.
    pub fn next_presentation(&mut self, ctx: &egui::Context) {
        if self.video_switches_held() {
            self.set_status(
                "Not while a video is recording: it would change the picture's size".into(),
                true,
            );
            return;
        }
        self.presentation = self.presentation.next();
        self.os_fullscreen_seen = false;
        ctx.send_viewport_cmd_to(
            ViewportId::ROOT,
            ViewportCommand::Fullscreen(self.presentation == Presentation::FullScreen),
        );
    }

    /// The keys that go round the three. The main window's only, as the
    /// quicksaves' are.
    pub(super) fn read_presentation_keys(&mut self, ctx: &egui::Context) {
        let (chord, f11, f_down) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::MAC_CMD, Key::F),
                i.consume_key(Modifiers::NONE, Key::F11),
                i.key_down(Key::F),
            )
        });
        self.chord_key_held = chord || (self.chord_key_held && f_down);
        if chord || f11 {
            self.next_presentation(ctx);
        }
        // Esc leaves full screen, unless it is already spoken for: giving up a
        // Find, read later in the frame, or giving back a captured mouse, which
        // has had first go at it by now. Only the display's full screen: the
        // filled window is a window, and a game may be being played in it.
        if self.presentation == Presentation::FullScreen
            && !self.gfx_picking
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.next_presentation(ctx);
        }
    }

    /// Whether the keys on the desk are the host's rather than the machine's
    /// just now: Command chords belong to the host, and the F of Command+F
    /// goes on being held after Command has been let go if it was let go
    /// first, when it would otherwise be typed at the Spectrum.
    pub(super) fn host_has_the_keys(&self, ctx: &egui::Context) -> bool {
        self.chord_key_held || ctx.input(|i| i.modifiers.mac_cmd)
    }

    pub(super) fn follow_the_window(&mut self, ctx: &egui::Context) {
        let reported = ctx.input(|i| i.viewport().fullscreen);
        (self.presentation, self.os_fullscreen_seen) =
            after_report(self.presentation, self.os_fullscreen_seen, reported);
    }

    /// Whether the window's size is its own to remember. Full screen it is the
    /// display's, and keeping that would bring the window back at that size
    /// on the next launch. Both tests are needed, since the window answers
    /// late in both directions.
    pub(super) fn window_size_is_its_own(&self, ctx: &egui::Context) -> bool {
        self.presentation != Presentation::FullScreen
            && ctx.input(|i| i.viewport().fullscreen) != Some(true)
    }
}
