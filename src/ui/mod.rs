//! egui front end: main window plus the detachable debug viewports.

pub mod back_buffer;
pub mod callflow;
pub mod cassette;
pub mod crt;
pub mod debugger;
pub mod disk;
pub mod diskface;
pub mod diskwin;
pub mod hardware;
pub mod inputwin;
pub mod keyboard;
pub mod microdrive;
pub mod microdrivewin;
pub mod printerwin;
pub mod profiler;
pub mod ram_map;
pub mod sprites;
pub mod tape;
pub mod theme;
pub mod trainwin;

use eframe::egui;
use egui::{ColorImage, TextureHandle, TextureOptions, ViewportBuilder, ViewportId};

use crate::audio_out::AudioOut;
use crate::machine::{self, Model, Spectrum, Stop};
use crate::prefs::{FileKind, Prefs, WindowRect};
use crate::screen;
use crate::snapshot;
use crate::zx81::{self, Zx81};

/// T-state of the pixel at (`px`, `py`), clamped into the frame.
fn beam_at(bus: &crate::machine::SpectrumBus, view: screen::View, px: usize, py: usize) -> u32 {
    let t = screen::t_at_pixel(view, bus.first_pixel_t(), bus.model.t_per_line(), px, py);
    t.clamp(0, bus.frame_t() as i64 - 1) as u32
}

/// Draw the ULA's beam over the picture: a line along what it is painting now
/// and a bright point where it has got to.
///
/// The line rather than only the point, because a point moving two pixels a
/// T-state is a speck nobody can follow; the line says which raster line the
/// machine is working on, which is what somebody watching a screen being
/// drawn wants to know.
fn draw_beam(
    painter: &egui::Painter,
    bus: &crate::machine::SpectrumBus,
    view: screen::View,
    picture: egui::Rect,
    scale: f32,
) {
    let (x, y) = screen::pixel_at_t(
        view,
        bus.first_pixel_t(),
        bus.model.t_per_line(),
        bus.tstates,
    );
    let (width, height) = (
        (view.border_x * 2 + 256) as i64,
        (view.border_top + 192 + view.border_bottom) as i64,
    );
    let at = |px: f32, py: f32| picture.min + egui::vec2(px * scale, py * scale);

    // Where it is, in the machine's own terms, so it can be read rather than
    // judged by eye. The line is counted from the first line of the display,
    // so the top border is negative — which is where the beam is when the
    // frame interrupt goes off.
    let line = y - view.border_top as i64;
    painter.text(
        picture.min + egui::vec2(4.0, 4.0),
        egui::Align2::LEFT_TOP,
        format!("T {} · line {line}", bus.tstates),
        egui::FontId::monospace(11.0),
        egui::Color32::from_rgba_unmultiplied(255, 240, 180, 200),
    );

    // Off the picture is where the beam really is during the flyback and the
    // top border a cropped view does not show, and drawing it at the edge
    // would say it was somewhere it is not.
    if y < 0 || y >= height {
        return;
    }
    let row = egui::Rect::from_min_max(at(0.0, y as f32), at(width as f32, y as f32 + 1.0));
    painter.rect_filled(
        row,
        0.0,
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 36),
    );

    if (0..width).contains(&x) {
        // Two pixels wide: that is how much of the line goes out in one
        // T-state, so it is as narrow as the beam can honestly be drawn.
        let spot =
            egui::Rect::from_min_max(at(x as f32, y as f32), at(x as f32 + 2.0, y as f32 + 1.0));
        painter.rect_filled(
            spot,
            0.0,
            egui::Color32::from_rgba_unmultiplied(255, 240, 180, 220),
        );
    }
}

/// ROM images found at startup, used when switching machines.
#[derive(Default, Clone)]
pub struct Roms {
    pub rom48: Option<Vec<u8>>,
    pub rom128: Option<Vec<u8>>,
    /// The 64K four-ROM image shared by the +2A and +3.
    pub rom_plus3: Option<Vec<u8>>,
    /// The ZX81's ROM, which is a different machine entirely.
    pub rom_zx81: Option<Vec<u8>>,
}

impl Roms {
    pub fn for_model(&self, model: Model) -> Option<&Vec<u8>> {
        match model {
            Model::Spectrum48 => self.rom48.as_ref(),
            Model::Spectrum128 => self.rom128.as_ref(),
            Model::Plus2A | Model::Plus3 => self.rom_plus3.as_ref(),
        }
    }

    /// Which machine a ROM image is for, judged by its size: 16K is a 48K
    /// ROM, 32K a 128K/+2 one, 64K the four-ROM +2A/+3 image.
    pub fn model_for_rom_size(len: usize) -> Option<Model> {
        match len {
            0x4000 => Some(Model::Spectrum48),
            0x8000 => Some(Model::Spectrum128),
            0x10000 => Some(Model::Plus3),
            _ => None,
        }
    }

    pub fn set_for_model(&mut self, model: Model, data: Vec<u8>) {
        match model {
            Model::Spectrum48 => self.rom48 = Some(data),
            Model::Spectrum128 => self.rom128 = Some(data),
            Model::Plus2A | Model::Plus3 => self.rom_plus3 = Some(data),
        }
    }

    /// ROM images in a directory, recognised by size. Where several fit the
    /// same machine, one whose name mentions it wins (`128.rom` beats
    /// `something.rom`), otherwise the first in alphabetical order.
    pub fn scan_directory(dir: &std::path::Path) -> Vec<(std::path::PathBuf, Model)> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut files: Vec<std::path::PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                matches!(
                    p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_ascii_lowercase())
                        .as_deref(),
                    Some("rom") | Some("bin")
                )
            })
            .collect();
        files.sort();

        let mut best: Vec<(std::path::PathBuf, Model)> = Vec::new();
        for path in files {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let Some(model) = Roms::model_for_rom_size(meta.len() as usize) else {
                continue;
            };
            let named = Roms::name_suggests(&path, model);
            match best.iter().position(|(_, m)| *m == model) {
                Some(i) => {
                    if named && !Roms::name_suggests(&best[i].0, model) {
                        best[i] = (path, model);
                    }
                }
                None => best.push((path, model)),
            }
        }
        best
    }

    /// Whether a file name mentions the machine its size implies.
    fn name_suggests(path: &std::path::Path, model: Model) -> bool {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match model {
            Model::Spectrum48 => name.contains("48"),
            Model::Spectrum128 => name.contains("128") || name.contains("plus2"),
            Model::Plus2A | Model::Plus3 => {
                name.contains("plus3") || name.contains("+3") || name.contains("plus2a")
            }
        }
    }

    /// File name to suggest when the ROM for a model is missing.
    pub fn expected_file(model: Model) -> &'static str {
        match model {
            Model::Spectrum48 => "roms/48.rom",
            Model::Spectrum128 => "roms/128.rom",
            Model::Plus2A | Model::Plus3 => "roms/plus3.rom",
        }
    }
}

/// How fast to run relative to a real Spectrum.
/// How a zoom factor is written on the dropdown.
pub fn zoom_label(scale: f32) -> String {
    if scale.fract() == 0.0 {
        format!("{scale:.0}x")
    } else {
        format!("{scale}x")
    }
}

/// What the clock can be wound up to, as multiples of the machine's own.
///
/// A machine's clock is its clock — everything about the ULA is counted in
/// T-states of it — so a faster one is the same machine running quicker rather
/// than a different machine. The steps are doublings because that is what the
/// hardware which did this offered: 3.5, 7, 14 and 28MHz, each the one before
/// it twice over.
pub const CLOCK_MULTIPLES: &[f32] = &[1.0, 2.0, 4.0, 8.0];

/// Why the switches that change the picture's size cannot be moved while a
/// video is being written.
pub const HELD_WHILE_RECORDING: &str =
    "Held while a video is being recorded: the file is one size throughout, \
     and changing it part way would shear the picture from there on. Stop the \
     recording to change it. Composite can be switched either way.";

/// The clock dropdown: the machine's own speed, and multiples of it.
///
/// In MHz rather than in multiples, since that is what a clock is measured in,
/// and worked out from whichever machine is running: a 48K's own is 3.5MHz and
/// a 128K's 3.5469, so "twice" is a different number on each.
/// What one entry of the clock dropdown says.
///
/// The machine's own clock is marked, because "3.50MHz" means nothing to
/// somebody who does not already know what a 48K runs at — and knowing which
/// one is the machine as built is the difference between choosing a speed and
/// wondering whether the emulator is lying about something.
pub fn clock_option_label(mult: f32, base: f64) -> String {
    let hz = format!("{:.2}MHz", base * mult as f64 / 1_000_000.0);
    if mult <= 1.0 {
        format!("{hz}  (default)")
    } else {
        format!("{hz}  ({mult:.0}x)")
    }
}

pub fn clock_dropdown(mult: &mut f32, base: f64, ui: &mut egui::Ui) {
    // The button carries the clock alone; the list says which is the machine's
    // own and what each of the others is a multiple of.
    let closed = format!("{:.2}MHz", base * *mult as f64 / 1_000_000.0);
    // As wide as the widest thing it can say, so choosing 28MHz does not push
    // everything after it along the toolbar. The widest label is laid out as
    // a real button, unseen, and its width taken: text plus padding comes out
    // a point and a half short of what egui makes of a button, which is how
    // the first attempt at this still grew at 14MHz.
    let widest = ('0'..='9')
        .map(|d| format!("{d}{d}.{d}{d}MHz  \u{25be}"))
        .map(|text| {
            let mut unseen = ui.new_child(egui::UiBuilder::new().invisible());
            unseen.add(egui::Button::new(text)).rect.width()
        })
        .fold(0.0, f32::max);
    theme::dropdown(ui, widest, closed, |ui| {
        for m in CLOCK_MULTIPLES {
            if ui
                .selectable_label(
                    (*mult - m).abs() < f32::EPSILON,
                    clock_option_label(*m, base),
                )
                .clicked()
            {
                *mult = *m;
            }
        }
    });
}

/// The speed presets, as a dropdown. The slider beside it still takes any
/// value; the list is for the ones worth a single click.
pub fn speed_dropdown(speed: &mut f32, ui: &mut egui::Ui) {
    let selected = SPEED_PRESETS
        .iter()
        .find(|(_, mult)| (*speed - mult).abs() < f32::EPSILON)
        .map(|(name, _)| (*name).to_string())
        // Racing runs at four thousandths of speed, which rounds to "0%".
        .unwrap_or_else(|| {
            if (*speed - RACE_SPEED).abs() < f32::EPSILON {
                "5s/frame".to_string()
            } else if *speed < 0.01 {
                format!("{:.1}%", *speed * 100.0)
            } else {
                format!("{:.0}%", *speed * 100.0)
            }
        });
    theme::dropdown(ui, 74.0, selected, |ui| {
        for (name, mult) in SPEED_PRESETS {
            if ui
                .selectable_label((*speed - mult).abs() < f32::EPSILON, name)
                .clicked()
            {
                *speed = mult;
            }
        }
    });
}

/// The display sizes, as a dropdown.
pub fn zoom_dropdown(scale: &mut f32, ui: &mut egui::Ui) {
    theme::dropdown(ui, 74.0, zoom_label(*scale), |ui| {
        for size in screen::SCALES {
            if ui
                .selectable_label((*scale - size).abs() < f32::EPSILON, zoom_label(size))
                .clicked()
            {
                *scale = size;
            }
        }
    });
}

/// What the emulator is called: the ZX Spectrum by way of the language it is
/// written in.
pub const APP_NAME: &str = "ZX-Rustrum";

/// The most frames of a recording to play in one go, so a long pause or a
/// slow host frame does not run half the tape past in one step.
const MAX_RECORDED_FRAMES: f32 = 24.0;

/// How many places a recording is remembered as having reached. Enough to
/// document a program, not enough to grow without limit over a long recording.
const VISITED_CAP: usize = 4096;

/// How many hand-stepped instructions can be stepped back over. Enough to see
/// how the machine got where it is, which is what stepping back is for; a
/// hundred would be a recording, and there is one of those already.
pub const REWIND: usize = 20;

pub const SPEED_PRESETS: [(&str, f32); 8] = [
    ("1%", 0.01),
    ("5%", 0.05),
    ("10%", 0.1),
    ("25%", 0.25),
    ("50%", 0.5),
    ("100%", 1.0),
    ("200%", 2.0),
    ("Max", MAX_SPEED),
];

/// How much emulated time Fastload will do in one host frame before it
/// stops to draw, in frames of the machine's own.
const FASTLOAD_FRAMES: u32 = 24;

/// And how much of the host's time it will spend doing it. The window has to
/// go on answering while it works, so it stops to draw twenty times a second —
/// which is still a hundred times the machine time Max speed manages.
const FASTLOAD_SLICE: std::time::Duration = std::time::Duration::from_millis(50);

/// How much of a hurry the tape is loaded in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hurry {
    /// At the speed it was recorded at.
    Normal,
    /// The machine flat out while the tape moves.
    Max,
    /// And blocks handed to the ROM's loader rather than played at all.
    Fastload,
}

/// How long a write stays marked before it has blended into the colour it
/// should be, in seconds of the user's time. Long enough to see where a write
/// landed while the frame it landed in is still on screen.
pub const TINT_SECONDS: f32 = 2.0;

/// Five seconds to a frame: 0.02s of machine time in 5s of ours. Slow enough
/// that a frame's drawing can be followed by eye, which is the only speed at
/// which watching the beam tells anybody anything.
pub const RACE_SPEED: f32 = 0.02 / 5.0;

/// As fast as the emulator will go. The work per host frame is capped as well,
/// so this is a ceiling rather than a promise.
pub const MAX_SPEED: f32 = 20.0;

/// Putting a window back where it was left takes a few frames, and sometimes
/// does not take at all.
struct Placement {
    asked_at: std::time::Instant,
    settled: bool,
}

/// What the machine dropdown was asked for, decided after the list is closed
/// so the borrow of the app inside it has ended.
enum Machine {
    Spectrum(Model),
    Zx81(zx81::Ram),
}

/// A recording being played back, and where it has got to.
pub struct RzxPlayback {
    pub recording: crate::rzx::Recording,
    pub path: std::path::PathBuf,
    /// Which frame is next.
    pub frame: usize,
    /// What is left of the current frame, when a breakpoint stopped it
    /// part-way through.
    pub remaining: u32,
    /// Fractional frames carried over, so playback runs at the speed the
    /// emulator is set to rather than at whatever the host frame rate is.
    pub owed: f32,
    /// Addresses the recording has actually reached: what AutoDoc reads on
    /// top of what it can work out from the code alone.
    pub visited: std::collections::BTreeSet<u16>,
    /// Run it as fast as the host will go, rather than at the speed it was
    /// played. A recording is often twenty minutes long and the interesting
    /// part is rarely at the start.
    pub max_speed: bool,
}

impl RzxPlayback {
    /// How far through, for the status line.
    pub fn progress(&self) -> f32 {
        if self.recording.is_empty() {
            return 0.0;
        }
        self.frame as f32 / self.recording.len() as f32
    }

    pub fn finished(&self) -> bool {
        self.frame >= self.recording.len()
    }
}

pub struct App {
    pub spec: Spectrum,
    /// Labels and comments for the listing, and the file they are kept in.
    pub notes: crate::notes::Notes,
    /// The tape in the deck, if it came from a file. The notes belong beside
    /// whatever is being disassembled, which is the tape when there is one.
    pub tape_path: Option<std::path::PathBuf>,
    /// The ROM the current machine booted from, for the same reason.
    pub rom_path: Option<std::path::PathBuf>,
    /// When the notes last changed, so they are written a moment later.
    notes_changed_at: Option<std::time::Instant>,
    /// The recording being played back, if there is one.
    pub rzx: Option<RzxPlayback>,
    pub running: bool,
    pub speed: f32,
    pub status: String,

    pub show_ram_map: bool,
    pub show_debugger: bool,
    pub show_back_buffer: bool,
    /// The program's loop, drawn.
    pub show_callflow: bool,
    pub callflow: callflow::CallFlowState,
    /// Memory read as graphics.
    pub show_sprites: bool,
    pub sprites: sprites::SpriteView,
    pub show_tape: bool,
    pub show_profiler: bool,
    /// The machine's own keyboard, drawn and pressable.
    pub show_keyboard: bool,
    /// The +3's drive, drawn.
    pub show_disk: bool,
    /// What is plugged into the back of the machine.
    pub show_hardware: bool,
    pub show_input: bool,
    pub show_printer: bool,
    printer_view: printerwin::View,
    /// A network learning to play, and the game it is learning.
    pub show_training: bool,
    pub training: trainwin::Training,
    /// Ten quicksaves, kept in memory for as long as the emulator is open, and
    /// the one Load would restore.
    pub quick: [Option<Box<Spectrum>>; 10],
    pub quick_slot: usize,
    /// Pointer movement smaller than one of the machine's pixels, carried to
    /// the next frame for the Kempston mouse.
    mouse_rest: egui::Vec2,
    /// Whether the host's pointer belongs to the Kempston mouse: taken by a
    /// click on the screen, given back by Esc or the window losing focus.
    pub mouse_captured: bool,
    /// What is being looked for in the Keyboard window.
    pub key_search: String,
    /// Waiting for a block of the screen to be clicked, for the Graphics
    /// window's Find.
    pub gfx_picking: bool,
    /// What the last Find looked for and turned up.
    pub gfx_found: Option<crate::gfxfind::Picked>,
    /// The direction of the DK'Tronics Programmable being given a key in the
    /// Input window.
    pub dkprog_picking: Option<crate::joystick::Way>,
    /// The host's left, middle and right buttons, while the pointer is
    /// captured; what is bound to a mouse button is added to these.
    host_buttons: [bool; 3],
    /// What the other windows are holding down, gathered while they draw
    /// and read by the next frame's keyboard.
    pub window_keys_next: HeldKeys,
    window_keys: HeldKeys,
    /// What on the desk works the stick, and which line is waiting for a key.
    pub joystick_map: Vec<inputwin::Binding>,
    pub joystick_binding: Option<usize>,
    /// The gamepads, and what they were doing when they were last looked at.
    /// `None` when the crate could not open them at all, which is a machine
    /// with no gamepad support rather than an error worth stopping for.
    pub gilrs: Option<gilrs::Gilrs>,
    pub pads: inputwin::Pads,
    /// The microdrives, drawn.
    pub show_microdrive: bool,
    /// Which drive Load, Blank and Eject act on.
    pub selected_drive: usize,
    /// A cartridge read and waiting to be told how its writes should be
    /// treated, and which drive it is going into.
    pub pending_cartridge: Option<(crate::ui::microdrive::Pending, usize)>,
    /// The disk drawn as a disk, kept between frames: rasterising forty rings
    /// of bits is not a thing to do sixty times a second.
    pub platter: diskface::Platter,
    /// A disk read and waiting to be told how its writes should be treated.
    pub pending_disk: Option<disk::Pending>,
    /// How the disk in drive A: was put in, if there is one.
    pub disk_mounted: Option<disk::Mounted>,
    /// Which keys are down and which are lit, for that window.
    pub keys: crate::keyboard::Keys,

    screen_pixels: Vec<u8>,
    screen_tex: Option<TextureHandle>,
    /// Whether the picture is shown on a tube: the curve of the glass and the
    /// gaps between the lines.
    pub crt: bool,
    /// Whether it arrives as composite video: colour smeared sideways, and the
    /// dot crawl the subcarrier beats against the dot clock. A monitor fed RGB
    /// had the tube and none of this, so it is a switch of its own — but only
    /// with the tube, since the aerial lead led to a television.
    pub composite: bool,
    /// The televised picture, kept between frames so it is not reallocated.
    crt_pixels: Vec<u8>,
    /// Whether the texture that exists was made with a gap under every line,
    /// which is what decides its height.
    crt_drawn: bool,
    /// The video file being written, while one is.
    pub video: Option<crate::video_out::Recording>,
    /// Emulated frames that have gone by without a frame of video written for
    /// them.
    ///
    /// The window repaints when the window system says so — sixty times a
    /// second on this screen, or not at all while it is behind another window
    /// — and the machine draws fifty. Writing a frame per repaint puts the
    /// wrong number of frames in the file and everything in it happens at the
    /// wrong speed; counting the machine's frames and writing that many is
    /// what makes a second of the file a second of the machine.
    video_due: f64,
    /// Which machine frame the last one was counted at.
    video_at: u64,
    /// How many times the machine's own clock it is being run at. One is the
    /// machine as it was built; the rest are the same machine running quicker,
    /// which is what an accelerator did.
    pub clock_mult: f32,
    pub scale: f32,
    /// Show the whole overscan area, or crop the border to television size.
    pub overscan: bool,
    /// Follow the raster with the mouse: the frame is replayed from its
    /// interrupt to wherever the cursor is.
    pub cursor_beam: bool,
    /// Beam position under the cursor last frame, in T-states.
    pub beam_t: Option<u32>,
    /// Run at five seconds a frame with the picture fading behind the beam,
    /// so a frame can be watched being drawn.
    pub racing: bool,
    /// How bright a pixel is left by the time the beam comes round to it
    /// again, as a fraction of how bright it was drawn.
    pub fade_floor: f32,
    /// The speed to go back to when racing is switched off.
    speed_before_race: f32,
    /// Whether the deck stopping itself has already been mentioned, so it is
    /// said once rather than sixty times a second.
    said_tape_stopped: bool,
    /// How well the deck is asked to behave. Kept here rather than on the deck
    /// so that it survives one tape being taken out and another put in.
    pub quality: crate::tape::Quality,
    /// The frame being raced: a copy of the machine taken at the interrupt,
    /// run forward to wherever the cursor is. Only ever set while the machine
    /// is stopped.
    pub race: Option<crate::race::Race>,

    pub ram: ram_map::RamMapState,
    pub dbg: debugger::DebuggerState,
    pub back: back_buffer::BackBufferState,
    pub tape: tape::TapeWindowState,
    pub profiler: profiler::ProfilerWindowState,

    pub last_stop: Option<Stop>,
    /// The last few instructions stepped by hand, newest last, so they can be
    /// stepped back over. What each one holds is what it changed rather than a
    /// copy of the machine.
    pub rewind: std::collections::VecDeque<crate::machine::Undo>,
    /// The machine as it was when recording started, waiting to be written
    /// into the file when it stops.
    pub recording_from: Option<Vec<u8>>,
    /// Emulated T-states left over from the previous host frame.
    leftover: f32,

    /// True when `status` is a failure the user should notice.
    pub status_is_error: bool,
    /// Model shown in the window title, so a switch is always visible.
    title_shown: String,

    /// How far ahead of the sound device to stay, in seconds.
    pub audio_latency_target: f32,

    /// The ZX81, when that is the machine in use.
    pub zx81: Option<Zx81>,
    /// Which RAM the ZX81 has fitted, remembered across switches.
    pub zx81_ram: zx81::Ram,

    /// Remembered settings, including the directories files were opened from.
    pub prefs: Prefs,
    /// The layout as last written, and when: the file is rewritten a moment
    /// after things settle, so a crash or a kill does not lose it.
    last_saved: Option<String>,
    last_save_at: Option<std::time::Instant>,

    pub roms: Roms,
    /// Kept alive for as long as the app runs; dropping it stops the sound.
    pub audio_out: Option<AudioOut>,
    /// Whether the theme has been applied to the context yet.
    styled: bool,
    /// The cassette artwork, rasterised from the SVGs.
    pub art: cassette::Art,
    /// How each debug window's placement is going.
    placed: std::collections::HashMap<&'static str, Placement>,
    pub audio_error: Option<String>,
}

impl App {
    pub fn new(spec: Spectrum, status: String) -> Self {
        Self::with_roms(spec, status, Roms::default(), None)
    }

    pub fn with_roms(
        spec: Spectrum,
        status: String,
        roms: Roms,
        audio_out: Option<AudioOut>,
    ) -> Self {
        App {
            spec,
            notes: crate::notes::Notes::unattached(),
            tape_path: None,
            rom_path: None,
            notes_changed_at: None,
            rzx: None,
            running: true,
            speed: 1.0,
            status,
            show_ram_map: true,
            show_debugger: true,
            show_back_buffer: false,
            show_callflow: false,
            callflow: callflow::CallFlowState::default(),
            show_sprites: false,
            sprites: sprites::SpriteView::default(),
            show_tape: false,
            show_profiler: false,
            show_keyboard: false,
            show_disk: false,
            show_hardware: false,
            show_input: false,
            show_printer: false,
            printer_view: printerwin::View::default(),
            show_training: false,
            training: trainwin::Training::default(),
            quick: Default::default(),
            quick_slot: 1,
            mouse_rest: egui::Vec2::ZERO,
            mouse_captured: false,
            key_search: String::new(),
            gfx_picking: false,
            gfx_found: None,
            dkprog_picking: None,
            host_buttons: [false; 3],
            window_keys_next: HeldKeys::default(),
            window_keys: HeldKeys::default(),
            joystick_map: inputwin::defaults(),
            joystick_binding: None,
            gilrs: gilrs::Gilrs::new().ok(),
            pads: inputwin::Pads::default(),
            show_microdrive: false,
            selected_drive: 0,
            pending_cartridge: None,
            platter: diskface::Platter::default(),
            pending_disk: None,
            disk_mounted: None,
            keys: crate::keyboard::Keys::default(),
            screen_pixels: vec![0; screen::View::OVERSCAN.buffer_len()],
            screen_tex: None,
            crt: false,
            composite: false,
            crt_pixels: Vec::new(),
            crt_drawn: false,
            video: None,
            video_due: 0.0,
            video_at: 0,
            clock_mult: 1.0,
            scale: 2.0,
            overscan: true,
            cursor_beam: false,
            racing: false,
            fade_floor: 0.5,
            speed_before_race: 1.0,
            said_tape_stopped: false,
            quality: crate::tape::Quality::default(),
            race: None,
            beam_t: None,
            ram: ram_map::RamMapState::default(),
            dbg: debugger::DebuggerState::default(),
            back: back_buffer::BackBufferState::default(),
            tape: tape::TapeWindowState::default(),
            profiler: profiler::ProfilerWindowState::default(),
            last_stop: None,
            rewind: std::collections::VecDeque::new(),
            recording_from: None,
            leftover: 0.0,
            status_is_error: false,
            title_shown: String::new(),
            audio_latency_target: 0.06,
            zx81: None,
            zx81_ram: zx81::Ram::K16,
            prefs: Prefs::default(),
            last_saved: None,
            last_save_at: None,
            roms,
            audio_out,
            styled: false,
            art: cassette::Art::default(),
            placed: std::collections::HashMap::new(),
            audio_error: None,
        }
    }

    /// Switch between the 48K and 128K machines, which needs the matching ROM.
    pub fn switch_model(&mut self, model: Model) {
        if self.zx81.take().is_some() {
            // Coming back from the ZX81; the Spectrum is still as it was.
            self.running = true;
            self.set_status(format!("Switched to {}", model.name()), false);
            if self.spec.bus.model == model {
                return;
            }
        }
        if self.spec.bus.model == model {
            self.set_status(format!("Already running as {}", model.name()), false);
            return;
        }
        let Some(rom) = self.roms.for_model(model).cloned() else {
            self.set_status(
                format!(
                    "Cannot switch to {}: no ROM. Put a {} ROM at {}, or use File ▸ Load ROM…",
                    model.name(),
                    model.name(),
                    Roms::expected_file(model)
                ),
                true,
            );
            return;
        };
        // Whatever was in the drive goes with the machine that had it: a
        // 48K has nowhere to put a disk, and the writes so far are the user's.
        if self.spec.bus.fdc.drives[0].is_some() {
            self.save_disk();
            self.spec.bus.fdc.drives[0] = None;
            self.disk_mounted = None;
        }
        self.spec.set_model(model, &rom);
        self.leftover = 0.0;
        self.running = true;
        self.set_status(format!("Switched to {}", model.name()), false);
    }

    /// Load a ROM image, switching to the machine its size implies. A 32K
    /// image is a 128K ROM, so loading one on a 48K has to change machine or
    /// the image would simply be truncated.
    pub fn load_rom_image(&mut self, path: &std::path::Path, data: Vec<u8>) {
        let Some(model) = Roms::model_for_rom_size(data.len()) else {
            let len = data.len();
            self.spec.load_rom(&data);
            self.spec.reset();
            self.set_status(
                format!(
                    "{}: {len} bytes is not a 16K, 32K or 64K ROM. Loaded it into the {} as-is.",
                    path.display(),
                    self.spec.bus.model.name()
                ),
                true,
            );
            return;
        };

        // Remember it, so the toolbar offers that machine from now on.
        self.roms.set_for_model(model, data.clone());
        self.rom_path = Some(path.to_path_buf());
        self.reload_notes();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        if self.spec.bus.model == model {
            self.spec.load_rom(&data);
            self.spec.reset();
        } else {
            self.spec.set_model(model, &data);
            self.leftover = 0.0;
        }
        self.running = true;

        self.prefs.remember_file(FileKind::Rom, path);
        // ROMs tend to arrive in sets, so pick up the machines this one's
        // neighbours can provide.
        let also = match path.parent() {
            Some(dir) => self.adopt_roms_from(dir),
            None => Vec::new(),
        };
        let extra = if also.is_empty() {
            String::new()
        } else {
            format!("; also found {}", also.join(", "))
        };
        self.set_status(format!("Loaded {name} as a {}{extra}", model.name()), false);
    }

    /// Fill in ROMs for machines that have none from the images in `dir`.
    /// Returns what was adopted, for the status line.
    pub fn adopt_roms_from(&mut self, dir: &std::path::Path) -> Vec<String> {
        let mut adopted = Vec::new();
        for (path, model) in Roms::scan_directory(dir) {
            if self.roms.for_model(model).is_some() {
                continue;
            }
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            self.roms.set_for_model(model, data);
            adopted.push(format!(
                "{} ({})",
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                model.name()
            ));
        }
        adopted
    }

    pub fn set_status(&mut self, text: String, is_error: bool) {
        self.status = text;
        self.status_is_error = is_error;
    }

    /// Keep the window title in step with the machine, so switching is
    /// visible even if the screen happens to look similar.
    fn sync_title(&mut self, ctx: &egui::Context) {
        let machine = if self.zx81.is_some() {
            self.zx81_ram.name().to_string()
        } else {
            format!("ZX Spectrum {}", self.spec.bus.model.name())
        };
        let want = format!("{APP_NAME} — {machine}");
        if self.title_shown != want {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(want.clone()));
            self.title_shown = want;
        }
    }

    /// Every machine the emulator can be, in one list. Those whose ROM is
    /// missing stay in it, saying so, rather than disappearing — otherwise
    /// there is nothing to click to find out what is wanted.
    fn machine_dropdown(&mut self, ui: &mut egui::Ui) {
        let selected = if self.on_zx81() {
            self.zx81_ram.name().to_string()
        } else {
            self.spec.bus.model.name().to_string()
        };
        let mut chosen: Option<Machine> = None;
        theme::dropdown(ui, 120.0, selected, |ui| {
            for model in [
                Model::Spectrum48,
                Model::Spectrum128,
                Model::Plus2A,
                Model::Plus3,
            ] {
                let current = !self.on_zx81() && self.spec.bus.model == model;
                let have_rom = self.roms.for_model(model).is_some();
                let label = if have_rom {
                    model.name().to_string()
                } else {
                    format!("{} (no ROM)", model.name())
                };
                if ui
                    .selectable_label(current, label)
                    .on_hover_text(if have_rom {
                        format!("Switch to the {} and reset", model.name())
                    } else {
                        format!("Needs {}", Roms::expected_file(model))
                    })
                    .clicked()
                {
                    chosen = Some(Machine::Spectrum(model));
                }
            }
            for ram in [zx81::Ram::K1, zx81::Ram::K16] {
                let current = self.on_zx81() && self.zx81_ram == ram;
                let have_rom = self.roms.rom_zx81.is_some();
                let label = if have_rom {
                    ram.name().to_string()
                } else {
                    format!("{} (no ROM)", ram.name())
                };
                if ui
                    .selectable_label(current, label)
                    .on_hover_text(if have_rom {
                        format!("Switch to a {} and reset", ram.name())
                    } else {
                        "Needs roms/zx81.rom".to_string()
                    })
                    .clicked()
                {
                    chosen = Some(Machine::Zx81(ram));
                }
            }
        });
        match chosen {
            Some(Machine::Spectrum(model)) => self.switch_model(model),
            Some(Machine::Zx81(ram)) => self.switch_to_zx81(ram),
            None => {}
        }
    }

    /// The second toolbar row: reset, and which windows are open. The machine,
    /// speed, zoom and video controls are on the row above.
    fn machine_row(&mut self, ui: &mut egui::Ui) {
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y + 8.0);
        let layout = egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true);
        ui.allocate_ui_with_layout(row, layout, |ui| {
            theme::group_label(ui, "Windows");
            theme::toggle(ui, &mut self.show_ram_map, "RAM map");
            theme::toggle(ui, &mut self.show_debugger, "Debugger");
            theme::toggle(ui, &mut self.show_tape, "Tape");
            theme::toggle(ui, &mut self.show_back_buffer, "Back buffer");
            theme::toggle(ui, &mut self.show_sprites, "Graphics");
            theme::toggle(ui, &mut self.show_callflow, "Call flow");
            theme::toggle(ui, &mut self.show_profiler, "Profiler");
            theme::toggle(ui, &mut self.show_keyboard, "Keyboard");
            if self.spec.bus.model.has_disk() && !self.on_zx81() {
                theme::toggle(ui, &mut self.show_disk, "Disk");
            }
            theme::toggle(ui, &mut self.show_hardware, "Hardware");
            theme::toggle(ui, &mut self.show_input, "Input");
            if !self.on_zx81() {
                theme::toggle(ui, &mut self.show_training, "Training");
            }
            if self
                .spec
                .bus
                .hardware
                .fitted(crate::hardware::Peripheral::Interface1)
            {
                theme::toggle(ui, &mut self.show_microdrive, "Microdrive");
            }
            if self.spec.bus.printer.is_some() && !self.on_zx81() {
                theme::toggle(ui, &mut self.show_printer, "Printer");
            }
            self.buttons(ui);
        });
    }

    /// The buttons on the boxes plugged into the back, which are on the front
    /// of the emulator because that is where a hand can reach them: a
    /// Multiface's whole purpose is being pressed while something else is
    /// running, and going and finding a window first is not that.
    ///
    /// Nothing is drawn when there is nothing to press. One button serves
    /// every Multiface on the back, as on the hardware.
    fn buttons(&mut self, ui: &mut egui::Ui) {
        let any = self
            .spec
            .bus
            .multifaces
            .iter()
            .any(|mf| mf.ready() && self.spec.bus.hardware.fitted(peripheral_of(mf.model)));
        if !any {
            return;
        }
        theme::group_label(ui, "Buttons");
        let label = egui::RichText::new("Red button").color(theme::RED);
        let button = egui::Button::new(label)
            .frame_when_inactive(true)
            .min_size(egui::vec2(0.0, theme::button_height(ui)));
        if ui
            .add(button)
            .on_hover_text("Stops the machine wherever it is and brings up the Multiface's menu")
            .clicked()
        {
            self.press_red_button();
        }
    }

    /// Start recording what the machine does, from where it is now.
    ///
    /// The snapshot is taken first: a recording is a machine to start from and
    /// then every byte read from a port after it, and one without the first is
    /// a list of numbers.
    pub fn start_recording(&mut self) {
        if self.rzx.is_some() {
            self.set_status("A recording is already playing".to_string(), true);
            return;
        }
        self.recording_from = Some(snapshot::save_sna(&self.spec));
        self.spec.bus.capture = Some(crate::rzx::Capture {
            start_t: self.spec.bus.tstates,
            mark: self.spec.bus.fetches,
            ..Default::default()
        });
        self.set_status("Recording".to_string(), false);
    }

    /// How many frames have been recorded so far, if anything is being.
    pub fn recorded_frames(&self) -> Option<usize> {
        self.spec
            .bus
            .capture
            .as_ref()
            .map(|capture| capture.frames.len())
    }

    /// Stop recording and hand back the file's bytes, or nothing if there was
    /// nothing to record.
    pub fn stop_recording(&mut self) -> Option<Vec<u8>> {
        let capture = self.spec.bus.capture.take()?;
        let snapshot = self.recording_from.take();
        if capture.frames.is_empty() {
            self.set_status("Nothing was recorded".to_string(), true);
            return None;
        }
        let recording = crate::rzx::Recording {
            creator: APP_NAME.to_string(),
            snapshot: snapshot.map(|data| crate::rzx::Snapshot {
                extension: "sna".to_string(),
                data,
            }),
            frames: capture.frames,
            start_t: capture.start_t,
        };
        Some(crate::rzx::write(&recording))
    }

    /// Where a recording should go by default: beside the tape that is in the
    /// deck, under the same name. A machine with nothing loaded has nothing to
    /// be named after, so it gets a plain one.
    pub fn recording_path(&self) -> std::path::PathBuf {
        match &self.tape_path {
            Some(path) => path.with_extension("rzx"),
            None => std::path::PathBuf::from("recording.rzx"),
        }
    }

    /// Where a snapshot should go by default: beside the tape in the deck,
    /// under the same name. A machine with nothing loaded has nothing to be
    /// named after, so it gets a plain one.
    pub fn snapshot_path(&self) -> std::path::PathBuf {
        match &self.tape_path {
            Some(path) => path.with_extension("sna"),
            None => std::path::PathBuf::from("snapshot.sna"),
        }
    }

    /// Write the machine as it stands out as a `.sna`.
    ///
    /// A snapshot is the machine's registers and its RAM, and nothing about
    /// where the tape had reached or what the sound was doing: those are not
    /// in the format, and a snapshot that claimed to hold them would be
    /// lying about what comes back.
    pub fn save_snapshot(&mut self) {
        if self.on_zx81() {
            self.set_status("The ZX81 has no snapshot format here".to_string(), true);
            return;
        }
        let suggested = self.snapshot_path().with_extension("szx");
        let dialog = rfd::FileDialog::new()
            .add_filter("Snapshot", &["szx", "sna"])
            .set_file_name(
                suggested
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "snapshot.szx".to_string()),
            );
        let directory = suggested
            .parent()
            .filter(|parent| parent.is_dir())
            .map(std::path::Path::to_path_buf)
            .or_else(|| self.prefs.dir_for(FileKind::Snapshot).cloned())
            .filter(|dir| dir.is_dir());
        let dialog = match directory {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        };
        let Some(path) = dialog.save_file() else {
            self.set_status("Snapshot not saved".to_string(), false);
            return;
        };
        // The name says which format: .szx carries the paging, the AY and
        // what is plugged in, and .sna is a 48K memory dump with the
        // registers on the front.
        let szx = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("szx"));
        let bytes = if szx {
            crate::szx::save(&self.spec)
        } else {
            snapshot::save_sna(&self.spec)
        };
        match std::fs::write(&path, &bytes) {
            Ok(()) => {
                self.prefs.remember_file(FileKind::Snapshot, &path);
                self.set_status(
                    format!("Wrote {} ({} bytes)", path.display(), bytes.len()),
                    false,
                );
            }
            Err(e) => self.set_status(format!("Could not write {}: {e}", path.display()), true),
        }
    }

    /// The host's mouse, once captured, is the Kempston mouse: movement in
    /// the machine's pixels, and the two buttons. Until then a click on the
    /// screen is what captures it, and the mouse sees nothing.
    ///
    /// Movement is kept as a remainder in points, so a slow drag on a
    /// scaled-up picture still moves the counters rather than being rounded
    /// away a frame at a time.
    fn feed_mouse(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if !self.mouse_captured {
            self.host_buttons = [false; 3];
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            if response.clicked() {
                self.capture_mouse(ui.ctx());
            }
            self.apply_mouse_buttons(ui.ctx());
            return;
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        let (motion, delta, buttons) = ui.input(|i| {
            (
                i.pointer.motion(),
                i.pointer.delta(),
                [
                    i.pointer.primary_down(),
                    i.pointer.middle_down(),
                    i.pointer.secondary_down(),
                ],
            )
        });
        if !self.running {
            self.host_buttons = [false; 3];
            self.mouse_rest = egui::Vec2::ZERO;
            self.apply_mouse_buttons(ui.ctx());
            return;
        }
        self.host_buttons = buttons;
        // A locked pointer does not move, so only the raw motion says the
        // mouse did; where there is none, the pointer's own movement does.
        // The raw motion's units are the platform's — points on macOS — and
        // are taken as points.
        let points = motion.unwrap_or(delta);
        let moved = self.mouse_rest + points / self.scale.max(0.01);
        let whole = egui::vec2(moved.x.trunc(), moved.y.trunc());
        self.mouse_rest = moved - whole;
        let (dx, dy) = (whole.x as i32, whole.y as i32);
        self.spec.bus.mouse.move_by(dx, dy);
        if let Some(amx) = self.spec.bus.amx.as_mut() {
            amx.queue(dx, dy);
        }
        self.apply_mouse_buttons(ui.ctx());
    }

    /// Hand the host's pointer to the mouse: hidden, and held in the
    /// window so it cannot wander off onto the desktop. macOS can only lock
    /// the pointer in place and Windows and X11 can only confine it, so each
    /// is asked for what it has.
    fn capture_mouse(&mut self, ctx: &egui::Context) {
        let grab = if cfg!(target_os = "macos") {
            egui::viewport::CursorGrab::Locked
        } else {
            egui::viewport::CursorGrab::Confined
        };
        self.mouse_captured = true;
        self.mouse_rest = egui::Vec2::ZERO;
        ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::CursorGrab(grab));
        ctx.send_viewport_cmd_to(
            ViewportId::ROOT,
            egui::ViewportCommand::CursorVisible(false),
        );
        self.set_status(
            "The mouse is the machine's now: Esc gives it back".into(),
            false,
        );
    }

    /// Gather what a window other than the main one has held down, for the
    /// machine's keyboard. Not while one of its text fields is being typed
    /// into — a label typed in the debugger is not meant for the Spectrum —
    /// and not from the main window, whose keys are read directly.
    fn collect_window_keys(&mut self, ctx: &egui::Context) {
        if ctx.viewport_id() == ViewportId::ROOT || ctx.text_edit_focused() {
            return;
        }
        let (keys, modifiers, pressed) = ctx.input(|i| {
            let pressed: Vec<egui::Key> = i
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => Some(*key),
                    _ => None,
                })
                .collect();
            (i.keys_down.clone(), i.modifiers, pressed)
        });
        let held = &mut self.window_keys_next;
        held.keys.extend(keys);
        held.shift |= modifiers.shift;
        held.symbol |= modifiers.alt || modifiers.ctrl;
        held.pressed.extend(pressed);
    }

    /// The Graphics window's Find: stop the machine, so what is on the screen
    /// stays there, and wait for a block of it to be clicked.
    pub fn start_graphics_find(&mut self) {
        self.running = false;
        self.gfx_picking = true;
        self.set_status(
            "Click an 8x8 block on the screen to look for it in memory; Esc to give up".into(),
            false,
        );
    }

    /// While a Find is waiting: ring the block under the pointer, and on a
    /// click take its eight bytes from the display file and look for them.
    fn pick_graphics_block(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        picture: egui::Rect,
        painter: &egui::Painter,
    ) {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.gfx_picking = false;
            self.set_status("No block picked".into(), false);
            return;
        }
        let view = self.view();
        let scale = self.scale;
        let cell_at = |p: egui::Pos2| -> Option<(u16, u16)> {
            let x = ((p.x - picture.left()) / scale) as i32 - view.border_x as i32;
            let y = ((p.y - picture.top()) / scale) as i32 - view.border_top as i32;
            ((0..256).contains(&x) && (0..192).contains(&y))
                .then_some(((x / 8) as u16, (y / 8) as u16))
        };
        let Some((col, row)) = response.hover_pos().and_then(cell_at) else {
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        let corner = picture.min
            + egui::vec2(
                (view.border_x as f32 + col as f32 * 8.0) * scale,
                (view.border_top as f32 + row as f32 * 8.0) * scale,
            );
        painter.rect_stroke(
            egui::Rect::from_min_size(corner, egui::vec2(8.0 * scale, 8.0 * scale)),
            0.0,
            egui::Stroke::new(2.0, theme::YELLOW),
            egui::StrokeKind::Outside,
        );
        if !response.clicked() {
            return;
        }
        let pattern: [u8; 8] = std::array::from_fn(|i| {
            self.peek(crate::machine::screen_bitmap_addr(row * 8 + i as u16, col))
        });
        self.gfx_picking = false;
        let result = crate::gfxfind::search(&|a| self.peek(a), pattern);
        let said = match &result {
            Ok(s) if s.matches.is_empty() => "The block is nowhere else in memory".to_string(),
            Ok(s) => format!(
                "Found the block {}{} time{} in memory",
                if s.more { "more than " } else { "" },
                s.matches.len(),
                if s.matches.len() == 1 { "" } else { "s" }
            ),
            Err(why) => why.clone(),
        };
        self.set_status(said, result.is_err());
        self.gfx_found = Some(crate::gfxfind::Picked {
            cell: (col, row),
            pattern,
            result,
        });
        self.show_sprites = true;
        self.sprites.raise = true;
    }

    /// Whether either mouse is plugged in.
    fn mouse_fitted(&self) -> bool {
        use crate::hardware::Peripheral;
        let hardware = &self.spec.bus.hardware;
        self.zx81.is_none()
            && (hardware.fitted(Peripheral::KempstonMouse) || hardware.fitted(Peripheral::AmxMouse))
    }

    fn release_mouse(&mut self, ctx: &egui::Context) {
        self.mouse_captured = false;
        self.mouse_rest = egui::Vec2::ZERO;
        self.host_buttons = [false; 3];
        ctx.send_viewport_cmd_to(
            ViewportId::ROOT,
            egui::ViewportCommand::CursorGrab(egui::viewport::CursorGrab::None),
        );
        ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::CursorVisible(true));
    }

    /// Give the pointer back on Esc, when the main window loses focus, or when
    /// the mouse is taken off.
    ///
    /// Run from `logic` rather than `ui`: eframe skips `ui` while the window is
    /// not visible, which on macOS includes switching away from it — the
    /// commonest way focus is lost — and the pointer would stay hidden and
    /// held with nothing left to let it go. The Esc is taken, so nothing else
    /// sees it.
    fn watch_mouse_capture(&mut self, ctx: &egui::Context) {
        if !self.mouse_captured {
            return;
        }
        let fitted = self.mouse_fitted();
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let focused = ctx.input(|i| i.focused);
        if escape || !focused || !fitted {
            self.release_mouse(ctx);
            if escape {
                self.set_status("The mouse is yours again".into(), false);
            } else if !focused {
                self.set_status(
                    "The mouse is yours again: the window lost focus".into(),
                    false,
                );
            }
        }
    }

    /// Put a blank tape in the deck and start recording onto it.
    ///
    /// What the machine saves through MIC is read back into blocks as it
    /// arrives, and each one is added to the tape once MIC has been quiet long
    /// enough for it to be over, so the tape window lists them as they are
    /// made.
    pub fn blank_tape(&mut self) {
        if self.on_zx81() {
            self.set_status(
                "Saving to a blank tape is a Spectrum's for now.".into(),
                true,
            );
            return;
        }
        self.set_tape(Some(crate::tape::Tape::from_blocks(
            "Blank tape".into(),
            Vec::new(),
        )));
        self.spec.bus.recorder = Some(crate::recorder::Recorder::new());
        self.set_status(
            "A blank tape is in the deck and recording: SAVE \"name\" on the machine writes \
             onto it, and Save… in the tape window writes the tape out."
                .into(),
            false,
        );
    }

    /// Write the tape in the deck out: `.tzx` keeps the pauses, `.tap` is what
    /// everything reads.
    pub fn save_tape(&mut self) {
        let Some(tape) = self.spec.bus.tape.as_ref() else {
            self.set_status("There is no tape in the deck".into(), true);
            return;
        };
        if tape.blocks.is_empty() {
            self.set_status(
                "The tape is blank: nothing has been saved to it yet".into(),
                true,
            );
            return;
        }
        let dialog = rfd::FileDialog::new()
            .add_filter("Tape", &["tzx", "tap"])
            .set_file_name("recording.tzx");
        let dialog = match self.prefs.dir_for(FileKind::Tape).cloned() {
            Some(dir) if dir.is_dir() => dialog.set_directory(dir),
            _ => dialog,
        };
        let Some(path) = dialog.save_file() else {
            self.set_status("Tape not saved".into(), false);
            return;
        };
        let tap = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("tap"));
        let blocks = &self.spec.bus.tape.as_ref().expect("checked").blocks;
        let bytes = if tap {
            crate::recorder::to_tap(blocks)
        } else {
            crate::recorder::to_tzx(blocks)
        };
        match bytes.and_then(|b| {
            std::fs::write(&path, &b)
                .map(|_| b.len())
                .map_err(|e| e.to_string())
        }) {
            Ok(len) => self.set_status(format!("Wrote {} ({len} bytes)", path.display()), false),
            Err(e) => self.set_status(format!("Could not write {}: {e}", path.display()), true),
        }
    }

    /// Write the screen the machine is showing to a `.scr`.
    ///
    /// A screen is not a snapshot: it is the 6,912 bytes the ULA is drawing
    /// from, which is what everyone means by a Spectrum screenshot and what
    /// every paint package on the machine reads and writes.
    pub fn save_screen(&mut self) {
        if self.on_zx81() {
            self.set_status(
                "A .scr is a Spectrum's display file; the ZX81's screen is a different \
                 thing altogether."
                    .to_string(),
                true,
            );
            return;
        }
        let bytes = crate::scr::save(&self.spec);
        let suggested = self.snapshot_path().with_extension("scr");
        let dialog = rfd::FileDialog::new()
            .add_filter("Screen", &["scr"])
            .set_file_name(
                suggested
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "screen.scr".to_string()),
            );
        let directory = suggested
            .parent()
            .filter(|parent| parent.is_dir())
            .map(std::path::Path::to_path_buf)
            .or_else(|| self.prefs.dir_for(FileKind::Snapshot).cloned())
            .filter(|dir| dir.is_dir());
        let dialog = match directory {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        };
        let Some(path) = dialog.save_file() else {
            self.set_status("Screen not saved".to_string(), false);
            return;
        };
        match std::fs::write(&path, &bytes) {
            Ok(()) => self.set_status(
                format!("Wrote {} ({} bytes)", path.display(), bytes.len()),
                false,
            ),
            Err(e) => self.set_status(format!("Could not write {}: {e}", path.display()), true),
        }
    }

    /// Stop recording and ask where to put it.
    ///
    /// The dialog opens beside the tape in the deck, under the same name with
    /// an `.rzx` on it, which is where somebody would go looking for a
    /// recording of that game.
    pub fn save_recording(&mut self) {
        let Some(bytes) = self.stop_recording() else {
            return;
        };
        let suggested = self.recording_path();
        let dialog = rfd::FileDialog::new().set_file_name(
            suggested
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "recording.rzx".to_string()),
        );
        // Beside the tape if there is one, and otherwise wherever the last
        // recording went.
        let directory = suggested
            .parent()
            .filter(|parent| parent.is_dir())
            .map(std::path::Path::to_path_buf)
            .or_else(|| self.prefs.dir_for(FileKind::Recording).cloned())
            .filter(|dir| dir.is_dir());
        let dialog = match directory {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        };
        let Some(path) = dialog.save_file() else {
            self.set_status("Recording thrown away".to_string(), true);
            return;
        };
        match std::fs::write(&path, &bytes) {
            Ok(()) => {
                // Where recordings go, for the next time one is opened or
                // written.
                self.prefs.remember_file(FileKind::Recording, &path);
                self.set_status(
                    format!("Wrote {} ({} bytes)", path.display(), bytes.len()),
                    false,
                );
            }
            Err(e) => self.set_status(format!("Could not write {}: {e}", path.display()), true),
        }
    }

    /// A file picker that opens where the last file of that kind came from.
    pub fn pick_file(&self, kind: Option<FileKind>) -> Option<std::path::PathBuf> {
        let mut dialog = rfd::FileDialog::new();
        dialog = match kind {
            Some(FileKind::Rom) => dialog.add_filter("ROM image", &["rom", "bin"]),
            Some(FileKind::Tape) => {
                dialog.add_filter("Tape", &["tzx", "tap", "p", "81", "p81", "zip"])
            }
            Some(FileKind::Snapshot) => {
                dialog.add_filter("Snapshot", &["sna", "z80", "szx", "scr"])
            }
            // No filter, deliberately. rfd's macOS backend sets the panel's
            // allowed types from the extension list through an API that wants
            // types the system knows, and nothing on the machine claims
            // `.rzx`: the recordings end up greyed out and unselectable. The
            // file is checked when it is opened instead.
            Some(FileKind::Recording) => dialog,
            // Same reason as the recordings: nothing on this machine claims
            // `.dsk` either, and a greyed-out disk is worse than no filter.
            Some(FileKind::Disk) => dialog,
            // The same again: nothing on this machine claims `.mdr` either.
            Some(FileKind::Cartridge) => dialog,
            None => dialog
                .add_filter(
                    "Tape, snapshot or ROM",
                    &[
                        "tzx", "tap", "p", "81", "p81", "sna", "z80", "szx", "scr", "rom", "bin",
                        "rzx", "zip",
                    ],
                )
                .add_filter("Tape", &["tzx", "tap", "p", "81", "p81"])
                .add_filter("Snapshot", &["sna", "z80", "szx", "scr"])
                .add_filter("Archive", &["zip"])
                .add_filter("ROM image", &["rom", "bin"]),
        };
        // For the catch-all picker, start wherever the most recent file of any
        // kind came from.
        let start = match kind {
            Some(k) => self.prefs.dir_for(k).cloned(),
            None => self
                .prefs
                .tape_dir
                .clone()
                .or_else(|| self.prefs.snapshot_dir.clone())
                .or_else(|| self.prefs.rom_dir.clone()),
        };
        if let Some(dir) = start.filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        dialog.pick_file()
    }

    /// One file picker for every supported type, dispatched by extension.
    pub fn load_any_file(&mut self) {
        if let Some(path) = self.pick_file(None) {
            self.load_path(&path);
        }
    }

    /// Put a tape in the deck of whichever machine is running.
    /// Load whatever is worth loading out of an archive.
    ///
    /// The first file of a kind this can read, which is how a download of one
    /// game is usually shaped: the tape, a scan of the inlay and a text file
    /// about the cracking group. An archive with nothing loadable in it does
    /// nothing, rather than guessing at the readme.
    fn load_zip(&mut self, path: &std::path::Path) {
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(e) => {
                return self.set_status(format!("Could not read {}: {e}", path.display()), true)
            }
        };
        let wanted = [
            "tzx", "tap", "p", "81", "p81", "rzx", "sna", "z80", "szx", "scr", "dsk", "ipf",
        ];
        let Some((name, bytes)) = crate::zip::first_with_extension(&data, &wanted) else {
            return self.set_status(
                format!(
                    "{} holds nothing this can load",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
                true,
            );
        };

        let inner = std::path::Path::new(&name);
        let ext = inner
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "rzx" => self.load_recording_bytes(path, &bytes),
            // A disk goes into a machine with a drive, whether it arrived on
            // its own or inside an archive.
            "dsk" | "ipf" => {
                if self.on_zx81() || !self.spec.bus.model.has_disk() {
                    self.switch_model(Model::Plus3);
                }
                if self.spec.bus.model.has_disk() {
                    let source = disk::Source::InArchive {
                        archive: path.to_path_buf(),
                        inner: name.clone(),
                    };
                    self.open_disk_bytes(source, &bytes);
                }
            }
            // A screen is not a snapshot: it goes into the machine that is
            // running rather than replacing it.
            "scr" => match crate::scr::load(&mut self.spec, &bytes) {
                Ok(()) => self.set_status(format!("{name} is on the screen"), false),
                Err(e) => self.set_status(format!("{name}: {e}"), true),
            },
            "szx" => match crate::szx::probe_model(&bytes) {
                Ok(model) => {
                    self.switch_model(model);
                    match crate::szx::load(&mut self.spec, &bytes) {
                        Ok(note) => self.set_status(format!("Loaded {name}: {note}"), false),
                        Err(e) => self.set_status(format!("{name}: {e}"), true),
                    }
                }
                Err(e) => self.set_status(format!("{name}: {e}"), true),
            },
            "sna" | "z80" => match snapshot::probe_model_bytes(&ext, &bytes) {
                Ok(model) => {
                    self.switch_model(model);
                    let loaded = if ext == "sna" {
                        snapshot::load_sna(&mut self.spec, &bytes)
                    } else {
                        snapshot::load_z80(&mut self.spec, &bytes)
                    };
                    match loaded {
                        Ok(()) => {
                            self.set_status(format!("Loaded {name} from {}", path.display()), false)
                        }
                        Err(e) => self.set_status(format!("{name}: {e}"), true),
                    }
                }
                Err(e) => self.set_status(format!("{name}: {e}"), true),
            },
            _ => {
                // A ZX81 program only plays into a ZX81, the same as one that
                // arrived on its own.
                if matches!(ext.as_str(), "p" | "81" | "p81") && !self.on_zx81() {
                    self.switch_to_zx81(self.zx81_ram);
                }
                self.insert_tape_bytes(path, &name, &bytes);
            }
        }
    }

    fn insert_tape(&mut self, path: &std::path::Path) {
        match crate::tape::Tape::load(path) {
            Ok(t) => self.accept_tape(path, t),
            Err(e) => self.set_status(format!("Tape load failed: {e}"), true),
        }
    }

    /// The same, for a tape that came out of an archive. The notes still go
    /// beside the archive: that is the file the user has, and unpacking it
    /// somewhere temporary to hold the annotations would lose them.
    fn insert_tape_bytes(&mut self, path: &std::path::Path, name: &str, data: &[u8]) {
        match crate::tape::Tape::from_bytes(name, data) {
            Ok(t) => self.accept_tape(path, t),
            Err(e) => self.set_status(format!("Tape load failed: {e}"), true),
        }
    }

    fn accept_tape(&mut self, path: &std::path::Path, t: crate::tape::Tape) {
        let blocks = t.blocks.len();
        let zx81 = matches!(t.blocks.first(), Some(crate::tape::Block::Zx81 { .. }));
        let name = t.name.clone();
        self.set_tape(Some(t));
        self.show_tape = true;
        self.tape.scroll_to_current = true;
        self.tape.last_block = None;
        self.prefs.remember_file(FileKind::Tape, path);
        self.tape_path = Some(path.to_path_buf());
        self.reload_notes();
        // A tape waits for Play, as a real one does; a ZX81 also needs
        // LOAD "" typed at it first, which is easy to forget.
        let hint = if zx81 {
            " — type LOAD \"\" then press Play"
        } else {
            " — press Play"
        };
        let plural = if blocks == 1 { "block" } else { "blocks" };
        self.set_status(format!("Tape: {name} ({blocks} {plural}){hint}"), false);
    }

    /// Write the notes out a little after they last changed.
    ///
    /// AutoDoc writes into them without anybody touching a field, so waiting
    /// for one to lose focus would leave a session's worth of guesses unsaved.
    /// The delay keeps it to one write rather than one per frame while the
    /// listing is being scrolled about.
    fn save_notes_if_due(&mut self) {
        const AFTER: std::time::Duration = std::time::Duration::from_secs(3);
        if !self.notes.is_dirty() {
            self.notes_changed_at = None;
            return;
        }
        let since = *self
            .notes_changed_at
            .get_or_insert_with(std::time::Instant::now);
        if since.elapsed() < AFTER {
            return;
        }
        self.notes_changed_at = None;
        if let Err(e) = self.notes.save_if_dirty() {
            self.set_status(format!("Could not save notes: {e}"), true);
        }
    }

    /// Load a recording and start it playing.
    ///
    /// The snapshot inside it says which machine it was made on, so that one
    /// is brought up first; everything after that is the recording's doing.
    fn load_recording(&mut self, path: &std::path::Path) {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                return self.set_status(format!("Could not read {}: {e}", path.display()), true)
            }
        };
        self.load_recording_bytes(path, &bytes);
        // Where recordings are kept, for the next time one is opened. Only
        // for a file that is really a recording on disk: an archive is
        // remembered as whatever it was opened as.
        self.prefs.remember_file(FileKind::Recording, path);
    }

    /// A recording from bytes rather than from a file, so one that arrived
    /// inside an archive can be played without being written out first. The
    /// path is still the archive's, which is where the notes go.
    fn load_recording_bytes(&mut self, path: &std::path::Path, bytes: &[u8]) {
        let recording = match crate::rzx::parse(bytes) {
            Ok(recording) => recording,
            Err(e) => return self.set_status(format!("{}: {e}", path.display()), true),
        };
        let Some(snapshot) = recording.snapshot.clone() else {
            return self.set_status(
                "The recording carries no snapshot to start from".into(),
                true,
            );
        };

        match snapshot::probe_model_bytes(&snapshot.extension, &snapshot.data) {
            Ok(model) => self.switch_model(model),
            Err(e) => return self.set_status(format!("{}: {e}", path.display()), true),
        }
        let loaded = match snapshot.extension.as_str() {
            "sna" => snapshot::load_sna(&mut self.spec, &snapshot.data),
            "z80" => snapshot::load_z80(&mut self.spec, &snapshot.data),
            other => Err(format!(
                "unsupported snapshot type in the recording: .{other}"
            )),
        };
        if let Err(e) = loaded {
            return self.set_status(format!("{}: {e}", path.display()), true);
        }

        let frames = recording.len();
        let by = if recording.creator.is_empty() {
            String::new()
        } else {
            format!(" by {}", recording.creator)
        };
        self.rzx = Some(RzxPlayback {
            recording,
            path: path.to_path_buf(),
            frame: 0,
            remaining: 0,
            owed: 0.0,
            visited: Default::default(),
            max_speed: false,
        });
        self.spec.bus.playback = Some(crate::machine::Playback::default());
        // A machine that was paused stays paused: somebody who stopped it to
        // look at something has not asked for a recording to start running the
        // moment it is loaded, and the first frame of one is worth looking at.
        let waiting = !self.running;
        // The notes belong beside the recording now: it is what is being read.
        self.reload_notes();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        self.set_status(
            if waiting {
                format!("Loaded {name}{by}: {frames} frames, paused. Press Run to play it.")
            } else {
                format!(
                    "Playing {name}{by}: {frames} frames. The keyboard is the \
                     recording's, not yours."
                )
            },
            false,
        );
    }

    /// Stop playing, and give the machine back to the user.
    pub fn stop_playback(&mut self) {
        self.rzx = None;
        self.spec.bus.playback = None;
        self.reload_notes();
    }

    /// Play the recording forward by however much time has passed.
    fn advance_recording(&mut self, dt: f32, boost: f32) {
        let Some(rzx) = &self.rzx else { return };
        if rzx.finished() {
            let done = rzx.recording.len();
            self.stop_playback();
            self.running = false;
            self.set_status(format!("The recording ended after {done} frames"), false);
            return;
        }

        // A recording is measured in frames rather than T-states, so the speed
        // control counts frames. At maximum speed it runs as many as the cap
        // allows every host frame instead of counting at all.
        //
        // The budget is kept as a fraction of a frame rather than a whole
        // number of them: below one frame per host frame — which is where
        // Race the Beam lives, at five seconds a frame — a whole number is
        // zero nearly always and one now and then, so the picture jumps a
        // frame at a time instead of the beam crawling down it.
        let mut budget = if rzx.max_speed {
            MAX_RECORDED_FRAMES
        } else {
            let rate = crate::machine::CPU_HZ as f32 / self.spec.bus.model.frame_t() as f32;
            (rzx.owed + dt * rate * self.speed * boost).min(MAX_RECORDED_FRAMES)
        };

        // In slow motion the frame is run in pieces, so the picture moves
        // under the beam instead of jumping a frame at a time; at normal
        // speed and above, whole frames, with the fraction carried over.
        //
        // From the speed the user asked for, not from the budget: a frame of
        // a 128K recording is a little longer than the fiftieth of a second a
        // host frame takes, so at full speed the budget is 0.99 frames a call
        // and every frame would be split for no reason.
        let crawling = self.speed < 1.0 && !rzx.max_speed;
        while budget > 0.0 {
            let Some(rzx) = &mut self.rzx else { return };
            if rzx.finished() {
                return;
            }
            let whole_frame = rzx.recording.frames[rzx.frame].fetches.max(1) as u32;
            // What the budget will pay for, of the frame the recording is on.
            // A budget too small to buy a single fetch buys nothing and is
            // carried over: rounding it up to one would run a sliver of the
            // next frame after every whole one, and leave the machine a few
            // dozen instructions into a frame it has not been asked to begin.
            let started = rzx.remaining > 0;
            let ask = if budget >= 1.0 {
                if started {
                    rzx.remaining
                } else {
                    whole_frame
                }
            } else if crawling || started {
                let piece = (budget * whole_frame as f32) as u32;
                if started {
                    rzx.remaining.min(piece)
                } else {
                    piece
                }
            } else {
                0
            };
            if ask == 0 {
                break;
            }
            // A frame that was interrupted part-way through is picked up where
            // it stopped, with the input it had already been handed. A frame
            // is not begun — and its input not put in front of the machine —
            // until there is budget to run some of it.
            if !started {
                let frame = &rzx.recording.frames[rzx.frame];
                let fetches = frame.fetches as u32;
                let inputs = frame.inputs.clone();
                if let Some(rzx) = &mut self.rzx {
                    rzx.remaining = fetches;
                }
                if let Some(playback) = &mut self.spec.bus.playback {
                    playback.inputs = inputs;
                    playback.cursor = 0;
                }
            }
            let (stop, ran) = self.spec.run_fetches(ask);
            budget -= ran as f32 / whole_frame as f32;
            let pc = self.spec.cpu.pc;

            let mut ended = false;
            if let Some(rzx) = &mut self.rzx {
                // A prefixed instruction is two fetches or more, so the last
                // one can carry past what was asked for. Saturating rather
                // than wrapping: the frame is over either way, and taking the
                // difference off an unsigned count leaves it enormous, which
                // ran the frame to the end of the recording and read input
                // that was never recorded.
                rzx.remaining = rzx.remaining.saturating_sub(ran);
                if rzx.remaining == 0 {
                    rzx.frame += 1;
                    // Where the recording actually got to: AutoDoc reads this
                    // on top of what it can work out from the code alone.
                    if rzx.visited.len() < VISITED_CAP {
                        rzx.visited.insert(pc);
                    }
                    ended = true;
                }
            }
            if ended {
                // The frame boundary is the recording's, so the video frame
                // ends here and the interrupt is raised here — not wherever
                // the T-state count happens to have reached. On the machine
                // the recording was made on these were the same moment, and
                // letting them drift apart puts every screen effect at the
                // wrong height.
                self.spec.bus.end_frame_here();
                self.spec.bus.raise_interrupt();
            }
            self.last_stop = Some(stop);
            if !matches!(stop, Stop::Budget) {
                self.handle_stop(stop);
                return;
            }
        }
        // What is left over is owed to the next host frame, so a pace of a
        // fifth of a frame a second adds up to a frame every five seconds.
        if let Some(rzx) = &mut self.rzx {
            rzx.owed = budget.max(0.0);
        }
    }

    /// The file the notes and symbols belong to: the recording if one is
    /// playing, otherwise the tape, otherwise the ROM.
    pub fn notes_source(&self) -> Option<std::path::PathBuf> {
        self.rzx
            .as_ref()
            .map(|rzx| rzx.path.clone())
            .or_else(|| self.tape_path.clone())
            .or_else(|| self.rom_path.clone())
    }

    /// Where names supplied by hand are read from: one file beside whatever is
    /// being disassembled, and one shared file in the preferences directory.
    ///
    /// The shared one is where a ROM disassembly goes. It describes the
    /// machine rather than any one game, so tying it to a tape would mean
    /// copying it beside every tape.
    pub fn symbol_files(&self) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        if let Some(dir) = crate::prefs::config_dir() {
            // Per machine as well as shared: the ZX81 and the Spectrum have
            // entirely different routines at the same addresses, so one file
            // for both would name each after the other.
            let machine = if self.on_zx81() {
                "zx81"
            } else {
                match self.spec.bus.model {
                    Model::Spectrum48 => "48",
                    Model::Spectrum128 => "128",
                    Model::Plus2A => "plus2a",
                    Model::Plus3 => "plus3",
                }
            };
            files.push(dir.join("symbols.txt"));
            files.push(dir.join(format!("symbols-{machine}.txt")));
            // The 128K's ROM is two 16K ROMs and the +3's is four, each
            // addressed $0000-$3FFF in its own right. A name only means
            // anything alongside which one is paged in, so they have a file
            // each and only the one in use is read.
            if !self.on_zx81() && self.spec.bus.rom_pages() > 1 {
                let rom = self.spec.bus.rom_in_use();
                files.push(dir.join(format!("symbols-{machine}-rom{rom}.txt")));
            }
        }
        if let Some(source) = self.notes_source() {
            // Named after the file itself: `manic.tap` has `manic.symbols.txt`
            // beside it, not `manic.zxrs.symbols.txt`.
            files.push(source.with_extension("symbols.txt"));
        }
        files
    }

    /// Point the notes at whatever is being disassembled: the tape in the
    /// deck if there is one, otherwise the ROM the machine booted from.
    /// Anything unsaved is written out first, so switching tapes does not
    /// throw away what was just typed.
    pub fn reload_notes(&mut self) {
        if let Err(e) = self.notes.save_if_dirty() {
            self.set_status(format!("Could not save notes: {e}"), true);
        }
        // A recording is what is being read when one is playing, so its notes
        // go beside it rather than beside the tape or the ROM.
        let source = self.notes_source();
        self.notes = match source {
            Some(path) => crate::notes::Notes::for_file(&path),
            None => crate::notes::Notes::unattached(),
        };
    }

    pub fn load_path(&mut self, path: &std::path::Path) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "tzx" | "tap" => self.insert_tape(path),
            // A ZX81 program only plays into a ZX81, so bring one up first.
            "p" | "81" | "p81" => {
                if !self.on_zx81() {
                    self.switch_to_zx81(self.zx81_ram);
                }
                if self.on_zx81() {
                    self.insert_tape(path);
                }
            }
            "rzx" => self.load_recording(path),
            // A cartridge goes into a microdrive, so there has to be an
            // Interface 1 for it to hang off.
            "mdr" => {
                if !self
                    .spec
                    .bus
                    .hardware
                    .fitted(crate::hardware::Peripheral::Interface1)
                {
                    self.fit(crate::hardware::Peripheral::Interface1, true);
                }
                self.open_cartridge(path, self.selected_drive);
            }
            // A disk only goes into a machine with a drive, so bring one up.
            "dsk" | "ipf" => {
                if self.on_zx81() || !self.spec.bus.model.has_disk() {
                    self.switch_model(Model::Plus3);
                }
                if self.spec.bus.model.has_disk() {
                    self.open_disk(path);
                }
            }
            // A screen goes into the machine that is running: it is 6,912
            // bytes of display file, not a machine to switch to.
            "scr" => match std::fs::read(path) {
                Ok(bytes) => match crate::scr::load(&mut self.spec, &bytes) {
                    Ok(()) => {
                        self.set_status(format!("{} is on the screen", path.display()), false)
                    }
                    Err(e) => self.set_status(format!("{}: {e}", path.display()), true),
                },
                Err(e) => self.set_status(format!("{}: {e}", path.display()), true),
            },
            "szx" => match std::fs::read(path) {
                Ok(bytes) => match crate::szx::probe_model(&bytes) {
                    Ok(model) => {
                        self.switch_model(model);
                        match crate::szx::load(&mut self.spec, &bytes) {
                            Ok(note) => {
                                self.prefs.remember_file(FileKind::Snapshot, path);
                                self.set_status(format!("Loaded {}: {note}", path.display()), false)
                            }
                            Err(e) => self.set_status(format!("Snapshot load failed: {e}"), true),
                        }
                    }
                    Err(e) => self.set_status(format!("Snapshot load failed: {e}"), true),
                },
                Err(e) => self.set_status(format!("{}: {e}", path.display()), true),
            },
            "sna" | "z80" => match snapshot::probe_model(path) {
                Ok(model) => {
                    self.switch_model(model);
                    match snapshot::load(&mut self.spec, path) {
                        Ok(()) => {
                            self.prefs.remember_file(FileKind::Snapshot, path);
                            self.set_status(format!("Loaded {}", path.display()), false)
                        }
                        Err(e) => self.set_status(format!("Snapshot load failed: {e}"), true),
                    }
                }
                Err(e) => self.set_status(format!("Snapshot load failed: {e}"), true),
            },
            "rom" | "bin" => match std::fs::read(path) {
                Ok(data) => self.load_rom_image(path, data),
                Err(e) => self.set_status(format!("ROM load failed: {e}"), true),
            },
            "zip" => self.load_zip(path),
            other => self.set_status(format!("Unsupported file type: .{other}"), true),
        }
    }

    /// Note where a window is now, so it can be put back next time.
    fn remember_window(&mut self, name: &str, ctx: &egui::Context) {
        let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        // Position comes from the outer rectangle and size from the inner one,
        // to match what the viewport builder takes: mixing them would grow
        // every window by the height of its title bar on each launch.
        if let (Some(outer), Some(inner)) = (outer, inner) {
            // A window with a fixed width does not have that width recorded:
            // only its height and where it sits.
            let w = Self::fix_width_of(name).unwrap_or_else(|| inner.width());
            self.prefs.set_window(
                name,
                WindowRect {
                    x: outer.min.x,
                    y: outer.min.y,
                    w,
                    h: inner.height(),
                },
            );
        }
    }

    /// Whether a debug window has been put back where it was left. Closing one
    /// clears this, so reopening places it again.
    pub fn window_is_placed(&self, name: &str) -> bool {
        self.placed.contains_key(name)
    }

    /// Fix a window's width at the window manager's level, so a sideways drag
    /// is refused rather than allowed and then undone.
    ///
    /// Sent as commands rather than left to the viewport builder: the builder
    /// is only diffed against the frame before, so a window that is rebuilt —
    /// which happens whenever it has not been drawn for a while — comes back
    /// without the constraint.
    /// Windows whose width is not up for negotiation, and what it is.
    ///
    /// The tape window is built around the cassette. The RAM map is 256 bytes
    /// across however it is drawn, so it is given the same width and drawn to
    /// fill it: two windows of one width sit together without a ragged edge,
    /// and neither has anything to gain from being dragged wider.
    pub fn fix_width_of(name: &str) -> Option<f32> {
        match name {
            // The disk window is built around a picture of the drive, which is
            // as wide as it is; the tape window's width suits it and two
            // windows of one width sit together without a ragged edge.
            "tape" | "ram_map" | "disk" | "microdrive" | "printer" => Some(cassette::WINDOW_W),
            _ => None,
        }
    }

    fn fix_width(&self, ctx: &egui::Context, width: f32) {
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize([width, 320.0].into()));
        ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize([width, 8000.0].into()));
    }

    /// Put a debug window back where it was left.
    ///
    /// The geometry in the viewport builder is not reliably honoured when the
    /// window is created — the window manager may centre a default-sized one
    /// instead — so it is sent again from inside the viewport, every frame,
    /// until the window reports that it has arrived. Recording where the
    /// window is only starts then, or the position it is being moved away
    /// from would be saved over the real one.
    ///
    /// Returns whether the window has settled, and can be recorded.
    fn place_window(
        &mut self,
        name: &'static str,
        ctx: &egui::Context,
        default_pos: [f32; 2],
        default_size: [f32; 2],
    ) -> bool {
        /// How long to keep asking before letting the window be where it is.
        const GIVE_UP: std::time::Duration = std::time::Duration::from_secs(2);
        /// A window landing within a couple of points is close enough.
        const NEAR: f32 = 2.0;

        let (pos, mut size) = match self.prefs.window(name) {
            Some(r) => ([r.x, r.y], [r.w, r.h]),
            None => (default_pos, default_size),
        };
        // A fixed-width window's width is not the user's to choose; the
        // height still is.
        if let Some(w) = Self::fix_width_of(name) {
            size[0] = w;
        }
        let placement = self.placed.entry(name).or_insert(Placement {
            asked_at: std::time::Instant::now(),
            settled: false,
        });
        if placement.settled {
            return true;
        }
        let arrived = ctx
            .input(|i| i.viewport().outer_rect)
            .is_some_and(|r| (r.min.x - pos[0]).abs() <= NEAR && (r.min.y - pos[1]).abs() <= NEAR);
        if arrived || placement.asked_at.elapsed() > GIVE_UP {
            placement.settled = true;
            return arrived;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos.into()));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
        false
    }

    /// Apply the saved geometry for a window, once.
    fn restore_window(
        &mut self,
        name: &'static str,
        builder: ViewportBuilder,
        default_pos: [f32; 2],
        default_size: [f32; 2],
    ) -> ViewportBuilder {
        // The geometry goes in every frame, not just the first. A window that
        // is not drawn for a while — while the application is in the
        // background, say — is retired, and the next frame builds it again; a
        // builder without a size gets the window system's default, which is
        // how alt-tabbing away and back used to resize everything to 800x600.
        // Asking for what the window already is costs nothing.
        let (pos, mut size) = match self.prefs.window(name) {
            Some(r) => ([r.x, r.y], [r.w, r.h]),
            None => (default_pos, default_size),
        };
        if let Some(w) = Self::fix_width_of(name) {
            size[0] = w;
        }
        builder.with_position(pos).with_inner_size(size)
    }

    /// Come up as the machine that was last in use, with what was plugged
    /// into it.
    ///
    /// A ROM that is not there is not an error: somebody who had a +3 and has
    /// since moved its ROM keeps the machine the emulator can actually be,
    /// rather than being told off at every start-up.
    fn apply_machine_settings(&mut self) {
        if let Some(name) = self.prefs.machine.clone() {
            match name.as_str() {
                "zx81-1k" => self.switch_to_zx81(zx81::Ram::K1),
                "zx81-16k" => self.switch_to_zx81(zx81::Ram::K16),
                other => {
                    if let Some(model) = [
                        Model::Spectrum48,
                        Model::Spectrum128,
                        Model::Plus2A,
                        Model::Plus3,
                    ]
                    .into_iter()
                    .find(|m| m.name() == other)
                    {
                        if self.roms.for_model(model).is_some() {
                            self.switch_model(model);
                        }
                    }
                }
            }
        }
        if let Some(fitted) = self.prefs.peripherals.clone() {
            for key in fitted {
                if let Some(what) = crate::hardware::Peripheral::from_key(&key) {
                    self.fit(what, true);
                }
            }
        }
        if let Some(kind) = self.prefs.joystick.clone() {
            if let Some(kind) = crate::joystick::Kind::from_key(&kind) {
                // Through the same door as the windows, so a file from before
                // the interfaces were in the Hardware window fits the one its
                // stick needs.
                self.spec.bus.set_joystick(kind);
            }
        }
        if let Some(program) = self.prefs.joystick_program.clone() {
            crate::joystick::program_from_text(&mut self.spec.bus.joystick, &program);
        }
        if let Some(map) = self.prefs.joystick_map.clone() {
            // An empty mapping is somebody having taken every line out, which
            // is theirs to do; a file with no mapping at all gets the arrows.
            self.joystick_map = inputwin::from_text(&map);
        }
        if let Some(drives) = self.prefs.microdrives {
            self.spec.bus.hardware.if1_drives = drives.clamp(1, crate::if1::MAX_DRIVES);
            if let Some(if1) = self.spec.bus.if1.as_mut() {
                if1.set_drive_count(drives);
            }
        }
        // Nothing was said about the machine before this was written down, so
        // the first close records whatever is running rather than losing it.
        self.remember_machine_settings();
    }

    /// What machine is in use, by the name it is saved under.
    pub fn machine_key(&self) -> String {
        if self.on_zx81() {
            match self.zx81_ram {
                zx81::Ram::K1 => "zx81-1k".into(),
                zx81::Ram::K16 => "zx81-16k".into(),
            }
        } else {
            self.spec.bus.model.name().to_string()
        }
    }

    fn remember_machine_settings(&mut self) {
        self.prefs.machine = Some(self.machine_key());
        self.prefs.peripherals = Some(
            self.spec
                .bus
                .hardware
                .all_fitted()
                .iter()
                .map(|p| p.key().to_string())
                .collect(),
        );
        self.prefs.microdrives = Some(self.spec.bus.hardware.if1_drives);
        self.prefs.joystick = Some(self.spec.bus.joystick.kind.key().to_string());
        self.prefs.joystick_map = Some(inputwin::to_text(&self.joystick_map));
        // Kept between launches as the bindings are. The box itself probably
        // forgot when the power went; somebody who has taught it keys for a
        // game should not have to teach it again.
        self.prefs.joystick_program =
            Some(crate::joystick::program_to_text(&self.spec.bus.joystick));
    }

    /// Write the window layout and display settings out. Called on close.
    pub fn save_window_state(&mut self) {
        self.prefs.display_scale = Some(self.scale);
        self.prefs.overscan = Some(self.overscan);
        self.prefs.open_windows = Some(self.open_windows());
        self.remember_tape_settings();
        self.remember_machine_settings();
        self.prefs.save();
        self.last_saved = Some(self.prefs.to_text());
        self.last_save_at = Some(std::time::Instant::now());
    }

    /// Save the layout if it has changed and has been still for a moment.
    fn save_window_state_if_settled(&mut self) {
        const SETTLE: std::time::Duration = std::time::Duration::from_secs(2);
        if self.last_save_at.is_some_and(|at| at.elapsed() < SETTLE) {
            return;
        }
        self.prefs.display_scale = Some(self.scale);
        self.prefs.overscan = Some(self.overscan);
        self.prefs.open_windows = Some(self.open_windows());
        self.remember_tape_settings();
        self.remember_machine_settings();
        let text = self.prefs.to_text();
        if self.last_saved.as_deref() == Some(text.as_str()) {
            self.last_save_at = Some(std::time::Instant::now());
            return;
        }
        self.save_window_state();
    }

    /// Put the tape window's settings into the preferences.
    ///
    /// Everything the window is set to, since a deck somebody has dialled in —
    /// a head out of square by a particular amount, a hiss at a particular
    /// level — is tedious to find again. Writing is left to the settling save,
    /// which only writes when something has actually changed, so dragging a
    /// slider does not write the file forty times a second.
    fn remember_tape_settings(&mut self) {
        let q = self.quality;
        let tape = &self.tape;
        let pairs: [(&str, String); 13] = [
            ("tape_scope", tape.show_scope.to_string()),
            ("tape_quality_shown", tape.show_quality.to_string()),
            ("tape_scope_us", tape.window_us.to_string()),
            (
                "tape_scope_trigger",
                match tape.trigger {
                    crate::ui::tape::Trigger::Rising => "rising",
                    crate::ui::tape::Trigger::Falling => "falling",
                    crate::ui::tape::Trigger::Off => "free",
                }
                .to_string(),
            ),
            (
                "tape_speed",
                match (self.tape_flash(), self.tape_boost()) {
                    (true, _) => "fastload",
                    (false, true) => "max",
                    (false, false) => "normal",
                }
                .to_string(),
            ),
            ("tape_wobble", q.wobble.to_string()),
            ("tape_wow", q.wow.to_string()),
            ("tape_flutter", q.flutter.to_string()),
            ("tape_alignment", q.alignment.to_string()),
            ("tape_alignment_offset", q.alignment_offset.to_string()),
            ("tape_alignment_wobble", q.alignment_wobble.to_string()),
            ("tape_noise", q.noise.to_string()),
            ("tape_noise_level", q.noise_level.to_string()),
        ];
        for (key, value) in pairs {
            self.prefs.set(key, value);
        }
    }

    /// Take the tape window's settings back out of the preferences.
    fn apply_tape_settings(&mut self) {
        if let Some(on) = self.prefs.get_as("tape_scope") {
            self.tape.show_scope = on;
        }
        if let Some(on) = self.prefs.get_as("tape_quality_shown") {
            self.tape.show_quality = on;
        }
        if let Some(us) = self
            .prefs
            .get_as::<f32>("tape_scope_us")
            .filter(|v| *v > 0.0)
        {
            self.tape.window_us = us;
        }
        match self.prefs.get("tape_scope_trigger") {
            Some("rising") => self.tape.trigger = crate::ui::tape::Trigger::Rising,
            Some("falling") => self.tape.trigger = crate::ui::tape::Trigger::Falling,
            Some("free") => self.tape.trigger = crate::ui::tape::Trigger::Off,
            _ => {}
        }
        match self.prefs.get("tape_speed") {
            Some("fastload") => self.set_hurry(Hurry::Fastload),
            Some("max") => self.set_hurry(Hurry::Max),
            Some("normal") => self.set_hurry(Hurry::Normal),
            _ => {}
        }
        let q = &mut self.quality;
        if let Some(v) = self.prefs.get_as("tape_wobble") {
            q.wobble = v;
        }
        if let Some(v) = self.prefs.get_as("tape_wow") {
            q.wow = v;
        }
        if let Some(v) = self.prefs.get_as("tape_flutter") {
            q.flutter = v;
        }
        if let Some(v) = self.prefs.get_as("tape_alignment") {
            q.alignment = v;
        }
        if let Some(v) = self.prefs.get_as("tape_alignment_offset") {
            q.alignment_offset = v;
        }
        if let Some(v) = self.prefs.get_as("tape_alignment_wobble") {
            q.alignment_wobble = v;
        }
        if let Some(v) = self.prefs.get_as("tape_noise") {
            q.noise = v;
        }
        if let Some(v) = self.prefs.get_as("tape_noise_level") {
            q.noise_level = v;
        }
    }

    /// Take the display settings from the preferences file, if it has any.
    pub fn apply_prefs(&mut self) {
        self.apply_tape_settings();
        self.apply_machine_settings();
        if let Some(scale) = self.prefs.display_scale.filter(|s| *s > 0.0) {
            // Somebody who last left it at half size has a preferences file
            // asking for a zoom that is not offered any more; they get the
            // smallest that is rather than a window nothing can set.
            let smallest = screen::SCALES[0];
            self.scale = scale.max(smallest);
        }
        if let Some(overscan) = self.prefs.overscan {
            self.overscan = overscan;
        }
        if let Some(open) = self.prefs.open_windows.clone() {
            let is_open = |name: &str| open.iter().any(|n| n == name);
            self.show_ram_map = is_open("ram_map");
            self.show_debugger = is_open("debugger");
            self.show_tape = is_open("tape");
            self.show_back_buffer = is_open("back_buffer");
            self.show_sprites = is_open("sprites");
            self.show_callflow = is_open("callflow");
            self.show_profiler = is_open("profiler");
            self.show_keyboard = is_open("keyboard");
            self.show_disk = is_open("disk");
            self.show_hardware = is_open("hardware");
            self.show_input = is_open("joystick");
            self.show_printer = is_open("printer");
            self.show_training = is_open("training");
            self.show_microdrive = is_open("microdrive");
        }
    }

    /// Which debug windows are open, by the names their geometry is saved
    /// under, so they can be opened again with the emulator.
    fn open_windows(&self) -> Vec<String> {
        [
            ("ram_map", self.show_ram_map),
            ("debugger", self.show_debugger),
            ("tape", self.show_tape),
            ("back_buffer", self.show_back_buffer),
            ("sprites", self.show_sprites),
            ("callflow", self.show_callflow),
            ("profiler", self.show_profiler),
            ("keyboard", self.show_keyboard),
            ("disk", self.show_disk),
            ("hardware", self.show_hardware),
            ("joystick", self.show_input),
            ("printer", self.show_printer),
            ("microdrive", self.show_microdrive),
            ("training", self.show_training),
        ]
        .into_iter()
        .filter(|(_, open)| *open)
        .map(|(name, _)| name.to_string())
        .collect()
    }

    /// What the machine is called, as the dropdown says it.
    pub fn machine_name(&self) -> String {
        if self.on_zx81() {
            self.zx81_ram.name().to_string()
        } else {
            self.spec.bus.model.name().to_string()
        }
    }

    /// What to say after a reset.
    ///
    /// The keyboard is dead for a second or so afterwards and it is worth
    /// saying why: the ROM checks every byte of RAM before it does anything
    /// else, with interrupts off, and the keyboard is read by the interrupt.
    /// Nothing is being ignored — there is nothing to ignore it yet.
    /// What the status line says after a reset, for the tests.
    pub fn starting_up_for_test(&self) -> String {
        self.starting_up()
    }

    fn starting_up(&self) -> String {
        // Measured, per machine, from reset to the ROM reading a key: a 48K
        // checks its RAM with the interrupt off and takes 85 frames over it,
        // and the later ROMs are quicker about it.
        let frames = match self.spec.bus.model {
            Model::Spectrum48 => 85.0,
            Model::Spectrum128 => 54.0,
            Model::Plus2A | Model::Plus3 => 57.0,
        } / self.speed.max(0.01);
        format!(
            "Reset ({}) — the ROM checks the RAM before it reads the keyboard, about {:.1}s",
            self.spec.bus.model.name(),
            frames / 50.0
        )
    }

    /// True when the ZX81 is the machine in use.
    pub fn on_zx81(&self) -> bool {
        self.zx81.is_some()
    }

    // ---- the tape deck, wherever it currently is ---------------------------
    //
    // Both machines have their own deck, because both have their own clock and
    // a tape is timed in T-states. These pick out whichever one is running so
    // the tape window does not have to care.

    pub fn tape_ref(&self) -> Option<&crate::tape::Tape> {
        match &self.zx81 {
            Some(zx) => zx.bus.tape.as_ref(),
            None => self.spec.bus.tape.as_ref(),
        }
    }

    pub fn tape_mut(&mut self) -> Option<&mut crate::tape::Tape> {
        match &mut self.zx81 {
            Some(zx) => zx.bus.tape.as_mut(),
            None => self.spec.bus.tape.as_mut(),
        }
    }

    pub fn set_tape(&mut self, tape: Option<crate::tape::Tape>) {
        // A tape put in the deck is to be played, not recorded onto: a SAVE
        // would otherwise be written onto the end of somebody's game.
        // `blank_tape` arms the recorder again after it comes through here.
        self.spec.bus.recorder = None;
        match &mut self.zx81 {
            Some(zx) => zx.bus.tape = tape,
            None => self.spec.bus.tape = tape,
        }
    }

    // ---- the machine that is running ---------------------------------------
    //
    // Both machines are a Z80 with memory and breakpoints, so the debugger and
    // the RAM map work through these rather than reaching for the Spectrum.

    pub fn cpu(&self) -> &crate::z80::Z80 {
        match &self.zx81 {
            Some(zx) => &zx.cpu,
            None => &self.spec.cpu,
        }
    }

    /// Show an address in the disassembly and mark the row, so what was asked
    /// for can be picked out of the twenty-odd lines around it.
    pub fn show_in_listing(&mut self, addr: u16) {
        self.dbg.view_addr = addr;
        self.dbg.follow_pc = false;
        self.dbg.marked = Some(addr);
        // Put in the middle of the listing rather than at whatever line it
        // happens to fall on, so what was asked for is where the eye goes.
        self.dbg.centre = true;
    }

    /// Read a block of memory as pictures: open the graphics viewer on it,
    /// bring it forward, and start with a sprite size the block divides into.
    ///
    /// Nothing in the bytes says how wide a sprite is meant to be — that is
    /// the one thing the viewer asks for — but a block's length usually
    /// divides by the size of the sprites in it, which is a better place to
    /// start guessing from than whatever was set last.
    pub fn show_as_graphics(&mut self, addr: u16, length: u16) {
        self.sprites.addr = addr;
        self.sprites.addr_text = format!("{addr:04X}");
        self.sprites.block_length = Some(length);
        self.show_sprites = true;
        self.sprites.raise = true;

        // The largest square sprite the block divides into, up to four cells:
        // a sheet of 16x16 sprites is 32 bytes each and a sheet of 8x8 is 8,
        // and either divides the block exactly.
        for cells in (1..=4usize).rev() {
            let stride = (cells * cells * 8) as u16;
            if length >= stride && length.is_multiple_of(stride) {
                self.sprites.cells_across = cells;
                self.sprites.cells_down = cells;
                break;
            }
        }
        // And enough across to show the whole block without scrolling, when
        // that is a sensible number.
        let stride = self.sprites.stride().max(1);
        let sprites = (length / stride) as usize;
        self.sprites.columns = sprites.clamp(1, 8);
    }

    /// The same, from another window: open the debugger, bring it to the
    /// front and give it the focus. Sending somebody to a listing in a window
    /// that is behind the one they are looking at is sending them nowhere.
    pub fn show_in_debugger(&mut self, addr: u16) {
        self.show_in_listing(addr);
        self.show_debugger = true;
        self.dbg.raise = true;
    }

    /// Write a byte into the machine that is running, as the debugger does
    /// when somebody types over one. The ROM ignores it, as the hardware does.
    /// Run to the end of the frame the machine is part-way through.
    ///
    /// What is wanted is a picture the ULA has finished painting: stopping
    /// wherever the machine happens to be leaves half a frame drawn and the
    /// rest of it left over from before. So it runs to the frame boundary,
    /// which is where the beam has finished the screen and gone back to the
    /// top, and stops there.
    pub fn next_frame(&mut self) {
        self.running = false;
        // At full speed, whatever else is set: the point is to arrive at the
        // frame boundary. Slow draw parks the CPU after a few writes and its
        // allowance is only refilled while the machine is running, so leaving
        // it on meant this worked once — on whatever allowance was left — and
        // then did nothing at all.
        let slow = self.spec.bus.slow.enabled;
        self.spec.bus.slow.enabled = false;
        self.spec.bus.slow.begin_slice();

        let was = self.frame_count();
        for _ in 0..32 {
            match &mut self.zx81 {
                Some(zx) => {
                    let frame = zx.frame_t();
                    zx.run(frame);
                }
                None => {
                    self.spec.run(self.spec.bus.frame_t());
                }
            }
            if self.frame_count() != was {
                break;
            }
        }
        self.spec.bus.slow.enabled = slow;
        self.spec.bus.slow.begin_slice();
        self.dbg.follow_pc = true;
        self.status = format!("Frame {}", self.frame_count());
    }

    /// Frames the machine has finished, whichever machine is running.
    pub fn frame_count(&self) -> u64 {
        match &self.zx81 {
            Some(zx) => zx.bus.frame,
            None => self.spec.bus.frame,
        }
    }

    /// Show the byte behind a pixel of the picture in the memory dump.
    ///
    /// The coordinates are of the whole picture, border and all, so the border
    /// is taken off before working out which cell of the display file the
    /// pixel belongs to. A click on the border itself has no byte behind it
    /// and is left alone.
    pub fn show_pixel_in_memory(&mut self, px: usize, py: usize) {
        // Only while the machine is stopped and the debugger is open to show
        // the answer. A click on a running picture is somebody playing a game,
        // not somebody asking where a byte is.
        if self.running || !self.show_debugger {
            return;
        }
        let view = self.view();
        let (Some(x), Some(y)) = (
            px.checked_sub(view.border_x),
            py.checked_sub(view.border_top),
        ) else {
            return;
        };
        if x >= 256 || y >= 192 {
            return;
        }
        let addr = crate::machine::screen_bitmap_addr(y as u16, (x / 8) as u16);
        self.dbg.mem_addr = addr;
        self.dbg.mem_text = format!("{addr:04X}");
        self.dbg.selected = Some((addr, crate::ui::debugger::Column::Hex));
        self.dbg.half_typed = None;
        self.show_debugger = true;
        self.dbg.raise = true;
    }

    /// Save the program in RAM as assembly source (`crate::asmexport`), named
    /// after what is loaded.
    pub fn export_asm(&mut self) {
        let program = self
            .notes
            .file()
            .and_then(|p| p.file_name())
            .map(|n| {
                n.to_string_lossy()
                    .trim_end_matches(".zxrs.txt")
                    .to_string()
            })
            .unwrap_or_else(|| "program".into());
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Assembly source", &["asm", "z80", "s"])
            .set_file_name(format!("{program}.asm"))
            .save_file()
        else {
            self.set_status("Nothing exported".into(), false);
            return;
        };
        let out = crate::asmexport::from_machine(&self.spec, &self.notes, None, &program);
        match std::fs::write(&path, &out.text) {
            Ok(()) => self.set_status(
                format!(
                    "Wrote {}: {} instructions, {} bytes as data",
                    path.display(),
                    out.instructions,
                    out.data_bytes
                ),
                false,
            ),
            Err(e) => self.set_status(format!("Could not write {}: {e}", path.display()), true),
        }
    }

    /// Open the debugger's memory dump at an address and bring the debugger to
    /// the front: whatever asked wants to read the bytes, and a debugger left
    /// behind another window is not somewhere they can be read.
    pub fn show_memory_at(&mut self, addr: u16) {
        self.dbg.mem_addr = addr;
        self.dbg.mem_text = format!("{addr:04X}");
        self.show_debugger = true;
        self.dbg.raise = true;
    }

    pub fn poke_byte(&mut self, addr: u16, value: u8) {
        match &mut self.zx81 {
            Some(zx) => zx.bus.poke(addr, value),
            None => self.spec.bus.poke(addr, value),
        }
    }

    pub fn peek(&self, addr: u16) -> u8 {
        match &self.zx81 {
            Some(zx) => zx.bus.peek_raw(addr),
            None => self.spec.bus.peek_raw(addr),
        }
    }

    pub fn breakpoints(&self) -> &Vec<u16> {
        match &self.zx81 {
            Some(zx) => &zx.breakpoints,
            None => &self.spec.breakpoints,
        }
    }

    pub fn breakpoints_mut(&mut self) -> &mut Vec<u16> {
        match &mut self.zx81 {
            Some(zx) => &mut zx.breakpoints,
            None => &mut self.spec.breakpoints,
        }
    }

    pub fn tracker(&self) -> &crate::tracker::Tracker {
        match &self.zx81 {
            Some(zx) => &zx.bus.tracker,
            None => &self.spec.bus.tracker,
        }
    }

    pub fn tracker_mut(&mut self) -> &mut crate::tracker::Tracker {
        match &mut self.zx81 {
            Some(zx) => &mut zx.bus.tracker,
            None => &mut self.spec.bus.tracker,
        }
    }

    pub fn phys_index(&self, addr: u16) -> usize {
        match &self.zx81 {
            Some(zx) => zx.bus.phys_index(addr),
            None => self.spec.bus.phys_index(addr),
        }
    }

    /// What sits at an address: a bank number on a Spectrum, ROM or RAM on a
    /// ZX81, which has nothing to page.
    pub fn slot_label(&self, addr: u16) -> String {
        match &self.zx81 {
            Some(zx) => if zx.bus.is_rom(addr) { "ROM" } else { "RAM" }.to_string(),
            None => match self.spec.bus.slot_of(addr) {
                crate::machine::Slot::Rom(p) => format!("ROM{p}"),
                crate::machine::Slot::Ram(b) => format!("RAM{b}"),
            },
        }
    }

    pub fn is_rom(&self, addr: u16) -> bool {
        match &self.zx81 {
            Some(zx) => zx.bus.is_rom(addr),
            None => matches!(self.spec.bus.slot_of(addr), crate::machine::Slot::Rom(_)),
        }
    }

    /// Frame number, T-states into the frame, and T-states in a whole frame.
    pub fn machine_clock(&self) -> (u64, u32, u32) {
        match &self.zx81 {
            Some(zx) => (
                zx.bus.frame,
                (zx.bus.tstates % zx.frame_t()) as u32,
                zx.frame_t() as u32,
            ),
            None => (
                self.spec.bus.frame,
                self.spec.bus.tstates,
                self.spec.bus.frame_t(),
            ),
        }
    }

    /// Reset the machine, and let go of the keyboard with it.
    ///
    /// A key the window is holding — a shift clicked and waiting for the key
    /// it shifts, most of all — would otherwise still be down on the machine
    /// that comes up. A machine answering every key with the shifted one
    /// reads as a machine ignoring the keyboard.
    pub fn reset_machine(&mut self) {
        self.keys.release_all();
        match &mut self.zx81 {
            Some(zx) => zx.reset(),
            None => self.spec.reset(),
        }
    }

    pub fn step_machine(&mut self) {
        match &mut self.zx81 {
            Some(zx) => zx.step_instruction(),
            None => {
                // Kept so it can be stepped back over. Only while stepping by
                // hand: a running machine writes millions of bytes a second
                // and none of them is going to be walked back through.
                let undo = self.spec.step_recording();
                if self.rewind.len() == REWIND {
                    self.rewind.pop_front();
                }
                self.rewind.push_back(undo);
            }
        }
    }

    /// How many instructions can be stepped back over.
    ///
    /// Enough to see how the machine got where it is, which is what stepping
    /// back is for; a hundred would be a recording, and there is one of those
    /// already.
    pub fn can_step_back(&self) -> bool {
        !self.rewind.is_empty()
    }

    /// Let the machine run, and throw away what could have been stepped back
    /// over.
    ///
    /// Running writes what an undo cannot put back: a step over a CALL is
    /// thousands of instructions, and the entries kept from before it describe
    /// a machine that no longer exists. Stepping back into that would put the
    /// registers somewhere plausible and leave the memory wrong, which is
    /// worse than not offering it.
    pub fn forget_rewind(&mut self) {
        self.rewind.clear();
    }

    /// Put the last instruction back the way it was.
    pub fn step_back(&mut self) {
        let Some(undo) = self.rewind.pop_back() else {
            self.set_status("Nothing to step back to".to_string(), false);
            return;
        };
        self.running = false;
        self.spec.undo_step(&undo);
        self.dbg.follow_pc = true;
        self.dbg.centre = true;
        self.last_stop = Some(crate::machine::Stop::Stepped);
        self.status = format!(
            "Stepped back to ${:04X}, {} left",
            self.cpu().pc,
            self.rewind.len()
        );
    }

    /// The mixer of the machine that is running.
    pub fn audio(&mut self) -> &mut crate::audio::Audio {
        match &mut self.zx81 {
            Some(zx) => &mut zx.bus.audio,
            None => &mut self.spec.bus.audio,
        }
    }

    pub fn tape_boost(&self) -> bool {
        match &self.zx81 {
            Some(zx) => zx.bus.tape_boost,
            None => self.spec.bus.tape_boost,
        }
    }

    /// Is a loader of the game's own reading the tape at this moment?
    ///
    /// Told by the sampling loop it is sitting in, which nearly every loader
    /// shares — see [`crate::flashload::at_sampler`]. Blocks cannot be handed
    /// to one of those, so what Fastload does for them is let the
    /// machine run.
    pub fn loader_is_reading(&self) -> bool {
        self.loader_reading().is_some()
    }

    /// Which loader's sampling loop it is, so the window can say so rather
    /// than only saying that one is at work.
    pub fn loader_reading(&self) -> Option<&'static str> {
        if self.zx81.is_some() {
            return None;
        }
        crate::flashload::sampler(&self.spec)
    }

    /// How much of a hurry a tape is loaded in. The three settle the two
    /// switches underneath between them: hurrying a tape means running the
    /// machine flat out as well, so they were never independent.
    pub fn set_hurry(&mut self, hurry: Hurry) {
        match hurry {
            Hurry::Normal => {
                self.set_tape_flash(false);
                *self.tape_boost_mut() = false;
            }
            Hurry::Max => {
                self.set_tape_flash(false);
                *self.tape_boost_mut() = true;
            }
            Hurry::Fastload => {
                self.set_tape_flash(true);
                *self.tape_boost_mut() = true;
            }
        }
    }

    /// Whether whole blocks are handed to the ROM's loader rather than
    /// played. The ZX81's ROM is a different one and has no such routine, so
    /// it is a Spectrum switch only.
    pub fn tape_flash(&self) -> bool {
        self.zx81.is_none() && self.spec.bus.tape_flash
    }

    pub fn set_tape_flash(&mut self, on: bool) {
        self.spec.bus.tape_flash = on;
    }

    pub fn tape_boost_mut(&mut self) -> &mut bool {
        match &mut self.zx81 {
            Some(zx) => &mut zx.bus.tape_boost,
            None => &mut self.spec.bus.tape_boost,
        }
    }

    pub fn tape_is_playing(&self) -> bool {
        self.tape_ref().is_some_and(|t| t.playing)
    }

    /// Whether the tape is actually loading something, as against sitting in
    /// the silence the program is meant to be watched through.
    ///
    /// Only the last pause of a tape, or one before a block that stops the
    /// tape, is worth coming back to normal speed for: that is where the
    /// loading ends and the program takes over. The silences between the
    /// blocks of a multi-load are the loader getting ready for the next one,
    /// and dropping to normal speed through every one of them makes a hurried
    /// tape barely quicker than an unhurried one.
    pub fn tape_is_loading(&self) -> bool {
        self.tape_ref()
            .is_some_and(|t| t.playing && !t.pause_ends_the_tape())
    }

    /// Clock of the running machine: tape times are in its T-states, and the
    /// ZX81's is not the Spectrum's.
    pub fn cpu_hz(&self) -> f64 {
        if self.on_zx81() {
            zx81::CPU_HZ
        } else {
            crate::machine::CPU_HZ
        }
    }

    /// The clock of whichever machine is running, before it is wound up.
    pub fn machine_cpu_hz(&self) -> f64 {
        match &self.zx81 {
            Some(_) => zx81::CPU_HZ,
            None => self.spec.bus.model.cpu_hz(),
        }
    }

    /// The clock the CPU is actually being run at.
    pub fn clock_hz(&self) -> f64 {
        self.machine_cpu_hz() * self.turbo() as f64
    }

    /// How many times the machine's own clock the CPU is being run at, after
    /// the two places it has to be held at one.
    ///
    /// **A recording playing.** An RZX frame is a number of opcode fetches and
    /// the recording's frame boundary is the video frame; a CPU getting
    /// through those fetches in a quarter of the ULA time would put four
    /// frames of input into one frame of picture.
    ///
    /// **A tape playing.** Every loader counts turns of its own loop against
    /// pulses that are in ULA time, so at 4× it counts four times as many for
    /// the same pulse and every length it knows is wrong. Nothing loads at
    /// all. That is what an accelerated machine did, which is why they had a
    /// switch — and here the switch throws itself.
    pub fn turbo(&self) -> u32 {
        if self.rzx.is_some() || self.tape_is_playing() {
            return 1;
        }
        (self.clock_mult.max(1.0) as u32).clamp(1, 8)
    }

    /// Why the CPU is not running at what the dropdown says, if it is not.
    pub fn turbo_held_because(&self) -> Option<&'static str> {
        if self.clock_mult <= 1.0 {
            return None;
        }
        if self.rzx.is_some() {
            return Some("a recording is playing");
        }
        if self.tape_is_playing() {
            return Some("a tape is loading");
        }
        None
    }

    /// Tell the mixer what the clock is now.
    ///
    /// Whether anything is being listened to at all, which is what the frame
    /// pacing asks: with every part muted there is no sound to keep in step
    /// with.
    fn sound_wanted(&self) -> bool {
        let audio = &self.spec.bus.audio;
        audio.enabled && (audio.beeper_on || audio.ay_on || audio.hardware_on)
    }

    /// Sound is made of T-states, so a machine running at twice its clock plays
    /// every note an octave up — which is what an accelerated one did, and only
    /// comes out that way if the mixer counts in the same T-states the machine
    /// does.
    pub fn apply_clock(&mut self) {
        // The ULA's clock, not the CPU's: samples are made of T-states of the
        // machine and those still pass at 3.5MHz however fast the CPU is being
        // run. A beeper note comes out an octave up at 2× on its own, because
        // the loop that makes it comes round in half the T-states — which is
        // what an accelerated machine sounded like.
        let hz = self.machine_cpu_hz();
        match &mut self.zx81 {
            Some(zx) => zx.bus.audio.set_cpu_hz(hz),
            None => self.spec.bus.audio.set_cpu_hz(hz),
        }
    }

    /// T-states since power-on for the running machine, which is the timebase
    /// a tape is played against.
    pub fn machine_t(&self) -> u64 {
        match &self.zx81 {
            Some(zx) => zx.bus.tstates,
            None => self.spec.bus.total_t(),
        }
    }

    /// Switch to a ZX81 with the given memory, or back to the Spectrum.
    pub fn switch_to_zx81(&mut self, ram: zx81::Ram) {
        let Some(rom) = self.roms.rom_zx81.clone() else {
            self.set_status(
                "Cannot switch to a ZX81: no ROM. Put an 8K ZX81 ROM at roms/zx81.rom".into(),
                true,
            );
            return;
        };
        if self.zx81.is_some() && self.zx81_ram == ram {
            self.set_status(format!("Already running as a {}", ram.name()), false);
            return;
        }
        let mut machine = Zx81::new(ram);
        machine.load_rom(&rom);
        machine.reset();
        // The ZX81 has its own mixer, on its own clock, so it needs its own
        // connection to the sound device — and the volume the user last set.
        if let Some(out) = &self.audio_out {
            machine.bus.audio.attach(out.queue.clone(), out.sample_rate);
        }
        machine.bus.audio.enabled = self.spec.bus.audio.enabled;
        machine.bus.audio.beeper_on = self.spec.bus.audio.beeper_on;
        machine.bus.audio.ay_on = self.spec.bus.audio.ay_on;
        machine.bus.audio.hardware_on = self.spec.bus.audio.hardware_on;
        machine.bus.audio.volume = self.spec.bus.audio.volume;
        self.zx81 = Some(machine);
        self.zx81_ram = ram;
        self.running = true;
        self.set_status(format!("Switched to a {}", ram.name()), false);
    }

    /// The picture as it was last drawn: RGBA, the size of [`App::view`].
    pub fn picture(&self) -> &[u8] {
        &self.screen_pixels
    }

    /// The machine as it stood `t` T-states into the frame being raced.
    ///
    /// The snapshot is taken the first time it is asked for, and again
    /// whenever the machine has moved since — stepping an instruction makes
    /// the frame that was being raced somebody else's.
    fn raced_to(&mut self, t: u32) -> &Spectrum {
        let stale = self
            .race
            .as_ref()
            .is_none_or(|race| !race.is_of(&self.spec));
        if stale {
            self.race = Some(crate::race::Race::start(&self.spec));
        }
        self.race.as_mut().expect("just made one").at(t)
    }

    /// Is the machine going slowly enough to watch the picture being drawn?
    ///
    /// The beam is shown and the picture is drawn as it is painted under the
    /// same condition, so the two always agree: a beam crawling over a picture
    /// that only changes when the frame ends would be telling a lie about
    /// where the machine is.
    pub fn crawling(&self) -> bool {
        self.speed < 1.0 || self.spec.bus.slow.enabled
    }

    /// How much border to draw.
    pub fn view(&self) -> screen::View {
        if self.overscan {
            screen::View::OVERSCAN
        } else {
            screen::View::CROPPED
        }
    }

    fn flash_on(&self) -> bool {
        (self.spec.bus.frame / 16) % 2 == 1
    }

    /// Advance the emulation by however much wall-clock time has passed.
    /// Run the machine for a slice of wall-clock time, as a frame of the UI
    /// would. Public so tests can stop it at a breakpoint without a window.
    /// Say so when the tape stops itself.
    ///
    /// A tape can carry a block that tells the deck to stop — the end of a
    /// part of a multi-load, where the program takes over and asks for the
    /// next part later. Nothing said so, and a deck that stops on its own part
    /// way through a tape looks exactly like a load that has gone wrong.
    /// Gauntlet III's does it half way through its first side.
    fn announce_tape_stops(&mut self) {
        let stopped = self
            .tape_ref()
            .is_some_and(|tape| tape.stopped_by_block && !tape.playing);
        if stopped && !self.said_tape_stopped {
            self.said_tape_stopped = true;
            self.set_status(
                "The tape asked the deck to stop — press Play for the next part".to_string(),
                false,
            );
        } else if !stopped {
            self.said_tape_stopped = false;
        }
    }

    pub fn advance(&mut self, dt: f32) {
        self.announce_tape_stops();
        // The deck is told how to behave rather than asked: a tape put in
        // after the sliders were set gets the same treatment as one already
        // in the machine.
        let quality = self.quality;
        if let Some(tape) = self.tape_mut() {
            tape.quality = quality;
        }
        if !self.running {
            return;
        }
        // Racing the beam is a way of looking at a stopped machine: it replays
        // one frame from its interrupt, and a machine that is running has
        // moved on to another frame before the cursor has been read.
        self.cursor_beam = false;
        self.race = None;
        if self.zx81.is_some() {
            // A ZX81 loads at about fifty bytes a second, so the boost matters
            // even more here than it does on a Spectrum.
            let boost = if self.tape_boost() && self.tape_is_loading() {
                MAX_SPEED
            } else {
                1.0
            };
            // Keep the sound buffer near its target depth, as for the
            // Spectrum, and mute when the speed is too far from normal.
            let target = self.audio_latency_target as f64;
            let pace = if self.speed == 1.0 && boost == 1.0 && self.sound_wanted() {
                self.audio().pace(target)
            } else {
                1.0
            };
            let effective = self.speed * boost;
            self.audio().speed_ok = (0.85..=1.2).contains(&effective);
            let zx = self.zx81.as_mut().expect("just checked");
            let dt = dt.clamp(0.0, 0.1);
            let want = zx81::CPU_HZ as f32 * dt * self.speed * boost * pace + self.leftover;
            let budget = want.max(0.0) as u64;
            self.leftover = want - budget as f32;
            // Cap the work per host frame, as for the Spectrum.
            let budget = budget.min(zx.frame_t() * 24);
            if let Some(pc) = zx.run(budget) {
                self.running = false;
                self.set_status(format!("Breakpoint at ${pc:04X}"), false);
            }
            return;
        }
        self.spec.bus.slow.begin_slice();

        let dt = dt.clamp(0.0, 0.1);
        // A recording is measured in frames of instructions rather than in
        // T-states, so it does its own running.
        if self.rzx.is_some() {
            let flat_out = self.rzx.as_ref().is_some_and(|rzx| rzx.max_speed);
            self.spec.bus.audio.speed_ok = !flat_out && (0.85..=1.2).contains(&self.speed);
            self.advance_recording(dt, 1.0);
            self.spec.bus.audio_sync();
            self.spec.bus.audio.flush();
            return;
        }
        // Loading a real tape takes minutes; run as fast as the emulator will
        // go while it moves.
        let boost = if self.tape_boost() && self.tape_is_loading() {
            MAX_SPEED
        } else {
            1.0
        };
        // Nudge the amount of work to keep the sound buffer near its target
        // depth, so it neither runs dry nor backs up.
        let pace = if self.speed == 1.0 && self.sound_wanted() {
            self.spec.bus.audio.pace(self.audio_latency_target as f64)
        } else {
            1.0
        };
        // The ULA's clock is what the budget is in: a faster CPU does more
        // inside the same T-states rather than being given more of them.
        self.spec.bus.turbo = self.turbo();
        let want = self.machine_cpu_hz() as f32 * dt * self.speed * boost * pace + self.leftover;
        let budget = want.max(0.0) as u32;
        self.leftover = want - budget as f32;

        // Sound only makes sense near real time; muting keeps fast-forward
        // from shrieking.
        let effective = self.speed * boost;
        self.spec.bus.audio.speed_ok = (0.85..=1.2).contains(&effective);

        // Cap the work per host frame so "Max" speed cannot lock up the UI.
        //
        // Fastload lifts the cap while a tape is moving, and keeps the
        // window answering by watching the clock instead: a game with a loader
        // of its own reads the tape itself, and the only thing that gets it
        // loaded quickly is letting the machine run. Twenty-four frames of
        // work a host frame is about twelve seconds of waiting for a Speedlock
        // tape; a tenth of a second of real work a host frame gets it down to
        // about one.
        // Right to the end of the tape, silence included. Max speed comes back
        // to normal for the pause the tape ends on, so that a loader finishing
        // sounds and looks as it should; Fastload is a promise to get
        // it over with, and Out Run Europa ends with twenty-two seconds of
        // silence that nothing is waiting for.
        let flat_out = self.spec.bus.tape_flash && self.tape_ref().is_some_and(|tape| tape.playing);
        // In a hurry the budget is a whole slice of work rather than whatever
        // the speed setting asked for: at the end of a tape the boost above is
        // off — Max speed comes back to normal for the last pause — and the
        // budget it leaves is a fraction of a frame.
        let budget = if flat_out {
            self.spec.bus.frame_t() * FASTLOAD_FRAMES
        } else {
            budget.min(self.spec.bus.frame_t() * 24)
        };
        self.rewind.clear();
        let started = std::time::Instant::now();
        // A network playing chooses between frames, so the machine is run a
        // frame at a time for it rather than in one slice of up to 24.
        let stop = match self.training.player.as_mut() {
            Some(player) => {
                let mut left = budget;
                loop {
                    player.play(&mut self.spec);
                    let slice = left.min(self.spec.bus.frame_t());
                    let stop = self.spec.run(slice);
                    left -= slice;
                    if left == 0 || !matches!(stop, Stop::Budget) {
                        break stop;
                    }
                }
            }
            None => self.spec.run(budget),
        };
        self.last_stop = Some(stop);
        if flat_out {
            // However much is left of the budget, stop when the host frame is
            // spent: the picture and the buttons still have to happen.
            while started.elapsed() < FASTLOAD_SLICE
                && matches!(self.last_stop, Some(Stop::Budget))
                && self.tape_ref().is_some_and(|tape| tape.playing)
            {
                let stop = self.spec.run(self.spec.bus.frame_t() * 24);
                self.last_stop = Some(stop);
            }
        }
        let stop = self.last_stop.expect("just set");
        self.handle_stop(stop);
        self.spec.bus.audio_sync();
        self.spec.bus.audio.flush();
    }

    /// What to do about the machine having stopped.
    fn handle_stop(&mut self, stop: Stop) {
        match stop {
            Stop::Breakpoint(pc) => {
                self.running = false;
                self.status = format!("Breakpoint at ${pc:04X}");
                self.dbg.follow_pc = true;
                // Stopping somewhere is only useful if you can see where: the
                // debugger is opened if it was closed and asks for the front
                // on the next frame it draws.
                self.show_debugger = true;
                self.dbg.raise = true;
            }
            Stop::Watched(event, at) => {
                self.running = false;
                self.status = format!("{} (PC ${at:04X})", event.describe());
                // The listing is put on the instruction that did it rather
                // than left following PC, which by then is the one after.
                self.show_in_listing(at);
                self.show_debugger = true;
                self.dbg.raise = true;
            }
            Stop::SlowDraw => {
                self.leftover = 0.0;
            }
            _ => {}
        }
    }

    /// The picture as it stands, for anything that wants to show it: the
    /// debugger keeps a small copy beside the registers.
    pub fn screen_texture(&self) -> Option<TextureHandle> {
        self.screen_tex.clone()
    }

    /// How the picture is sampled when it is drawn.
    ///
    /// Nearest, so a pixel is a hard square, unless the set is on: a tube has
    /// no pixel edges, and drawing one through a nearest sample beats against
    /// the screen it is being shown on. That beat is where the moiré came
    /// from — the line gaps and the herringbone are both about a pixel wide,
    /// and a picture scaled by anything but a whole number samples some of
    /// them twice and some not at all.
    pub fn picture_filter(&self) -> TextureOptions {
        if self.crt {
            TextureOptions::LINEAR
        } else {
            TextureOptions::NEAREST
        }
    }

    /// What the two switches ask of the picture.
    ///
    /// The tube gives every line a gap under it, which is what doubles the
    /// height; the aerial lead gives the colour its smear and the picture its
    /// herringbone. A monitor fed RGB had the first and none of the second, so
    /// either can be had without the other.
    pub fn crt_settings(&self) -> crate::crt::Crt {
        let plain = crate::crt::Crt::default();
        // The lead only with the set: the switch is disabled without it, and
        // what it was left set to is kept for when the set comes back on.
        let composite = self.crt && self.composite;
        crate::crt::Crt {
            interference: if composite { plain.interference } else { 0.0 },
            bleed: if composite { plain.bleed } else { 0.0 },
            // Only where there is room to draw them. A gap under every line
            // needs two rows of screen for every row of picture, and asking
            // for them at a zoom that has not got two is asking for a moiré:
            // some gaps drawn, some not, in a pattern of their own.
            scanlines: if self.crt && self.scale >= 2.0 {
                plain.scanlines
            } else {
                0.0
            },
            line_gaps: self.crt && self.scale >= 2.0,
        }
    }

    fn draw_screen_texture(&mut self, ctx: &egui::Context) {
        if self.zx81.is_some() {
            let view = if self.overscan {
                zx81::View::OVERSCAN
            } else {
                zx81::View::CROPPED
            };
            if self.screen_pixels.len() != view.buffer_len() {
                self.screen_pixels = vec![0; view.buffer_len()];
                self.screen_tex = None;
            }
            if let Some(zx) = self.zx81.as_ref() {
                zx.bus.render(view, &mut self.screen_pixels);
            }
            let img = ColorImage::from_rgba_unmultiplied([view.w, view.h], &self.screen_pixels);
            match &mut self.screen_tex {
                Some(t) => t.set(img, TextureOptions::NEAREST),
                None => {
                    self.screen_tex =
                        Some(ctx.load_texture("zx81-screen", img, TextureOptions::NEAREST))
                }
            }
            return;
        }
        let view = self.view();
        let flash = self.flash_on();
        let gaps = self.crt_settings().line_gaps;
        if self.screen_pixels.len() != view.buffer_len() || gaps != self.crt_drawn {
            self.screen_pixels = vec![0; view.buffer_len()];
            // The texture has to be remade at the new size, and switching the
            // set on or off doubles the height or halves it again.
            self.screen_tex = None;
            self.crt_drawn = gaps;
        }
        // Bring the painted frame up to where the beam has got to. It is
        // otherwise only caught up when the program writes to the screen, so a
        // machine stopped between two writes would show the last of them as
        // the beam's position rather than where the beam actually is.
        self.spec.bus.catch_up_painting();
        // Racing replays a frame rather than reading the machine, so the
        // pixels are borrowed out of the way of the copy being run.
        let mut pixels = std::mem::take(&mut self.screen_pixels);
        match self.beam_t.filter(|_| self.cursor_beam && !self.running) {
            Some(beam) => {
                let raced = self.raced_to(beam);
                screen::render_racing(&raced.bus, view, &mut pixels, flash, beam);
            }
            // While the machine is crawling, the picture is the one being
            // painted, so that it builds under the beam rather than sitting
            // still until the frame ends.
            // Racing: the frame being painted, fading behind the beam, with
            // the writes since marked by which side of the beam they landed.
            None if self.racing => {
                let fade = screen::Fade {
                    now: self.spec.bus.tstates,
                    floor: self.fade_floor,
                };
                // The user is given the blend in seconds of their own time;
                // how much of the machine's time that is depends on how
                // slowly it is being run.
                let over = (TINT_SECONDS * self.speed * self.spec.bus.model.cpu_hz() as f32) as u64;
                let tint = self.spec.bus.tints.as_ref().map(|tints| screen::Tinting {
                    tints,
                    now: self.spec.bus.total_t(),
                    over,
                });
                screen::render_fading(&self.spec.bus, view, &mut pixels, flash, fade, tint);
            }
            None if self.crawling() => {
                screen::render_painting(&self.spec.bus, view, &mut pixels, flash)
            }
            None => screen::render(&self.spec.bus, view, &mut pixels, flash),
        }
        self.screen_pixels = pixels;
        let img = if self.crt {
            // The tube gives every line a gap under it, which is what doubles
            // the height; the aerial lead gives the colour its smear and the
            // picture its herringbone. Either can be had without the other.
            let set = self.crt_settings();
            let frame = self.spec.bus.frame;
            // The dot the view starts at keeps the interference still against
            // the picture when the border is shown or hidden — the pattern
            // belongs to the machine's clock, not to the window's edge.
            let x0 = -((view.border_x * 2) as f64);
            crate::crt::televise(
                &self.screen_pixels,
                view.width(),
                view.height(),
                &mut self.crt_pixels,
                set,
                frame,
                x0,
            );
            let rows = if set.line_gaps { 2 } else { 1 };
            ColorImage::from_rgba_unmultiplied(
                [view.width(), view.height() * rows],
                &self.crt_pixels,
            )
        } else {
            ColorImage::from_rgba_unmultiplied([view.width(), view.height()], &self.screen_pixels)
        };
        self.record_video_frame();
        let filter = self.picture_filter();
        match &mut self.screen_tex {
            Some(t) => t.set(img, filter),
            None => self.screen_tex = Some(ctx.load_texture("spectrum-screen", img, filter)),
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        // A wrapping row rather than a menu bar: a bar does not wrap, and with
        // only one menu left on it the rest of the row would be clipped off
        // the edge of a narrow window.
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y + 8.0);
        let layout = egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true);
        ui.allocate_ui_with_layout(row, layout, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Load ROM…").clicked() {
                    if let Some(path) = self.pick_file(Some(FileKind::Rom)) {
                        self.load_path(&path);
                    }
                    ui.close();
                }
                if ui.button("Load tape…").clicked() {
                    if let Some(path) = self.pick_file(Some(FileKind::Tape)) {
                        self.load_path(&path);
                    }
                    ui.close();
                }
                if ui.button("Load disk…").clicked() {
                    if let Some(path) = self.pick_file(Some(FileKind::Disk)) {
                        self.load_path(&path);
                    }
                    ui.close();
                }
                if ui.button("Load cartridge…").clicked() {
                    if let Some(path) = self.pick_file(Some(FileKind::Cartridge)) {
                        self.load_path(&path);
                    }
                    ui.close();
                }
                if ui.button("Create blank disk…").clicked() {
                    self.create_blank_disk();
                    ui.close();
                }
                if ui.button("Load recording…").clicked() {
                    if let Some(path) = self.pick_file(Some(FileKind::Recording)) {
                        self.load_path(&path);
                    }
                    ui.close();
                }
                if ui.button("Load snapshot…").clicked() {
                    if let Some(path) = self.pick_file(Some(FileKind::Snapshot)) {
                        match snapshot::probe_model(&path) {
                            Ok(model) => {
                                self.switch_model(model);
                                match snapshot::load(&mut self.spec, &path) {
                                    Ok(()) => self.status = format!("Loaded {}", path.display()),
                                    Err(e) => {
                                        self.set_status(format!("Snapshot load failed: {e}"), true)
                                    }
                                }
                            }
                            Err(e) => self.set_status(format!("Snapshot load failed: {e}"), true),
                        }
                    }
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_enabled(!self.on_zx81(), egui::Button::new("Save snapshot…"))
                    .on_hover_text(
                        "Write the machine as it stands, to be loaded back later or \
                         carried to another emulator. Name it .szx to keep the paging, \
                         the AY and the joystick with it; .sna is the old 48K dump.",
                    )
                    .clicked()
                {
                    self.save_snapshot();
                    ui.close();
                }
                if ui
                    .add_enabled(!self.on_zx81(), egui::Button::new("Save screen…"))
                    .on_hover_text(
                        "Write what is on the screen to a .scr: the 6,912 bytes the ULA \
                         draws from, which is what a Spectrum screenshot is.",
                    )
                    .clicked()
                {
                    self.save_screen();
                    ui.close();
                }
                ui.separator();
                if ui.button("Reset").clicked() {
                    self.reset_machine();
                    self.set_status(self.starting_up(), false);
                    ui.close();
                }
            });
            theme::divider(ui);

            if theme::run_pause_button(ui, self.running).clicked() {
                self.running = !self.running;
            }

            theme::divider(ui);
            theme::group_label(ui, "Machine");
            self.machine_dropdown(ui);
            theme::group_label(ui, "CPU");
            let base = self.machine_cpu_hz();
            let before = self.clock_mult;
            let held = self.turbo_held_because();
            clock_dropdown(&mut self.clock_mult, base, ui);
            self.speed_control(ui);
            if let Some(why) = held {
                ui.label(
                    egui::RichText::new(format!("at {:.2}MHz — {why}", base / 1e6))
                        .small()
                        .color(theme::DIM),
                );
            }
            if self.clock_mult != before {
                // The mixer counts in T-states, so it has to be told: a
                // beeper note would otherwise come out at the pitch the
                // machine was built for rather than the one it is running
                // at.
                self.apply_clock();
            }
            self.late_timing(ui);
            if ui.button("Reset").clicked() {
                self.reset_machine();
                if self.on_zx81() {
                    let name = self.zx81_ram.name();
                    self.set_status(format!("Reset ({name})"), false);
                } else {
                    let what = self.starting_up();
                    self.set_status(what, false);
                }
            }

            theme::divider(ui);
            theme::group_label(ui, "Record");
            self.rzx_button(ui);
            self.video_button(ui);

            theme::divider(ui);
            theme::group_label(ui, "Quick");
            self.quick_buttons(ui);
        });
    }

    /// Load and Save for the ten quicksaves, and the slot they work on.
    fn quick_buttons(&mut self, ui: &mut egui::Ui) {
        let zx81 = self.on_zx81();
        let recording = self.rzx.is_some() || self.recorded_frames().is_some();
        let filled = self.quick[self.quick_slot].is_some();
        if ui
            .add_enabled(!zx81 && !recording && filled, egui::Button::new("Load"))
            .on_hover_text(if zx81 {
                "Quicksaves are of a Spectrum: the ZX81 cannot be copied yet.".to_string()
            } else if recording {
                "Not while a recording is playing or being made: it would stop \
                 describing the machine it plays into."
                    .to_string()
            } else if filled {
                format!(
                    "Put the machine back as it was in slot {}.",
                    self.quick_slot
                )
            } else {
                format!(
                    "Slot {} is empty. Save puts something in it, or F{} from anywhere.",
                    self.quick_slot,
                    if self.quick_slot == 0 {
                        10
                    } else {
                        self.quick_slot
                    }
                )
            })
            .clicked()
        {
            self.quick_load();
        }
        if ui
            .add_enabled(!zx81, egui::Button::new("Save"))
            .on_hover_text(format!(
                "Keep the machine as it is now in slot {}, in memory, until the emulator \
                 is closed. F1 to F9 save to slots 1 to 9 and F10 to slot 0.",
                self.quick_slot
            ))
            .clicked()
        {
            self.quick_save(self.quick_slot);
        }
        let mut slot = self.quick_slot;
        theme::dropdown(ui, 34.0, slot.to_string(), |ui| {
            for n in 0..10 {
                // A filled slot says so, so the one with something in it can
                // be found without loading each in turn.
                let label = if self.quick[n].is_some() {
                    format!("{n} •")
                } else {
                    n.to_string()
                };
                if ui.selectable_label(slot == n, label).clicked() {
                    slot = n;
                    ui.close();
                }
            }
        });
        self.quick_slot = slot;
    }

    /// Keep the machine as it is in a slot, and make that slot the one Load
    /// restores.
    ///
    /// A quicksave is a copy of the whole machine rather than a snapshot file:
    /// everything on the back of it, the tape where it had got to, the chips
    /// mid-note — none of which a .sna can hold and not all of which a .szx
    /// can. The copy is cut off from the sound card, since a copy that shares
    /// the queue would play into it.
    pub fn quick_save(&mut self, slot: usize) {
        let slot = slot % 10;
        self.quick_slot = slot;
        if self.on_zx81() {
            self.set_status(
                "Quicksaves are of a Spectrum: the ZX81 cannot be copied yet.".to_string(),
                true,
            );
            return;
        }
        let mut copy = Box::new(self.spec.clone());
        copy.bus.audio.detach();
        self.quick[slot] = Some(copy);
        self.set_status(format!("Saved to quick slot {slot}"), false);
    }

    /// Put the machine back as it was in the selected slot.
    pub fn quick_load(&mut self) {
        let slot = self.quick_slot;
        if self.on_zx81() {
            self.set_status(
                "Quicksaves are of a Spectrum: the ZX81 cannot be copied yet.".to_string(),
                true,
            );
            return;
        }
        if self.rzx.is_some() || self.recorded_frames().is_some() {
            self.set_status(
                "Not while a recording is playing or being made: it would stop describing \
                 the machine it plays into."
                    .to_string(),
                true,
            );
            return;
        }
        let Some(saved) = self.quick[slot].as_ref() else {
            self.set_status(format!("Quick slot {slot} is empty"), true);
            return;
        };
        let mut restored = (**saved).clone();
        restored
            .bus
            .audio
            .take_output_from(&mut self.spec.bus.audio);
        self.spec = restored;
        // The time the emulator owes the machine belongs to the machine that
        // has just gone.
        self.leftover = 0.0;
        self.set_status(format!("Loaded quick slot {slot}"), false);
    }

    /// Recording what the machine reads, so a run of a game can be played back
    /// and read instruction by instruction.
    ///
    /// Not offered while a recording is playing: what would be captured is the
    /// recording.
    fn rzx_button(&mut self, ui: &mut egui::Ui) {
        match self.recorded_frames() {
            None => {
                if ui
                    .add_enabled(self.rzx.is_none(), egui::Button::new("⏺ RZX"))
                    .on_hover_text(
                        "Record everything the machine reads from now on, so it \
                         can be played back and read instruction by instruction. \
                         It is kept in memory until you stop.",
                    )
                    .clicked()
                {
                    self.start_recording();
                }
            }
            Some(frames) => {
                if ui
                    .button("⏹ Stop RZX")
                    .on_hover_text("Stop recording and write it out")
                    .clicked()
                {
                    self.save_recording();
                }
                ui.label(
                    egui::RichText::new(format!("● {frames} frames"))
                        .color(theme::RED)
                        .monospace(),
                );
            }
        }
    }

    /// Start writing the picture to a file, having asked where to put it.
    ///
    /// The size is whatever the picture is at the moment — the view, and
    /// whether the set is on, decide it — and it cannot change while the file
    /// is being written, so the frame that is refused is the one that says the
    /// window was changed part way through.
    pub fn start_video(&mut self) {
        if !crate::video_out::available() {
            self.set_status(
                format!(
                    "No {} on the path: it is what writes the file.",
                    crate::video_out::FFMPEG
                ),
                true,
            );
            return;
        }
        let Some(path) = self.pick_video_path() else {
            return;
        };
        let (w, h, pixel_aspect, fps, smooth) = self.video_settings();
        let rate = self.spec.bus.audio.sample_rate;
        match crate::video_out::Recording::start(path, w, h, pixel_aspect, fps, smooth, rate) {
            Ok(recording) => {
                // The sound is kept from now on, and thrown away again when
                // the recording stops.
                self.spec.bus.audio.tap = Some(Vec::new());
                self.video_due = 0.0;
                self.video_at = self.spec.bus.frame;
                self.set_status(
                    format!("Recording video to {}", recording.path.display()),
                    false,
                );
                self.video = Some(recording);
            }
            Err(e) => self.set_status(e, true),
        }
    }

    /// Whether the switches that change the picture's *size* are held while a
    /// video is being written.
    ///
    /// The encoder is told the frame size once, when the pipe opens, and reads
    /// a headerless stream of raw bytes: it slices frames out of it by that
    /// number alone, so a frame of another size shears the picture from there
    /// on. The refusal in `video_out` catches that and stops the recording,
    /// which is better than a ruined file — but a switch that stops the
    /// recording is a worse thing to offer than one that waits.
    ///
    /// The three that change the size are the tube, which gives every line a
    /// gap under it; the overscan, which is a different amount of border; and
    /// the zoom, since the gaps are only drawn from 2× up. Composite is not
    /// one of them — it changes what the pixels are, not how many — so it
    /// stays live and can be switched while the film is running.
    pub fn video_switches_held(&self) -> bool {
        self.video.is_some()
    }

    /// What the encoder is told about the picture: its size, how tall a row
    /// stands for against how wide a column does, the machine's frame rate,
    /// and whether to scale it smoothly.
    ///
    /// With the set on, every line of the picture is two rows of the buffer —
    /// a line and the gap under it — so a row stands for half as much height
    /// as a column does width. Written as though those rows were square, the
    /// televised picture went into the file twice as tall as it should be.
    pub fn video_settings(&self) -> (usize, usize, f64, f64, bool) {
        let (w, h) = self.picture_size();
        (
            w,
            h,
            if self.crt_settings().line_gaps {
                0.5
            } else {
                1.0
            },
            self.machine_cpu_hz() / self.spec.bus.frame_t() as f64,
            self.picture_filter() == TextureOptions::LINEAR,
        )
    }

    /// Where the video goes: beside the tape if there is one, under its name.
    fn pick_video_path(&mut self) -> Option<std::path::PathBuf> {
        let suggested = self
            .tape_path
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("recording"))
            .with_extension("mp4");
        let dialog = rfd::FileDialog::new().set_file_name(
            suggested
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "recording.mp4".to_string()),
        );
        let directory = suggested
            .parent()
            .filter(|parent| parent.is_dir())
            .map(std::path::Path::to_path_buf)
            .or_else(|| self.prefs.dir_for(FileKind::Recording).cloned())
            .filter(|dir| dir.is_dir());
        let dialog = match directory {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        };
        dialog.save_file()
    }

    /// How big the picture is at the moment, in pixels of the buffer that is
    /// written: the set doubles the height, since every line has a gap.
    pub fn picture_size(&self) -> (usize, usize) {
        let (w, h) = match &self.zx81 {
            Some(_) if self.overscan => (zx81::View::OVERSCAN.w, zx81::View::OVERSCAN.h),
            Some(_) => (zx81::View::CROPPED.w, zx81::View::CROPPED.h),
            None => {
                let view = self.view();
                (view.width(), view.height())
            }
        };
        let rows = if self.crt_settings().line_gaps { 2 } else { 1 };
        (w, h * rows)
    }

    /// Close the file and say what was written.
    pub fn stop_video(&mut self) {
        self.spec.bus.audio.tap = None;
        let Some(mut recording) = self.video.take() else {
            return;
        };
        match recording.finish() {
            Ok((path, frames)) => {
                self.prefs.remember_file(FileKind::Recording, &path);
                self.set_status(format!("Wrote {} ({frames} frames)", path.display()), false);
            }
            Err(e) => self.set_status(format!("Video recording failed: {e}"), true),
        }
    }

    /// Hand the frame just drawn to the encoder, if one is running.
    ///
    /// The buffer that becomes the texture, so what goes into the file is what
    /// the window shows: the line structure, the composite colour and the dot
    /// crawl are already in these pixels.
    /// How many frames of video the picture just drawn is worth.
    ///
    /// The window repaints when the window system says so — sixty times a
    /// second here, and not at all while it is behind another window — and the
    /// machine draws fifty. A file written a frame per repaint has the wrong
    /// number of frames in it and plays back at the wrong speed; this counts
    /// the machine's frames instead, so a second of the file is a second of the
    /// machine. Capped, because a machine being run flat out draws thousands
    /// and the file is of what the window showed.
    pub fn video_frames_owed(due: f64) -> u32 {
        (due as u32).min(4)
    }

    fn record_video_frame(&mut self) {
        if self.video.is_none() {
            return;
        }
        // How many frames the machine has drawn since the last time the
        // picture was written out. Not one per repaint: the window repaints
        // when the window system says so and the machine draws fifty times a
        // second, and a file with sixty frames for every fifty plays back a
        // fifth too slow.
        let now = self.spec.bus.frame;
        self.video_due += now.saturating_sub(self.video_at) as f64;
        self.video_at = now;
        // A machine being run flat out draws thousands; the file is of what
        // the window showed, so it gets what the window showed.
        let write = Self::video_frames_owed(self.video_due);
        if write == 0 {
            return;
        }
        self.video_due -= write as f64;

        let televised = self.crt || self.composite;
        let (w, h) = self.picture_size();
        let sound: Vec<f32> = self
            .spec
            .bus
            .audio
            .tap
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        let failed = {
            let pixels = if televised {
                &self.crt_pixels
            } else {
                &self.screen_pixels
            };
            let recording = self.video.as_mut().expect("checked above");
            for _ in 0..write {
                recording.frame(pixels, w, h);
            }
            recording.sound(&sound);
            recording.failed.clone()
        };
        if let Some(why) = failed {
            self.set_status(format!("Video recording stopped: {why}"), true);
            self.stop_video();
        }
    }

    /// Recording the picture itself, as a video file.
    fn video_button(&mut self, ui: &mut egui::Ui) {
        match &self.video {
            None => {
                if ui
                    .button("⏺ Video")
                    .on_hover_text(
                        "Write the picture to an H.264 file, as the window shows \
                         it: the line structure, the composite colour and the dot \
                         crawl go into the file with it. Needs ffmpeg on the \
                         path. The curve of the glass does not: that is the shape \
                         the picture is drawn on rather than something done to \
                         the pixels.",
                    )
                    .clicked()
                {
                    self.start_video();
                }
            }
            Some(recording) => {
                let frames = recording.frames;
                if ui
                    .button("⏹ Stop video")
                    .on_hover_text("Stop recording and close the file")
                    .clicked()
                {
                    self.stop_video();
                }
                ui.label(
                    egui::RichText::new(format!("● {frames} frames"))
                        .color(theme::RED)
                        .monospace(),
                );
            }
        }
    }

    /// Zoom and the ways of watching the picture, on a line of their own: the
    /// machine row is long enough without them, and these belong together.
    fn video_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            // Claim the height of the tallest thing in the row before
            // anything is placed. egui centres each control in the row as it
            // stands when the control is added, so a slider half way along an
            // otherwise short row leaves everything before it sitting three
            // pixels higher than everything after it.
            ui.set_min_height(theme::ROW_H);
            theme::group_label(ui, "Zoom");
            let held = self.video_switches_held();
            ui.add_enabled_ui(!held, |ui| {
                zoom_dropdown(&mut self.scale, ui);
            })
            .response
            .on_disabled_hover_text(HELD_WHILE_RECORDING);

            theme::divider(ui);
            theme::group_label(ui, "Video");

            // Five seconds a frame, with the picture fading behind the beam:
            // one frame's drawing, slowed down until it can be read.
            if theme::toggle(ui, &mut self.racing, "Race the Beam")
                .on_hover_text(
                    "Run at five seconds a frame and watch the beam go down the \
                     screen. The picture fades behind it, so how long ago each \
                     part was drawn is visible at a glance.",
                )
                .changed()
            {
                self.set_racing(self.racing);
            }
            ui.add_enabled_ui(self.racing, |ui| {
                theme::slider(
                    ui,
                    egui::Slider::new(&mut self.fade_floor, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                        .text("fades to"),
                )
                .on_hover_text(
                    "How much of its brightness a pixel is left with by the \
                     time the beam comes round to draw it again.",
                );
            });

            theme::divider(ui);
            ui.add_enabled_ui(!held, |ui| {
                theme::toggle(ui, &mut self.crt, "CRT").on_hover_text(
                    "Show the picture on a tube: the curve of the glass, and a \
                     gap under every line.",
                );
            })
            .response
            .on_disabled_hover_text(HELD_WHILE_RECORDING);
            // Only with the tube: composite video is how the picture reached a
            // television, and a television is what the other switch is. There
            // is no picture that arrives down an aerial lead and is then shown
            // on something that is not a set.
            ui.add_enabled_ui(self.crt, |ui| {
                theme::toggle(ui, &mut self.composite, "Composite").on_hover_text(
                    "Take the picture down an aerial lead: colour smeared sideways \
                 by the subcarrier it rides on, and the herringbone that \
                 subcarrier beats against the machine's dot clock. A monitor \
                 fed RGB had neither.",
                );
            });

            theme::divider(ui);
            // Only while stopped: the frame is replayed from its interrupt,
            // which means being able to hold the machine still and run a copy
            // of it instead.
            ui.add_enabled_ui(!self.running, |ui| {
                theme::toggle(ui, &mut self.cursor_beam, "Cursor Beam").on_hover_text(
                    "Replay the next frame from its interrupt. Hover the picture and \
                     everything above the cursor is the screen as the machine had it \
                     by the time the beam reached that point — every instruction up to \
                     there executed, and no more. Below it is what the display file \
                     holds at that moment, dimmed, since the ULA has not put it out \
                     yet. Stopped machines only.",
                );
            });
            ui.add_enabled_ui(!held, |ui| {
                theme::toggle(ui, &mut self.overscan, "Overscan").on_hover_text(
                    "Show the whole border the ULA draws, not just a \
                     television's worth.",
                );
            })
            .response
            .on_disabled_hover_text(HELD_WHILE_RECORDING);

            // One whole picture at a time, for watching a game draw itself.
            // Only while it is stopped: a running machine is already doing
            // this fifty times a second.
            if ui
                .add_enabled(!self.running, egui::Button::new("Next frame"))
                .on_hover_text(
                    "Run to the end of the frame the machine is in the middle \
                     of, so the picture on screen is one the ULA has finished \
                     painting. Only while it is paused.",
                )
                .clicked()
            {
                self.next_frame();
            }
        });
    }

    /// Switch the slowed-down, fading picture on or off.
    ///
    /// Switching it on takes the speed down to five seconds a frame and starts
    /// the machine — there is nothing to watch otherwise — and switching it
    /// off puts the speed back where it was.
    pub fn set_racing(&mut self, on: bool) {
        // Marking every write costs a branch on the busiest path in the
        // emulator, so it is only done while somebody is looking at the marks.
        self.spec.bus.tints = on.then(machine::Tints::default);
        if on {
            self.speed_before_race = self.speed;
            self.speed = RACE_SPEED;
            self.running = true;
        } else {
            self.speed = self.speed_before_race;
        }
        self.racing = on;
    }

    /// How fast the machine is run, beside the clock it is run at.
    fn speed_control(&mut self, ui: &mut egui::Ui) {
        // Choosing a speed by hand is choosing not to race.
        speed_dropdown(&mut self.speed, ui);
        if self.racing && self.speed != RACE_SPEED {
            self.racing = false;
        }
    }

    /// The 48K's two timings, which only it has.
    fn late_timing(&mut self, ui: &mut egui::Ui) {
        if self.on_zx81() || self.spec.bus.model != Model::Spectrum48 {
            return;
        }
        let mut late = self.spec.bus.late_timing;
        if theme::toggle(ui, &mut late, "Late")
            .on_hover_text(
                "Late timing. Later 48K machines run the display one T-state later \
                 relative to the interrupt. HALT2INT tells them apart.",
            )
            .changed()
        {
            self.spec.bus.set_late_timing(late);
            self.set_status(
                format!("48K {} timing", if late { "late" } else { "early" }),
                false,
            );
        }

        // Which bits of a write to $FE the EAR input hears back. A tape
        // protection that listens for the line to be alive hears the MIC bit
        // on an issue 2 board and nothing on an issue 3 one.
        let mut issue2 = self.spec.bus.issue2;
        if theme::toggle(ui, &mut issue2, "I2")
            .on_hover_text(
                "Issue 2 board. The EAR input hears the machine's own loudspeaker. An issue 2 \
                 board hears the MIC bit as well as the speaker's, which is what \
                 a tape protection listening for a live line expects — Head over \
                 Heels needs it. Switch it off for a strict issue 3.",
            )
            .changed()
        {
            self.spec.bus.issue2 = issue2;
            self.set_status(
                format!("48K issue {}", if issue2 { "2" } else { "3" }),
                false,
            );
        }
    }

    fn controls_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            theme::toggle(ui, &mut self.spec.bus.slow.enabled, "Slow draw")
                .on_hover_text(
                    "Park the CPU after a set number of writes to watched video memory \
                     so the picture visibly builds up over several host frames.",
                );
            ui.add_enabled_ui(self.spec.bus.slow.enabled, |ui| {
                theme::slider(
                    ui,
                    egui::Slider::new(&mut self.spec.bus.slow.writes_per_slice, 1..=4096)
                        .logarithmic(true)
                        .text("writes/frame"),
                );
                theme::toggle(ui, &mut self.spec.bus.slow.watch_screen, "video RAM");
                theme::toggle(ui, &mut self.spec.bus.slow.watch_back_buffer, "back buffer");
            });
            ui.separator();
            if let Some(rzx) = &self.rzx {
                let (frame, total) = (rzx.frame, rzx.recording.len());
                let mut max_speed = rzx.max_speed;
                let short = self.spec.bus.playback.as_ref().map_or(0, |p| p.short);
                let percent = if total == 0 {
                    0.0
                } else {
                    frame as f32 * 100.0 / total as f32
                };

                // Said plainly: the machine is not taking orders from the
                // keyboard at the moment, and it should be obvious why.
                ui.label(
                    egui::RichText::new("⏵ REPLAY")
                        .strong()
                        .color(theme::AMBER),
                )
                .on_hover_text("Playing back a recording. The keyboard is the recording's.");
                ui.label(
                    egui::RichText::new(format!("{frame}/{total}  {percent:.0}%"))
                        .monospace()
                        .color(theme::LCD_FG),
                );

                if ui
                    .toggle_value(&mut max_speed, "Max speed")
                    .on_hover_text(
                        "Run the recording as fast as this machine can rather \
                         than at the speed it was played. Twenty minutes of \
                         play is a long wait for the part you want to see.",
                    )
                    .changed()
                {
                    if let Some(rzx) = &mut self.rzx {
                        rzx.max_speed = max_speed;
                        rzx.owed = 0.0;
                    }
                }

                // A recording that asks for more input than was recorded has
                // come adrift from the machine: what is on screen after that
                // is the emulator's guess, not what was played, and saying so
                // is better than letting it look authentic.
                if short > 0 {
                    ui.label(egui::RichText::new(format!("out of step ({short})")).color(theme::RED))
                        .on_hover_text(
                            "The program has read more from the ports than the \
                             recording holds, so it is no longer following the \
                             path it was recorded taking.",
                        );
                }
                if ui.button("Stop").clicked() {
                    self.stop_playback();
                    self.set_status("Stopped the recording".into(), false);
                }
                theme::divider(ui);
            }
            if self.tape_ref().is_some() {
                let playing = self.tape_is_playing();
                if ui
                    .button(if playing { "⏸ Tape" } else { "▶ Tape" })
                    .clicked()
                {
                    let now = self.machine_t();
                    let t = self.tape_mut().unwrap();
                    if playing {
                        t.stop();
                    } else {
                        t.play(now);
                    }
                }
                ui.separator();
            }
            // Everything about the sound lives here: what the top menu used to
            // hold as well, since it was the same two controls twice.
            let sound = match (&self.audio_out, &self.audio_error) {
                (Some(out), _) => format!(
                    "{} @ {} Hz\nbuffer {} samples ({:.0} ms), {} dropped",
                    out.device_name,
                    out.sample_rate,
                    self.spec.bus.audio.queue_len(),
                    self.spec.bus.audio.latency() * 1000.0,
                    self.spec.bus.audio.dropped
                ),
                (None, Some(e)) => e.clone(),
                (None, None) => "no audio device".to_string(),
            };
            let failed = self.audio_out.is_none();
            ui.label(if failed { "🔇" } else { "🔊" })
                .on_hover_text(&sound);
            // Three switches rather than one, because the machine has three
            // sorts of sound in it and they are not equally wanted: the AY
            // over a beeper that is only clicking, or the SpecDrum on its own.
            theme::toggle(ui, &mut self.audio().beeper_on, "Beeper")
                .on_hover_text("The machine's own speaker, and the tape's hiss with it.");
            // Only where there is one: a 48K has no AY unless something was
            // plugged into it, and a ZX81 has nothing at all.
            let has_ay = self.spec.bus.model.has_ay() || self.spec.bus.audio.extra_ay.is_some();
            if has_ay && !self.on_zx81() {
                theme::toggle(ui, &mut self.audio().ay_on, "AY").on_hover_text(
                    "The sound chip: the 128K's, and a Fuller Box's if one is fitted.",
                );
            }
            theme::toggle(ui, &mut self.audio().hardware_on, "Hardware")
                .on_hover_text("What the add-ons make — the SpecDrum's converter.");
            theme::slider(
                ui,
                egui::Slider::new(&mut self.audio().volume, 0.0..=1.0)
                    .show_value(false)
                    .text("vol"),
            );
            theme::toggle(ui, &mut self.spec.bus.audio.mute_off_speed, "auto-mute")
                .on_hover_text("Silence the sound unless the machine is running at about normal speed, so fast-forwarding does not shriek.");
            theme::slider(
                ui,
                egui::Slider::new(&mut self.audio_latency_target, 0.02..=0.25)
                    .show_value(false)
                    .text("buffer"),
            )
            .on_hover_text(
                "How far ahead of the sound card to stay. Raise it if you hear \
                 crackling, lower it for a more immediate beeper.",
            );
        });
    }
}

/// What the keyboard is doing in the windows other than the main one, which
/// goes to the machine as well: a window clicked last keeps the keyboard in
/// the window system's eyes, and the machine must not go deaf because of it.
#[derive(Clone, Default, Debug)]
pub struct HeldKeys {
    pub keys: std::collections::HashSet<egui::Key>,
    pub shift: bool,
    /// Alt or Ctrl, which are SYMBOL SHIFT.
    pub symbol: bool,
    /// Keys that went down this frame, for a binding waiting for one.
    pub pressed: Vec<egui::Key>,
}

impl eframe::App for App {
    /// Run the machine and put the debug windows up.
    ///
    /// This is deliberately not part of `ui`: eframe skips `ui` while the main
    /// window is not visible — which on macOS includes the moment the
    /// application is switched away from — and then prunes every viewport that
    /// frame did not declare, destroying the windows. They came back on the
    /// next switch as new windows, in creation order, which is why they
    /// vanished, reappeared and shuffled themselves about. `logic` runs
    /// whether the window is visible or not, so the windows stay put.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw_main(ui);
    }

    /// Keep the layout for next time, and write anything that has changed.
    fn on_exit(&mut self) {
        self.save_window_state();
        let _ = self.notes.save_if_dirty();
        // A disk written to during the session, written back before the
        // window goes: the machine has already been told the write happened.
        self.save_disk();
        self.save_cartridges();
    }
}

impl App {
    /// The whole user interface for one frame. Separate from the `eframe::App`
    /// impl so tests can drive it without a real window.
    /// Everything for one frame: used by the tests, which drive the interface
    /// through a single `Ui` rather than through eframe's split.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.frame_logic(&ctx);
        self.draw_main(ui);
    }

    /// The machine, the windows around it, and the repaint that keeps both
    /// going. Runs every frame, visible or not.
    pub fn frame_logic(&mut self, ctx: &egui::Context) {
        self.window_keys = std::mem::take(&mut self.window_keys_next);
        self.watch_mouse_capture(ctx);
        if !self.styled {
            theme::apply(ctx);
            self.styled = true;
        }
        let ctx = ctx.clone();
        let dt = ctx.input(|i| i.stable_dt);
        self.read_keyboard(&ctx);
        self.read_quick_keys(&ctx);
        self.advance(dt);
        self.sync_title(&ctx);
        self.remember_window("main", &ctx);
        match &mut self.zx81 {
            Some(zx) => zx.bus.tape_tick(),
            None => self.spec.bus.tape_tick(),
        }
        match &mut self.zx81 {
            // The back-buffer detector watches the Spectrum's display file, so
            // there is nothing for it to do here; the heat maps still have to
            // fade, or every byte the ROM has ever touched stays lit.
            Some(zx) => zx.bus.tracker.fade(),
            None => self.spec.bus.frame_visuals(),
        }
        self.draw_screen_texture(&ctx);

        // The debug windows are rendered before this viewport's own panels:
        // an immediate viewport runs a nested pass over the same Context, and
        // doing that after a menu popup has been opened discards the popup.
        self.debug_viewports(&ctx);

        // Watching runs while somebody is looking for something, and is
        // switched on from the window doing the looking rather than from a
        // toggle somewhere else. Set here because the window need not be open
        // for the machine to be running.
        // Two windows can want it: the call flow looking for something, and
        // the debugger working out where the routines are. Either is enough,
        // and neither switches the other off — this used to be a plain
        // assignment from the call flow, which quietly undid the debugger's
        // request on the next frame.
        self.spec.bus.observer.enabled = self.callflow.looking || self.dbg.watching_blocks;

        self.save_window_state_if_settled();
        self.save_notes_if_due();
        ctx.request_repaint();
    }

    /// The main window itself: its toolbars, the picture and the status line.
    fn draw_main(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu").show(ui, |ui| {
            self.menu(ui);
            self.machine_row(ui);
            self.video_row(ui);
        });
        let ctx = ui.ctx().clone();
        self.disk_prompt(&ctx);
        self.cartridge_prompt(&ctx);
        egui::Panel::bottom("status").show(ui, |ui| {
            self.controls_row(ui);
            let status = self.status.clone();
            ui.horizontal(|ui| {
                if self.status_is_error {
                    ui.colored_label(theme::RED, status);
                } else {
                    ui.colored_label(theme::DIM, status);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::rainbow(ui);
                });
            });
        });
        let mut beam = None;
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(tex) = &self.screen_tex {
                let (w, h) = match &self.zx81 {
                    Some(_) if self.overscan => (zx81::View::OVERSCAN.w, zx81::View::OVERSCAN.h),
                    Some(_) => (zx81::View::CROPPED.w, zx81::View::CROPPED.h),
                    None => {
                        let v = self.view();
                        (v.width(), v.height())
                    }
                };
                let size = egui::vec2(w as f32 * self.scale, h as f32 * self.scale);
                // Take the whole panel and put the picture in the middle of it,
                // so the space around the display is equal on all four sides.
                let (area, response) =
                    ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
                let painter = ui.painter_at(area);
                // The picture sits in a bevelled surround rather than on bare
                // black, as a set does in its case.
                painter.rect_filled(area, 0.0, theme::CASE_DARK);
                let picture = screen::centred(area, size);
                let bezel = picture.expand(10.0);
                painter.rect_filled(bezel, 6.0, egui::Color32::from_rgb(0x1a, 0x17, 0x13));
                painter.rect_stroke(
                    bezel,
                    6.0,
                    egui::Stroke::new(1.0, egui::Color32::BLACK),
                    egui::StrokeKind::Inside,
                );
                painter.rect_stroke(
                    picture.expand(1.0),
                    2.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(0x3a, 0x35, 0x2d)),
                    egui::StrokeKind::Outside,
                );
                if self.crt {
                    // Onto the glass, which is part of a sphere: the corners
                    // sit further out than the edges.
                    painter.add(egui::Shape::mesh(crt::curved(
                        tex.id(),
                        picture,
                        crt::CURVE,
                    )));
                } else {
                    painter.image(
                        tex.id(),
                        picture,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }

                // Clicking a pixel picks out the byte behind it in the
                // debugger's dump: "what draws this?" starts with knowing
                // which byte it is, and counting rows and thirds by hand to
                // work out a display address is a job nobody should be doing.
                //
                // Not with a Kempston mouse fitted: then a click over the
                // picture is the mouse's button, and the machine has it.
                let mouse = self.mouse_fitted();
                // Picking a block for the Graphics window's Find has the pointer
                // to itself: the click is the pick, not a mouse button or the
                // debugger's "which byte is this?".
                let picking = self.gfx_picking;
                if picking {
                    self.pick_graphics_block(ui, &response, picture, &painter);
                } else if mouse {
                    self.feed_mouse(ui, &response);
                }
                if response.clicked() && !mouse && !picking {
                    if let Some(at) = response.interact_pointer_pos() {
                        if picture.contains(at) {
                            let px = ((at.x - picture.left()) / self.scale) as usize;
                            let py = ((at.y - picture.top()) / self.scale) as usize;
                            self.show_pixel_in_memory(px, py);
                        }
                    }
                }

                // The ULA's own beam, drawn where it has actually reached.
                //
                // Only worth showing while the machine is going slowly enough
                // to see it: at full speed a frame of work is done between one
                // repaint and the next, so the beam would sit at the top of
                // the frame looking broken. The ZX81 has no ULA drawing a
                // picture — the CPU does it — so there is no beam to show.
                if self.crawling() && self.zx81.is_none() {
                    draw_beam(&painter, &self.spec.bus, self.view(), picture, self.scale);
                }

                // Where is the beam? Wherever the cursor is over the picture.
                // The ZX81 draws with the CPU, so there is no beam to race.
                let spectrum_view = self.view();
                beam = (self.zx81.is_none())
                    .then(|| {
                        response
                            .hover_pos()
                            .filter(|p| picture.contains(*p))
                            .map(|p| {
                                let px = ((p.x - picture.left()) / self.scale) as usize;
                                let py = ((p.y - picture.top()) / self.scale) as usize;
                                beam_at(&self.spec.bus, spectrum_view, px, py)
                            })
                    })
                    .flatten();
            }
        });
        self.beam_t = beam;
    }

    fn debug_viewports(&mut self, ctx: &egui::Context) {
        // A window that has been closed loses its viewport, and the next one
        // opened under the same name is a new window that the system will
        // place where it likes. Forget that it was ever positioned, so it is
        // put back where it was left rather than wherever it reappears.
        for (name, shown) in [
            ("ram_map", self.show_ram_map),
            ("debugger", self.show_debugger),
            ("profiler", self.show_profiler),
            ("tape", self.show_tape),
            ("back_buffer", self.show_back_buffer),
            ("sprites", self.show_sprites),
            ("callflow", self.show_callflow),
            ("keyboard", self.show_keyboard),
            ("disk", self.show_disk),
            ("hardware", self.show_hardware),
            ("joystick", self.show_input),
            ("printer", self.show_printer),
            ("microdrive", self.show_microdrive),
            ("training", self.show_training),
        ] {
            if !shown {
                self.placed.remove(name);
            }
        }

        if self.show_ram_map {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("ram-map"),
                self.restore_window(
                    "ram_map",
                    ViewportBuilder::default().with_title("RAM access map"),
                    [1120.0, 40.0],
                    [cassette::WINDOW_W, 700.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    self.fix_width(&ctx, cassette::WINDOW_W);
                    if self.place_window(
                        "ram_map",
                        &ctx,
                        [1120.0, 40.0],
                        [cassette::WINDOW_W, 700.0],
                    ) {
                        self.remember_window("ram_map", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| ram_map::ui(self, ui));
                },
            );
            self.show_ram_map = open;
        }

        if self.show_debugger {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("debugger"),
                self.restore_window(
                    "debugger",
                    ViewportBuilder::default().with_title("Debugger"),
                    [20.0, 40.0],
                    [debugger::WINDOW_W, 780.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    self.fix_width(&ctx, debugger::WINDOW_W);
                    if self.dbg.raise {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        self.dbg.raise = false;
                    }
                    if self.place_window(
                        "debugger",
                        &ctx,
                        [20.0, 40.0],
                        [debugger::WINDOW_W, 780.0],
                    ) {
                        self.remember_window("debugger", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| debugger::ui(self, ui));
                },
            );
            self.show_debugger = open;
        }

        if self.show_profiler {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("profiler"),
                self.restore_window(
                    "profiler",
                    ViewportBuilder::default().with_title("Profiler"),
                    [160.0, 320.0],
                    [900.0, 620.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    if self.place_window("profiler", &ctx, [220.0, 120.0], [900.0, 620.0]) {
                        self.remember_window("profiler", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| profiler::ui(self, ui));
                },
            );
            self.show_profiler = open;
        }

        if self.show_tape {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("tape"),
                self.restore_window(
                    "tape",
                    ViewportBuilder::default().with_title("Tape"),
                    [300.0, 120.0],
                    // Tall enough that the block list still has rows in it
                    // under everything above it: the cassette, the two rows of
                    // controls — five with the deck's failings on show — and
                    // the scope.
                    [720.0, 900.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    self.fix_width(&ctx, cassette::WINDOW_W);
                    if self.place_window("tape", &ctx, [260.0, 120.0], [cassette::WINDOW_W, 880.0])
                    {
                        // Built around the cassette: the height is the user's
                        // to drag, the width is not.
                        self.remember_window("tape", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| tape::ui(self, ui));
                },
            );
            self.show_tape = open;
        }

        if self.show_callflow {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("callflow"),
                self.restore_window(
                    "callflow",
                    ViewportBuilder::default().with_title("Call flow"),
                    [360.0, 180.0],
                    [520.0, 700.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    if self.place_window("callflow", &ctx, [360.0, 180.0], [520.0, 700.0]) {
                        self.remember_window("callflow", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| callflow::ui(self, ui));
                },
            );
            self.show_callflow = open;
        }

        if self.show_sprites {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("sprites"),
                self.restore_window(
                    "sprites",
                    ViewportBuilder::default().with_title("Graphics"),
                    [340.0, 200.0],
                    [720.0, 640.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    if self.sprites.raise {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        self.sprites.raise = false;
                    }
                    if self.place_window("sprites", &ctx, [340.0, 200.0], [720.0, 640.0]) {
                        self.remember_window("sprites", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| sprites::ui(self, ui));
                },
            );
            self.show_sprites = open;
        }

        if self.show_disk && diskwin::available(self) {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("disk"),
                self.restore_window(
                    "disk",
                    ViewportBuilder::default().with_title("Disk"),
                    [980.0, 80.0],
                    [cassette::WINDOW_W, 720.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    if self.place_window("disk", &ctx, [980.0, 80.0], [cassette::WINDOW_W, 720.0]) {
                        self.remember_window("disk", &ctx);
                    }
                    self.fix_width(&ctx, cassette::WINDOW_W);
                    egui::CentralPanel::default().show(ui, |ui| diskwin::ui(self, ui));
                },
            );
            self.show_disk = open;
        }

        if self.show_hardware {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("hardware"),
                self.restore_window(
                    "hardware",
                    ViewportBuilder::default().with_title("Hardware"),
                    [420.0, 120.0],
                    [520.0, 720.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    if self.place_window("hardware", &ctx, [420.0, 120.0], [520.0, 720.0]) {
                        self.remember_window("hardware", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| crate::ui::hardware::ui(self, ui));
                },
            );
            self.show_hardware = open;
        }

        if self.show_microdrive {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("microdrive"),
                self.restore_window(
                    "microdrive",
                    ViewportBuilder::default().with_title("Microdrive"),
                    [1040.0, 140.0],
                    [cassette::WINDOW_W, 640.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    if self.place_window(
                        "microdrive",
                        &ctx,
                        [1040.0, 140.0],
                        [cassette::WINDOW_W, 640.0],
                    ) {
                        self.remember_window("microdrive", &ctx);
                    }
                    self.fix_width(&ctx, cassette::WINDOW_W);
                    egui::CentralPanel::default()
                        .show(ui, |ui| crate::ui::microdrivewin::ui(self, ui));
                },
            );
            self.show_microdrive = open;
        }

        if self.show_keyboard {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("keyboard"),
                self.restore_window(
                    "keyboard",
                    ViewportBuilder::default().with_title("Keyboard"),
                    [360.0, 760.0],
                    [760.0, 300.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    if self.place_window("keyboard", &ctx, [360.0, 760.0], [760.0, 300.0]) {
                        self.remember_window("keyboard", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| keyboard::ui(self, ui));
                },
            );
            self.show_keyboard = open;
        }

        if self.show_input {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("joystick"),
                self.restore_window(
                    "joystick",
                    ViewportBuilder::default().with_title("Input"),
                    [420.0, 200.0],
                    [460.0, 520.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    if self.place_window("joystick", &ctx, [420.0, 200.0], [460.0, 520.0]) {
                        self.remember_window("joystick", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| inputwin::ui(self, ui));
                },
            );
            self.show_input = open;
        }

        if self.show_printer && self.spec.bus.printer.is_some() && self.zx81.is_none() {
            let mut open = true;
            let size = [cassette::WINDOW_W, 360.0];
            let default = [cassette::WINDOW_W, 620.0];
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("printer"),
                self.restore_window(
                    "printer",
                    ViewportBuilder::default().with_title("Printer"),
                    size,
                    default,
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    if self.place_window("printer", &ctx, size, default) {
                        self.remember_window("printer", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| printerwin::ui(self, ui));
                },
            );
            self.show_printer = open;
        }

        if self.show_training && self.zx81.is_none() {
            let mut open = true;
            let size = [560.0, 420.0];
            let default = [640.0, 900.0];
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("training"),
                self.restore_window(
                    "training",
                    ViewportBuilder::default().with_title("Training"),
                    size,
                    default,
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    if self.place_window("training", &ctx, size, default) {
                        self.remember_window("training", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| trainwin::ui(self, ui));
                },
            );
            self.show_training = open;
        }

        if self.show_back_buffer {
            let mut open = true;
            ctx.show_viewport_immediate(
                ViewportId::from_hash_of("back-buffer"),
                self.restore_window(
                    "back_buffer",
                    ViewportBuilder::default().with_title("Back buffer"),
                    [420.0, 240.0],
                    [680.0, 700.0],
                ),
                |ui, _class| {
                    self.collect_window_keys(ui.ctx());
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        open = false;
                    }
                    let ctx = ui.ctx().clone();
                    if self.place_window("back_buffer", &ctx, [300.0, 160.0], [680.0, 700.0]) {
                        self.remember_window("back_buffer", &ctx);
                    }
                    egui::CentralPanel::default().show(ui, |ui| back_buffer::ui(self, ui));
                },
            );
            self.show_back_buffer = open;
        }
    }

    /// What the desk and the pads are doing to the stick, and the line waiting
    /// to be bound.
    ///
    /// A binding is a source and an action: a key or a pad control, working
    /// one of the stick's five switches or a key of the machine's own
    /// keyboard.
    fn read_joystick(&mut self, ctx: &egui::Context) {
        // A line that is waiting takes the next thing pressed — a key or a pad
        // control — rather than working the stick with it.
        if let Some(i) = self.joystick_binding {
            let key = ctx.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => Some(*key),
                    _ => None,
                })
            });
            let key = key.or_else(|| self.window_keys.pressed.first().copied());
            let from = match key {
                Some(key) => Some(inputwin::From::Key(key)),
                None => self
                    .gilrs
                    .as_mut()
                    .and_then(inputwin::pad_pressed)
                    .map(inputwin::From::Pad),
            };
            if let Some(from) = from {
                if let Some(binding) = self.joystick_map.get_mut(i) {
                    binding.from = from;
                }
                self.joystick_binding = None;
            }
            return;
        }

        // What the pads are doing, kept for the window to show as well.
        self.pads = match self.gilrs.as_mut() {
            Some(gilrs) => inputwin::read_pads(gilrs),
            None => inputwin::Pads::default(),
        };

        // The mouse buttons are worked whether or not a stick is plugged in.
        self.apply_mouse_buttons(ctx);

        if self.spec.bus.joystick.kind == crate::joystick::Kind::None {
            self.spec.bus.joystick.release();
            return;
        }
        let window = self.window_keys.keys.clone();
        let down = |key| ctx.input(|i: &egui::InputState| i.key_down(key)) || window.contains(&key);
        for way in crate::joystick::Way::ALL {
            let over = self.joystick_map.iter().any(|binding| {
                binding.does == inputwin::Does::Way(way)
                    && inputwin::holding(binding.from, &down, &self.pads)
            });
            self.spec.bus.joystick.set(way, over);
        }
    }

    /// The buttons of both mice: the host's own, while the pointer is
    /// captured, and anything in the Input window bound to one.
    fn apply_mouse_buttons(&mut self, ctx: &egui::Context) {
        use inputwin::MouseButton as B;
        let window = &self.window_keys.keys;
        let down = |key| ctx.input(|i: &egui::InputState| i.key_down(key)) || window.contains(&key);
        let bound = |button: B| {
            self.joystick_map.iter().any(|binding| {
                binding.does == inputwin::Does::Mouse(button)
                    && inputwin::holding(binding.from, &down, &self.pads)
            })
        };
        let [left, middle, right] = self.host_buttons;
        let kempston = (
            left || bound(B::KempstonLeft),
            right || bound(B::KempstonRight),
        );
        let amx = (
            left || bound(B::AmxLeft),
            middle || bound(B::AmxMiddle),
            right || bound(B::AmxRight),
        );
        self.spec.bus.mouse.set_buttons(kempston.0, kempston.1);
        if let Some(mouse) = self.spec.bus.amx.as_mut() {
            mouse.set_buttons(amx.0, amx.1, amx.2);
        }
    }

    /// F1 to F9 save to slots 1 to 9 and F10 to slot 0, and each makes its
    /// slot the one Load restores.
    ///
    /// Read from the main window only: the debugger has F5, F7 and F8 for
    /// stepping, and its window takes its own keys.
    fn read_quick_keys(&mut self, ctx: &egui::Context) {
        use egui::Key;
        const KEYS: [(Key, usize); 10] = [
            (Key::F1, 1),
            (Key::F2, 2),
            (Key::F3, 3),
            (Key::F4, 4),
            (Key::F5, 5),
            (Key::F6, 6),
            (Key::F7, 7),
            (Key::F8, 8),
            (Key::F9, 9),
            (Key::F10, 0),
        ];
        for (key, slot) in KEYS {
            if ctx.input(|i| i.key_pressed(key)) {
                self.quick_save(slot);
            }
        }
    }

    /// Map host keys onto the 8x5 Spectrum keyboard matrix.
    fn read_keyboard(&mut self, ctx: &egui::Context) {
        use egui::Key;
        // A recording supplies every byte the machine reads from a port,
        // including the keyboard: typing at it would do nothing, and letting
        // the host keys through would only be confusing.
        if self.rzx.is_some() {
            return;
        }
        // Nor while a network is playing: it has the controls, and two hands
        // on them is neither playing nor watching.
        if self.training.player.is_some() {
            return;
        }
        let mut matrix = [0xffu8; 8];
        let mut press = |row: usize, bit: u8| matrix[row] &= !(1 << bit);

        // The stick first: a key bound to it works it instead of the machine's
        // own keyboard, or holding the arrows would type as well as steer.
        // A line waiting for a key takes the next one pressed rather than
        // acting on it.
        self.read_joystick(ctx);
        // Only a binding that can do something takes its key: Space is the
        // stick's fire by default, and with no stick plugged in — or one that
        // does nothing with fire, like a programmable interface nobody has
        // taught — it has to be the machine's space again, or it is a key
        // that does nothing at all.
        let stick = &self.spec.bus.joystick;
        let hardware = &self.spec.bus.hardware;
        let bound: Vec<egui::Key> = self
            .joystick_map
            .iter()
            .filter(|b| match b.does {
                inputwin::Does::Way(way) => stick.does_something(way),
                inputwin::Does::Key(..) => true,
                inputwin::Does::Mouse(button) => hardware.fitted(button.on()),
            })
            .filter_map(|b| match b.from {
                inputwin::From::Key(key) => Some(key),
                inputwin::From::Pad(_) => None,
            })
            .collect();
        let pads = self.pads.clone();
        // The other windows' keys count as the main window's: whichever of
        // them was clicked last, the machine is what is being typed at. And
        // the main window's own go to a text field in it rather than to the
        // machine while one is being typed into, as the other windows' do.
        let window = self.window_keys.clone();
        let typing = ctx.text_edit_focused();
        let down = |key| {
            (!typing && ctx.input(|i: &egui::InputState| i.key_down(key)))
                || window.keys.contains(&key)
        };
        for binding in &self.joystick_map {
            if let inputwin::Does::Key(row, bit) = binding.does {
                if inputwin::holding(binding.from, &down, &pads) {
                    press(row, bit);
                }
            }
        }

        ctx.input(|i| {
            const MAP: &[(Key, usize, u8)] = &[
                (Key::Z, 0, 1),
                (Key::X, 0, 2),
                (Key::C, 0, 3),
                (Key::V, 0, 4),
                (Key::A, 1, 0),
                (Key::S, 1, 1),
                (Key::D, 1, 2),
                (Key::F, 1, 3),
                (Key::G, 1, 4),
                (Key::Q, 2, 0),
                (Key::W, 2, 1),
                (Key::E, 2, 2),
                (Key::R, 2, 3),
                (Key::T, 2, 4),
                (Key::Num1, 3, 0),
                (Key::Num2, 3, 1),
                (Key::Num3, 3, 2),
                (Key::Num4, 3, 3),
                (Key::Num5, 3, 4),
                (Key::Num0, 4, 0),
                (Key::Num9, 4, 1),
                (Key::Num8, 4, 2),
                (Key::Num7, 4, 3),
                (Key::Num6, 4, 4),
                (Key::P, 5, 0),
                (Key::O, 5, 1),
                (Key::I, 5, 2),
                (Key::U, 5, 3),
                (Key::Y, 5, 4),
                (Key::Enter, 6, 0),
                (Key::L, 6, 1),
                (Key::K, 6, 2),
                (Key::J, 6, 3),
                (Key::H, 6, 4),
                (Key::Space, 7, 0),
                (Key::M, 7, 2),
                (Key::N, 7, 3),
                (Key::B, 7, 4),
            ];
            let key_down = |key| (!typing && i.key_down(key)) || window.keys.contains(&key);
            for &(key, row, bit) in MAP {
                if key_down(key) && !bound.contains(&key) {
                    press(row, bit);
                }
            }
            if (!typing && i.modifiers.shift) || window.shift {
                press(0, 0); // CAPS SHIFT
            }
            if (!typing && (i.modifiers.alt || i.modifiers.ctrl)) || window.symbol {
                press(7, 1); // SYMBOL SHIFT
            }
            // Convenience keys that need CAPS SHIFT on real hardware.
            if key_down(Key::Backspace) && !bound.contains(&Key::Backspace) {
                press(0, 0);
                press(4, 0);
            }
            if key_down(Key::ArrowLeft) && !bound.contains(&Key::ArrowLeft) {
                press(0, 0);
                press(3, 4);
            }
            if key_down(Key::ArrowDown) && !bound.contains(&Key::ArrowDown) {
                press(0, 0);
                press(4, 4);
            }
            if key_down(Key::ArrowUp) && !bound.contains(&Key::ArrowUp) {
                press(0, 0);
                press(4, 3);
            }
            if key_down(Key::ArrowRight) && !bound.contains(&Key::ArrowRight) {
                press(0, 0);
                press(4, 2);
            }
        });
        // What the keyboard window is holding down is pressed as well, and
        // everything the machine can see down is lit — including the keys of
        // the host keyboard, which is what makes the window a view of what is
        // being typed rather than only a thing to click.
        let now = std::time::Instant::now();
        let clicked = self.keys.matrix(now);
        for (row, keys) in matrix.iter_mut().enumerate() {
            *keys &= clicked[row];
            for bit in 0..5u8 {
                if *keys & (1 << bit) == 0 {
                    self.keys.lit(row, bit, now);
                }
            }
        }
        // The DK'Tronics Programmable, with its slider at 2, is taught the key
        // pressed while a direction is held.
        self.spec.bus.joystick.learn(&matrix);
        match &mut self.zx81 {
            // The ZX81's matrix is wired the same way, minus the bottom row's
            // shift keys.
            Some(zx) => zx.bus.keys = matrix,
            None => self.spec.bus.keys = matrix,
        }
    }
}

/// Which peripheral a Multiface model is, so the button can tell whether the
/// box is still plugged in.
fn peripheral_of(model: crate::multiface::Model) -> crate::hardware::Peripheral {
    match model {
        crate::multiface::Model::One => crate::hardware::Peripheral::MultifaceOne,
        crate::multiface::Model::OneTwentyEight => crate::hardware::Peripheral::Multiface128,
        crate::multiface::Model::Three => crate::hardware::Peripheral::Multiface3,
    }
}
