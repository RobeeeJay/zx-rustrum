//! Knight Lore rebuilt for the 128K, checked the way the rebuild-for-128k
//! skill checks every stage: test variants with LD A,R taken out, played pass
//! for pass from the same pass with the same keys, and compared at the end of
//! every pass. The builds are in assemblies/knight-lore (not in the
//! repository); each test skips itself when they are absent.

use zx_rustrum::machine::{Model, Spectrum};

const STAGE1: &str = "assemblies/knight-lore/out/stage1 fixed.szx";
const STAGE1_MAP: &str = "assemblies/knight-lore/out/stage1 fixed.map";

/// The pass count end_of_pass keeps.
const PASS_COUNT: u16 = 0x5BA2;
/// The end of the forty objects from $5C08.
const OBJECTS_END: u16 = 0x6108;
/// The pass both builds are compared from.
const FIRST_PASS: u16 = 128;

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

fn passes(spec: &Spectrum) -> u16 {
    u16::from_le_bytes([
        spec.bus.peek_raw(PASS_COUNT),
        spec.bus.peek_raw(PASS_COUNT + 1),
    ])
}

/// From game_start through the menu, with 0 held long enough to be seen
/// between two notes of its tune, and into the game: until frame 1,500, or
/// half of FIRST_PASS, well short of where the builds are compared from.
fn start_a_game(spec: &mut Spectrum) {
    let t = spec.bus.model.frame_t();
    let before = passes(spec);
    for f in 0..1_500 {
        match f {
            600 => key(spec, 4, 0, true),
            620 => key(spec, 4, 0, false),
            _ => {}
        }
        spec.run(t);
        if passes(spec).wrapping_sub(before) >= FIRST_PASS / 2 {
            return;
        }
    }
}

/// Q, P, A and O in turn.
const WALK: [(usize, u8); 4] = [(2, 0), (5, 0), (1, 0), (5, 1)];

/// Where main_loop puts the stack. Below it, $5B00-$5B9F, is nothing but
/// the stack: no instruction names an address there. What is left there
/// after a call returns is return addresses, which differ between builds
/// that put code in different places, and nothing reads below SP, so it is
/// left out.
const STACK_TOP: u16 = 0x5BA0;

/// The screen, the variables and the forty objects, less the stack. Three
/// words, $5BCB, $5BCD and $5BCF, point into draw_list, which moves when the
/// code before it changes length, so they are compared as how far into it
/// they point.
fn comparable(spec: &Spectrum, map: &str) -> Vec<u8> {
    let mut state: Vec<u8> = (0x4000..OBJECTS_END)
        .map(|a| {
            if (0x5B00..STACK_TOP).contains(&a) {
                0
            } else {
                spec.bus.peek_raw(a)
            }
        })
        .collect();
    let list = address(map, "draw_list");
    for at in [0x5BCBu16, 0x5BCD, 0x5BCF] {
        let i = usize::from(at - 0x4000);
        let word = u16::from_le_bytes([state[i], state[i + 1]]).wrapping_sub(list);
        state[i..i + 2].copy_from_slice(&word.to_le_bytes());
    }
    state
}

/// What the game is at the end of each of `count` passes from FIRST_PASS,
/// walking.
fn play_by_pass(snap: &str, map: &str, count: usize) -> Vec<Vec<u8>> {
    play_on(machine(snap), map, count).0
}

/// The same from a machine, and the machine afterwards.
fn play_on(mut spec: Spectrum, map: &str, count: usize) -> (Vec<Vec<u8>>, Spectrum) {
    start_a_game(&mut spec);
    let end_of_pass = address(map, "end_of_pass");
    let mut out = Vec::new();
    let mut last = None;
    let mut steps = 0u64;
    while out.len() < count {
        let walking = WALK[out.len() / 30 % WALK.len()];
        for (row, bit) in WALK {
            key(&mut spec, row, bit, (row, bit) == walking);
        }
        spec.step_instruction();
        steps += 1;
        assert!(steps < 2_000_000_000, "the game stopped making passes");
        let count = passes(&spec);
        if spec.cpu.pc == end_of_pass && count >= FIRST_PASS && last != Some(count) {
            last = Some(count);
            out.push(comparable(&spec, map));
        }
    }
    (out, spec)
}

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

// Everything after this compares one build with another pass for pass, so
// the harness has to be shown to work first: a game started from the menu,
// walked for 300 passes, comes out the same twice over, and the player has
// moved, so the keys reach it and the passes are real.
#[test]
fn the_same_build_plays_the_same_game_twice() {
    if !have(&[STAGE1, STAGE1_MAP, "roms/128.rom"]) {
        return;
    }
    let a = play_by_pass(STAGE1, STAGE1_MAP, 300);
    let b = play_by_pass(STAGE1, STAGE1_MAP, 300);
    assert_eq!(first_difference(&a, &b), None, "the same build, twice");
    // The player's legs, x and y, at $5C09 and $5C0A.
    let at = |p: &Vec<u8>| (p[0x1C09], p[0x1C0A]);
    let moved = a.iter().filter(|p| at(p) != at(&a[0])).count();
    assert!(
        moved > 100,
        "the player moved on only {moved} of 300 passes"
    );
}

// The original keyboard read does OUT ($FD),A with the key row as the port's
// high byte, and on a 128K a row with bit 7 clear is the paging port: $7E
// pages bank 6 over the game's code at $C000 and locks the paging. Started on
// a 128K, the original made one pass and ran off the end of memory. Every
// 128K build has a NOP there instead, and the paging has to be as it started
// after a game of walking about.
#[test]
fn reading_the_keyboard_leaves_the_paging_alone() {
    if !have(&[STAGE1, STAGE1_MAP, "roms/128.rom"]) {
        return;
    }
    let (played, spec) = play_on(machine(STAGE1), STAGE1_MAP, 300);
    assert_eq!(played.len(), 300);
    assert_eq!(
        (spec.bus.page_reg, spec.bus.paging_locked),
        (0x10, false),
        "$7FFD after 300 passes"
    );
}

const BANKED: &str = "assemblies/knight-lore/out/banked fixed.szx";
const BANKED_MAP: &str = "assemblies/knight-lore/out/banked fixed.map";

/// A machine in a game, stopped at the end of pass FIRST_PASS.
fn at_first_pass(snap: &str, map: &str) -> Spectrum {
    let mut spec = machine(snap);
    start_a_game(&mut spec);
    let end_of_pass = address(map, "end_of_pass");
    for _ in 0..500_000_000u64 {
        if spec.cpu.pc == end_of_pass && passes(&spec) >= FIRST_PASS {
            return spec;
        }
        spec.step_instruction();
    }
    panic!("{snap} never reached pass {FIRST_PASS}");
}

/// Calls `at` and runs until it returns, putting SP, PC and the interrupt
/// state back afterwards.
fn call(spec: &mut Spectrum, at: u16) {
    let (was, pc, iff1, iff2) = (spec.cpu.sp, spec.cpu.pc, spec.cpu.iff1, spec.cpu.iff2);
    let top = was.wrapping_sub(16);
    spec.cpu.sp = top;
    spec.bus.poke(top, 0);
    spec.bus.poke(top + 1, 0);
    spec.cpu.pc = at;
    spec.cpu.iff1 = false;
    spec.cpu.iff2 = false;
    for _ in 0..50_000_000 {
        spec.step_instruction();
        if spec.cpu.pc == 0 && spec.cpu.sp == top + 2 {
            (spec.cpu.sp, spec.cpu.pc, spec.cpu.iff1, spec.cpu.iff2) = (was, pc, iff1, iff2);
            return;
        }
    }
    panic!("${at:04X} did not return");
}

// Step 1 of the 128K banks moves the room data and build_room_objects, its
// only reader, to bank 3, behind a stub that pages it in and out. Walking
// never leaves the first room, but every pass draws it and every object in
// it moves, so the banked build has to play the same game as stage 1.
#[test]
fn the_room_bank_plays_the_same_game_pass_for_pass() {
    if !have(&[STAGE1, STAGE1_MAP, BANKED, BANKED_MAP, "roms/128.rom"]) {
        return;
    }
    let stage1 = play_by_pass(STAGE1, STAGE1_MAP, 300);
    let banked = play_by_pass(BANKED, BANKED_MAP, 300);
    assert_eq!(
        first_difference(&stage1, &banked),
        None,
        "walking from the start of a game"
    );
}

/// What building a room leaves: the variables and objects, the charm list,
/// and the draw buffer.
fn after_building(spec: &Spectrum) -> Vec<u8> {
    (STACK_TOP..0x6248u16)
        .chain(0x6FF2..0x7112)
        .chain(0xD8F3..0xF0F3)
        .map(|a| spec.bus.peek_raw(a))
        .collect()
}

// So every room number is built in both, from the same game, through the
// stub: the pieces and objects come out of the moved data through its moved
// pointer words, and a word left pointing at $6xxx would read the freed
// space. A number that is no room clears the objects, and is compared too.
#[test]
fn every_room_is_built_the_same_from_bank_3() {
    if !have(&[STAGE1, STAGE1_MAP, BANKED, BANKED_MAP, "roms/128.rom"]) {
        return;
    }
    let starts = [
        at_first_pass(STAGE1, STAGE1_MAP),
        at_first_pass(BANKED, BANKED_MAP),
    ];
    assert_eq!(
        after_building(&starts[0]),
        after_building(&starts[1]),
        "the two games were not in the same state to begin with"
    );
    let mut different = 0;
    for room in 0..=255u8 {
        let built: Vec<Vec<u8>> = starts
            .iter()
            .zip([STAGE1_MAP, BANKED_MAP])
            .map(|(start, map)| {
                let mut spec = start.clone();
                spec.bus.poke(0x5C10, room);
                spec.cpu.ix = 0x5C08;
                call(&mut spec, address(map, "build_room"));
                assert_eq!(spec.bus.page_reg, 0x10, "bank 0 back after room {room}");
                after_building(&spec)
            })
            .collect();
        if let Some(i) = built[0].iter().zip(&built[1]).position(|(a, b)| a != b) {
            panic!(
                "room {room}: byte {i} is ${:02X} and ${:02X}",
                built[0][i], built[1][i]
            );
        }
        different += usize::from(built[0] != after_building(&starts[0]));
    }
    assert!(
        different > 100,
        "only {different} rooms built anything different"
    );
}

const AY_FREE: &str = "assemblies/knight-lore/out/ay free fixed.szx";
const AY_FREE_MAP: &str = "assemblies/knight-lore/out/ay free fixed.map";

// Stage 3 puts the sound on the AY, queued and played by an IM 2 interrupt,
// instead of the beeper's waiting loops, and moves three sprites to make room
// for the interrupt's table. The sound is no part of the game, so the game
// has to be the same pass for pass as the room-bank build it was made from.
#[test]
fn the_ay_build_plays_the_same_game_pass_for_pass() {
    if !have(&[BANKED, BANKED_MAP, AY_FREE, AY_FREE_MAP, "roms/128.rom"]) {
        return;
    }
    let banked = play_by_pass(BANKED, BANKED_MAP, 300);
    let ay = play_by_pass(AY_FREE, AY_FREE_MAP, 300);
    assert_eq!(
        first_difference(&banked, &ay),
        None,
        "walking from the start of a game"
    );
}

// And the sound has to be there: the interrupt taken nearly every frame
// (only the sprite drawer has it off), the queue moving, and the AY given
// tones while the player walks about, footsteps and all.
#[test]
fn the_ay_build_plays_its_sound_on_the_interrupt() {
    if !have(&[AY_FREE, AY_FREE_MAP, "roms/128.rom"]) {
        return;
    }
    let mut spec = at_first_pass(AY_FREE, AY_FREE_MAP);
    let ticks_at = address(AY_FREE_MAP, "ay_ticks");
    let ticks =
        |s: &Spectrum| u16::from_le_bytes([s.bus.peek_raw(ticks_at), s.bus.peek_raw(ticks_at + 1)]);
    let (frame, tick) = (spec.bus.frame, ticks(&spec));
    let t = spec.bus.model.frame_t();
    let mut tones = std::collections::BTreeSet::new();
    for f in 0..1_000usize {
        let (row, bit) = WALK[f / 50 % WALK.len()];
        for (r, b) in WALK {
            key(&mut spec, r, b, (r, b) == (row, bit));
        }
        spec.run(t);
        let regs = spec.bus.audio.ay.regs;
        tones.insert(u16::from_le_bytes([regs[0], regs[1] & 0x0F]));
    }
    let (frames, taken) = (
        spec.bus.frame - frame,
        u64::from(ticks(&spec).wrapping_sub(tick)),
    );
    eprintln!(
        "{taken} interrupts in {frames} frames; {} tones",
        tones.len()
    );
    // A third are lost while the sprite drawer has interrupts off, as in
    // Alien 8's stage 3; stage 4 takes them inside sprites.
    assert!(
        taken * 10 >= frames * 6,
        "{taken} interrupts taken in {frames} frames"
    );
    assert_eq!(spec.cpu.im, 2, "the interrupt mode");
    assert!(
        tones.len() > 3,
        "only {} tones set while walking",
        tones.len()
    );
}

// Stage 3 moved sprites 5D, 5E and 5F out of the way of the interrupt's table,
// and the sprite table's words for them with them. Walking may never draw
// them, so every one of the 189 object types is drawn through the game's own
// draw_sprite in both builds, aligned and not, and the buffers compared: a
// word left pointing where a sprite was would draw the interrupt's table.
#[test]
fn every_sprite_draws_the_same_after_the_move() {
    if !have(&[BANKED, BANKED_MAP, AY_FREE, AY_FREE_MAP, "roms/128.rom"]) {
        return;
    }
    let starts = [
        (at_first_pass(BANKED, BANKED_MAP), BANKED_MAP),
        (at_first_pass(AY_FREE, AY_FREE_MAP), AY_FREE_MAP),
    ];
    let mut drawn = 0;
    for kind in 0..189u8 {
        for x in [0x40u8, 0x43] {
            let buffers: Vec<Vec<u8>> = starts
                .iter()
                .map(|(start, map)| {
                    let mut spec = start.clone();
                    let buffer = address(map, "buffer");
                    for i in 0..0x1800 {
                        spec.bus.poke(buffer + i, 0);
                    }
                    let ix = address(map, "l_BFDB");
                    spec.bus.poke(ix, kind);
                    spec.bus.poke(ix + 7, 0);
                    spec.bus.poke(ix + 0x1A, x);
                    spec.bus.poke(ix + 0x1B, 0x40);
                    spec.cpu.ix = ix;
                    call(&mut spec, address(map, "draw_sprite"));
                    (buffer..buffer + 0x1800)
                        .map(|a| spec.bus.peek_raw(a))
                        .collect()
                })
                .collect();
            if let Some(i) = buffers[0].iter().zip(&buffers[1]).position(|(a, b)| a != b) {
                panic!(
                    "type ${kind:02X} at x ${x:02X}: buffer byte {i} is ${:02X} and ${:02X}",
                    buffers[0][i], buffers[1][i]
                );
            }
            drawn += usize::from(buffers[0].iter().any(|b| *b != 0));
        }
    }
    assert!(
        drawn > 200,
        "only {drawn} of 378 draws put anything in the buffer"
    );
}

const BUSY: [u32; 3] = [9_000, 25_000, 91_000];

/// A build with the game's state from `sna` (the original at end_of_pass in a
/// busy room) carried into it: the screen, the variables and objects, and the
/// buffer, which is wherever the build's map says. Started at end_of_pass,
/// with a Kempston stick fitted as the recording was played with (none would
/// leave the game reading the floating bus, which depends on timing), and the
/// AY's interrupt where the build has one.
fn busy_machine(sna: &[u8], snap: &str, map: &str) -> Spectrum {
    let mut spec = machine(snap);
    for a in 0x4000..OBJECTS_END {
        spec.bus.poke(a, sna[27 + usize::from(a) - 0x4000]);
    }
    let buffer = address(map, "buffer");
    for i in 0..0x1800u16 {
        spec.bus
            .poke(buffer + i, sna[27 + 0xD8F3 - 0x4000 + usize::from(i)]);
    }
    let list = address(map, "draw_list");
    for at in [0x5BCB, 0x5BCD, 0x5BCF] {
        spec.bus.poke(at, list as u8);
        spec.bus.poke(at + 1, (list >> 8) as u8);
    }
    spec.bus.set_joystick(zx_rustrum::joystick::Kind::Kempston);
    spec.cpu.pc = address(map, "end_of_pass");
    spec.cpu.sp = STACK_TOP;
    let ay = std::fs::read_to_string(map)
        .unwrap()
        .contains("\nay_vectors $");
    if ay {
        spec.cpu.i = (address(map, "ay_vectors") >> 8) as u8;
        spec.cpu.im = 2;
    }
    spec.cpu.iff1 = ay;
    spec.cpu.iff2 = ay;
    spec
}

/// What the game is at the end of each of `count` passes, keys left alone.
fn play_still(mut spec: Spectrum, map: &str, count: usize) -> Vec<Vec<u8>> {
    let end_of_pass = address(map, "end_of_pass");
    let (mut out, mut last, mut steps) = (Vec::new(), None, 0u64);
    while out.len() < count {
        spec.step_instruction();
        steps += 1;
        assert!(steps < 2_000_000_000, "the game stopped making passes");
        let n = passes(&spec);
        if spec.cpu.pc == end_of_pass && last != Some(n) {
            last = Some(n);
            out.push(comparable(&spec, map));
        }
    }
    out
}

// Walking never leaves the first room, which has a dozen objects. The three
// busiest rooms of the recording have 38 to 40, the drawing order and the
// copying at their busiest, and every build has to play the same game in
// each as stage 1 does.
#[test]
fn every_build_plays_the_same_game_in_busy_rooms() {
    let builds = [
        (STAGE1, STAGE1_MAP),
        (BANKED, BANKED_MAP),
        (AY_FREE, AY_FREE_MAP),
        (FAST, FAST_MAP),
        (DRAWING1, DRAWING1_MAP),
        (DRAWING, DRAWING_MAP),
        (MEMORY1, MEMORY1_MAP),
        (MEMORY2, MEMORY2_MAP),
        (MEMORY, MEMORY_MAP),
    ];
    let mut need: Vec<String> = BUSY
        .iter()
        .map(|f| format!("assemblies/knight-lore/out/busy{f}.sna"))
        .collect();
    need.extend(
        builds
            .iter()
            .flat_map(|(s, m)| [s.to_string(), m.to_string()]),
    );
    need.push("roms/128.rom".into());
    if !have(&need.iter().map(String::as_str).collect::<Vec<_>>()) {
        return;
    }
    let mut changed = 0;
    for frame in BUSY {
        let sna = std::fs::read(format!("assemblies/knight-lore/out/busy{frame}.sna")).unwrap();
        let runs: Vec<Vec<Vec<u8>>> = builds
            .iter()
            .map(|(s, m)| play_still(busy_machine(&sna, s, m), m, 120))
            .collect();
        for (i, run) in runs.iter().enumerate().skip(1) {
            if let Some(what) = first_difference(&runs[0], run) {
                panic!(
                    "from frame {frame}, {} against stage 1: {what}",
                    builds[i].0
                );
            }
        }
        // Busy, and alive: the objects in use, and the game moving on.
        let objects = |p: &Vec<u8>| (0..40usize).filter(|o| p[0x1C08 + o * 32] != 0).count();
        let (first, last) = (&runs[0][0], &runs[0][119]);
        assert!(
            objects(first) >= 30,
            "from frame {frame}: only {} objects",
            objects(first)
        );
        // Room 240 (frame 9,000) is mostly fixed blocks, and only the
        // player and one other object change; the other two have a dozen or
        // more moving.
        let moved = (0x1C08..0x2108).filter(|i| first[*i] != last[*i]).count();
        assert!(
            moved > 0,
            "from frame {frame}: nothing in the objects changed in 120 passes"
        );
        changed += moved;
    }
    assert!(
        changed > 60,
        "{changed} bytes of the objects changed in all three rooms"
    );
}

const FAST: &str = "assemblies/knight-lore/out/fast fixed.szx";
const FAST_MAP: &str = "assemblies/knight-lore/out/fast fixed.map";

/// Calls `at` with BC, DE and HL set, as `call` does.
fn call_with(spec: &mut Spectrum, at: u16, bc: u16, de: u16, hl: u16) {
    spec.cpu.b = (bc >> 8) as u8;
    spec.cpu.c = bc as u8;
    spec.cpu.d = (de >> 8) as u8;
    spec.cpu.e = de as u8;
    spec.cpu.h = (hl >> 8) as u8;
    spec.cpu.l = hl as u8;
    call(spec, at);
}

// Stage 4 makes the drawing cheaper with Alien 8's changes where Knight Lore's
// routines are the same (copy_rect, the sound queue's running total, the scan
// that marks what to redraw), and takes interrupts inside sprites. None of
// it may change the game.
#[test]
fn the_fast_build_plays_the_same_game_pass_for_pass() {
    if !have(&[AY_FREE, AY_FREE_MAP, FAST, FAST_MAP, "roms/128.rom"]) {
        return;
    }
    let ay = play_by_pass(AY_FREE, AY_FREE_MAP, 300);
    let fast = play_by_pass(FAST, FAST_MAP, 300);
    assert_eq!(
        first_difference(&ay, &fast),
        None,
        "walking from the start of a game"
    );
}

// copy_rect works up the screen from a rectangle's bottom line, and a row
// ending at the right-hand edge on the last line of a third carries into D as
// the LDIs count on, which play rarely does. Every column, width up to ten and
// several bottoms are copied by both builds' copy_rect and the screens compared.
#[test]
fn copy_rect_copies_every_rectangle_as_the_old_one_did() {
    if !have(&[AY_FREE, AY_FREE_MAP, FAST, FAST_MAP, "roms/128.rom"]) {
        return;
    }
    let mut old = machine(AY_FREE);
    let mut new = machine(FAST);
    let (at_old, at_new) = (
        address(AY_FREE_MAP, "copy_rect"),
        address(FAST_MAP, "copy_rect"),
    );
    let (buf_old, buf_new) = (address(AY_FREE_MAP, "buffer"), address(FAST_MAP, "buffer"));
    for (spec, buffer) in [(&mut old, buf_old), (&mut new, buf_new)] {
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
            for (bottom, rows) in [
                (7u16, 8u16),
                (63, 20),
                (64, 1),
                (191, 30),
                (127, 9),
                (100, 13),
                (130, 40),
                (66, 3),
                (191, 192),
            ] {
                let row = (191 - bottom) * 32 + x;
                let (bc, de) = (rows << 8 | width, screen(x, bottom));
                for i in 0x4000..0x5800u16 {
                    old.bus.poke(i, 0);
                    new.bus.poke(i, 0);
                }
                call_with(&mut old, at_old, bc, de, buf_old + row);
                call_with(&mut new, at_new, bc, de, buf_new + row);
                tried += 1;
                if let Some(a) =
                    (0x4000..0x5800u16).find(|a| old.bus.peek_raw(*a) != new.bus.peek_raw(*a))
                {
                    panic!(
                        "{width} wide, {rows} rows up from line {bottom} at column {x}: ${a:04X} is ${:02X} and ${:02X}",
                        old.bus.peek_raw(a),
                        new.bus.peek_raw(a)
                    );
                }
            }
        }
    }
    assert!(tried > 1_000, "{tried}");
}

// ay_total stands in for walking the queue on every click, so at the end of
// every pass it has to be what that walk would find.
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
    // The route play_on walks, which keeps the game going for 300 passes:
    // another one ends at pass 284 with the game sat in read_keys.
    let mut spec = machine(FAST);
    start_a_game(&mut spec);
    let (mut checked, mut sounding, mut steps, mut last) = (0, 0, 0u64, None);
    while checked < 300 {
        let walking = WALK[checked / 30 % WALK.len()];
        for (r, b) in WALK {
            key(&mut spec, r, b, (r, b) == walking);
        }
        spec.step_instruction();
        steps += 1;
        assert!(
            steps < 2_000_000_000,
            "the game stopped making passes after {checked}, at ${:04X}",
            spec.cpu.pc
        );
        let count = passes(&spec);
        if spec.cpu.pc != end_of_pass || count < FIRST_PASS || last == Some(count) {
            continue;
        }
        last = Some(count);
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

/// Over 1,000 frames walking from the first pass: how many frames had their
/// interrupt reach ay_tick, and how many of those came from inside a sprite,
/// told by the SP ay_int saved being off the game's stack ($5B00-$5BA1).
fn interrupts_in_1000_frames(snap: &str, map: &str) -> (usize, usize) {
    let tick = address(map, "ay_tick");
    let saved = MAPS_WITH_AY_INT
        .contains(&map)
        .then(|| address(map, "ay_int_sp"));
    let mut spec = at_first_pass(snap, map);
    let (mut taken, mut in_sprites) = (0, 0);
    for f in 0..1_000 {
        let (row, bit) = WALK[f / 25 % WALK.len()];
        for (r, b) in WALK {
            key(&mut spec, r, b, (r, b) == (row, bit));
        }
        let frame = spec.bus.frame;
        let mut got = false;
        while spec.bus.frame == frame {
            spec.step_instruction();
            if spec.cpu.pc == tick && !got {
                got = true;
                if let Some(saved) = saved {
                    let from = u16::from_le_bytes([
                        spec.bus.peek_raw(saved),
                        spec.bus.peek_raw(saved + 1),
                    ]);
                    in_sprites += usize::from(!(0x5B00..0x5BA2).contains(&from));
                }
            }
        }
        taken += usize::from(got);
    }
    (taken, in_sprites)
}

const MAPS_WITH_AY_INT: [&str; 1] = [FAST_MAP];

// The drawer reads a sprite with POP, SP inside it, and the AY-free build had
// interrupts off for the whole sprite, so nearly a third of frames had no
// interrupt: the sound ran late and the pacing would count frames short. The
// handler now runs on a stack of its own and writes back the two bytes the
// last POP read, so interrupts are taken inside sprites. Nearly every frame
// has to have its interrupt, some taken inside a sprite; that nothing drawn
// is spoiled is the pass-for-pass tests' to say.
#[test]
fn interrupts_are_taken_inside_sprites_and_none_are_lost() {
    if !have(&[AY_FREE, AY_FREE_MAP, FAST, FAST_MAP, "roms/128.rom"]) {
        return;
    }
    let (before, _) = interrupts_in_1000_frames(AY_FREE, AY_FREE_MAP);
    let (taken, in_sprites) = interrupts_in_1000_frames(FAST, FAST_MAP);
    // Measured: 701 before, 965 after (Alien 8's took 978). The rest come
    // while a sprite's first row is read, before DE holds anything to put
    // back, or while ay_append or the driver itself has them off.
    assert!(
        before < 850,
        "the AY-free build took {before} of 1,000, so this measures nothing"
    );
    assert!(
        taken >= 950,
        "{taken} interrupts in 1,000 frames, against {before} before ({in_sprites} in sprites)"
    );
    assert!(
        in_sprites > 50,
        "only {in_sprites} were taken inside a sprite"
    );
}

/// Passes made in 1,000 frames from a busy room, keys left alone.
fn passes_in_1000_frames(sna: &[u8], snap: &str, map: &str) -> u16 {
    let mut spec = busy_machine(sna, snap, map);
    let t = spec.bus.model.frame_t();
    let before = passes(&spec);
    for _ in 0..1_000 {
        spec.run(t);
    }
    passes(&spec).wrapping_sub(before)
}

// Stage 4 bought the interrupts back, and almost nothing in speed: in the
// busy rooms the AY-free build made 209, 179 and 207 passes in 1,000 frames
// and this one 209, 180 and 207. copy_rect saves 1% of the time; walking
// the slots with HL saves nothing when 38 to 40 of them are live; and taking
// interrupts inside sprites costs about what copy_rect saves. Alien 8's
// gain was mostly depth_order, which Knight Lore's pair test does not take.
// So what is asked here is only that it is not slower.
#[test]
fn the_fast_build_is_no_slower_in_busy_rooms() {
    let mut need: Vec<String> = BUSY
        .iter()
        .map(|f| format!("assemblies/knight-lore/out/busy{f}.sna"))
        .collect();
    need.extend([AY_FREE, AY_FREE_MAP, FAST, FAST_MAP, "roms/128.rom"].map(String::from));
    if !have(&need.iter().map(String::as_str).collect::<Vec<_>>()) {
        return;
    }
    for frame in BUSY {
        let sna = std::fs::read(format!("assemblies/knight-lore/out/busy{frame}.sna")).unwrap();
        let before = passes_in_1000_frames(&sna, AY_FREE, AY_FREE_MAP);
        let after = passes_in_1000_frames(&sna, FAST, FAST_MAP);
        assert!(
            after >= before,
            "from frame {frame}: {after} passes in 1,000 frames against {before}"
        );
    }
}

const DRAWING1: &str = "assemblies/knight-lore/out/drawing1 fixed.szx";
const DRAWING1_MAP: &str = "assemblies/knight-lore/out/drawing1 fixed.map";

// drawing.py's first step stops the pair test as soon as one axis settles
// it, instead of working out all three and looking the case up. It is the
// same subtractions on the same bytes, so the game has to be the same.
#[test]
fn the_pair_test_that_stops_early_plays_the_same_game() {
    if !have(&[FAST, FAST_MAP, DRAWING1, DRAWING1_MAP, "roms/128.rom"]) {
        return;
    }
    let fast = play_by_pass(FAST, FAST_MAP, 300);
    let drawing = play_by_pass(DRAWING1, DRAWING1_MAP, 300);
    assert_eq!(
        first_difference(&fast, &drawing),
        None,
        "walking from the start of a game"
    );
}

/// Passes in 1,000 frames from each busy room, for each build given.
fn busy_speeds(builds: &[(&str, &str)]) -> Option<Vec<Vec<u16>>> {
    let mut need: Vec<String> = BUSY
        .iter()
        .map(|f| format!("assemblies/knight-lore/out/busy{f}.sna"))
        .collect();
    need.extend(
        builds
            .iter()
            .flat_map(|(s, m)| [s.to_string(), m.to_string()]),
    );
    need.push("roms/128.rom".into());
    if !have(&need.iter().map(String::as_str).collect::<Vec<_>>()) {
        return None;
    }
    Some(
        BUSY.iter()
            .map(|frame| {
                let sna =
                    std::fs::read(format!("assemblies/knight-lore/out/busy{frame}.sna")).unwrap();
                builds
                    .iter()
                    .map(|(s, m)| passes_in_1000_frames(&sna, s, m))
                    .collect()
            })
            .collect(),
    )
}

// Play reaches only some of the ways two boxes can lie: a charm sitting in
// something else's place, which is depth_overlap, hardly ever. So the objects
// of the busiest room are scattered about at random, a few made charms, all
// marked to be drawn, and both builds put them in order and draw them: the
// buffer and the objects (depth_overlap rewrites a type) have to agree.
#[test]
fn every_scattered_room_is_drawn_in_the_same_order() {
    let sna_path = "assemblies/knight-lore/out/busy91000.sna";
    if !have(&[
        sna_path,
        FAST,
        FAST_MAP,
        DRAWING1,
        DRAWING1_MAP,
        "roms/128.rom",
    ]) {
        return;
    }
    let sna = std::fs::read(sna_path).unwrap();
    let builds = [(FAST, FAST_MAP), (DRAWING1, DRAWING1_MAP)];
    let mut machines: Vec<Spectrum> = builds
        .iter()
        .map(|(s, m)| busy_machine(&sna, s, m))
        .collect();
    let mut seed = 0x2545_F491u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let (mut overlaps, mut drawn) = (0, 0);
    for trial in 0..300 {
        let mut objects = sna[27 + 0x1C08..27 + 0x2108].to_vec();
        for slot in objects.chunks_mut(32) {
            if slot[0] == 0 {
                continue;
            }
            // x, y and height moved by up to 20 either way.
            for at in &mut slot[1..=3] {
                *at = at.wrapping_add((next() % 41) as u8).wrapping_sub(20);
            }
            if next() % 8 == 0 {
                slot[0] = 0x60 + (next() % 7) as u8;
            }
            slot[7] |= 0x10;
        }
        let mut after = Vec::new();
        for ((_, map), spec) in builds.iter().zip(machines.iter_mut()) {
            for (i, b) in objects.iter().enumerate() {
                spec.bus.poke(0x5C08 + i as u16, *b);
            }
            let buffer = address(map, "buffer");
            for i in 0..0x1800u16 {
                spec.bus.poke(buffer + i, 0);
            }
            call(spec, address(map, "list_to_draw"));
            call(spec, address(map, "draw_in_order"));
            let mut state: Vec<u8> = (0..0x1800u16)
                .map(|i| spec.bus.peek_raw(buffer + i))
                .collect();
            state.extend((0x5C08..0x6108u16).map(|a| spec.bus.peek_raw(a)));
            after.push(state);
        }
        if let Some(at) = after[0].iter().zip(&after[1]).position(|(a, b)| a != b) {
            panic!(
                "trial {trial}: {} byte {at:#06X} is ${:02X} in the fast build and ${:02X} after",
                if at < 0x1800 { "buffer" } else { "objects" },
                after[0][at],
                after[1][at]
            );
        }
        overlaps += (0..40)
            .filter(|o| after[0][0x1800 + o * 32] == 0xBB && objects[o * 32] != 0xBB)
            .count();
        drawn += usize::from(after[0][..0x1800].iter().any(|b| *b != 0));
    }
    assert!(drawn > 250, "only {drawn} of 300 rooms drew anything");
    assert!(
        overlaps > 10,
        "only {overlaps} charms were found overlapping something"
    );
}

// The point of the pair test stopping early. Measured: 209, 180 and 207
// passes in 1,000 frames in the busy rooms became 228, 190 and 215, the
// most where the most objects are drawn in the most overlapping places.
#[test]
fn the_pair_test_that_stops_early_is_faster_in_every_busy_room() {
    let Some(speeds) = busy_speeds(&[(FAST, FAST_MAP), (DRAWING1, DRAWING1_MAP)]) else {
        return;
    };
    for (frame, s) in BUSY.iter().zip(&speeds) {
        assert!(
            u32::from(s[1]) * 100 >= u32::from(s[0]) * 103,
            "from frame {frame}: {} passes against the fast build's {}; all: {speeds:?}",
            s[1],
            s[0]
        );
    }
}

const DRAWING: &str = "assemblies/knight-lore/out/drawing fixed.szx";
const DRAWING_MAP: &str = "assemblies/knight-lore/out/drawing fixed.map";

// drawing.py's second step draws only the rows of a sprite that meet a place
// cleared this pass. What it leaves out was drawn into the buffer and never
// copied, so the screen has to be the same, pass for pass. The walk goes
// through three rooms (67, 69 and 68), so a room drawn whole after a door is
// in it as well.
#[test]
fn drawing_only_the_rows_that_show_leaves_the_screen_the_same() {
    if !have(&[DRAWING1, DRAWING1_MAP, DRAWING, DRAWING_MAP, "roms/128.rom"]) {
        return;
    }
    let before = play_by_pass(DRAWING1, DRAWING1_MAP, 300);
    let after = play_by_pass(DRAWING, DRAWING_MAP, 300);
    assert_eq!(
        first_difference(&before, &after),
        None,
        "walking from the start of a game"
    );
}

// Measured: 228, 190 and 215 passes in 1,000 frames in the busy rooms became
// 240, 195 and 229. Alien 8 gained more from the same step (16-24%): its busy
// rooms had more of each sprite outside the cleared places.
#[test]
fn drawing_only_the_rows_that_show_is_faster_in_every_busy_room() {
    let Some(speeds) = busy_speeds(&[(DRAWING1, DRAWING1_MAP), (DRAWING, DRAWING_MAP)]) else {
        return;
    };
    for (frame, s) in BUSY.iter().zip(&speeds) {
        assert!(
            u32::from(s[1]) * 100 >= u32::from(s[0]) * 102,
            "from frame {frame}: {} passes against step 1's {}; all: {speeds:?}",
            s[1],
            s[0]
        );
    }
}

const PACED: &str = "assemblies/knight-lore/out/paced fixed.szx";
const PACED_MAP: &str = "assemblies/knight-lore/out/paced fixed.map";

// paced.py swaps the busy-wait at the end of a pass for a wait in HALT, so
// only time changes: the game, pass for pass, must not. It paces the build
// with the memory reorganised, the last before it.
#[test]
fn the_paced_build_plays_the_same_game() {
    if !have(&[MEMORY, MEMORY_MAP, PACED, PACED_MAP, "roms/128.rom"]) {
        return;
    }
    let unpaced = play_by_pass(MEMORY, MEMORY_MAP, 300);
    let paced = play_by_pass(PACED, PACED_MAP, 300);
    assert_eq!(
        first_difference(&unpaced, &paced),
        None,
        "walking from the start of a game"
    );
}

/// Passes in 1,000 frames at `turbo`, keys left alone: in each busy room,
/// then in the first room. Walking can't measure speed, since a faster build
/// covers more ground in the same frames and goes through doors, and a room
/// change costs more than a pass.
fn still_speeds(snap: &str, map: &str, turbo: u32) -> Vec<u16> {
    let mut machines: Vec<Spectrum> = BUSY
        .iter()
        .map(|frame| {
            let sna = std::fs::read(format!("assemblies/knight-lore/out/busy{frame}.sna")).unwrap();
            busy_machine(&sna, snap, map)
        })
        .collect();
    machines.push(at_first_pass(snap, map));
    machines
        .into_iter()
        .map(|mut spec| {
            spec.bus.turbo = turbo;
            let t = spec.bus.model.frame_t();
            let before = passes(&spec);
            for _ in 0..1_000 {
                spec.run(t);
            }
            passes(&spec).wrapping_sub(before)
        })
        .collect()
}

fn have_still_rooms(builds: &[&str]) -> bool {
    let mut need: Vec<String> = BUSY
        .iter()
        .map(|f| format!("assemblies/knight-lore/out/busy{f}.sna"))
        .collect();
    need.extend(builds.iter().map(|b| b.to_string()));
    need.push("roms/128.rom".into());
    have(&need.iter().map(String::as_str).collect::<Vec<_>>())
}

// The point of pacing: under the CPU turbo the game no longer runs away. At
// four times the speed the unpaced build made 1,009, 820 and 854 passes in
// 1,000 frames in the busy rooms and 1,805 standing in the first room; the
// paced one 250 in all four, four frames a pass. Even at normal speed the
// first room, unpaced, makes 439. One more than 250 is a pass that ended
// just inside the thousandth frame.
#[test]
fn the_paced_build_holds_four_frames_a_pass_under_turbo() {
    if !have_still_rooms(&[MEMORY, MEMORY_MAP, PACED, PACED_MAP]) {
        return;
    }
    let free = still_speeds(MEMORY, MEMORY_MAP, 4);
    let paced = still_speeds(PACED, PACED_MAP, 4);
    let normal = still_speeds(PACED, PACED_MAP, 1);
    for (i, ((f, p), n)) in free.iter().zip(&paced).zip(&normal).enumerate() {
        assert!(
            *f > 400,
            "room {i}: the unpaced build made only {f} at four times the speed"
        );
        assert!(
            (245..=251).contains(p),
            "room {i}: the paced build made {p} at four times the speed; all {paced:?}"
        );
        assert!(
            *n <= 251,
            "room {i}: the paced build made {n} at normal speed; all {normal:?}"
        );
    }
}

// Pacing costs a little below the cap: a pass that comes in under four frames
// is held to four and one over it is not made up, so a room averages less than
// the 250 its quickest passes would allow. Measured on the memory build: 241,
// 196 and 261 passes unpaced in the busy rooms, 239, 195 and 243 paced; the
// room at frame 91,000, which goes quicker than the pace unpaced, loses the
// most. More than 4% under the lesser of the two is a wait that is overdone.
#[test]
fn the_paced_build_waits_for_little_more_than_the_pace_in_busy_rooms() {
    if !have_still_rooms(&[MEMORY, MEMORY_MAP, PACED, PACED_MAP]) {
        return;
    }
    let free = still_speeds(MEMORY, MEMORY_MAP, 1);
    let paced = still_speeds(PACED, PACED_MAP, 1);
    for (frame, (f, p)) in BUSY.iter().zip(free.iter().zip(&paced)) {
        assert!(
            u32::from(*p) * 100 >= u32::from((*f).min(250)) * 96,
            "from frame {frame}: {p} passes paced against {f} unpaced"
        );
    }
}

const ORIGINAL_TAPE: &str = "tapes/Knight Lore (1984)(Ultimate Play The Game).zip";

/// The original on a 48K with its tape playing and LOAD "" typed.
fn loading_the_original() -> Spectrum {
    let zip = std::fs::read(ORIGINAL_TAPE).unwrap();
    let (name, bytes) = zx_rustrum::zip::first_with_extension(&zip, &["tzx", "tap"]).unwrap();
    let mut tape = zx_rustrum::tape::Tape::from_bytes(&name, &bytes).unwrap();
    let mut spec = Spectrum::with_model(zx_rustrum::machine::Model::Spectrum48);
    spec.load_rom(&std::fs::read("roms/48.rom").unwrap());
    let t = spec.bus.model.frame_t();
    for _ in 0..150 {
        spec.run(t);
    }
    tape.play(spec.bus.total_t());
    spec.bus.tape = Some(tape);
    // LOAD "": J, symbol shift and P twice, ENTER.
    for keys in [
        &[(6usize, 3u8)][..],
        &[(7, 1), (5, 0)],
        &[(7, 1), (5, 0)],
        &[(6, 0)],
    ] {
        for (row, bit) in keys {
            key(&mut spec, *row, *bit, true);
        }
        for _ in 0..5 {
            spec.run(t);
        }
        for (row, bit) in keys {
            key(&mut spec, *row, *bit, false);
        }
        for _ in 0..5 {
            spec.run(t);
        }
    }
    spec
}

const LOADING_SCREEN: &str = "assemblies/knight-lore/loading screen.scr";

// The rebuilt game's tape shows the original's loading screen, which can't be
// lifted off the original tape: Speedlock 1 loads it encrypted, in turbo
// blocks of its own, and puts it on the screen all at once (frame 1,617, as
// block 135 is reached). So the original is loaded on a 48K at the tape's
// speed, and the screen kept is the one that stood still while the last
// block went in (6,093 frames), as it was the frame before the game first
// drew over it. Written for assemblies/tap.py; run by build.sh.
#[test]
#[ignore]
fn keep_the_original_loading_screen() {
    if !have(&[ORIGINAL_TAPE, "roms/48.rom"]) {
        return;
    }
    let mut spec = loading_the_original();
    let t = spec.bus.model.frame_t();
    let screen = |s: &Spectrum| {
        (0x4000..0x5B00u16)
            .map(|a| s.bus.peek_raw(a))
            .collect::<Vec<u8>>()
    };
    let block = |s: &Spectrum| s.bus.tape.as_ref().map_or(0, |t| t.block);
    let (mut last, mut since, mut from) = (screen(&spec), spec.bus.frame, 0usize);
    for _ in 0..20_000 {
        spec.run(t);
        let now = screen(&spec);
        if now != last {
            let quiet = spec.bus.frame - since;
            if quiet > 200 && from >= 135 && last[0x1800..].iter().any(|b| *b != last[0x1800]) {
                std::fs::write(LOADING_SCREEN, &last).unwrap();
                eprintln!(
                    "the screen of frame {}, still for {quiet} frames",
                    spec.bus.frame - 1
                );
                return;
            }
            since = spec.bus.frame;
            from = block(&spec);
        }
        last = now;
    }
    panic!("no screen stood still after block 135 in 20,000 frames");
}

const TAPE: &str = "assemblies/knight-lore/Knight Lore 128.tap";
const PLAYED: &str = "assemblies/knight-lore/Knight Lore paced.szx";
const PLAYED_MAP: &str = "assemblies/knight-lore/Knight Lore paced.map";

/// A 128K that has loaded `TAPE` through its Tape Loader, stopped as the
/// loader's last instruction, a RET in the ROM, lands on game_start: each
/// block handed to the ROM's LD-BYTES whole, as fastload does, or `played`
/// at the ROM's speed.
fn loaded_from_tape(played: bool) -> Spectrum {
    let mut spec = Spectrum::with_model(zx_rustrum::machine::Model::Spectrum128);
    spec.load_rom(&std::fs::read("roms/128.rom").unwrap());
    let mut tape = zx_rustrum::tape::Tape::from_bytes(TAPE, &std::fs::read(TAPE).unwrap()).unwrap();
    tape.play(0);
    spec.bus.tape = Some(tape);
    spec.bus.tape_flash = !played;
    let t = spec.bus.model.frame_t();
    for _ in 0..120 {
        spec.run(t);
    }
    // ENTER on the menu's first line, the Tape Loader.
    key(&mut spec, 6, 0, true);
    for _ in 0..5 {
        spec.run(t);
    }
    key(&mut spec, 6, 0, false);
    let game_start = address(PLAYED_MAP, "game_start");
    for _ in 0..5_000_000_000u64 {
        if spec.cpu.pc == game_start && spec.bus.page_reg & 7 == 0 {
            return spec;
        }
        spec.step_instruction();
    }
    panic!("the tape never got to game_start; PC ${:04X}", spec.cpu.pc);
}

// The rebuilt game as a tape that loads through the ROM, which has to start
// the game as the snapshot does: the image from the stack up, all of bank 0
// (the game's code and buffer, so stage 2 lives in bank 6 and pages bank 0
// in through the ROM's OUT and RET), bank 3 as far as the game has anything
// in it, SP, IY, the paging and a silent AY. The screen is the loading
// screen, which the game clears.
#[test]
fn the_tape_starts_the_game_as_the_snapshot_does() {
    if !have(&[TAPE, PLAYED, PLAYED_MAP, "roms/128.rom"]) {
        return;
    }
    starts_as_the_snapshot_does(loaded_from_tape(false));
}

// The same at the ROM's speed: fastload hands each block over without the
// ROM's routine running, so only this shows the loader living with it, its
// EI as it returns and the bank changing under it between blocks.
#[test]
fn the_tape_loads_at_the_roms_own_speed() {
    if !have(&[TAPE, PLAYED, PLAYED_MAP, "roms/128.rom"]) {
        return;
    }
    let tape = loaded_from_tape(true);
    assert!(
        tape.bus.frame > 15_000,
        "loaded in {} frames: that was not at the ROM's speed",
        tape.bus.frame
    );
    starts_as_the_snapshot_does(tape);
}

fn starts_as_the_snapshot_does(tape: Spectrum) {
    if let Ok(kept) = std::fs::read(LOADING_SCREEN) {
        let shown: Vec<u8> = (0x4000..0x5B00u16).map(|a| tape.bus.peek_raw(a)).collect();
        if let Some(i) = shown.iter().zip(&kept).position(|(a, b)| a != b) {
            panic!(
                "the loading screen: byte {i:#06X} is ${:02X} and the original's ${:02X}",
                shown[i], kept[i]
            );
        }
    }
    let snap = machine(PLAYED);
    let differs = |what: &str, a: &[u8], b: &[u8]| {
        if let Some(i) = a.iter().zip(b).position(|(x, y)| x != y) {
            panic!(
                "{what}: byte {i:#06X} is ${:02X} from the tape and ${:02X} in the snapshot",
                a[i], b[i]
            );
        }
    };
    // Below SP is the stack's: the loader's last return address is there.
    let main = |s: &Spectrum| {
        (STACK_TOP..0xC000u16)
            .map(|a| s.bus.peek_raw(a))
            .collect::<Vec<u8>>()
    };
    differs("$5BA0-$BFFF", &main(&tape), &main(&snap));
    // Each bank as far as the game has anything in it, from the snapshot:
    // taken from the tape's blocks instead, a block cut short passed.
    for n in [0usize, 1, 3, 4, 7] {
        let length = (0..0x4000u16)
            .rev()
            .find(|o| snap.bus.bank_byte(n, *o) != 0)
            .map_or(0, |o| o + 1);
        assert!(
            length > 0x0E00,
            "bank {n} of the snapshot holds only {length:#06X} bytes"
        );
        let bank = |s: &Spectrum| {
            (0..length)
                .map(|o| s.bus.bank_byte(n, o))
                .collect::<Vec<u8>>()
        };
        differs(&format!("bank {n}"), &bank(&tape), &bank(&snap));
    }
    assert_eq!(
        (tape.cpu.sp, tape.cpu.iy, tape.bus.page_reg, tape.cpu.iff1),
        (STACK_TOP, 0x5C3A, 0x10, false),
        "SP, IY, $7FFD and interrupts as the game starts"
    );
    assert_eq!(tape.bus.audio.ay.regs[..14], [0u8; 14], "the AY silent");
    // And from there it is the same game: the menu it draws over the loading
    // screen on one and a blank screen on the other is the same.
    let (mut tape, mut snap) = (tape, snap);
    let t = tape.bus.model.frame_t();
    for _ in 0..400 {
        tape.run(t);
        snap.run(t);
    }
    let screen = |s: &Spectrum| {
        (0x4000..0x5B00u16)
            .map(|a| s.bus.peek_raw(a))
            .collect::<Vec<u8>>()
    };
    assert!(
        screen(&snap).iter().any(|b| *b != 0),
        "the snapshot drew nothing in 400 frames"
    );
    differs("the menu after 400 frames", &screen(&tape), &screen(&snap));
}

/// T-states spent under each label of the main image, nearest label below
/// the PC: labels in a bank (a third field in the map) left out, since they
/// share addresses with fixed code.
fn profile_into(
    spec: &mut Spectrum,
    labels: &[(u16, String)],
    frames: u64,
    walk: bool,
    into: &mut std::collections::HashMap<String, u64>,
) {
    let end = spec.bus.total_t() + frames * u64::from(spec.bus.model.frame_t());
    let start_frame = spec.bus.frame;
    while spec.bus.total_t() < end {
        if walk {
            let walking = WALK[((spec.bus.frame - start_frame) / 100) as usize % WALK.len()];
            for (row, bit) in WALK {
                key(spec, row, bit, (row, bit) == walking);
            }
        }
        let (pc, t) = (spec.cpu.pc, spec.bus.total_t());
        spec.step_instruction();
        let i = labels.partition_point(|(a, _)| *a <= pc);
        if i > 0 {
            *into.entry(labels[i - 1].1.clone()).or_default() += spec.bus.total_t() - t;
        }
    }
}

// Written for assemblies/knight-lore/memory.py, which puts the hottest code
// in uncontended memory a byte at a time by this: the drawing build, in the
// three busy rooms and walking from the start, 1,000 frames each.
#[test]
#[ignore]
fn profile_the_drawing_build() {
    let mut labels: Vec<(u16, String)> = std::fs::read_to_string(DRAWING_MAP)
        .unwrap()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() == 2).then(|| {
                (
                    u16::from_str_radix(&f[1][1..], 16).unwrap(),
                    f[0].to_string(),
                )
            })
        })
        .collect();
    labels.sort();
    let mut spent = std::collections::HashMap::new();
    for frame in BUSY {
        let sna = std::fs::read(format!("assemblies/knight-lore/out/busy{frame}.sna")).unwrap();
        let mut spec = busy_machine(&sna, DRAWING, DRAWING_MAP);
        profile_into(&mut spec, &labels, 1_000, false, &mut spent);
    }
    let mut spec = at_first_pass(DRAWING, DRAWING_MAP);
    profile_into(&mut spec, &labels, 1_000, true, &mut spent);
    let mut rows: Vec<_> = spent.into_iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    let text: String = std::iter::once("label tstates\n".to_string())
        .chain(rows.iter().map(|(n, t)| format!("{n} {t}\n")))
        .collect();
    std::fs::write("assemblies/knight-lore/out/profile.txt", text).unwrap();
}

const MEMORY1: &str = "assemblies/knight-lore/out/memory1 fixed.szx";
const MEMORY1_MAP: &str = "assemblies/knight-lore/out/memory1 fixed.map";

// memory.py's first step exchanges the sprites with everything that was
// above $C000: the sprites up into bank 0, and the code, the buffer and the
// tables down into the room they leave, the hottest code uncontended. Only
// where things are changes, so the game has to be the same.
#[test]
fn the_exchanged_memory_plays_the_same_game() {
    if !have(&[DRAWING, DRAWING_MAP, MEMORY1, MEMORY1_MAP, "roms/128.rom"]) {
        return;
    }
    let before = play_by_pass(DRAWING, DRAWING_MAP, 300);
    let after = play_by_pass(MEMORY1, MEMORY1_MAP, 300);
    assert_eq!(
        first_difference(&before, &after),
        None,
        "walking from the start of a game"
    );
}

// print_bcd is handed the buffer address of a number's bottom row, written in
// the code as a number: the days and the lives, which the walk prints, and
// three more on the screen at the end of a game, which nothing else here
// reaches. So both builds draw that screen from the busiest room's state, to
// where it waits for the keys to be let go, and the screens have to agree.
#[test]
fn the_end_of_a_game_is_printed_in_the_same_places() {
    let sna_path = "assemblies/knight-lore/out/busy91000.sna";
    if !have(&[
        sna_path,
        DRAWING,
        DRAWING_MAP,
        MEMORY1,
        MEMORY1_MAP,
        "roms/128.rom",
    ]) {
        return;
    }
    let sna = std::fs::read(sna_path).unwrap();
    let screens: Vec<Vec<u8>> = [(DRAWING, DRAWING_MAP), (MEMORY1, MEMORY1_MAP)]
        .iter()
        .map(|(snap, map)| {
            let mut spec = busy_machine(&sna, snap, map);
            spec.cpu.pc = address(map, "game_over_screen");
            spec.cpu.iff1 = false;
            spec.cpu.iff2 = false;
            let stop = address(map, "wait_keys_released");
            for _ in 0..50_000_000 {
                spec.step_instruction();
                if spec.cpu.pc == stop {
                    return (0x4000..0x5B00u16).map(|a| spec.bus.peek_raw(a)).collect();
                }
            }
            panic!("{snap} never finished the end-of-game screen");
        })
        .collect();
    let lit = screens[0][..0x1800].iter().filter(|b| **b != 0).count();
    assert!(
        lit > 500,
        "only {lit} bytes of the end-of-game screen are drawn"
    );
    if let Some(i) = screens[0].iter().zip(&screens[1]).position(|(a, b)| a != b) {
        panic!(
            "${:04X} is ${:02X} before the exchange and ${:02X} after",
            0x4000 + i,
            screens[0][i],
            screens[1][i]
        );
    }
}

const MEMORY2: &str = "assemblies/knight-lore/out/memory2 fixed.szx";
const MEMORY2_MAP: &str = "assemblies/knight-lore/out/memory2 fixed.map";

// memory.py's second step keeps a mirrored copy of every sprite in bank 4 at
// the same address, and pages in whichever faces the right way instead of
// mirroring a sprite in place. What is drawn has to be the same.
#[test]
fn drawing_from_mirrored_copies_plays_the_same_game() {
    if !have(&[MEMORY1, MEMORY1_MAP, MEMORY2, MEMORY2_MAP, "roms/128.rom"]) {
        return;
    }
    let before = play_by_pass(MEMORY1, MEMORY1_MAP, 300);
    let after = play_by_pass(MEMORY2, MEMORY2_MAP, 300);
    assert_eq!(
        first_difference(&before, &after),
        None,
        "walking from the start of a game"
    );
}

// Bank 4's copies are made by memory.py, not by the game, so each has to be
// what the game would have made: the step 1 build is asked to mirror every
// sprite in place, from a fresh machine each time, and the result compared
// with bank 4 byte for byte.
#[test]
fn every_mirrored_copy_is_what_the_game_would_have_made() {
    if !have(&[MEMORY1, MEMORY1_MAP, MEMORY2, MEMORY2_MAP, "roms/128.rom"]) {
        return;
    }
    let mirrored = machine(MEMORY2);
    let fresh = machine(MEMORY1);
    let table = address(MEMORY1_MAP, "sprite_table");
    let mut sprites: Vec<u16> = (0..188u16)
        .map(|i| {
            u16::from_le_bytes([
                fresh.bus.peek_raw(table + 2 * i),
                fresh.bus.peek_raw(table + 2 * i + 1),
            ])
        })
        .filter(|a| fresh.bus.peek_raw(*a) & 0x0F != 0)
        .collect();
    sprites.sort();
    sprites.dedup();
    let orient = address(MEMORY1_MAP, "orient_sprite");
    for at in &sprites {
        let mut spec = machine(MEMORY1);
        let first = spec.bus.peek_raw(*at);
        let size = 2 + 2 * u16::from(first & 0x0F) * u16::from(spec.bus.peek_raw(at + 1));
        // An object wanting it the other way round, and the same way up.
        spec.cpu.ix = 0x5C08;
        spec.bus.poke(0x5C08 + 7, first ^ 0x40);
        call_with(&mut spec, orient, 0, *at, 0);
        for o in 0..size {
            let (made, kept) = (
                spec.bus.peek_raw(at + o),
                mirrored.bus.bank_byte(4, at - 0xC000 + o),
            );
            assert_eq!(
                made, kept,
                "sprite at ${at:04X}, byte {o}: the game made ${made:02X}, bank 4 has ${kept:02X}"
            );
        }
    }
    assert!(sprites.len() > 90, "only {} sprites", sprites.len());
}

// Measured: in the busy room at frame 91,000, where mirroring took 13% of the
// time, 229 passes in 1,000 frames became 261; the other two, which mirror
// little, stayed at 241 and 196, and so did the first room standing still.
#[test]
fn drawing_from_mirrored_copies_is_faster_where_sprites_turn() {
    if !have_still_rooms(&[MEMORY1, MEMORY1_MAP, MEMORY2, MEMORY2_MAP]) {
        return;
    }
    let before = still_speeds(MEMORY1, MEMORY1_MAP, 1);
    let after = still_speeds(MEMORY2, MEMORY2_MAP, 1);
    assert!(
        u32::from(after[2]) * 100 >= u32::from(before[2]) * 110,
        "from frame 91,000: {} passes against {}; all {before:?} and {after:?}",
        after[2],
        before[2]
    );
    for (i, (b, a)) in before.iter().zip(&after).enumerate() {
        assert!(
            u32::from(*a) * 100 >= u32::from(*b) * 99,
            "room {i}: {a} against {b}"
        );
    }
}

// memory.py's second step cut the end off build_tables, which made the
// bit-reverse table that only the mirroring read. What is left still has to
// make the shift tables. The image already holds them, taken from a running
// machine, so build_tables rewriting the same bytes shows nothing: they are
// emptied first, then made, and compared with the image's.
#[test]
fn build_tables_still_makes_the_shift_tables() {
    if !have(&[MEMORY2, MEMORY2_MAP, "roms/128.rom"]) {
        return;
    }
    let mut spec = machine(MEMORY2);
    let at = address(MEMORY2_MAP, "shift_tables");
    let kept: Vec<u8> = (at..0x9000).map(|a| spec.bus.peek_raw(a)).collect();
    assert!(
        kept.iter().any(|b| *b != 0xFF),
        "the image's shift tables are blank"
    );
    for a in at..0x9000 {
        spec.bus.poke(a, 0);
    }
    call(&mut spec, address(MEMORY2_MAP, "build_tables"));
    let made: Vec<u8> = (at..0x9000).map(|a| spec.bus.peek_raw(a)).collect();
    if let Some(i) = made.iter().zip(&kept).position(|(m, k)| m != k) {
        panic!(
            "${:04X}: build_tables made ${:02X}, the image has ${:02X}",
            at + i as u16,
            made[i],
            kept[i]
        );
    }
}

const MEMORY: &str = "assemblies/knight-lore/out/memory fixed.szx";
const MEMORY_MAP: &str = "assemblies/knight-lore/out/memory fixed.map";

// memory.py's third step draws a sprite already shifted, where banks 1 and 7
// hold a copy, by the aligned path instead of shifting it as it goes. What is
// drawn has to be the same.
#[test]
fn drawing_sprites_already_shifted_plays_the_same_game() {
    if !have(&[MEMORY2, MEMORY2_MAP, MEMORY, MEMORY_MAP, "roms/128.rom"]) {
        return;
    }
    let before = play_by_pass(MEMORY2, MEMORY2_MAP, 300);
    let after = play_by_pass(MEMORY, MEMORY_MAP, 300);
    assert_eq!(
        first_difference(&before, &after),
        None,
        "walking from the start of a game"
    );
}

// Every object type at every shift, facing both ways, drawn over a pattern and
// over all ones by step 2 and by step 3, and the buffers compared. All ones is
// there because a picture bit outside its mask only shows over a set bit: the
// shifted path XORs the picture in and the aligned path ORs it. The draws a
// copy should serve are counted from step 3's directories, 469 of them, so the
// test can't pass by never using one.
#[test]
fn every_sprite_draws_the_same_from_a_copy_already_shifted() {
    if !have(&[MEMORY2, MEMORY2_MAP, MEMORY, MEMORY_MAP, "roms/128.rom"]) {
        return;
    }
    let mut old = at_first_pass(MEMORY2, MEMORY2_MAP);
    let mut new = at_first_pass(MEMORY, MEMORY_MAP);
    let table = address(MEMORY_MAP, "sprite_table");
    let (types, slots) = (0xFD00u16, 0xFDC0u16);
    let mut served = 0;
    for kind in 0..188u16 {
        let sprite = u16::from_le_bytes([
            new.bus.peek_raw(table + 2 * kind),
            new.bus.peek_raw(table + 2 * kind + 1),
        ]);
        let stored = new.bus.bank_byte(0, sprite - 0xC000);
        if stored & 0x0F == 0 {
            continue;
        }
        for shift in 0..8u8 {
            for flags in [0u8, 0x40] {
                // Which copy the object gets: bank 4's if it wants the other way.
                let bank = if (stored ^ flags) & 0x40 != 0 { 4 } else { 0 };
                let slot = new.bus.bank_byte(bank, types - 0xC000 + kind);
                if shift > 0 && slot > 0 {
                    let o = slots - 0xC000 + 16 * (u16::from(slot) - 1) + 2 * u16::from(shift);
                    served += usize::from(
                        new.bus.bank_byte(bank, o) | new.bus.bank_byte(bank, o + 1) != 0,
                    );
                }
                for ones in [false, true] {
                    let mut after = Vec::new();
                    for (spec, map) in [(&mut old, MEMORY2_MAP), (&mut new, MEMORY_MAP)] {
                        let buffer = address(map, "buffer");
                        for i in 0..0x1800u16 {
                            let b = if ones {
                                0xFF
                            } else {
                                (i.wrapping_mul(37) ^ (i >> 5)) as u8
                            };
                            spec.bus.poke(buffer + i, b);
                        }
                        let ix = address(map, "l_BFDB");
                        spec.cpu.ix = ix;
                        spec.bus.poke(ix, kind as u8);
                        spec.bus.poke(ix + 7, flags);
                        spec.bus.poke(ix + 0x1A, 0x40 + shift);
                        spec.bus.poke(ix + 0x1B, 0x40);
                        call(spec, address(map, "draw_sprite"));
                        after.push(
                            (0..0x1800u16)
                                .map(|i| spec.bus.peek_raw(buffer + i))
                                .collect::<Vec<u8>>(),
                        );
                    }
                    if let Some(i) = after[0].iter().zip(&after[1]).position(|(a, b)| a != b) {
                        panic!(
                            "type {kind:#04X} shift {shift} flags {flags:#04X} over {}: buffer byte {i:#06X} is ${:02X} shifted and ${:02X} from the copy",
                            if ones { "ones" } else { "the pattern" },
                            after[0][i],
                            after[1][i]
                        );
                    }
                }
            }
        }
    }
    assert!(served > 400, "only {served} draws had a copy to come from");
}

// What the copies are worth, standing still: 241, 196, 261 and 439 passes in
// 1,000 frames became 243, 199, 263 and 447, 1.3% together. Less than the
// 2.3% the copies' share of the shifted bytes promised: banks 1 and 7 are
// contended, and every shifted draw now looks in the directory first.
#[test]
fn drawing_sprites_already_shifted_is_a_little_faster() {
    if !have_still_rooms(&[MEMORY2, MEMORY2_MAP, MEMORY, MEMORY_MAP]) {
        return;
    }
    let before = still_speeds(MEMORY2, MEMORY2_MAP, 1);
    let after = still_speeds(MEMORY, MEMORY_MAP, 1);
    for (i, (b, a)) in before.iter().zip(&after).enumerate() {
        assert!(
            a >= b,
            "room {i}: {a} passes against {b}; all {before:?} and {after:?}"
        );
    }
    let (b, a) = (
        before.iter().map(|x| u32::from(*x)).sum::<u32>(),
        after.iter().map(|x| u32::from(*x)).sum::<u32>(),
    );
    assert!(
        a * 100 >= b * 101,
        "{a} passes in all four rooms against {b}"
    );
}
