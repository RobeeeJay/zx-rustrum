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
        .map(|a| u16::from_str_radix(a, 16).unwrap())
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
    let end_of_pass = address(map, "end_of_pass");
    let list = address(map, "l_C745");
    let mut out = Vec::new();
    let mut steps = 0u64;
    let mut since = 0u64;
    let mut last = None;
    while out.len() < passes {
        let walking = if out.is_empty() {
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
        if spec.cpu.pc == end_of_pass && count >= FIRST_PASS && last != Some(count) {
            last = Some(count);
            since = steps;
            let mut state: Vec<u8> = (0x4000..0x5B00).map(|a| spec.bus.peek_raw(a)).collect();
            state.extend((0x5B00..0x6288).map(|a| spec.bus.peek_raw(a)));
            // Two words here point into the draw list at l_C745, which is
            // not where it was once the code before it has changed length:
            // they are compared as how far into the list they point.
            for at in [0x5B2Bu16, 0x5B2D] {
                let i = usize::from(at - 0x4000);
                let word = u16::from_le_bytes([state[i], state[i + 1]]).wrapping_sub(list);
                state[i..i + 2].copy_from_slice(&word.to_le_bytes());
            }
            // $5B2F is where the pair test last looked. The faster build
            // only writes it for a pair it is putting in order, the one
            // thing that reads it, so at the end of a pass it holds an
            // older place than the free build's and means nothing.
            state[0x1B2F..0x1B31].fill(0);
            out.push(state);
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
    spec.cpu.sp = 0xF0F0;
    spec.bus.poke(0xF0F0, (sentinel & 0xFF) as u8);
    spec.bus.poke(0xF0F1, (sentinel >> 8) as u8);
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
        if spec.cpu.pc == sentinel && spec.cpu.sp == 0xF0F2 {
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
            after.push((
                flags,
                spec.cpu.d,
                spec.cpu.e,
                spec.cpu.h,
                spec.cpu.l,
                spec.cpu.iy,
                spec.cpu.b,
            ));
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
