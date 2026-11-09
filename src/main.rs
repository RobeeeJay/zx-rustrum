// No console window behind the app on Windows; stdout still works when the
// binary is run from a terminal.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use zx_rustrum::{
    audio_out::AudioOut, demo_rom, machine::Model, machine::Spectrum, prefs::Prefs, resources,
    snapshot, tape::Tape, ui,
};

/// Look for ROM images in every place a ROM might plausibly live (see
/// [`resources::search_dirs`]). The 48K one is needed to boot anything real;
/// without it the built-in demo ROM runs instead.
/// Where the ROMs the emulator starts with were found, so the notes for a
/// listing can be kept beside the ROM they describe.
struct RomPaths {
    spectrum: Option<PathBuf>,
    zx81: Option<PathBuf>,
}

fn find_roms(prefs: &Prefs, dirs: &[PathBuf]) -> (ui::Roms, RomPaths) {
    // Spectrum ROMs are 16K or more; the ZX81's is 8K, so it needs its own
    // floor rather than being quietly rejected as too small.
    let found = std::cell::RefCell::new(Vec::new());
    let read_min = |names: &[&str], min: usize| {
        resources::find_file(dirs, names, min).map(|(path, data)| {
            found.borrow_mut().push((names[0].to_string(), path));
            data
        })
    };
    let read = |names: &[&str]| read_min(names, 0x4000);
    let mut roms = ui::Roms {
        rom48: read(&["48.rom", "48k.rom", "spectrum48.rom"]),
        rom128: read(&["128.rom", "128k.rom"]),
        rom_plus3: read(&["plus3.rom", "plus2a.rom"]),
        rom_zx81: read_min(&["zx81.rom"], 0x2000),
    };
    let found = found.into_inner();
    let path_of = |name: &str| {
        found
            .iter()
            .find(|(found, _)| found == name)
            .map(|(_, path)| path.clone())
    };
    // The 48K image is the one the emulator boots into unless told otherwise.
    let paths = RomPaths {
        spectrum: path_of("48.rom"),
        zx81: path_of("zx81.rom"),
    };

    // Then whatever is in the directory the last ROM was opened from, so the
    // machines stay available between sessions.
    if let Some(dir) = &prefs.rom_dir {
        for (path, model) in ui::Roms::scan_directory(dir) {
            if roms.for_model(model).is_none() {
                if let Ok(data) = std::fs::read(&path) {
                    roms.set_for_model(model, data);
                }
            }
        }
    }
    (roms, paths)
}

/// Files named on the command line are dispatched by extension:
/// `.rom` replaces the ROM, `.tzx`/`.tap` load a tape, `.sna`/`.z80` a snapshot.
/// `--128`, `--48`, `--zx81` and `--zx81-1k` choose the machine.
///
/// ZX81 tapes are the exception: they can only go into a ZX81's deck, which
/// does not exist until the app has switched machines, so they are handed back
/// to be loaded once it has.
fn load_cli_files(
    spec: &mut Spectrum,
    roms: &mut ui::Roms,
    dirs: &[PathBuf],
    status: &mut String,
) -> (bool, Option<PathBuf>) {
    let mut opened_tape = false;
    let mut zx81_tape = None;
    for arg in std::env::args().skip(1) {
        if arg.starts_with("--") {
            continue; // handled before the machine was built
        }
        // A relative name on the command line is worth resolving against the
        // search path too, so a bundled app can be handed "game.tzx".
        let path = resources::resolve(dirs, std::path::Path::new(&arg));
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "p" | "81" | "p81" => zx81_tape = Some(path.clone()),
            "tzx" | "tap" => match Tape::load(&path) {
                Ok(t) => {
                    // Loaded stopped: the tape waits for Play, like a real one.
                    *status = format!("Tape: {} ({} blocks) — press Play", t.name, t.blocks.len());
                    spec.bus.tape = Some(t);
                    opened_tape = true;
                }
                Err(e) => *status = format!("Tape load failed: {e}"),
            },
            "sna" | "z80" | "szx" => {
                if let Ok(model) = snapshot::probe_model(&path) {
                    if model != spec.bus.model {
                        if let Some(rom) = roms.for_model(model) {
                            spec.set_model(model, rom);
                        }
                    }
                }
                match snapshot::load(spec, &path) {
                    Ok(_) => *status = format!("Loaded {}", path.display()),
                    Err(e) => *status = format!("Snapshot load failed: {e}"),
                }
            }
            "rom" | "bin" => match std::fs::read(&path) {
                Ok(data) => {
                    // The size says which machine the image belongs to, so a
                    // 32K ROM on the command line selects a 128K.
                    match ui::Roms::model_for_rom_size(data.len()) {
                        Some(model) => {
                            roms.set_for_model(model, data.clone());
                            if spec.bus.model == model {
                                spec.load_rom(&data);
                                spec.reset();
                            } else {
                                spec.set_model(model, &data);
                            }
                            *status = format!("ROM: {} ({})", path.display(), model.name());
                        }
                        None => {
                            spec.load_rom(&data);
                            spec.reset();
                            *status = format!(
                                "ROM: {} ({} bytes is an odd size)",
                                path.display(),
                                data.len()
                            );
                        }
                    }
                }
                Err(e) => *status = format!("ROM load failed: {e}"),
            },
            _ => *status = format!("Ignoring {arg}: unknown file type"),
        }
    }
    (opened_tape, zx81_tape)
}

/// The window and dock icon: the artwork in `icon.png`, decoded at startup.
///
/// The drawn logo stands in if that cannot be read, so a corrupt or missing
/// file costs an icon rather than the emulator.
fn window_icon() -> eframe::egui::IconData {
    const ART: &[u8] = include_bytes!("../icon.png");
    const SIZE: u32 = 512;
    match decode_png(ART) {
        Some((pixels, width, height)) => eframe::egui::IconData {
            // Shaped the way macOS shapes an icon: the artwork inside a
            // rounded square with clear space around it, so it sits in the
            // dock at the same visual size as everything else.
            rgba: zx_rustrum::appicon::shaped(&pixels, width, height, SIZE),
            width: SIZE,
            height: SIZE,
        },
        None => eframe::egui::IconData {
            rgba: zx_rustrum::logo::rgba(256),
            width: 256,
            height: 256,
        },
    }
}

/// A PNG as RGBA, whatever it was stored as.
fn decode_png(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(data));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    buffer.truncate(info.buffer_size());

    // Kept as RGBA whatever the file holds; ours is RGB.
    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 0xFF])
            .collect(),
        png::ColorType::Grayscale => buffer.iter().flat_map(|v| [*v, *v, *v, 0xFF]).collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    Some((rgba, info.width, info.height))
}

fn main() -> eframe::Result<()> {
    // Created on first launch, so there is always a file to look at.
    let prefs = Prefs::load_or_create();
    let dirs = resources::search_dirs();
    let (mut roms, rom_paths) = find_roms(&prefs, &dirs);
    let model = std::env::args()
        .find_map(|a| match a.as_str() {
            "--48" => Some(Model::Spectrum48),
            "--128" => Some(Model::Spectrum128),
            "--plus2a" | "--+2a" => Some(Model::Plus2A),
            "--plus3" | "--+3" => Some(Model::Plus3),
            _ => None,
        })
        .unwrap_or(Model::Spectrum48);

    let mut spec = Spectrum::with_model(model);
    let mut status = match roms.for_model(model) {
        Some(rom) => {
            spec.load_rom(rom);
            format!("{} ROM loaded", model.name())
        }
        None => {
            spec.load_rom(&demo_rom::DEMO_ROM);
            format!(
                "No {} ROM found (File ▸ Load ROM…). Running the built-in back-buffer demo.",
                model.name()
            )
        }
    };

    // Sound is optional: if no device opens, the emulator still runs silently.
    let (audio_out, audio_error) = match AudioOut::start() {
        Ok(out) => {
            spec.bus.audio.attach(out.queue.clone(), out.sample_rate);
            (Some(out), None)
        }
        Err(e) => (None, Some(e)),
    };

    let (opened_tape, zx81_tape) = load_cli_files(&mut spec, &mut roms, &dirs, &mut status);
    let mut zx81_ram = std::env::args().find_map(|a| match a.as_str() {
        "--zx81" | "--zx81-16k" => Some(zx_rustrum::zx81::Ram::K16),
        "--zx81-1k" => Some(zx_rustrum::zx81::Ram::K1),
        _ => None,
    });
    // A ZX81 program on the command line implies the machine to run it on.
    if zx81_tape.is_some() && zx81_ram.is_none() {
        zx81_ram = Some(zx_rustrum::zx81::Ram::K16);
    }

    // Put the main window back where it was last time.
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(ui::APP_NAME)
        .with_icon(window_icon());
    viewport = match prefs.window("main") {
        Some(r) => viewport
            .with_position([r.x, r.y])
            .with_inner_size([r.w, r.h]),
        None => viewport.with_inner_size([720.0, 660.0]),
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        ui::APP_NAME,
        options,
        Box::new(move |_cc| {
            let mut app = ui::App::with_roms(spec, status, roms, audio_out);
            app.prefs = prefs;
            app.apply_prefs();
            if let Some(ram) = zx81_ram {
                app.switch_to_zx81(ram);
            }
            // Notes are kept beside whatever is being disassembled; with no
            // tape loaded that is the ROM the machine booted from.
            app.rom_path = if app.on_zx81() {
                rom_paths.zx81.clone()
            } else {
                rom_paths.spectrum.clone()
            };
            app.reload_notes();
            if let Some(path) = zx81_tape {
                app.load_path(&path);
            }
            // A tape on the command line opens the deck; otherwise whichever
            // windows were open last time have already been restored.
            if opened_tape {
                app.show_tape = true;
            }
            app.audio_error = audio_error;
            Ok(Box::new(app))
        }),
    )
}
