//! Fingerprinting games and placing them in a family tree.

use std::collections::{BTreeMap, HashSet};

use zx_rustrum::lineage::{self, Author, Block, Fingerprint, Game, Lineage};
use zx_rustrum::mcp::json::{parse, Json};
use zx_rustrum::mcp::Server;

/// A memory image with code put where asked, and the instruction starts of
/// each piece marked as having run, as the tracker would have them.
struct Machine {
    memory: Vec<u8>,
    ran: HashSet<u16>,
}

impl Machine {
    fn new() -> Machine {
        Machine {
            memory: vec![0; 0x10000],
            ran: HashSet::new(),
        }
    }

    fn put(&mut self, at: u16, code: &[u8]) {
        self.memory[at as usize..at as usize + code.len()].copy_from_slice(code);
        let peek = |a: u16| self.memory[a as usize];
        let mut here = at;
        let mut starts = Vec::new();
        while (here as usize) < at as usize + code.len() {
            starts.push(here);
            here += zx_rustrum::disasm::disasm(&peek, here).len.max(1) as u16;
        }
        self.ran.extend(starts);
    }

    fn take(&self, rom: &HashSet<u64>) -> Fingerprint {
        let peek = |a: u16| self.memory[a as usize];
        lineage::take(&peek, &|a| self.ran.contains(&a), &|_| None, rom, &[])
    }
}

/// A drawing loop of the sort a game moves about with its tables: the page
/// its buffer is on and the routine it calls are the only things that change.
fn routine(page: u8, call: u16) -> Vec<u8> {
    let [low, high] = call.to_le_bytes();
    vec![
        0x06, 0x08, // LD B,$08
        0x26, page, // LD H,page
        0x6F, // LD L,A
        0x7E, // LD A,(HL)
        0x24, // INC H
        0xC6, 0x10, // ADD A,$10
        0x77, // LD (HL),A
        0xCD, low, high, // CALL nn
        0x05, // DEC B
        0x20, 0xF4, // JR NZ,loop
        0xC9, // RET
    ]
}

fn grams(blocks: &[Block]) -> HashSet<u64> {
    blocks
        .iter()
        .flat_map(|b| b.grams.iter().copied())
        .collect()
}

// The commonest way code is shared is a programmer taking a routine into the
// next game, where it lands somewhere else with its tables somewhere else.
// Blanking only the sixteen-bit operands lost those: the page byte in LD H,n
// moved with the tables.
#[test]
fn a_routine_moved_to_another_address_with_its_tables_still_matches() {
    let mut here = Machine::new();
    here.put(0x8000, &routine(0xB7, 0x9123));
    let mut there = Machine::new();
    there.put(0xC000, &routine(0xC2, 0xD456));
    let (a, b) = (
        grams(&here.take(&HashSet::new()).blocks),
        grams(&there.take(&HashSet::new()).blocks),
    );
    assert!(!a.is_empty(), "the routine should yield fragments");
    assert_eq!(
        a, b,
        "the same routine at $8000 and $C000 should give the same fragments"
    );
}

#[test]
fn different_code_shares_no_fragments() {
    let mut here = Machine::new();
    here.put(0x8000, &routine(0xB7, 0x9123));
    let mut there = Machine::new();
    // LD A,(HL) / AND A / RLA / LD (DE),A / INC DE / DJNZ / RET.
    there.put(0x8000, &[0x7E, 0xA7, 0x17, 0x12, 0x13, 0x10, 0xF9, 0xC9]);
    let common: Vec<u64> = grams(&here.take(&HashSet::new()).blocks)
        .intersection(&grams(&there.take(&HashSet::new()).blocks))
        .copied()
        .collect();
    assert!(
        common.is_empty(),
        "unrelated code should share nothing, shared {}",
        common.len()
    );
}

// A game that copies a ROM routine into RAM is not thereby related to every
// other game that does.
#[test]
fn a_copy_of_a_rom_routine_is_not_counted() {
    let code = routine(0x58, 0x0D6B);
    let mut rom = vec![0u8; 0x4000];
    rom[0x1000..0x1000 + code.len()].copy_from_slice(&code);
    let rom_grams = lineage::rom_grams(&|a: u16| rom.get(a as usize).copied().unwrap_or(0));
    let mut machine = Machine::new();
    machine.put(0x9000, &code);
    let blocks = machine.take(&rom_grams).blocks;
    assert_eq!(
        grams(&blocks).len(),
        0,
        "the copy's fragments are all the ROM's: {blocks:?}"
    );
}

// A loader belongs to whoever protected the tape. Speedlock's code runs on
// $145 bytes above its sampling loop in Head over Heels and RoboCop, and left
// in, it made two games by different people look 17% related.
#[test]
fn the_whole_of_a_packed_loader_is_left_out_not_only_the_part_beside_its_sampler() {
    let mut machine = Machine::new();
    machine.put(0x8000, &routine(0xB7, 0x9123));
    // The ROM's sampling loop with the BREAK test taken out, as Speedlock has it.
    machine.put(
        0xFC00,
        &[
            0x04, 0xC8, 0x3E, 0x7F, 0xDB, 0xFE, 0x1F, 0xA9, 0xE6, 0x20, 0x28, 0xF4, 0xC9,
        ],
    );
    // More of the loader every $30 bytes, the last well past $100 above it.
    for at in (0xFC30..=0xFD50).step_by(0x30) {
        machine.put(at, &routine(0x40, 0xFC00));
    }
    let Fingerprint {
        blocks,
        techniques,
        left_out,
        ..
    } = machine.take(&HashSet::new());
    let starts: Vec<u16> = blocks.iter().map(|b| b.start).collect();
    assert_eq!(
        starts,
        vec![0x8000],
        "only the game's routine should be left; left out {left_out:?}"
    );
    assert!(
        left_out.iter().any(|(low, _, _)| *low == 0xFD50),
        "the run $150 above the sampler is the loader too: {left_out:?}"
    );
    assert!(
        techniques.keys().any(|t| t.starts_with("loader:")),
        "which loader it was is still worth saying: {techniques:?}"
    );
}

#[test]
fn a_fingerprint_reads_back_as_it_was_written() {
    let mut machine = Machine::new();
    machine.put(0x8000, &routine(0xB7, 0x9123));
    let Fingerprint {
        blocks,
        techniques,
        left_out,
        ..
    } = machine.take(&HashSet::new());
    let mut blocks = blocks;
    blocks[0].label = "draw_loop".into();
    let print = Fingerprint {
        game: Game {
            title: "Saboteur!".into(),
            year: Some(1985),
            publisher: "Durell Software Ltd".into(),
            authors: lineage::parse_authors("Clive Townsend; Tim Hayward (Inlay/Poster Art)"),
            zxdb: Some(4293),
        },
        source: "Saboteur (1986)(Durell Software).tzx".into(),
        blocks,
        techniques,
        left_out: vec![(
            0xFC00,
            0xFC0C,
            "part of the loader round the ROM's sampler".into(),
        )]
        .into_iter()
        .chain(left_out)
        .collect(),
    };
    let text = lineage::to_text(&print);
    assert_eq!(lineage::parse(&text), print, "the file was:\n{text}");
}

#[test]
fn a_tosec_name_gives_the_title_year_and_publisher() {
    let game = lineage::from_file_name("Jetpac (1983)(Ultimate Play The Game)[16K].tzx");
    assert_eq!(
        (game.title.as_str(), game.year, game.publisher.as_str()),
        ("Jetpac", Some(1983), "Ultimate Play The Game")
    );
}

const ZXDB: &str = "\
# id\ttitle\tyear\tpublisher\tauthors
3012\tManic Miner\t1983\tBug-Byte Software Ltd\tMatthew Smith; Roger Tissyman (Inlay/Poster Art)
28346\tManic Miner\t1992\tRevelation Software\tMatthew Holt (Code, Level Design)
4295\tSaboteur II\t1987\tDurell Software Ltd\tClive Townsend (Code, In-game Graphics); Rob Hubbard (Music)
9000\tTwice\t\t\tSomebody
9001\tTwice\t\t\tSomebody Else
";

// ZXDB has many titles several times over, so a title alone is not enough to
// say whose a game is, and a wrong author is worse than none.
#[test]
fn zxdb_chooses_between_titles_by_year_and_will_not_guess_between_equals() {
    let wanted = lineage::from_file_name("Manic Miner (1983)(Bug-Byte Software).tzx");
    let found = lineage::find_in_zxdb(ZXDB, &wanted).unwrap();
    assert_eq!(
        found.zxdb,
        Some(3012),
        "the 1983 release, not the 1992 remake: {found:?}"
    );
    assert_eq!(
        found.coders(),
        vec!["Matthew Smith"],
        "an author with no role may be the coder"
    );

    let twice = Game {
        title: "Twice".into(),
        ..Game::default()
    };
    let refused = lineage::find_in_zxdb(ZXDB, &twice);
    assert!(
        refused.is_err(),
        "two equal candidates should be refused: {refused:?}"
    );
}

#[test]
fn a_tosec_subtitle_is_dropped_when_zxdb_has_the_title_without_it() {
    let wanted =
        lineage::from_file_name("Saboteur II - Avenging Angel (1987)(Durell Software)[128K].tzx");
    let found = lineage::find_in_zxdb(ZXDB, &wanted).unwrap();
    assert_eq!(found.zxdb, Some(4295), "{found:?}");
}

fn game(title: &str, year: u16, coder: &str, grams: impl IntoIterator<Item = u64>) -> Fingerprint {
    let grams: Vec<u64> = grams.into_iter().collect();
    Fingerprint {
        game: Game {
            title: title.into(),
            year: Some(year),
            publisher: String::new(),
            authors: vec![Author {
                name: coder.into(),
                roles: vec!["Code".into()],
            }],
            zxdb: None,
        },
        source: String::new(),
        blocks: vec![Block {
            start: 0x8000,
            end: 0x8100,
            label: "engine".into(),
            instructions: grams.len() + 4,
            grams,
        }],
        techniques: BTreeMap::new(),
        left_out: Vec::new(),
    }
}

// The tree is the point of all this: each game hangs off the earlier game it
// holds most of, and a handful of common fragments is not a family.
#[test]
fn a_game_hangs_off_the_earlier_game_holding_its_code_and_a_few_shared_idioms_do_not_count() {
    let tree = Lineage::new(vec![
        game("Later", 1985, "Carol", (300..400).chain(1..4)),
        game("First", 1983, "Alice", 1..101),
        game("Second", 1984, "Bob", (1..41).chain(200..261)),
    ]);
    let titles: Vec<&str> = tree.games.iter().map(|g| g.game.title.as_str()).collect();
    assert_eq!(titles, vec!["First", "Second", "Later"], "oldest first");

    let link = &tree.links[tree.parent[1].expect("Second should have a parent")];
    assert_eq!(link.earlier, 0, "Second's code came from First");
    // Fragments 1-3 are in all three games and count for 2/3 each; 4-40 are in
    // two and count whole. Second's mass is 3*2/3 + 37 + 61 = 100, of which 39
    // is shared.
    assert!(
        (link.of_later - 0.39).abs() < 1e-9,
        "share was {}",
        link.of_later
    );
    assert_eq!(
        tree.parent[2], None,
        "three common fragments are not a family"
    );
}

#[test]
fn a_routine_from_someone_elses_game_names_them_as_a_suspected_author() {
    let tree = Lineage::new(vec![
        game("First", 1983, "Alice", 1..101),
        game("Second", 1984, "Bob", 1..60),
        game("Third", 1985, "Alice", 1..60),
    ]);
    let origins = tree.origins(1);
    assert_eq!(origins.len(), 1, "{origins:?}");
    assert_eq!(origins[0].from, 0, "the routine is earliest in First");
    assert_eq!(
        tree.suspected(1, 0),
        vec!["Alice"],
        "Bob's game holds Alice's code"
    );
    assert!(
        tree.suspected(2, 0).is_empty(),
        "Alice taking her own code is no surprise"
    );
}

fn call(server: &mut Server, tool: &str, args: Json) -> Result<String, String> {
    let line = Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", Json::num(1)),
        ("method", Json::str("tools/call")),
        (
            "params",
            Json::obj([("name", Json::str(tool)), ("arguments", args)]),
        ),
    ])
    .to_string();
    let reply = parse(&server.handle(&line).unwrap()).unwrap();
    let result = reply.get("result").unwrap().clone();
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|i| i.first())
        .and_then(|i| i.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    match result.get("isError").and_then(|e| e.as_bool()) {
        Some(true) => Err(text),
        _ => Ok(text),
    }
}

#[test]
fn a_machine_with_no_rom_is_not_fingerprinted() {
    let mut server = Server::new();
    let refused = call(
        &mut server,
        "fingerprint",
        Json::obj([("path", Json::str("/dev/null"))]),
    );
    assert!(
        refused.as_ref().is_err_and(|why| why.contains("no ROM")),
        "an empty machine runs RST $38 and nothing else: {refused:?}"
    );
}

// The end-to-end check against games whose relationship is known: Jetpac and
// Pssst are both Chris Stamper's, a few months apart, and shared code.
#[test]
fn pssst_hangs_off_jetpac_when_both_are_fingerprinted_from_their_tapes() {
    let tapes = [
        "tapes/Jetpac (1983)(Ultimate Play The Game)[16K].tzx",
        "tapes/Pssst (1983)(Ultimate Play The Game)[16K].tzx",
    ];
    if std::fs::read("roms/48.rom").is_err() || tapes.iter().any(|t| std::fs::read(t).is_err()) {
        eprintln!("need roms/48.rom and the Jetpac and Pssst tapes; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("zxrs-lineage-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (i, tape) in tapes.iter().enumerate() {
        let mut server = Server::new();
        call(
            &mut server,
            "load_tape",
            Json::obj([("path", Json::str(*tape))]),
        )
        .unwrap();
        call(
            &mut server,
            "run_frames",
            Json::obj([("frames", Json::num(500))]),
        )
        .unwrap();
        let path = dir.join(format!("{i}.lineage.txt"));
        let made = call(
            &mut server,
            "fingerprint",
            Json::obj([("path", Json::str(path.to_str().unwrap()))]),
        )
        .unwrap();
        assert!(made.contains("instructions that ran"), "{made}");
    }
    let mut server = Server::new();
    let tree = call(
        &mut server,
        "lineage",
        Json::obj([("dir", Json::str(dir.to_str().unwrap()))]),
    )
    .unwrap();
    let html = dir.join("lineage.html");
    call(
        &mut server,
        "export_lineage",
        Json::obj([
            ("dir", Json::str(dir.to_str().unwrap())),
            ("path", Json::str(html.to_str().unwrap())),
        ]),
    )
    .unwrap();
    let page = std::fs::read_to_string(&html).unwrap();
    std::fs::remove_dir_all(&dir).ok();

    let line = tree
        .lines()
        .find(|l| l.contains("Pssst"))
        .unwrap_or_else(|| panic!("no Pssst in:\n{tree}"));
    assert!(
        line.contains("of its code is in Jetpac"),
        "Pssst should hang off Jetpac:\n{tree}"
    );
    let share: f64 = line
        .split("— ")
        .nth(1)
        .and_then(|s| s.split('%').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| panic!("no share in {line:?}"));
    assert!(
        share >= 20.0,
        "Jetpac should hold a good part of Pssst, held {share}%"
    );
    assert!(
        page.contains("<title>Game Lineage</title>") && page.contains("Pssst"),
        "the page names the games"
    );
}
