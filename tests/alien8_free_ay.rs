//! The free-running AY build of Alien 8 in `assemblies/`: the beeper made
//! into AY writes that no longer wait, an IM 2 interrupt that plays them, and
//! the game given the time the sound used to take.
//!
//! The rebuilt games are the game's own code and are not in the repository,
//! so each test here skips itself when they are absent, as the tape tests do.

use zx_rustrum::machine::{Model, Spectrum};
use zx_rustrum::scope;

const RATE: f64 = 48_000.0;
const BEEPER: &str = "assemblies/Alien 8 start.z80";
const FREE: &str = "assemblies/Alien 8 AY free start.szx";
const FREE_MAP: &str = "assemblies/out/Alien 8 AY free.map";

fn have(files: &[&str]) -> bool {
    let missing: Vec<_> = files
        .iter()
        .filter(|f| std::fs::metadata(f).is_err())
        .collect();
    if !missing.is_empty() {
        eprintln!("no {missing:?}; skipping");
    }
    missing.is_empty()
}

fn machine(snap: &str) -> Spectrum {
    let (model, rom) = if snap.ends_with(".szx") {
        (Model::Spectrum128, "roms/128.rom")
    } else {
        (Model::Spectrum48, "roms/48.rom")
    };
    let mut spec = Spectrum::with_model(model);
    spec.load_rom(&std::fs::read(rom).unwrap());
    let data = std::fs::read(snap).unwrap();
    if model == Model::Spectrum128 {
        zx_rustrum::szx::load(&mut spec, &data).unwrap();
    } else {
        zx_rustrum::snapshot::load_z80(&mut spec, &data).unwrap();
    }
    spec.bus.audio.volume = 1.0;
    spec.bus.audio.tap = Some(Vec::new());
    spec
}

fn frames(spec: &mut Spectrum, n: usize) {
    let t = spec.bus.model.frame_t();
    for _ in 0..n {
        spec.run(t);
    }
}

fn key(spec: &mut Spectrum, row: usize, bit: u8, down: bool) {
    if down {
        spec.bus.keys[row] &= !(1 << bit);
    } else {
        spec.bus.keys[row] |= 1 << bit;
    }
}

/// The main loop's count of passes, which it keeps at $5B02.
fn passes(spec: &Spectrum) -> u16 {
    u16::from_le_bytes([spec.bus.peek_raw(0x5B02), spec.bus.peek_raw(0x5B03)])
}

/// From game_start: the menu, then 0 held long enough to be seen between two
/// notes of its tune, which is the only time the menu looks.
fn start_a_game(spec: &mut Spectrum) {
    frames(spec, 600);
    key(spec, 4, 0, true);
    frames(spec, 20);
    key(spec, 4, 0, false);
    frames(spec, 685);
}

/// Q, P, A and O in turn, fifty frames each: walking about the first room.
const WALK: [(usize, u8); 4] = [(2, 0), (5, 0), (1, 0), (5, 1)];

/// The tune as notes: the loudest pitch in 85ms either side of each 20ms, and how many 20ms it
/// held within 6%. Silence and the odd 20ms spent between two notes are left
/// out, so two builds can be compared note for note.
fn notes(samples: &[f32]) -> Vec<(f64, usize)> {
    let hop = (RATE / 50.0) as usize;
    let mut out: Vec<(f64, usize)> = Vec::new();
    for w in samples.windows(4096).step_by(hop) {
        let rms = (w.iter().map(|s| s * s).sum::<f32>() / 4096.0).sqrt();
        let hz = if rms < 0.01 {
            0.0
        } else {
            scope::loudest(&scope::spectrum(w), RATE).map_or(0.0, |(hz, _)| hz)
        };
        match out.last_mut() {
            Some((last, n)) if *last > 0.0 && (hz / *last - 1.0).abs() < 0.06 => *n += 1,
            _ => out.push((hz, 1)),
        }
    }
    out.retain(|(hz, n)| *hz > 0.0 && *n > 2);
    // Two notes of one pitch in a row are one run, however they were struck.
    let mut runs: Vec<(f64, usize)> = Vec::new();
    for (hz, n) in out {
        match runs.last_mut() {
            Some((last, m)) if (hz / *last - 1.0).abs() < 0.06 => *m += n,
            _ => runs.push((hz, n)),
        }
    }
    runs
}

// Taking the waits out moved the tune's timing from the CPU's loops to the
// interrupt, and its pitch from the speaker to the AY's tone registers; the
// first try faded every note out sixteen times too fast and the tune came out
// as clicks at 59 and 82Hz. It has to be the beeper's tune, note for note.
// Two notes of one pitch are counted as one run, since the beeper's join up
// and the AY's are struck apart; the fading end of a run can be a 20ms or two
// shorter. Pitch is held to 6%, not finer: the AY's output is one-sided, so a
// fading note carries the envelope's sixteen steps as a tone of their own,
// 103Hz for the tune's long notes, and its harmonic beside a 209Hz note pulls
// the reading by as much as 5%. The tune's notes are 12% apart or more.
#[test]
fn the_menu_tune_is_the_beeper_builds_note_for_note() {
    if !have(&[BEEPER, FREE, "roms/48.rom", "roms/128.rom"]) {
        return;
    }
    let tune = |snap: &str| {
        let mut spec = machine(snap);
        frames(&mut spec, 600);
        notes(spec.bus.audio.tap.as_ref().unwrap())
    };
    let (beeper, free) = (tune(BEEPER), tune(FREE));
    // How much of the tune is heard: the 20ms steps within 20dB of its loudest.
    let sounding = |snap: &str| {
        let mut spec = machine(snap);
        frames(&mut spec, 600);
        let a = spec.bus.audio.tap.take().unwrap();
        let steps: Vec<f32> = a[48_000..]
            .chunks(960)
            .map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt())
            .collect();
        let loud = steps.iter().fold(0f32, |m, r| m.max(*r));
        steps.iter().filter(|r| **r > loud / 10.0).count() as f64 / steps.len() as f64
    };
    // The notes have to last, not only start at the right pitch: faded out
    // sixteen times too fast they still read as the right notes, from their
    // first few milliseconds. The beeper's are heard in every 20ms of the
    // tune; the AY's fade, and are still heard in nine in ten of them.
    let (heard_beeper, heard_free) = (sounding(BEEPER), sounding(FREE));
    assert!(
        heard_free > 0.75,
        "the tune is heard in {:.0}% of its 20ms steps, the beeper build's in {:.0}%",
        heard_free * 100.0,
        heard_beeper * 100.0
    );
    assert!(beeper.len() >= 16, "the beeper build's tune: {beeper:?}");
    for (i, ((want, wn), (got, gn))) in beeper.iter().zip(&free).take(16).enumerate() {
        assert!(
            (got / want - 1.0).abs() < 0.06 && wn.abs_diff(*gn) <= 2,
            "note {i}: the beeper build plays {want:.0}Hz for {wn}, this {got:.0}Hz for {gn}\n\
             beeper {beeper:?}\nfree {free:?}"
        );
    }
}

// The sprite drawer reads a sprite with POP, SP pointing into it. An
// interrupt taken there pushes its return address into the sprite; without
// the DI round it, three did within sixteen frames and the machine crashed.
#[test]
fn no_interrupt_is_taken_while_the_sprite_drawer_has_the_stack() {
    if !have(&[FREE, FREE_MAP, "roms/128.rom"]) {
        return;
    }
    let tick = std::fs::read_to_string(FREE_MAP)
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("ay_tick $"))
        .map(|a| u16::from_str_radix(a, 16).unwrap())
        .expect("ay_tick in the map");
    let mut spec = machine(FREE);
    let (mut taken, mut stray) = (0, Vec::new());
    for f in 0..1_500usize {
        match f {
            600 => key(&mut spec, 4, 0, true),
            620 => key(&mut spec, 4, 0, false),
            _ => {}
        }
        if f >= 1_300 && (f - 1_300) % 50 == 0 {
            for (row, bit) in WALK {
                key(&mut spec, row, bit, false);
            }
            let (row, bit) = WALK[(f - 1_300) / 50 % WALK.len()];
            key(&mut spec, row, bit, true);
        }
        let end = spec.bus.total_t() + u64::from(spec.bus.model.frame_t());
        while spec.bus.total_t() < end {
            spec.step_instruction();
            // The step that takes an interrupt runs the JP at $EDED as well.
            if spec.cpu.pc == tick {
                taken += 1;
                if !(0xF000..=0xF100).contains(&spec.cpu.sp) {
                    stray.push((f, spec.cpu.sp));
                }
            }
        }
    }
    assert!(taken > 1_000, "only {taken} interrupts in 1,500 frames");
    assert!(stray.is_empty(), "taken with SP off the stack: {stray:?}");
    assert!(passes(&spec) > 0, "the game did not get going");
}

// The game's only pacing is a busy-wait at the end of each pass, so the time
// the beeper's loops took is time the game now runs in: 14% of it while
// walking about, measured on the beeper build.
#[test]
fn walking_about_the_game_gets_round_its_loop_more_often() {
    if !have(&[BEEPER, FREE, "roms/48.rom", "roms/128.rom"]) {
        return;
    }
    let walk = |snap: &str| {
        let mut spec = machine(snap);
        start_a_game(&mut spec);
        let before = passes(&spec);
        for i in 0..60 {
            let (row, bit) = WALK[i % WALK.len()];
            key(&mut spec, row, bit, true);
            frames(&mut spec, 50);
            key(&mut spec, row, bit, false);
        }
        passes(&spec).wrapping_sub(before)
    };
    let (beeper, free) = (walk(BEEPER), walk(FREE));
    assert!(
        f64::from(free) > f64::from(beeper) * 1.08,
        "3,000 frames of walking: {beeper} passes with the beeper, {free} with the AY free"
    );
}
