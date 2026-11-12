//! The faster build of Alien 8 in `assemblies/`: the free-running AY build
//! with its drawing made cheaper.
//!
//! A change made for speed must not change the game, and the game is easy to
//! change without meaning to: it stirs the R register into its random
//! numbers, and R counts instructions. So the comparison is made between two
//! test variants with R taken out, fed the same keys on the same pass of the
//! main loop, and they have to agree on the screen and on every object at
//! the end of every pass. The rebuilt games are not in the repository, and
//! each test skips itself when they are absent.

use zx_rustrum::machine::{Model, Spectrum};

const FREE: &str = "assemblies/out/free fixed.szx";
const FAST: &str = "assemblies/out/fast fixed.szx";
const FREE_MAP: &str = "assemblies/out/free fixed.map";
const FAST_MAP: &str = "assemblies/out/fast fixed.map";
const PACED: &str = "assemblies/out/paced fixed.szx";
const PACED_MAP: &str = "assemblies/out/paced fixed.map";
const BANKED: &str = "assemblies/out/banked fixed.szx";
const BANKED_MAP: &str = "assemblies/out/banked fixed.map";

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

fn address(map: &str, name: &str) -> u16 {
    std::fs::read_to_string(map)
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{name} $")))
        // "name $ADDR", or "name $ADDR bankN" for a label in a bank.
        .map(|a| u16::from_str_radix(a.split_whitespace().next().unwrap(), 16).unwrap())
        .unwrap_or_else(|| panic!("{name} is not in {map}"))
}

fn machine(snap: &str) -> Spectrum {
    let mut spec = Spectrum::with_model(Model::Spectrum128);
    spec.load_rom(&std::fs::read("roms/128.rom").unwrap());
    zx_rustrum::szx::load(&mut spec, &std::fs::read(snap).unwrap()).unwrap();
    spec
}

fn key(spec: &mut Spectrum, row: usize, bit: u8, down: bool) {
    if down {
        spec.bus.keys[row] &= !(1 << bit);
    } else {
        spec.bus.keys[row] |= 1 << bit;
    }
}

/// From game_start through the menu, with 0 held long enough to be seen
/// between two notes of its tune.
fn start_a_game(spec: &mut Spectrum) {
    let t = spec.bus.model.frame_t();
    for f in 0..1_000 {
        match f {
            600 => key(spec, 4, 0, true),
            620 => key(spec, 4, 0, false),
            _ => {}
        }
        spec.run(t);
    }
}

/// Q, P, A and O in turn: walking about the rooms.
const WALK: [(usize, u8); 4] = [(2, 0), (5, 0), (1, 0), (5, 1)];

/// The pass both builds are compared from; the menu and a few seconds with no
/// keys come before it.
const FIRST_PASS: u16 = 128;

/// What the game is at the end of each of `passes` passes of its main loop,
/// walking a new way every thirty passes: the screen, and $5B00 to the end of
/// the last object.
fn play_by_pass(snap: &str, map: &str, passes: usize) -> Vec<Vec<u8>> {
    let mut spec = machine(snap);
    start_a_game(&mut spec);
    play_on(spec, map, passes, FIRST_PASS, true)
}

/// The same from a machine already in a game, from the pass `first` on,
/// walking or with the controls left alone.
fn play_on(mut spec: Spectrum, map: &str, passes: usize, first: u16, walk: bool) -> Vec<Vec<u8>> {
    let end_of_pass = address(map, "end_of_pass");
    let mut out = Vec::new();
    let mut steps = 0u64;
    let mut since = 0u64;
    let mut last = None;
    while out.len() < passes {
        let walking = if out.is_empty() || !walk {
            (0, 7) // nothing
        } else {
            WALK[out.len() / 30 % WALK.len()]
        };
        for (row, bit) in WALK {
            key(&mut spec, row, bit, (row, bit) == walking);
        }
        spec.step_instruction();
        steps += 1;
        // A game that has ended makes no more passes: what there is, is all.
        if steps > since + 5_000_000 {
            return out;
        }
        // From the same pass in both: the faster build has been round more
        // often by any given frame, and the game is the same game by pass.
        // An interrupt taken at end_of_pass comes back to it, so the same
        // pass is seen there twice; each pass is kept once, by its count.
        let count = u16::from_le_bytes([spec.bus.peek_raw(0x5B02), spec.bus.peek_raw(0x5B03)]);
        if spec.cpu.pc == end_of_pass && count >= first && last != Some(count) {
            last = Some(count);
            since = steps;
            out.push(comparable(&spec, map));
        }
    }
    out
}

// The comparison has to be able to fail: the same build against itself is
// the same, and two passes in a row are not, or nothing is being compared.
#[test]
fn the_faster_build_plays_the_same_game_pass_for_pass() {
    if !have(&[FREE, FAST, FREE_MAP, FAST_MAP, "roms/128.rom"]) {
        return;
    }
    let passes = 2_000;
    let free = play_by_pass(FREE, FREE_MAP, passes);
    let fast = play_by_pass(FAST, FAST_MAP, passes);
    assert_eq!(
        (free.len(), fast.len()),
        (passes, passes),
        "the game ended early"
    );
    let moving = free.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(
        moving > passes / 2,
        "only {moving} of {passes} passes changed anything"
    );
    for (pass, (a, b)) in free.iter().zip(&fast).enumerate() {
        if let Some(at) = a.iter().zip(b).position(|(x, y)| x != y) {
            panic!(
                "pass {pass}: ${:04X} is ${:02X} in the free build and ${:02X} in the faster one, \
                 and {} bytes differ",
                0x4000 + at,
                a[at],
                b[at],
                a.iter().zip(b).filter(|(x, y)| x != y).count()
            );
        }
    }
}

/// Runs the routine at `at` in `spec` with the registers given, until it
/// returns to a sentinel address pushed for it.
fn call(spec: &mut Spectrum, at: u16, bc: u16, de: u16, hl: u16) {
    let sentinel = 0x0000;
    // Below wherever the machine keeps its stack: bank 0's $F0xx until Step
    // 4 of the memory plan, fixed memory after, where a trampoline that pages
    // can push in one bank and pop in another.
    // SP is put back afterwards, or a test making thousands of calls walks
    // the stack down into the buffer.
    let was = spec.cpu.sp;
    let top = was.wrapping_sub(64);
    spec.cpu.sp = top;
    spec.bus.poke(top, (sentinel & 0xFF) as u8);
    spec.bus.poke(top + 1, (sentinel >> 8) as u8);
    spec.cpu.pc = at;
    spec.cpu.iff1 = false;
    spec.cpu.iff2 = false;
    spec.cpu.b = (bc >> 8) as u8;
    spec.cpu.c = bc as u8;
    spec.cpu.d = (de >> 8) as u8;
    spec.cpu.e = de as u8;
    spec.cpu.h = (hl >> 8) as u8;
    spec.cpu.l = hl as u8;
    for _ in 0..1_000_000 {
        spec.step_instruction();
        if spec.cpu.pc == sentinel && spec.cpu.sp == top + 2 {
            spec.cpu.sp = was;
            return;
        }
    }
    panic!("${at:04X} did not return");
}

// copy_rect works up the screen from the bottom line of a rectangle. Rows
// that end at the right-hand edge on the last line of a third carry into D
// as they are copied, which never happened in six hundred passes of the game,
// so every column, width and height is tried here in both builds and the
// screens compared.
#[test]
fn copy_rect_copies_every_rectangle_as_the_old_one_did() {
    if !have(&[FREE, FAST, FREE_MAP, FAST_MAP, "roms/128.rom"]) {
        return;
    }
    let mut free = machine(FREE);
    let mut fast = machine(FAST);
    let (at_free, at_fast) = (
        address(FREE_MAP, "copy_rect"),
        address(FAST_MAP, "copy_rect"),
    );
    let buffer = address(FREE_MAP, "buffer");
    for spec in [&mut free, &mut fast] {
        for i in 0..0x1800u16 {
            spec.bus
                .poke(buffer + i, (i.wrapping_mul(97) >> 3) as u8 ^ (i as u8));
        }
    }
    let screen =
        |x: u16, y: u16| 0x4000 | ((y & 0xC0) << 5) | ((y & 0x07) << 8) | ((y & 0x38) << 2) | x;
    let mut tried = 0;
    for x in 0..32u16 {
        for width in 1..=(32 - x).min(10) {
            for (bottom, rows) in [(7u16, 8u16), (63, 20), (64, 1), (191, 30), (127, 9)] {
                let (bc, de, hl) = (
                    rows << 8 | width,
                    screen(x, bottom),
                    // The buffer runs the other way: a row up the screen is
                    // 32 bytes on in the buffer, so start low enough in it.
                    buffer + (191 - bottom) * 32 + x,
                );
                for i in 0x4000..0x5800u16 {
                    free.bus.poke(i, 0);
                    fast.bus.poke(i, 0);
                }
                call(&mut free, at_free, bc, de, hl);
                call(&mut fast, at_fast, bc, de, hl);
                tried += 1;
                if let Some(a) =
                    (0x4000..0x5800u16).find(|a| free.bus.peek_raw(*a) != fast.bus.peek_raw(*a))
                {
                    panic!(
                        "{width} wide, {rows} rows up from line {bottom} at column {x}: ${a:04X} is ${:02X} \
                         from the old copy_rect and ${:02X} from the new",
                        free.bus.peek_raw(a),
                        fast.bus.peek_raw(a)
                    );
                }
            }
        }
    }
    assert!(tried > 1_000, "{tried}");
}

// ay_total stands in for walking the queue on every click, so at any moment
// outside the driver it has to be what that walk would have found: what is
// left of each sound from the front of the queue to the back.
#[test]
fn the_running_total_is_what_is_left_in_the_queue() {
    if !have(&[FAST, FAST_MAP, "roms/128.rom"]) {
        return;
    }
    let at = |name: &str| address(FAST_MAP, name);
    let (end_of_pass, head, tail, queue, total) = (
        at("end_of_pass"),
        at("ay_head"),
        at("ay_tail"),
        at("ay_queue"),
        at("ay_total"),
    );
    let mut spec = machine(FAST);
    start_a_game(&mut spec);
    let (mut checked, mut sounding) = (0, 0);
    while checked < 400 {
        let (row, bit) = WALK[checked / 30 % WALK.len()];
        for (r, b) in WALK {
            key(&mut spec, r, b, (r, b) == (row, bit));
        }
        spec.step_instruction();
        if spec.cpu.pc != end_of_pass {
            continue;
        }
        let peek = |a: u16| spec.bus.peek_raw(a);
        let (h, t) = (peek(head), peek(tail));
        let mut sum = 0u32;
        let mut i = h;
        while i != t {
            let entry = queue + u16::from(i);
            sum += u32::from(peek(entry + 2)) | u32::from(peek(entry + 3)) << 8;
            i = i.wrapping_add(4);
        }
        let kept = u32::from(peek(total)) | u32::from(peek(total + 1)) << 8;
        assert_eq!(
            kept,
            sum.min(0xFFFF),
            "pass {checked}: the queue holds {sum} units, ay_total says {kept}"
        );
        checked += 1;
        sounding += usize::from(h != t);
    }
    assert!(sounding > 20, "only {sounding} passes had anything queued");
}

/// Two objects in the draw list with the given x, y, z, size_x, size_y,
/// height, and the pair test run from the first: where it went, and the IY it
/// went there with.
fn pair_test(spec: &mut Spectrum, map: &str, a: [u8; 6], b: [u8; 6]) -> (&'static str, u16) {
    let list = address(map, "l_C745");
    for (i, o) in [a, b].iter().enumerate() {
        let at = 0x5B88 + 32 * i as u16;
        spec.bus.poke(at, 0x20);
        for (k, v) in o.iter().enumerate() {
            spec.bus.poke(at + 1 + k as u16, *v);
        }
        spec.bus.poke(at + 7, 0x10);
    }
    for (i, v) in [0u8, 1, 0xFF].iter().enumerate() {
        spec.bus.poke(list + i as u16, *v);
    }
    spec.bus.poke(0x5B2D, (list + 1) as u8);
    spec.bus.poke(0x5B2E, ((list + 1) >> 8) as u8);
    spec.cpu.ix = 0x5B88;
    spec.cpu.iy = 0;
    spec.cpu.sp = 0xF0F0;
    spec.cpu.iff1 = false;
    spec.cpu.iff2 = false;
    // The faster build takes IX's bounds when IX changes; this is that change.
    if let Some(bounds) = std::fs::read_to_string(map)
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("depth_bounds $"))
    {
        let at = u16::from_str_radix(bounds, 16).unwrap();
        spec.bus.poke(0xF0EE, 0x00);
        spec.bus.poke(0xF0EF, 0x00);
        spec.cpu.sp = 0xF0EE;
        spec.cpu.pc = at;
        while spec.cpu.pc != 0 {
            spec.step_instruction();
        }
    }
    spec.cpu.d = (list >> 8) as u8;
    spec.cpu.e = list as u8;
    spec.cpu.pc = address(map, "find_in_front");
    let ends = [
        ("draw_this_one", address(map, "draw_this_one")),
        ("depth_iy_first", address(map, "depth_iy_first")),
        ("depth_skip_passing", address(map, "depth_skip_passing")),
    ];
    for _ in 0..10_000 {
        spec.step_instruction();
        if let Some((name, _)) = ends.iter().find(|(_, at)| *at == spec.cpu.pc) {
            // IY is only read on the way to putting a pair in order. Drawing
            // reads nothing of it, and draw_in_order puts it back at the end,
            // so the new test does not leave the last object it looked at
            // there the way the old one did.
            let iy = if *name == "draw_this_one" {
                0
            } else {
                spec.cpu.iy
            };
            return (name, iy);
        }
    }
    panic!("the pair test went nowhere");
}

// The pair test decides what is drawn in front of what, in 27 cases by how
// two boxes lie on three axes, with 8-bit sums that wrap. Walking about never
// puts two boxes through each other, case 13, so random pairs are tried here
// in both builds, those included, and have to go to the same place.
#[test]
fn the_pair_test_puts_every_pair_in_the_same_order() {
    if !have(&[FREE, FAST, FREE_MAP, FAST_MAP, "roms/128.rom"]) {
        return;
    }
    let mut free = machine(FREE);
    let mut fast = machine(FAST);
    let mut seed = 0x2545_F491u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let mut seen = std::collections::BTreeMap::new();
    for i in 0..3_000 {
        let mut object = |near: Option<[u8; 6]>| -> [u8; 6] {
            let mut o = [0u8; 6];
            for (k, v) in o.iter_mut().enumerate() {
                *v = match (near, k) {
                    // Half the time close to the first, so they touch.
                    (Some(n), 0..=2) if i % 2 == 0 => {
                        n[k].wrapping_add((next() % 9) as u8).wrapping_sub(4)
                    }
                    (_, 0..=2) => next() as u8,
                    _ => (next() % 12) as u8 + 1,
                };
            }
            o
        };
        let a = object(None);
        let b = object(Some(a));
        let old = pair_test(&mut free, FREE_MAP, a, b);
        let new = pair_test(&mut fast, FAST_MAP, a, b);
        assert_eq!(
            old, new,
            "boxes {a:?} and {b:?}: the old pair test went to {old:?}, the new to {new:?}"
        );
        *seen.entry(old.0).or_insert(0) += 1;
    }
    assert!(seen.len() == 3, "only reached {seen:?}");
}

// mark_overlapping's scan sets the redraw flag on every object whose cell
// lies in the rectangle a moving object has left and entered, with sums
// that wrap at the screen's edges. Walking about one room tries few of those,
// so random rooms of 56 objects are scanned here in both builds, from where
// the rectangle has been worked out, and every flag has to come out the same.
#[test]
fn the_scan_marks_the_same_objects() {
    if !have(&[FREE, FAST, FREE_MAP, FAST_MAP, "roms/128.rom"]) {
        return;
    }
    let mut free = machine(FREE);
    let mut fast = machine(FAST);
    let mut seed = 0x9E37_79B9u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let mut marked = 0;
    for trial in 0..500 {
        let objects: Vec<[u8; 6]> = (0..56)
            .map(|_| {
                let r = next();
                // type, flags, screen_x, screen_y, sprite_w, sprite_h
                [
                    if r % 4 == 0 { 0 } else { (r >> 8) as u8 | 1 },
                    (r >> 16) as u8,
                    next() as u8,
                    (next() % 200) as u8,
                    (next() % 6) as u8,
                    (next() % 40) as u8,
                ]
            })
            .collect();
        let (left, width, top, height) = (
            (next() % 32) as u8,
            (next() % 8) as u8 + 1,
            (next() % 192) as u8,
            (next() % 60) as u8 + 1,
        );
        let mut after = Vec::new();
        for (spec, map) in [(&mut free, FREE_MAP), (&mut fast, FAST_MAP)] {
            for (i, o) in objects.iter().enumerate() {
                let at = 0x5B88 + 32 * i as u16;
                spec.bus.poke(at, o[0]);
                spec.bus.poke(at + 7, o[1]);
                spec.bus.poke(at + 0x1A, o[2]);
                spec.bus.poke(at + 0x1B, o[3]);
                spec.bus.poke(at + 0x18, o[4]);
                spec.bus.poke(at + 0x19, o[5]);
            }
            spec.cpu.e = left;
            spec.cpu.d = width;
            spec.cpu.l = top;
            spec.cpu.a = top.wrapping_add(height);
            spec.cpu.b = 0x38;
            spec.cpu.iy = 0x5B88;
            spec.cpu.sp = 0xF0F0;
            spec.bus.poke(0xF0F0, 0);
            spec.bus.poke(0xF0F1, 0);
            spec.cpu.iff1 = false;
            spec.cpu.iff2 = false;
            spec.cpu.pc = address(map, "rect_height");
            while spec.cpu.pc != 0 {
                spec.step_instruction();
            }
            let flags: Vec<u8> = (0..56)
                .map(|i| spec.bus.peek_raw(0x5B88 + 32 * i + 7))
                .collect();
            // DE and HL are not left as the old scan left them: nothing after
            // it reads them before loading them.
            after.push((flags, spec.cpu.iy, spec.cpu.b));
        }
        assert_eq!(
            after[0], after[1],
            "trial {trial}: the rectangle at {left},{top}, {width} by {height}, marks differently"
        );
        marked += (0..56)
            .filter(|i| after[0].0[*i] & 0x10 != 0 && objects[*i][1] & 0x10 == 0)
            .count();
    }
    assert!(marked > 100, "only {marked} objects were newly marked");
}

/// A build with the game's state from `sna` (the original, at end_of_pass)
/// carried into it: the screen, the variables and objects, and the buffer.
fn busy_machine(sna: &[u8], snap: &str, map: &str) -> Spectrum {
    let mut spec = machine(snap);
    for a in 0x4000..0x6288u32 {
        spec.bus.poke(a as u16, sna[27 + a as usize - 0x4000]);
    }
    // The original's buffer is at $D200; a build's is wherever its map says,
    // which is $9800 since Step 5 of the memory plan.
    let buffer = address(map, "buffer");
    for i in 0..0x1800u16 {
        spec.bus
            .poke(buffer + i, sna[27 + 0xD200 - 0x4000 + usize::from(i)]);
    }
    // The original game's pointers into its own draw list; both builds set
    // them before they read them.
    let list = address(map, "l_C745");
    for at in [0x5B2B, 0x5B2D, 0x5B2F] {
        spec.bus.poke(at, list as u8);
        spec.bus.poke(at + 1, (list >> 8) as u8);
    }
    // And into its room data, which is at $6460 in the original and may not
    // be in this build: the room's entry, and the low byte of the kinds
    // table (see room_pointers_as_offsets).
    let moved = room_base(map).wrapping_sub(0x6460);
    let entry = u16::from_le_bytes([spec.bus.peek_raw(0x5B34), spec.bus.peek_raw(0x5B35)]);
    let entry = entry.wrapping_add(moved);
    spec.bus.poke(0x5B34, entry as u8);
    spec.bus.poke(0x5B35, (entry >> 8) as u8);
    let kinds = spec.bus.peek_raw(0x5B20).wrapping_add(moved as u8);
    spec.bus.poke(0x5B20, kinds);
    spec.bus.set_joystick(zx_rustrum::joystick::Kind::Kempston);
    spec.cpu.pc = address(map, "end_of_pass");
    spec.cpu.sp = stack_top(map);
    // The IM 2 table is wherever the build keeps it: $EC00 until Step 3 of
    // the memory plan moved it.
    spec.cpu.i = (address(map, "ay_vectors") >> 8) as u8;
    spec.cpu.im = 2;
    spec.cpu.iff1 = true;
    spec.cpu.iff2 = true;
    spec
}

const BUSY: [u32; 3] = [45_000, 50_000, 90_000];

// The first room has few objects in it, and walking about never leaves it,
// so three busy rooms are taken from the original's recording too
// (assemblies/busy_states.py), each at end_of_pass. Only the game's state is
// carried into each build: the screen, the variables and objects, and the
// buffer, since a rebuilt game's code is not where the original's was. The
// recording was played with a Kempston joystick, which has to be fitted:
// without one, the game reads the floating bus, whose value depends on the
// timing, which is exactly what differs between the builds.
#[test]
fn the_faster_build_plays_the_same_game_in_busy_rooms() {
    let frames = BUSY;
    let states: Vec<String> = frames
        .iter()
        .map(|f| format!("assemblies/out/busy{f}.sna"))
        .collect();
    let mut need: Vec<&str> = states.iter().map(String::as_str).collect();
    need.extend([FREE, FAST, FREE_MAP, FAST_MAP, "roms/128.rom"]);
    if !have(&need) {
        return;
    }
    for (frame, state) in frames.iter().zip(&states) {
        let sna = std::fs::read(state).unwrap();
        let free = play_on(busy_machine(&sna, FREE, FREE_MAP), FREE_MAP, 120, 0, false);
        let fast = play_on(busy_machine(&sna, FAST, FAST_MAP), FAST_MAP, 120, 0, false);
        assert_eq!(
            (free.len(), fast.len()),
            (120, 120),
            "from frame {frame} of the recording, the game stopped making passes"
        );
        for (pass, (a, b)) in free.iter().zip(&fast).enumerate() {
            if let Some(at) = a.iter().zip(b).position(|(x, y)| x != y) {
                panic!(
                    "from frame {frame} of the recording, pass {pass}: ${:04X} is ${:02X} in the \
                     free build and ${:02X} in the faster one",
                    0x4000 + at,
                    a[at],
                    b[at]
                );
            }
        }
    }
}

// What the changes are for. In a busy room a pass takes ten frames, and the
// game's only pacing is a busy-wait, so less time a pass is a faster game.
// Measured: 7% less in the room at frame 45,000, 15% less at 90,000.
#[test]
fn a_busy_room_takes_less_time_a_pass() {
    let states: Vec<String> = BUSY
        .iter()
        .map(|f| format!("assemblies/out/busy{f}.sna"))
        .collect();
    let mut need: Vec<&str> = states.iter().map(String::as_str).collect();
    need.extend([FREE, FAST, FREE_MAP, FAST_MAP, "roms/128.rom"]);
    if !have(&need) {
        return;
    }
    for (frame, state) in BUSY.iter().zip(&states) {
        let sna = std::fs::read(state).unwrap();
        let time = |snap: &str, map: &str| {
            let mut spec = busy_machine(&sna, snap, map);
            let end_of_pass = address(map, "end_of_pass");
            let (mut seen, mut last, mut from) = (0, None, 0);
            while seen < 61 {
                spec.step_instruction();
                let count =
                    u16::from_le_bytes([spec.bus.peek_raw(0x5B02), spec.bus.peek_raw(0x5B03)]);
                if spec.cpu.pc == end_of_pass && last != Some(count) {
                    last = Some(count);
                    seen += 1;
                    if seen == 1 {
                        from = spec.bus.total_t();
                    }
                }
            }
            spec.bus.total_t() - from
        };
        let (free, fast) = (time(FREE, FREE_MAP), time(FAST, FAST_MAP));
        let ratio = fast as f64 / free as f64;
        assert!(
            ratio < 0.95,
            "from frame {frame}: sixty passes took {free} T-states in the free build and {fast} in \
             the faster one, {ratio:.3} of the time"
        );
    }
}

// The sprite drawer reads a sprite with POP, SP inside it, and had
// interrupts off for the whole of each sprite: 30% of them were lost, which
// made the sound late and the pacing count frames short. The faster build
// takes them inside sprites too, on a stack of its own, putting back the two
// bytes the return address lands on. Nearly every frame has to have its
// interrupt now, some of them taken inside a sprite, and nothing drawn may
// differ, which the pass-for-pass tests see to.
#[test]
fn interrupts_are_taken_inside_sprites_and_none_are_lost() {
    if !have(&[FAST, FAST_MAP, "roms/128.rom"]) {
        return;
    }
    let tick = address(FAST_MAP, "ay_tick");
    let saved = address(FAST_MAP, "ay_int_sp");
    let mut spec = machine(FAST);
    start_a_game(&mut spec);
    let frame = u64::from(spec.bus.model.frame_t());
    let (mut taken, mut in_sprites) = (0, 0);
    for i in 0..40 {
        let (row, bit) = WALK[i % WALK.len()];
        key(&mut spec, row, bit, true);
        let end = spec.bus.total_t() + 25 * frame;
        while spec.bus.total_t() < end {
            spec.step_instruction();
            if spec.cpu.pc == tick {
                taken += 1;
                // Where it came from is the SP ay_int saved.
                let from =
                    u16::from_le_bytes([spec.bus.peek_raw(saved), spec.bus.peek_raw(saved + 1)]);
                if from >> 8 != 0xF0 {
                    in_sprites += 1;
                }
            }
        }
        key(&mut spec, row, bit, false);
    }
    // Measured: 978. The rest come while a sprite's first row is read, before
    // DE holds anything to put back, or while ay_append has them off.
    assert!(taken >= 970, "{taken} interrupts in 1,000 frames");
    assert!(
        in_sprites > 50,
        "only {in_sprites} were taken inside a sprite"
    );
}

/// Passes made in 1,000 frames at `turbo`: walking about the first room from
/// the start, and left alone in the busy room at frame 90,000.
fn passes_in_1000_frames(snap: &str, map: &str, turbo: u32) -> (u16, u16) {
    let count = |s: &Spectrum| u16::from_le_bytes([s.bus.peek_raw(0x5B02), s.bus.peek_raw(0x5B03)]);
    let mut spec = machine(snap);
    start_a_game(&mut spec);
    spec.bus.turbo = turbo;
    let t = spec.bus.model.frame_t();
    let before = count(&spec);
    for i in 0..20 {
        let (row, bit) = WALK[i % WALK.len()];
        key(&mut spec, row, bit, true);
        for _ in 0..50 {
            spec.run(t);
        }
        key(&mut spec, row, bit, false);
    }
    let room_1 = count(&spec).wrapping_sub(before);
    let sna = std::fs::read("assemblies/out/busy90000.sna").unwrap();
    let mut busy = busy_machine(&sna, snap, map);
    busy.bus.turbo = turbo;
    let before = count(&busy);
    for _ in 0..1_000 {
        busy.run(t);
    }
    (room_1, count(&busy).wrapping_sub(before))
}

// The game's only pacing was a busy-wait, so the emulator's CPU turbo made it
// run away: at 4x the faster build goes round 785 times in 1,000 frames of the
// first room. The paced build waits for the interrupt to hold every pass to
// five frames, about the original's quickest, in any room. Before interrupts
// were let in during sprites it held 179, not 200: the frames it counted
// were short by the ones lost.
#[test]
fn the_paced_build_holds_five_frames_a_pass_at_turbo() {
    if !have(&[
        PACED,
        PACED_MAP,
        "assemblies/out/busy90000.sna",
        "roms/128.rom",
    ]) {
        return;
    }
    let (room_1, busy) = passes_in_1000_frames(PACED, PACED_MAP, 4);
    assert!(
        (195..=200).contains(&room_1) && (195..=200).contains(&busy),
        "at 4x, {room_1} passes in 1,000 frames of the first room and {busy} of a busy one; \
         a pass every five frames is 200"
    );
}

// The build that is played is the paced one with Step 2's banking on top,
// and R left in its random numbers, so no other test runs it. Starting a
// game goes through bank 3, so a banked build that was wrong would not get
// as far as a pass, let alone hold five frames each.
#[test]
fn the_banked_build_that_is_played_holds_five_frames_a_pass_too() {
    let (snap, map) = (
        "assemblies/Alien 8 AY banked start.szx",
        "assemblies/out/Alien 8 AY banked.map",
    );
    if !have(&[snap, map, "assemblies/out/busy90000.sna", "roms/128.rom"]) {
        return;
    }
    let (room_1, busy) = passes_in_1000_frames(snap, map, 4);
    assert!(
        (195..=200).contains(&room_1) && (195..=200).contains(&busy),
        "at 4x, {room_1} passes in 1,000 frames of the first room and {busy} of a busy one"
    );
}

// Pacing only holds a pass back. A busy room takes ten frames a pass at the
// machine's own speed, so there it must cost nothing.
#[test]
fn pacing_costs_a_busy_room_nothing_at_normal_speed() {
    if !have(&[
        PACED,
        PACED_MAP,
        FAST,
        FAST_MAP,
        "assemblies/out/busy90000.sna",
        "roms/128.rom",
    ]) {
        return;
    }
    let (_, paced) = passes_in_1000_frames(PACED, PACED_MAP, 1);
    let (_, fast) = passes_in_1000_frames(FAST, FAST_MAP, 1);
    assert!(
        paced + 2 >= fast,
        "in 1,000 frames of a busy room, {paced} passes paced and {fast} not"
    );
}

// Waiting changes when a pass happens, never what it does.
#[test]
fn the_paced_build_plays_the_same_game() {
    if !have(&[PACED, PACED_MAP, FAST, FAST_MAP, "roms/128.rom"]) {
        return;
    }
    let paced = play_by_pass(PACED, PACED_MAP, 300);
    let fast = play_by_pass(FAST, FAST_MAP, 300);
    for (pass, (a, b)) in fast.iter().zip(&paced).enumerate() {
        if let Some(at) = a.iter().zip(b).position(|(x, y)| x != y) {
            panic!(
                "pass {pass}: ${:04X} is ${:02X} in the faster build and ${:02X} paced",
                0x4000 + at,
                a[at],
                b[at]
            );
        }
    }
}

// Step 1 of assemblies/memory-plan.md: the snapshot writer puts a bank the
// assembler wrote into its page, and can leave the paging free, which the
// game has to have once it pages anything. Locked, as before, unless asked.
#[test]
fn a_snapshot_carries_a_bank_and_can_leave_paging_unlocked() {
    let needs = [
        "assemblies/snapshot.py",
        "assemblies/out/recording.z80",
        "assemblies/out/original.map",
    ];
    if !have(&[&needs[..], &[FAST_MAP, "roms/128.rom"]].concat()) {
        return;
    }
    let dir = std::env::temp_dir().join(format!("zxrs-bank-snapshot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let page: Vec<u8> = (0..0x4000u32).map(|i| (i * 7 % 251) as u8).collect();
    let bank = dir.join("bank3.bin");
    std::fs::write(&bank, &page).unwrap();
    let write = |name: &str, extra: &[&str]| {
        let out = dir.join(name);
        let run = std::process::Command::new("python3")
            .arg(needs[0])
            .args([
                needs[1],
                needs[2],
                "assemblies/out/fast fixed.bin",
                FAST_MAP,
            ])
            .arg(&out)
            .args(["--start", "--ay", "0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0"])
            .args(extra)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        machine(out.to_str().unwrap())
    };
    let bank_arg = format!("3={}", bank.display());
    let free = write("free.szx", &["--unlocked", "--bank", &bank_arg]);
    let locked = write("locked.szx", &[]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(&free.bus.ram[3 * 0x4000..4 * 0x4000], &page[..], "bank 3");
    assert!(!free.bus.paging_locked && locked.bus.paging_locked);
}

/// The screen, the variables and the objects, with the pointers in them that
/// say where a build keeps things made into offsets.
fn comparable(spec: &Spectrum, map: &str) -> Vec<u8> {
    let mut state: Vec<u8> = (0x4000..0x6288).map(|a| spec.bus.peek_raw(a)).collect();
    // Two words here point into the draw list at l_C745, which is not where
    // it was once the code before it has changed length: they are compared
    // as how far into the list they point.
    let list = address(map, "l_C745");
    for at in [0x5B2Bu16, 0x5B2D] {
        let i = usize::from(at - 0x4000);
        let word = u16::from_le_bytes([state[i], state[i + 1]]).wrapping_sub(list);
        state[i..i + 2].copy_from_slice(&word.to_le_bytes());
    }
    // $5B2F is where the pair test last looked. The faster build only writes
    // it for a pair it is putting in order, the one thing that reads it, so
    // at the end of a pass it holds an older place than the free build's and
    // means nothing.
    state[0x1B2F..0x1B31].fill(0);
    room_pointers_as_offsets(&mut state, map);
    // $5B09 is where draw_sprite keeps SP while it reads the sprite with POP,
    // and nothing else reads it. Step 4 moved the stack, and a sprite drawn
    // from bank 3 is drawn through a trampoline, four bytes deeper, so what
    // it holds afterwards says nothing about the game.
    state[0x1B09..0x1B0B].fill(0);
    state
}

/// Where the stack starts: reverse_table's page in bank 0, until Step 4 of
/// the memory plan moved it into fixed memory.
fn stack_top(map: &str) -> u16 {
    let moved = std::fs::read_to_string(map)
        .unwrap()
        .lines()
        .any(|l| l.starts_with("stack_top $"));
    if moved {
        address(map, "stack_top")
    } else {
        0xF100
    }
}

/// Where the room data starts: $6460, or bank 3 once Step 2 has moved it.
fn room_base(map: &str) -> u16 {
    let banked = std::fs::read_to_string(map)
        .unwrap()
        .lines()
        .any(|l| l.starts_with("room_data $"));
    if banked {
        address(map, "room_data")
    } else {
        0x6460
    }
}

/// $5B34 (the entry of the room the robot is in) points into the room data,
/// which Step 2 moves; in a state from $4000, it is made into how far into
/// it it points. So is $5B20, the table of object kinds a room is built
/// from, but the game uses $5B21 for something else once the room is built,
/// so when the word no longer points into the room data only its low byte
/// is. Getting that wrong one way and not the other can only make a
/// difference, never hide one.
fn room_pointers_as_offsets(state: &mut [u8], map: &str) {
    let base = room_base(map);
    let word = u16::from_le_bytes([state[0x1B34], state[0x1B35]]).wrapping_sub(base);
    state[0x1B34..0x1B36].copy_from_slice(&word.to_le_bytes());
    let kinds = u16::from_le_bytes([state[0x1B20], state[0x1B21]]).wrapping_sub(base);
    if kinds < 0x76E3 - 0x6460 {
        state[0x1B20..0x1B22].copy_from_slice(&kinds.to_le_bytes());
    } else {
        state[0x1B20] = kinds as u8;
    }
}

/// The first pass at which two runs part, with where and how many bytes.
fn first_difference(a: &[Vec<u8>], b: &[Vec<u8>]) -> Option<String> {
    a.iter().zip(b).enumerate().find_map(|(pass, (x, y))| {
        let at = x.iter().zip(y).position(|(p, q)| p != q)?;
        Some(format!(
            "pass {pass}: ${:04X} is ${:02X} and ${:02X}, {} bytes differ",
            0x4000 + at,
            x[at],
            y[at],
            x.iter().zip(y).filter(|(p, q)| p != q).count()
        ))
    })
}

// Step 2 of assemblies/memory-plan.md moves the room data and the routines
// that read it into bank 3, called with interrupts off. A game start reads
// it (build_room_list), and the walk goes through a door at pass 34, so both
// paths are in this; the busy rooms are the passes in between.
#[test]
fn the_banked_build_plays_the_same_game_pass_for_pass() {
    let states: Vec<String> = BUSY
        .iter()
        .map(|f| format!("assemblies/out/busy{f}.sna"))
        .collect();
    let mut need: Vec<&str> = states.iter().map(String::as_str).collect();
    need.extend([FAST, FAST_MAP, BANKED, BANKED_MAP, "roms/128.rom"]);
    if !have(&need) {
        return;
    }
    let passes = 2_000;
    let fast = play_by_pass(FAST, FAST_MAP, passes);
    let banked = play_by_pass(BANKED, BANKED_MAP, passes);
    assert_eq!(
        (fast.len(), banked.len()),
        (passes, passes),
        "the game ended early"
    );
    if let Some(what) = first_difference(&fast, &banked) {
        panic!("walking from the start of a game, {what}");
    }
    for (frame, state) in BUSY.iter().zip(&states) {
        let sna = std::fs::read(state).unwrap();
        let fast = play_on(busy_machine(&sna, FAST, FAST_MAP), FAST_MAP, 120, 0, false);
        let banked = play_on(
            busy_machine(&sna, BANKED, BANKED_MAP),
            BANKED_MAP,
            120,
            0,
            false,
        );
        assert_eq!(
            (fast.len(), banked.len()),
            (120, 120),
            "from frame {frame}, the game stopped"
        );
        if let Some(what) = first_difference(&fast, &banked) {
            panic!("from frame {frame} of the recording, {what}");
        }
    }
}

/// A machine in a game, stopped at the end of pass FIRST_PASS.
fn at_first_pass(snap: &str, map: &str) -> Spectrum {
    let mut spec = machine(snap);
    start_a_game(&mut spec);
    let end_of_pass = address(map, "end_of_pass");
    for _ in 0..50_000_000 {
        let count = u16::from_le_bytes([spec.bus.peek_raw(0x5B02), spec.bus.peek_raw(0x5B03)]);
        if spec.cpu.pc == end_of_pass && count >= FIRST_PASS {
            return spec;
        }
        spec.step_instruction();
    }
    panic!("{snap} never reached pass {FIRST_PASS}");
}

/// The room data as the game would read it, wherever it is kept: $6460 to
/// the valve list in the faster build, the start of bank 3 in the banked one.
fn room_data(spec: &Spectrum, banked: bool) -> Vec<u8> {
    (0..0x76E3 - 0x6460u16)
        .map(|o| {
            if banked {
                spec.bus.bank_byte(3, o)
            } else {
                spec.bus.peek_raw(0x6460 + o)
            }
        })
        .collect()
}

/// What building a room leaves: the variables and objects, the valve list,
/// the buffer and the room data, with the pointer tables in it taken out,
/// since those are moved with the data on purpose.
fn after_building(spec: &Spectrum, map: &str) -> Vec<u8> {
    let banked = room_base(map) != 0x6460;
    let mut out = comparable(spec, map);
    out.extend((0x76E3..0x7827).map(|a| spec.bus.peek_raw(a)));
    let buffer = address(map, "buffer");
    out.extend((buffer..buffer + 0x1800).map(|a| spec.bus.peek_raw(a)));
    let mut room = room_data(spec, banked);
    for (at, words) in [(0x73CAusize, 30), (0x740A, 7), (0x7519, 14)] {
        room[at - 0x6460..at - 0x6460 + 2 * words].fill(0);
    }
    out.extend(room);
    out
}

// The walk only ever goes through one door, so every room number the
// builder can be asked for is built in both, from the same game: the room's
// pieces and objects come out of the moved tables, and a pointer in them
// left unmoved would read the hole in fixed memory. A number that is no
// room clears the objects instead, and is compared as well. count_valves,
// which reads every room's entry for the score at the end, is run after.
#[test]
fn every_room_is_built_the_same_from_bank_3() {
    if !have(&[FAST, FAST_MAP, BANKED, BANKED_MAP, "roms/128.rom"]) {
        return;
    }
    let fast = at_first_pass(FAST, FAST_MAP);
    let banked = at_first_pass(BANKED, BANKED_MAP);
    assert_eq!(
        first_difference(
            &[after_building(&fast, FAST_MAP)],
            &[after_building(&banked, BANKED_MAP)]
        ),
        None,
        "the two games were not in the same state to begin with"
    );
    let mut built = std::collections::BTreeSet::new();
    for room in 0..=255u8 {
        let mut states = Vec::new();
        for (spec, map) in [(&fast, FAST_MAP), (&banked, BANKED_MAP)] {
            let mut spec = spec.clone();
            spec.bus.poke(0x5B90, room);
            spec.cpu.ix = 0x5B88;
            call(&mut spec, address(map, "build_room"), 0, 0, 0);
            if spec.bus.peek_raw(0x5C08) != 0 {
                built.insert(room);
            }
            call(&mut spec, address(map, "count_valves"), 0, 0, 0);
            states.push(after_building(&spec, map));
        }
        if let Some(what) = first_difference(&states[..1], &states[1..]) {
            panic!("room {room}: {what} (past $6287, an offset into the valve list, buffer and room data)");
        }
    }
    // Alien 8 has 128 rooms; a builder that never found one would pass the
    // comparison by clearing every room the same.
    assert_eq!(built.len(), 128, "room numbers that built a room");
}

// The one write a pass makes into the room data: when a room's chambers are
// all fitted, the low three bits of its entry are set, through the pointer
// room_found left at $5B34. In the banked build that pointer is into bank 3,
// and the write has to land there and not in the hole in fixed memory.
#[test]
fn fitting_a_room_marks_its_entry_in_bank_3() {
    if !have(&[BANKED, BANKED_MAP, "roms/128.rom"]) {
        return;
    }
    let mut spec = at_first_pass(BANKED, BANKED_MAP);
    let entry = u16::from_le_bytes([spec.bus.peek_raw(0x5B34), spec.bus.peek_raw(0x5B35)]);
    assert!((0xC000..0xD200).contains(&entry), "$5B34 is ${entry:04X}");
    let before = spec.bus.bank_byte(3, entry - 0xC000);
    let fixed = spec.bus.bank_byte(0, entry - 0xC000);
    call(&mut spec, address(BANKED_MAP, "room_fitted"), 0, 0, 0);
    assert_eq!(
        (
            spec.bus.bank_byte(3, entry - 0xC000),
            spec.bus.peek_raw(0x5B0D),
            spec.bus.bank_byte(0, entry - 0xC000)
        ),
        (before | 7, 7, fixed),
        "the entry in bank 3, the room's kind of chamber, and bank 0 at the same address"
    );
}

// Step 3 of assemblies/memory-plan.md moves the AY driver, the IM 2 table
// and the handler out of bank 0, so an interrupt can be taken with bank 3
// in, and the stubs into bank 3 leave interrupts on. Under Step 2 alone they
// turned them off, and the interrupt that came while a room was built was
// lost, a tick of the sound with it. Every room is built here with
// interrupts on, and every frame that starts while it is being built has
// to be counted by the driver.
#[test]
fn building_a_room_loses_no_interrupts() {
    if !have(&[BANKED, BANKED_MAP, "roms/128.rom"]) {
        return;
    }
    let start = at_first_pass(BANKED, BANKED_MAP);
    let build_room = address(BANKED_MAP, "build_room");
    let ticks_at = address(BANKED_MAP, "ay_ticks");
    let ticks =
        |s: &Spectrum| u16::from_le_bytes([s.bus.peek_raw(ticks_at), s.bus.peek_raw(ticks_at + 1)]);
    let (mut frames, mut counted) = (0u64, 0u16);
    for room in 0..=255u8 {
        let mut spec = start.clone();
        spec.bus.poke(0x5B90, room);
        spec.cpu.ix = 0x5B88;
        let sentinel = 0x0000;
        let top = spec.cpu.sp.wrapping_sub(64);
        spec.cpu.sp = top;
        spec.bus.poke(top, 0);
        spec.bus.poke(top + 1, 0);
        spec.cpu.pc = build_room;
        spec.cpu.iff1 = true;
        spec.cpu.iff2 = true;
        let (frame, tick) = (spec.bus.frame, ticks(&spec));
        let mut returned = false;
        for _ in 0..1_000_000 {
            spec.step_instruction();
            if spec.cpu.pc == sentinel && spec.cpu.sp == top + 2 {
                returned = true;
                break;
            }
        }
        assert!(returned, "building room {room} did not return");
        frames += spec.bus.frame - frame;
        counted = counted.wrapping_add(ticks(&spec).wrapping_sub(tick));
    }
    assert!(
        frames > 100,
        "only {frames} frames went by in 256 rooms: nothing measured"
    );
    assert_eq!(
        u64::from(counted),
        frames,
        "interrupts the driver counted, against frames that began, building 256 rooms"
    );
}

/// Runs `spec` until it fetches from `at`, or panics after `limit` instructions.
fn run_to(spec: &mut Spectrum, at: u16, limit: u64, what: &str) {
    for _ in 0..limit {
        if spec.cpu.pc == at {
            return;
        }
        spec.step_instruction();
    }
    panic!("never reached {what} (${at:04X})");
}

// Step 4 moves the end of a game into bank 3: the report, the count of rooms
// visited and the rating, the tune, and the screen that follows it, which
// leaves by jumping into main_loop for good. Nothing else plays a game to its
// end, so both builds are sent to game_over from the same game and compared
// with the report up and waiting for a key, as the main loop is entered
// again, and for the passes after.
#[test]
fn a_game_ends_the_same_from_bank_3() {
    if !have(&[FAST, FAST_MAP, BANKED, BANKED_MAP, "roms/128.rom"]) {
        return;
    }
    let mut ends = Vec::new();
    for (snap, map) in [(FAST, FAST_MAP), (BANKED, BANKED_MAP)] {
        let mut spec = at_first_pass(snap, map);
        spec.cpu.pc = address(map, "game_over");
        // read_keys is first called once the report is on the screen.
        run_to(
            &mut spec,
            address(map, "read_keys"),
            50_000_000,
            "the report",
        );
        let report = comparable(&spec, map);
        // No key is pressed: the wait after the report gives up on its own.
        // A key pressed on a given frame would land somewhere different in
        // each build, which run at different speeds, and what it interrupts
        // changes the screen after: the faster build against itself, with
        // the key twenty frames later, differs in 111 bytes.
        run_to(
            &mut spec,
            address(map, "main_loop"),
            200_000_000,
            "main_loop",
        );
        let after = comparable(&spec, map);
        let passes = play_on(spec, map, 60, 0, false);
        ends.push((report, after, passes));
    }
    let (fast, banked) = (&ends[0], &ends[1]);
    let attributes = &fast.0[0x1800..0x1B00];
    assert!(
        attributes.iter().any(|a| *a != attributes[0]),
        "nothing was drawn for the report"
    );
    assert_eq!(
        first_difference(
            std::slice::from_ref(&fast.0),
            std::slice::from_ref(&banked.0)
        ),
        None,
        "the report"
    );
    assert_eq!(
        first_difference(
            std::slice::from_ref(&fast.1),
            std::slice::from_ref(&banked.1)
        ),
        None,
        "back in main_loop"
    );
    assert_eq!(
        (fast.2.len(), banked.2.len()),
        (60, 60),
        "passes after the end of the game"
    );
    assert_eq!(
        first_difference(&fast.2, &banked.2),
        None,
        "the passes after"
    );
}

// Step 5 of the memory plan moves the bit-reversal and shift tables from
// $F100 to the 4K block at `tables`, and build_tables, which makes them at
// the start of a game, has its top page written in as a number. The image
// already holds the finished tables, so a builder writing them somewhere
// else would go unnoticed by everything that only reads them: here they
// are emptied and built again in both builds, and have to come out the same
// and where they are read.
#[test]
fn the_tables_are_built_where_they_are_read() {
    if !have(&[FAST, FAST_MAP, BANKED, BANKED_MAP, "roms/128.rom"]) {
        return;
    }
    let mut built = Vec::new();
    for (snap, map) in [(FAST, FAST_MAP), (BANKED, BANKED_MAP)] {
        let mut spec = machine(snap);
        let at = address(map, "reverse_table");
        for a in at..=at + 0xEFF {
            spec.bus.poke(a, 0);
        }
        call(&mut spec, address(map, "build_tables"), 0, 0, 0);
        built.push(
            (at..=at + 0xEFF)
                .map(|a| spec.bus.peek_raw(a))
                .collect::<Vec<u8>>(),
        );
    }
    assert!(
        built[0].iter().any(|b| *b != 0),
        "the faster build built nothing"
    );
    assert_eq!(
        first_difference(&built[..1], &built[1..]),
        None,
        "the tables, from reverse_table in each build"
    );
}

const STEP4: &str = "assemblies/out/banked step4.szx";
const STEP4_MAP: &str = "assemblies/out/banked step4.map";
const PROFILE: &str = "assemblies/out/profile.txt";

/// T-states spent at each label of the main image, while `spec` runs `frames`.
fn profile(
    spec: &mut Spectrum,
    map: &str,
    frames: u64,
    into: &mut std::collections::BTreeMap<String, u64>,
) {
    let mut names: Vec<(u16, String)> = std::fs::read_to_string(map)
        .unwrap()
        .lines()
        .filter(|l| l.split_whitespace().count() == 2)
        .filter_map(|l| {
            let (n, a) = l.split_once(" $")?;
            Some((u16::from_str_radix(a, 16).ok()?, n.to_string()))
        })
        .collect();
    names.sort();
    let end = spec.bus.frame + frames;
    while spec.bus.frame < end {
        let pc = spec.cpu.pc;
        let t = spec.bus.total_t();
        spec.step_instruction();
        // Above $C000 only bank 0 is the main image's.
        if pc < 0x4000 || pc >= 0xC000 && spec.bus.page_reg & 7 != 0 {
            continue;
        }
        let i = names.partition_point(|(a, _)| *a <= pc);
        if i > 0 {
            *into.entry(names[i - 1].1.clone()).or_default() += spec.bus.total_t() - t;
        }
    }
}

// Step 5 of the memory plan puts the hottest code in bank 2, which is not
// contended, and the rest in bank 5, which is. It first went by the calls a
// frame in the notes, which put mirror_row and mirror_store, a fifth of the
// time in a busy room, in contended memory. So it goes by this instead: the
// T-states each label took in the Step 4 build, walking about the first room
// and in the three busy rooms. Run by assemblies/fast.sh, which then builds
// Step 5 from it.
#[test]
#[ignore]
fn profile_the_layout_of_step_5_goes_by() {
    if !have(&[STEP4, STEP4_MAP, "roms/128.rom"]) {
        return;
    }
    let mut spent = std::collections::BTreeMap::new();
    let mut spec = at_first_pass(STEP4, STEP4_MAP);
    let t = spec.bus.model.frame_t();
    for i in 0..20 {
        let (row, bit) = WALK[i % WALK.len()];
        key(&mut spec, row, bit, true);
        profile(&mut spec, STEP4_MAP, 50, &mut spent);
        key(&mut spec, row, bit, false);
    }
    let _ = t;
    for frame in BUSY {
        let sna = std::fs::read(format!("assemblies/out/busy{frame}.sna")).unwrap();
        let mut spec = busy_machine(&sna, STEP4, STEP4_MAP);
        profile(&mut spec, STEP4_MAP, 1_000, &mut spent);
    }
    let total: u64 = spent.values().sum();
    assert!(total > 100_000_000, "only {total} T-states profiled");
    let mut out =
        String::from("label T-states (Step 4 build: first room walked, three busy rooms)\n");
    for (name, t) in &spent {
        out.push_str(&format!("{name} {t}\n"));
    }
    std::fs::write(PROFILE, out).unwrap();
}

// The CPU puts I and R on the address bus to refresh memory after every
// opcode fetch. With I at $40-$7F that is an address in contended memory:
// the ULA snows, this emulator slows every instruction down, and a real
// 128K is known to crash. Step 3 of the memory plan put the IM 2 table at
// $6500 and did all three; it was found as a 3% slowdown in Step 5. The
// table is in bank 2 now, and I has to stay out of contended memory for a
// whole game: started, walked, and through a room change.
#[test]
fn i_never_points_into_contended_memory() {
    if !have(&[BANKED, BANKED_MAP, "roms/128.rom"]) {
        return;
    }
    let mut spec = machine(BANKED);
    let t = spec.bus.model.frame_t();
    let mut seen = std::collections::BTreeSet::new();
    for f in 0..3_000 {
        key(&mut spec, 4, 0, (600..620).contains(&f));
        spec.run(t);
        if spec.cpu.im == 2 {
            seen.insert(spec.cpu.i);
        }
    }
    assert!(!seen.is_empty(), "the game never set IM 2");
    assert!(
        seen.iter().all(|i| !(0x40..0x80).contains(i)),
        "I was ${seen:02X?}"
    );
}
