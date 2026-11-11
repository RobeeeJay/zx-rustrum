//! Step 0 of assemblies/memory-plan.md: what of Alien 8's memory is used
//! while its objects move, while it draws a frame, and while it is doing
//! something else (the menu, a tune, building a room).
//!
//! The original plays its own recording, which goes through more rooms than
//! anything scripted can reach. Each phase has its own copy of the bus's
//! tracker, swapped in at the addresses that start a phase, so a read, write
//! or opcode fetch is counted against the phase it happened in. Played for
//! 131,000 frames, before the recording parts from the machine, it takes a
//! few minutes, so it is run by hand:
//!
//! ```text
//! cargo test --release --test alien8_usage -- --ignored --nocapture
//! ```
//!
//! and writes what it found to assemblies/out/usage.txt.

use zx_rustrum::machine::{Model, Playback, Spectrum, Stop};
use zx_rustrum::tracker::Tracker;

const RECORDING: &str = "recordings/Alien8.rzx";
const MAP: &str = "assemblies/out/original.map";
const OUT: &str = "assemblies/out/usage.txt";
const FRAMES: usize = 131_000;

/// Objects moving, a frame being drawn, and anything else.
const PHASES: [&str; 3] = ["logic", "draw", "other"];

fn labels() -> Vec<(u16, String)> {
    let mut out: Vec<(u16, String)> = std::fs::read_to_string(MAP)
        .unwrap()
        .lines()
        .filter_map(|l| {
            let (name, at) = l.split_once(' ')?;
            Some((
                u16::from_str_radix(at.strip_prefix('$')?, 16).ok()?,
                name.to_string(),
            ))
        })
        .collect();
    out.sort();
    out
}

#[test]
#[ignore]
fn what_each_phase_of_a_pass_uses() {
    if [RECORDING, MAP, "roms/48.rom"]
        .iter()
        .any(|f| std::fs::metadata(f).is_err())
    {
        eprintln!("no recording, map or ROM; skipping");
        return;
    }
    let labels = labels();
    let at = |name: &str| labels.iter().find(|(_, n)| n == name).unwrap().0;
    // Where each phase starts. A pass is main_loop to end_of_pass, objects
    // moving, then end_of_pass back round, the frame drawn. Anything that
    // builds a room, plays a tune or shows a menu is "other" until the next
    // pass starts.
    let starts: Vec<(u16, usize)> = vec![
        (at("main_loop"), 0),
        (at("end_of_pass"), 1),
        (at("game_start"), 2),
        (at("room_left_build_again"), 2),
        (at("enter_room"), 2),
        (at("build_room"), 2),
        (at("start_life"), 2),
        (at("play_tune"), 2),
        (at("play_tune_once"), 2),
        (at("menu"), 2),
        (at("game_over"), 2),
    ];

    let recording = zx_rustrum::rzx::parse(&std::fs::read(RECORDING).unwrap()).unwrap();
    let snap = recording.snapshot.clone().unwrap();
    let mut spec = Spectrum::with_model(Model::Spectrum48);
    spec.load_rom(&std::fs::read("roms/48.rom").unwrap());
    zx_rustrum::snapshot::load_bytes(&mut spec, &snap.extension, &snap.data).unwrap();
    spec.bus.playback = Some(Playback::default());
    spec.breakpoints = starts.iter().map(|(a, _)| *a).collect();

    let mut trackers: Vec<Tracker> = (0..3).map(|_| Tracker::new()).collect();
    // The recording starts in the menu's tune.
    let mut phase = 2;
    std::mem::swap(&mut spec.bus.tracker, &mut trackers[phase]);
    let mut carry = 0;
    let mut switches = [0u64; 3];
    for frame in 0..FRAMES.min(recording.len()) {
        if let Some(snapshot) = recording.snapshot_before(frame) {
            zx_rustrum::rzx::carry_on_from(&mut spec, snapshot).unwrap();
        }
        let f = &recording.frames[frame];
        let (asked, left) = zx_rustrum::rzx::owed(f.fetches.max(1) as u32, carry);
        let mut ask = asked;
        if let Some(playback) = &mut spec.bus.playback {
            playback.start_frame(f.inputs.clone());
        }
        let mut done_all = 0;
        while ask > 0 {
            let (stop, done) = spec.run_fetches(ask);
            done_all += done;
            ask = ask.saturating_sub(done);
            match stop {
                Stop::Breakpoint(pc) => {
                    let next = starts.iter().find(|(a, _)| *a == pc).unwrap().1;
                    if next != phase {
                        std::mem::swap(&mut spec.bus.tracker, &mut trackers[phase]);
                        phase = next;
                        std::mem::swap(&mut spec.bus.tracker, &mut trackers[phase]);
                        switches[phase] += 1;
                    }
                    // Off the breakpoint, or it stops here for ever.
                    let (_, done) = {
                        let keep = std::mem::take(&mut spec.breakpoints);
                        let r = spec.run_fetches(1);
                        spec.breakpoints = keep;
                        r
                    };
                    done_all += done;
                    ask = ask.saturating_sub(done);
                }
                Stop::Budget => {}
                other => panic!("frame {frame}: stopped by {other:?}"),
            }
        }
        // An instruction run past the frame counts against the next, as the
        // recording counted it.
        carry = left + done_all.saturating_sub(asked);
        spec.bus.end_frame_here();
        spec.bus.raise_interrupt();
    }
    std::mem::swap(&mut spec.bus.tracker, &mut trackers[phase]);
    // A recording that has parted from the machine reads input that was
    // never recorded, and what it measured would be some other game.
    let playback = spec.bus.playback.as_ref().unwrap();
    assert_eq!(
        (playback.short, playback.unread),
        (0, 0),
        "the recording came adrift: short reads and unread input"
    );

    // Per address from $5B00: which phases executed, read and wrote it.
    let flags = |addr: u16| -> [u8; 3] {
        let i = spec.bus.phys_index(addr);
        let mut out = [0u8; 3];
        for (p, t) in trackers.iter().enumerate() {
            out[p] = u8::from(t.executed[i]) << 2
                | u8::from(t.read_count[i] > 0) << 1
                | u8::from(t.write_count[i] > 0);
        }
        out
    };
    let describe = |f: [u8; 3]| -> String {
        let mut parts = Vec::new();
        for (p, bits) in f.iter().enumerate() {
            if *bits != 0 {
                let mut s = String::new();
                if bits & 4 != 0 {
                    s.push('x');
                }
                if bits & 2 != 0 {
                    s.push('r');
                }
                if bits & 1 != 0 {
                    s.push('w');
                }
                parts.push(format!("{}:{s}", PHASES[p]));
            }
        }
        if parts.is_empty() {
            "untouched".to_string()
        } else {
            parts.join(" ")
        }
    };

    let mut report = format!(
        "Alien 8, the original playing {} frames of its recording.\n\
         Passes begun: {}; frames drawn: {}; times anything else began: {}.\n\
         x executed, r read, w written, in each phase.\n\n",
        FRAMES, switches[0], switches[1], switches[2]
    );
    // Runs of the same use, named by the label they start in.
    let mut totals = std::collections::BTreeMap::<String, u32>::new();
    let mut run_start = 0x5B00u32;
    let mut run = describe(flags(0x5B00));
    for a in 0x5B01u32..=0x10000 {
        let now = if a < 0x10000 {
            describe(flags(a as u16))
        } else {
            String::new()
        };
        if now != run {
            let name = labels
                .iter()
                .rev()
                .find(|(l, _)| u32::from(*l) <= run_start)
                .map(|(l, n)| format!("{n}+{}", run_start - u32::from(*l)))
                .unwrap_or_default();
            if a - run_start >= 16 {
                report.push_str(&format!(
                    "${:04X}-${:04X} {:5}  {:28} {}\n",
                    run_start,
                    a - 1,
                    a - run_start,
                    name,
                    run
                ));
            }
            *totals.entry(run.clone()).or_insert(0) += a - run_start;
            run_start = a;
            run = now;
        }
    }
    report.push_str("\nBytes from $5B00 by what used them:\n");
    let mut totals: Vec<_> = totals.into_iter().collect();
    totals.sort_by_key(|t| std::cmp::Reverse(t.1));
    for (what, bytes) in totals {
        report.push_str(&format!("{bytes:6}  {what}\n"));
    }
    // The same, region by region, for memory-plan.md.
    let regions: [(&str, u32, u32); 10] = [
        ("variables", 0x5B00, 0x5B88),
        ("objects", 0x5B88, 0x6288),
        ("unnamed data", 0x6288, 0x7827),
        ("sprite_table", 0x7827, 0x792F),
        ("sprites", 0x792F, 0xA5B2),
        ("code and its data", 0xA5B2, 0xD200),
        ("buffer", 0xD200, 0xEA00),
        ("free (now the AY driver)", 0xEA00, 0xF000),
        ("stack page", 0xF000, 0xF100),
        ("tables", 0xF100, 0x10000),
    ];
    report.push_str("\nBy region: bytes used only while objects move, only drawing, by both, only outside a pass, by a pass and outside it, and never:\n");
    for (name, lo, hi) in regions {
        let mut c = [0u32; 6];
        for a in lo..hi {
            let f = flags(a as u16);
            let (l, d, o) = (f[0] != 0, f[1] != 0, f[2] != 0);
            let i = match (l || d, o) {
                (false, false) => 5,
                (false, true) => 3,
                (true, true) => 4,
                (true, false) => match (l, d) {
                    (true, false) => 0,
                    (false, true) => 1,
                    _ => 2,
                },
            };
            c[i] += 1;
        }
        report.push_str(&format!(
            "${lo:04X}-${:04X} {name:26} logic {:5}  draw {:5}  both {:5}  other {:5}  pass+other {:5}  never {:5}\n",
            hi - 1,
            c[0],
            c[1],
            c[2],
            c[3],
            c[4],
            c[5]
        ));
    }
    std::fs::write(OUT, &report).unwrap();
    eprintln!("{}", &report[..report.len().min(400)]);
}
