//! What of Knight Lore's memory its own recording uses: the measurements the
//! rebuild-for-128k skill takes before anything is moved. The original plays
//! its recording, which goes through more rooms than anything scripted
//! reaches. It takes a minute or two, so it is run by hand:
//!
//! ```text
//! cargo test --release --test knightlore_usage -- --ignored --nocapture
//! ```

use zx_rustrum::machine::{Model, Playback, Spectrum, Stop};
use zx_rustrum::tracker::Tracker;

const RECORDING: &str = "recordings/KnightLore.rzx";

thread_local! {
    /// Whether replay turns the observer on, which costs a branch an access.
    static OBSERVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Plays the original from its recording with a fresh tracker, frame by
/// frame, until the recording ends or comes adrift (reads input it never
/// recorded, or leaves some unread), and says how far it got.
fn replay() -> (Spectrum, usize) {
    replay_from(None, &mut |_, _| {})
}

/// The same, starting from `start` (a .z80 of a build standing where the
/// recording starts) instead of the recording's own snapshot, and calling
/// `each` after every frame.
fn replay_from(start: Option<&str>, each: &mut dyn FnMut(&Spectrum, usize)) -> (Spectrum, usize) {
    let recording = zx_rustrum::rzx::parse(&std::fs::read(RECORDING).unwrap()).unwrap();
    let snap = recording.snapshot.clone().unwrap();
    let mut spec = Spectrum::with_model(Model::Spectrum48);
    spec.load_rom(&std::fs::read("roms/48.rom").unwrap());
    match start {
        Some(path) => {
            zx_rustrum::snapshot::load_bytes(&mut spec, "z80", &std::fs::read(path).unwrap())
                .unwrap();
        }
        None => {
            zx_rustrum::snapshot::load_bytes(&mut spec, &snap.extension, &snap.data).unwrap();
        }
    }
    spec.bus.playback = Some(Playback::default());
    spec.bus.tracker = Tracker::new();
    spec.bus.observer.enabled = OBSERVE.with(|o| o.get());
    let mut carry = 0;
    for frame in 0..recording.len() {
        if let Some(snapshot) = recording.snapshot_before(frame) {
            zx_rustrum::rzx::carry_on_from(&mut spec, snapshot).unwrap();
        }
        let f = &recording.frames[frame];
        let (asked, left) = zx_rustrum::rzx::owed(f.fetches.max(1) as u32, carry);
        if let Some(playback) = &mut spec.bus.playback {
            playback.start_frame(f.inputs.clone());
        }
        let (stop, done) = spec.run_fetches(asked);
        assert!(
            matches!(stop, Stop::Budget),
            "frame {frame}: stopped by {stop:?}"
        );
        carry = left + done.saturating_sub(asked);
        spec.bus.end_frame_here();
        spec.bus.raise_interrupt();
        let playback = spec.bus.playback.as_ref().unwrap();
        if playback.short > 0 || playback.unread > 0 {
            return (spec, frame);
        }
        each(&spec, frame);
    }
    let frames = recording.len();
    (spec, frames)
}

// The code area ends with 21 bytes of text, "COPYRIGHT 1984 A.C.G.", that no
// code reaches. A hidden copyright is what a protection check would read, so
// before it is moved or left out it has to be shown that nothing reads it
// either, over the whole recording.
#[test]
#[ignore]
fn nothing_reads_the_hidden_copyright() {
    if std::fs::metadata(RECORDING).is_err() || std::fs::metadata("roms/48.rom").is_err() {
        eprintln!("no recording or ROM; skipping");
        return;
    }
    let (spec, frames) = replay();
    eprintln!("played {frames} frames before the recording ended or came adrift");
    let t = &spec.bus.tracker;
    let touched: Vec<String> = (0xD8DEu16..=0xD8F2)
        .filter(|a| {
            let i = spec.bus.phys_index(*a);
            t.read_count[i] > 0 || t.executed[i]
        })
        .map(|a| format!("${a:04X}"))
        .collect();
    // The check is only worth something if the tracker was counting: the
    // first sprite is read whenever it is drawn, and game_start runs.
    let i = spec.bus.phys_index(0xAF7F);
    assert!(t.executed[i], "game_start never ran: nothing was counted");
    let sprites = spec.bus.phys_index(0x7112);
    assert!(
        t.read_count[sprites] > 0,
        "the sprite table was never read: reads were not counted"
    );
    assert!(frames > 100_000, "only {frames} frames played");
    assert!(touched.is_empty(), "read or run: {touched:?}");
}

const MOVED: &str = "assemblies/knight-lore/out/moved.z80";

/// The screen, pixels and attributes, as the program left them.
fn screen(spec: &Spectrum) -> Vec<u8> {
    (0x4000..0x5B00u16).map(|a| spec.bus.peek_raw(a)).collect()
}

// Stage 1 of the rebuild-for-128k skill makes the export movable: every
// address into the code area a label. The only proof is moving it. The
// variant has every byte of code 16 bytes on (not 256, so a table found by
// its page would show too), and has to play the recording with the
// original's screen every 2,000 frames and without coming adrift.
//
// Not to the end, though, and nothing can: each pass, end_of_pass adds to
// the random number at $5BA5 the byte at the address its own pass count
// makes, so the game reads its way up through its memory. Once the count
// reaches a byte the move changed (here a pointer into the code, among the
// variables at $5B7A, at frame 221,8xx), the random numbers differ and the
// game goes another way. So the variant must match until the first frame the
// byte the count reads differs between the two, and may part after it.
#[test]
#[ignore]
fn the_code_moved_on_plays_the_recording_as_the_original_does() {
    if [RECORDING, MOVED, "roms/48.rom"]
        .iter()
        .any(|f| std::fs::metadata(f).is_err())
    {
        eprintln!("no recording, moved build or ROM; skipping");
        return;
    }
    // Per frame: the byte the pass count reads, and every 2,000 the screen.
    let watch = |start: Option<&str>| {
        let (mut read, mut screens) = (Vec::new(), Vec::new());
        let (_, frames) = replay_from(start, &mut |s, f| {
            let count = u16::from_le_bytes([s.bus.peek_raw(0x5BA2), s.bus.peek_raw(0x5BA3)]);
            read.push(s.bus.peek_raw(count));
            if f % 2000 == 0 {
                screens.push(screen(s));
            }
        });
        (read, screens, frames)
    };
    let (read, original, frames) = watch(None);
    let (moved_read, moved, moved_frames) = watch(Some(MOVED));
    let layout = read
        .iter()
        .zip(&moved_read)
        .position(|(a, b)| a != b)
        .unwrap_or(frames);
    eprintln!(
        "the count first reads a moved byte at frame {layout}; the moved build came adrift at \
         {moved_frames} of {frames}"
    );
    assert!(
        layout > 200_000,
        "the count reached moved bytes at frame {layout}"
    );
    assert!(
        moved_frames >= layout,
        "the moved build came adrift at frame {moved_frames}, before the count read a moved \
         byte at {layout}"
    );
    let compared = layout / 2000;
    if let Some(i) = original[..compared]
        .iter()
        .zip(&moved[..compared])
        .position(|(a, b)| a != b)
    {
        panic!("the screens part at frame {}, before {layout}", i * 2000);
    }
    eprintln!("{compared} screens the same");
}

// The AY build needs room for its driver and a page for its pitch table,
// which has to be memory the game never uses: zero at the recording's first
// frame, and never read, written or run in all of it. Written to
// assemblies/knight-lore/out/unused.txt, longest first.
#[test]
#[ignore]
fn where_nothing_is_used() {
    if std::fs::metadata(RECORDING).is_err() || std::fs::metadata("roms/48.rom").is_err() {
        eprintln!("no recording or ROM; skipping");
        return;
    }
    let recording = zx_rustrum::rzx::parse(&std::fs::read(RECORDING).unwrap()).unwrap();
    let snap = recording.snapshot.clone().unwrap();
    let mut first = Spectrum::with_model(Model::Spectrum48);
    zx_rustrum::snapshot::load_bytes(&mut first, &snap.extension, &snap.data).unwrap();
    let (spec, frames) = replay();
    assert_eq!(
        frames,
        recording.len(),
        "the recording came adrift at {frames}"
    );
    let t = &spec.bus.tracker;
    let unused = |a: u16| {
        let i = spec.bus.phys_index(a);
        first.bus.peek_raw(a) == 0
            && t.read_count[i] == 0
            && t.write_count[i] == 0
            && !t.executed[i]
    };
    let mut runs: Vec<(u16, u16)> = Vec::new();
    for a in 0x5B00u16..=0xFFFF {
        if unused(a) {
            match runs.last_mut() {
                Some(r) if r.1 == a - 1 => r.1 = a,
                _ => runs.push((a, a)),
            }
        }
    }
    runs.sort_by_key(|r| std::cmp::Reverse(r.1 - r.0));
    let text: String = runs
        .iter()
        .take(20)
        .map(|(a, b)| format!("${a:04X}-${b:04X} {}\n", b - a + 1))
        .collect();
    std::fs::write("assemblies/knight-lore/out/unused.txt", &text).unwrap();
    eprint!("{text}");
    // A measurement, so it fails only if nothing was counted: the sprite table
    // is read whenever anything is drawn. Knight Lore's longest run is 16
    // bytes, which is the finding (assemblies/knight-lore/plan.md, stage 2).
    let sprites = spec.bus.phys_index(0x7112);
    assert!(
        t.read_count[sprites] > 0,
        "the sprite table was never read: reads were not counted"
    );
}

// Stage 6's first step moves what is only read to build a room into bank 3,
// so it needs who reads the room data ($6248-$7111: kinds, rooms, object
// kinds, background pieces and the charm list) and what of it is written in
// play. Written to assemblies/knight-lore/out/room-readers.txt.
#[test]
#[ignore]
fn who_reads_the_room_data() {
    if std::fs::metadata(RECORDING).is_err() || std::fs::metadata("roms/48.rom").is_err() {
        eprintln!("no recording or ROM; skipping");
        return;
    }
    let map = std::fs::read_to_string("assemblies/knight-lore/out/original.map").unwrap();
    let mut names: Vec<(u16, String)> = map
        .lines()
        .filter_map(|l| {
            let (n, a) = l.split_once(" $")?;
            Some((
                u16::from_str_radix(a.split_whitespace().next()?, 16).ok()?,
                n.to_string(),
            ))
        })
        .collect();
    names.sort();
    let name = |a: u16| {
        let i = names.partition_point(|(x, _)| *x <= a);
        if i == 0 {
            format!("${a:04X}")
        } else {
            let (x, n) = &names[i - 1];
            if *x == a {
                n.clone()
            } else {
                format!("{n}+{}", a - x)
            }
        }
    };
    OBSERVE.with(|o| o.set(true));
    let (spec, frames) = replay();
    OBSERVE.with(|o| o.set(false));
    let mut out = format!("{frames} frames\nreaders of each page:\n");
    for page in 0x62u8..=0x71 {
        let who: Vec<String> = spec
            .bus
            .observer
            .readers_of(page)
            .into_iter()
            .map(name)
            .collect();
        out.push_str(&format!("  ${page:02X}xx {}\n", who.join(", ")));
    }
    out.push_str("written in play:\n");
    let t = &spec.bus.tracker;
    let mut run: Option<(u16, u16)> = None;
    for a in 0x6248u16..0x7112 {
        let written = t.write_count[spec.bus.phys_index(a)] > 0;
        match (written, run) {
            (true, Some((s, e))) if e == a - 1 => run = Some((s, a)),
            (true, _) => {
                if let Some((s, e)) = run {
                    out.push_str(&format!("  {}..{}\n", name(s), name(e)));
                }
                run = Some((a, a));
            }
            _ => {}
        }
    }
    if let Some((s, e)) = run {
        out.push_str(&format!("  {}..{}\n", name(s), name(e)));
    }
    std::fs::write("assemblies/knight-lore/out/room-readers.txt", &out).unwrap();
    eprint!("{out}");
}
