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

/// The screen, the variables and the forty objects.
fn comparable(spec: &Spectrum) -> Vec<u8> {
    (0x4000..OBJECTS_END)
        .map(|a| spec.bus.peek_raw(a))
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
