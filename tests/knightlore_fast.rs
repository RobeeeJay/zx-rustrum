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

/// The screen, the variables and the forty objects, less the stack.
fn comparable(spec: &Spectrum) -> Vec<u8> {
    (0x4000..OBJECTS_END)
        .map(|a| {
            if (0x5B00..STACK_TOP).contains(&a) {
                0
            } else {
                spec.bus.peek_raw(a)
            }
        })
        .collect()
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
            out.push(comparable(&spec));
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
