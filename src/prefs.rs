//! Preferences: a small file in the usual place for the platform, created on
//! first launch and rewritten whenever something worth remembering changes.
//!
//! The format is a TOML-compatible subset — `key = "value"` lines — so it can
//! be read and edited by hand. Keys the emulator does not recognise are kept
//! as they are rather than being thrown away on the next save.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "preferences.toml";
/// Set this to put the preferences somewhere else; used by the tests.
pub const DIR_OVERRIDE_VAR: &str = "ZX_SPECTRUM_CONFIG_DIR";

/// Which convention to follow for the configuration directory.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Platform {
    MacOs,
    Windows,
    Unix,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Unix
        }
    }
}

/// Where preferences live, given a way to read environment variables.
///
/// * macOS — `~/Library/Application Support/ZX Spectrum Emulator`
/// * Windows — `%APPDATA%\ZX Spectrum Emulator`
/// * anything else — `$XDG_CONFIG_HOME/zx-rustrum`, falling back to
///   `~/.config/zx-rustrum`
pub fn config_dir_from(
    platform: Platform,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    if let Some(dir) = env(DIR_OVERRIDE_VAR).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    match platform {
        Platform::MacOs => {
            let home = env("HOME").filter(|h| !h.is_empty())?;
            Some(
                PathBuf::from(home)
                    .join("Library")
                    .join("Application Support")
                    .join("ZX Spectrum Emulator"),
            )
        }
        Platform::Windows => {
            let appdata = env("APPDATA").filter(|d| !d.is_empty())?;
            Some(PathBuf::from(appdata).join("ZX Spectrum Emulator"))
        }
        Platform::Unix => {
            let base = match env("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
                Some(dir) => PathBuf::from(dir),
                None => PathBuf::from(env("HOME").filter(|h| !h.is_empty())?).join(".config"),
            };
            Some(base.join("zx-rustrum"))
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    config_dir_from(Platform::current(), &|k| std::env::var(k).ok())
}

/// Position and size of a window, in points.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WindowRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl WindowRect {
    fn parse(text: &str) -> Option<WindowRect> {
        let mut n = text.split(',').map(|p| p.trim().parse::<f32>());
        let (x, y, w, h) = (
            n.next()?.ok()?,
            n.next()?.ok()?,
            n.next()?.ok()?,
            n.next()?.ok()?,
        );
        // A window with no area is not worth restoring.
        (w >= 1.0 && h >= 1.0 && x.is_finite() && y.is_finite()).then_some(WindowRect {
            x,
            y,
            w,
            h,
        })
    }

    fn to_text(self) -> String {
        format!("{:.0},{:.0},{:.0},{:.0}", self.x, self.y, self.w, self.h)
    }
}

/// Key prefix used for window geometry.
const WINDOW_PREFIX: &str = "window.";

/// The remembered settings.
#[derive(Clone, Debug, Default)]
pub struct Prefs {
    /// Where the file lives, if one could be located at all.
    pub path: Option<PathBuf>,
    /// Directory the last ROM was opened from.
    pub rom_dir: Option<PathBuf>,
    /// Directory the last tape was opened from.
    pub tape_dir: Option<PathBuf>,
    /// Directory the last snapshot was opened from.
    pub snapshot_dir: Option<PathBuf>,
    /// Directory the last recording was opened from or written to. Its own
    /// rather than the snapshots': recordings are kept with the games they are
    /// of, and sending somebody back to wherever they last opened a snapshot
    /// is sending them somewhere else entirely.
    pub recording_dir: Option<PathBuf>,
    /// Where the +3 disks are.
    pub disk_dir: Option<PathBuf>,
    /// Where the microdrive cartridges are.
    pub cartridge_dir: Option<PathBuf>,
    /// Which machine was in use when the emulator was last closed, by the name
    /// the dropdown gives it.
    pub machine: Option<String>,
    /// What was plugged into the back of it, by the keys in `hardware.rs`, and
    /// how many microdrives were on the chain.
    pub peripherals: Option<Vec<String>>,
    pub microdrives: Option<usize>,
    /// Which joystick interface the stick is plugged into, and what on the
    /// desk works it.
    pub joystick: Option<String>,
    pub joystick_map: Option<String>,
    /// What the DK'Tronics Programmable has been taught.
    pub joystick_program: Option<String>,
    /// Where each window was when the emulator last closed.
    pub windows: BTreeMap<String, WindowRect>,
    /// Display scale, as a multiple of the Spectrum's own pixels.
    pub display_scale: Option<f32>,
    /// Whether the whole overscan border was being shown.
    pub overscan: Option<bool>,
    /// Which debug windows were open, so they come back with the emulator.
    pub open_windows: Option<Vec<String>>,
    /// Simple settings written through [`Prefs::set`], and anything else
    /// already in the file, kept so hand edits survive. One map for both: a
    /// setting the emulator writes and a key somebody typed in are the same
    /// thing to the file, and keeping them apart would mean listing every key
    /// twice.
    other: BTreeMap<String, String>,
}

impl Prefs {
    /// Read the preferences, creating the file (and its directory) if it is
    /// not there yet. Failure is not fatal: the emulator runs without it.
    pub fn load_or_create() -> Prefs {
        let Some(dir) = config_dir() else {
            return Prefs::default();
        };
        Prefs::load_or_create_in(&dir)
    }

    pub fn load_or_create_in(dir: &Path) -> Prefs {
        let path = dir.join(FILE_NAME);
        let mut prefs = match std::fs::read_to_string(&path) {
            Ok(text) => Prefs::parse(&text),
            Err(_) => Prefs::default(),
        };
        prefs.path = Some(path.clone());
        if !path.exists() {
            // Create it straight away, so there is something to edit and it is
            // obvious where settings are kept.
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(&path, prefs.to_text());
        }
        prefs
    }

    pub fn parse(text: &str) -> Prefs {
        let mut prefs = Prefs::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim().trim_matches('"').to_string();
            let path = (!value.is_empty()).then(|| PathBuf::from(&value));
            match key {
                "rom_dir" => prefs.rom_dir = path,
                "tape_dir" => prefs.tape_dir = path,
                "snapshot_dir" => prefs.snapshot_dir = path,
                "recording_dir" => prefs.recording_dir = path,
                "disk_dir" => prefs.disk_dir = path,
                "cartridge_dir" => prefs.cartridge_dir = path,
                "display_scale" => prefs.display_scale = value.parse().ok(),
                "overscan" => prefs.overscan = value.parse().ok(),
                "machine" => prefs.machine = Some(value),
                "microdrives" => prefs.microdrives = value.parse().ok(),
                "joystick" => prefs.joystick = Some(value.to_string()),
                "joystick_map" => prefs.joystick_map = Some(value.to_string()),
                "joystick_program" => prefs.joystick_program = Some(value.to_string()),
                "peripherals" => {
                    prefs.peripherals = Some(
                        value
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect(),
                    )
                }
                "open_windows" => {
                    prefs.open_windows = Some(
                        value
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect(),
                    )
                }
                k if k.starts_with(WINDOW_PREFIX) => {
                    if let Some(rect) = WindowRect::parse(&value) {
                        prefs
                            .windows
                            .insert(k[WINDOW_PREFIX.len()..].to_string(), rect);
                    }
                }
                other => {
                    prefs.other.insert(other.to_string(), value);
                }
            }
        }
        prefs
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from(
            "# ZX Spectrum emulator preferences.\n\
             # Written automatically; edit freely, unknown keys are kept.\n\n",
        );
        let line = |s: &mut String, key: &str, value: &Option<PathBuf>| {
            let v = value
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            s.push_str(&format!("{key} = \"{v}\"\n"));
        };
        line(&mut s, "rom_dir", &self.rom_dir);
        line(&mut s, "tape_dir", &self.tape_dir);
        line(&mut s, "snapshot_dir", &self.snapshot_dir);
        line(&mut s, "recording_dir", &self.recording_dir);
        line(&mut s, "disk_dir", &self.disk_dir);
        line(&mut s, "cartridge_dir", &self.cartridge_dir);
        if let Some(scale) = self.display_scale {
            s.push_str(&format!("display_scale = \"{scale}\"\n"));
        }
        if let Some(overscan) = self.overscan {
            s.push_str(&format!("overscan = \"{overscan}\"\n"));
        }
        if let Some(machine) = &self.machine {
            s.push_str(&format!("machine = \"{machine}\"\n"));
        }
        if let Some(fitted) = &self.peripherals {
            s.push_str(&format!("peripherals = \"{}\"\n", fitted.join(",")));
        }
        if let Some(joystick) = &self.joystick {
            s.push_str(&format!("joystick = \"{joystick}\"\n"));
        }
        if let Some(map) = &self.joystick_map {
            s.push_str(&format!("joystick_map = \"{map}\"\n"));
        }
        if let Some(program) = &self.joystick_program {
            s.push_str(&format!("joystick_program = \"{program}\"\n"));
        }
        if let Some(drives) = self.microdrives {
            s.push_str(&format!("microdrives = \"{drives}\"\n"));
        }
        if let Some(open) = &self.open_windows {
            s.push_str(&format!("open_windows = \"{}\"\n", open.join(",")));
        }
        if !self.windows.is_empty() {
            s.push_str("\n# Window geometry, as x,y,width,height in points.\n");
            for (name, rect) in &self.windows {
                s.push_str(&format!("{WINDOW_PREFIX}{name} = \"{}\"\n", rect.to_text()));
            }
        }
        for (k, v) in &self.other {
            s.push_str(&format!("{k} = \"{v}\"\n"));
        }
        s
    }

    /// Write the file back. Quietly does nothing if there is nowhere to write.
    pub fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, self.to_text());
    }

    /// Remember the directory a file was opened from, and save.
    pub fn remember_file(&mut self, kind: FileKind, file: &Path) {
        let Some(dir) = file.parent().map(Path::to_path_buf) else {
            return;
        };
        match kind {
            FileKind::Rom => self.rom_dir = Some(dir),
            FileKind::Tape => self.tape_dir = Some(dir),
            FileKind::Snapshot => self.snapshot_dir = Some(dir),
            FileKind::Recording => self.recording_dir = Some(dir),
            FileKind::Disk => self.disk_dir = Some(dir),
            FileKind::Cartridge => self.cartridge_dir = Some(dir),
        }
        self.save();
    }

    /// Read a setting written by [`Prefs::set`], if it is there.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.other.get(key).map(String::as_str)
    }

    /// A setting parsed as whatever it is meant to be, if it is there and
    /// makes sense. A key somebody has hand-edited into nonsense is ignored
    /// rather than being allowed to stop the emulator starting.
    pub fn get_as<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.get(key)?.parse().ok()
    }

    /// Write a setting. Saving is the caller's to do, since settings are
    /// written in batches.
    pub fn set(&mut self, key: &str, value: impl std::fmt::Display) {
        self.other.insert(key.to_string(), value.to_string());
    }

    /// Remember where a window is.
    pub fn set_window(&mut self, name: &str, rect: WindowRect) {
        self.windows.insert(name.to_string(), rect);
    }

    pub fn window(&self, name: &str) -> Option<WindowRect> {
        self.windows.get(name).copied()
    }

    /// Directory a file picker for `kind` should open in.
    pub fn dir_for(&self, kind: FileKind) -> Option<&PathBuf> {
        match kind {
            FileKind::Rom => self.rom_dir.as_ref(),
            FileKind::Tape => self.tape_dir.as_ref(),
            FileKind::Snapshot => self.snapshot_dir.as_ref(),
            // Until one has been opened, wherever the games are is the best
            // guess there is: a recording is of something, and that something
            // came off a tape.
            FileKind::Recording => self
                .recording_dir
                .as_ref()
                .or(self.tape_dir.as_ref())
                .or(self.snapshot_dir.as_ref()),
            FileKind::Disk => self.disk_dir.as_ref().or(self.tape_dir.as_ref()),
            FileKind::Cartridge => self.cartridge_dir.as_ref().or(self.tape_dir.as_ref()),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum FileKind {
    Rom,
    Tape,
    Snapshot,
    /// An RZX recording. Kept where its game is rather than with the
    /// snapshots, and remembered separately.
    Recording,
    /// A +3 disk image. Kept apart from tapes: somebody with disks has a
    /// directory of them.
    Disk,
    /// A microdrive cartridge.
    Cartridge,
}

impl FileKind {
    /// Which kind of file an extension names, if any.
    pub fn of_path(path: &Path) -> Option<FileKind> {
        match path
            .extension()
            .and_then(|e| e.to_str())?
            .to_ascii_lowercase()
            .as_str()
        {
            "rom" | "bin" => Some(FileKind::Rom),
            "tzx" | "tap" => Some(FileKind::Tape),
            "sna" | "z80" | "szx" => Some(FileKind::Snapshot),
            "dsk" | "ipf" => Some(FileKind::Disk),
            _ => None,
        }
    }
}
