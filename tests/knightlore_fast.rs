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
