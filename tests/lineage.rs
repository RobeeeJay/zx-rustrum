//! Placing disassembled games in a family tree by the code they share.

use std::collections::{BTreeMap, HashSet};

use zx_rustrum::lineage::{self, read, Author, Block, Fingerprint, Game, Lineage};
use zx_rustrum::mcp::json::{parse, Json};
use zx_rustrum::mcp::Server;

fn fingerprint(asm: &str, rom: &HashSet<u64>) -> Fingerprint {
    let disassembly = read::read_asm(asm).unwrap_or_else(|e| panic!("{e}\n{asm}"));
    lineage::from_disassembly(&disassembly, rom, &[])
}

fn grams(blocks: &[Block]) -> HashSet<u64> {
    blocks
        .iter()
        .flat_map(|b| b.grams.iter().copied())
        .collect()
}

/// A drawing loop of the sort a game takes with it into the next one: where
/// it is, the page its buffer is on and what it calls are all that change.
fn routine(org: &str, page: &str, call: &str) -> String {
    format!(
        "        ORG {org}
draw:
        LD B,$08
        LD H,{page}
        LD L,A
again:
        LD A,(HL)
        INC H
        ADD A,$10
        LD (HL),A
        CALL {call}
        DEC B
        JR NZ,again
        RET
"
    )
}

// The commonest way code is shared is a programmer taking a routine into the
// next game, where it lands somewhere else with its tables somewhere else.
// Blanking only the sixteen-bit operands lost those: the page byte in LD H,n
// moved with the tables.
#[test]
fn a_routine_moved_to_another_address_with_its_tables_still_matches() {
    let here = fingerprint(&routine("$8000", "$B7", "plot"), &HashSet::new());
    let there = fingerprint(&routine("$C000", "$C2", "$D456"), &HashSet::new());
    let (a, b) = (grams(&here.blocks), grams(&there.blocks));
    assert!(!a.is_empty(), "the routine should yield fragments");
    assert_eq!(
        a, b,
        "the same routine at $8000 and $C000 should give the same fragments"
    );
}

#[test]
fn different_code_shares_no_fragments() {
    let here = fingerprint(&routine("$8000", "$B7", "plot"), &HashSet::new());
    let there = fingerprint(
        "        ORG $8000
        LD A,(HL)
        AND A
        RLA
        LD (DE),A
        INC DE
        DJNZ $8000
        RET
",
        &HashSet::new(),
    );
    let common = grams(&here.blocks)
        .intersection(&grams(&there.blocks))
        .count();
    assert_eq!(common, 0, "unrelated code should share nothing");
}

// The same instruction is written many ways — in decimal, in hex, by a name —
// and a fragment has to come out the same however it was written, or two
// disassemblies of one program would look unrelated.
#[test]
fn an_instruction_has_one_shape_however_it_is_written() {
    let shape = |text: &str| read::shape(&read::canonical(text));
    assert_eq!(shape("ld a,(32768)"), "LD A,(n)");
    assert_eq!(shape("LD A,(score)"), "LD A,(n)");
    assert_eq!(shape("LD A,($8000)"), "LD A,(n)");
    assert_eq!(shape("BIT 7,(IX+3)"), "BIT 7,(IX+n)");
    assert_eq!(shape("bit 7,(ix-$03)"), "BIT 7,(IX+n)");
    assert_eq!(shape("EX AF,AF'"), "EX AF,AF'");
    assert_eq!(
        shape("RST $38"),
        shape("RST 56"),
        "a restart is part of the opcode"
    );
    assert_ne!(shape("RST $38"), shape("RST $10"));
    assert_eq!(read::canonical("out (254),a"), "OUT ($FE),A");
}

// An .asm says nothing about where each instruction is, so the addresses are
// counted from the ORG. One instruction of a length nobody knows would put
// everything after it in the wrong place, which is worse than stopping.
#[test]
fn an_instruction_of_unknown_length_stops_the_reading_with_its_line() {
    let refused = read::read_asm("        ORG $8000\n        NOP\n        FROB A\n");
    assert!(
        refused.as_ref().is_err_and(|e| e.contains("line 3")),
        "the reading should stop at line 3: {refused:?}"
    );
}

// Every label the notes place is where counting up from the ORG puts it, in
// each disassembly there is to check: if one were a byte out, every routine
// named after it would be named wrongly.
#[test]
fn every_label_in_an_exported_disassembly_is_where_the_notes_put_it() {
    let pairs = [
        (
            "disassembly/Jetpac (1983)(Ultimate Play The Game)[16K].asm",
            "tapes/Jetpac (1983)(Ultimate Play The Game)[16K].zxrs.txt",
        ),
        (
            "disassembly/Knight Lore (1984)(Ultimate Play The Game).asm",
            "recordings/KnightLore.zxrs.txt",
        ),
        (
            "disassembly/Alien 8 (1985)(Ultimate Play The Game).asm",
            "recordings/Alien8.zxrs.txt",
        ),
    ];
    let mut checked = 0;
    for (asm, notes) in pairs {
        let (Ok(asm_text), Ok(notes_text)) =
            (std::fs::read_to_string(asm), std::fs::read_to_string(notes))
        else {
            eprintln!("need {asm} and {notes}; skipping that one");
            continue;
        };
        let disassembly = read::read_asm(&asm_text).unwrap();
        let notes = zx_rustrum::notes::parse(&notes_text);
        let placed: BTreeMap<&str, u16> = notes
            .iter()
            .filter(|(_, n)| !n.label.is_empty())
            .map(|(at, n)| (n.label.as_str(), *at))
            .collect();
        for (at, name) in &disassembly.labels {
            if let Some(there) = placed.get(name.as_str()) {
                assert_eq!(
                    *at, *there,
                    "{asm}: {name} counted to ${at:04X}, the notes say ${there:04X}"
                );
                checked += 1;
            }
        }
    }
    eprintln!("{checked} labels checked");
}

// A game that copies a ROM routine into RAM is not thereby related to every
// other game that does.
#[test]
fn a_copy_of_a_rom_routine_is_not_counted() {
    // The routine's bytes, at $1000 in a ROM.
    let code = [
        0x06, 0x08, 0x26, 0x58, 0x6F, 0x7E, 0x24, 0xC6, 0x10, 0x77, 0xCD, 0x6B, 0x0D, 0x05, 0x20,
        0xF4, 0xC9,
    ];
    let mut rom = vec![0u8; 0x4000];
    rom[0x1000..0x1000 + code.len()].copy_from_slice(&code);
    let rom_grams = lineage::rom_grams(&|a: u16| rom.get(a as usize).copied().unwrap_or(0));
    let copy = fingerprint(&routine("$9000", "$58", "$0D6B"), &rom_grams);
    assert_eq!(
        grams(&copy.blocks).len(),
        0,
        "the copy's fragments are all the ROM's: {:?}",
        copy.blocks
    );
}

// A loader belongs to whoever protected the tape. Speedlock's code runs on
// $145 bytes above its sampling loop in Head over Heels and RoboCop, and left
// in, it made two games by different people look 17% related.
#[test]
fn the_whole_of_a_packed_loader_is_left_out_not_only_the_part_beside_its_sampler() {
    let mut asm = routine("$8000", "$B7", "plot");
    asm.push_str(
        "        ORG $FC00
        INC B
        RET Z
        LD A,$7F
        IN A,($FE)
        RRA
        XOR C
        AND $20
        JR Z,$FC00
        RET
",
    );
    for at in (0xFC30..=0xFD50).step_by(0x30) {
        asm.push_str(&routine(&format!("${at:04X}"), "$40", "$FC00"));
    }
    let print = fingerprint(&asm, &HashSet::new());
    let starts: Vec<u16> = print.blocks.iter().map(|b| b.start).collect();
    assert_eq!(
        starts,
        vec![0x8000],
        "only the game's routine should be left; left out {:?}",
        print.left_out
    );
    assert!(
        print.left_out.iter().any(|(low, _, _)| *low == 0xFD50),
        "the run $150 above the sampler is the loader too: {:?}",
        print.left_out
    );
    assert!(
        print.techniques.keys().any(|t| t.starts_with("loader:")),
        "which loader it was is still worth saying: {:?}",
        print.techniques
    );
}

// The loaders are recognised by their instructions' shapes, written out by
// hand; this keeps each one to the bytes the fast loader matches, so the two
// cannot drift apart.
#[test]
fn each_loader_shape_is_the_sampling_loop_flashload_knows() {
    for (name, bytes) in zx_rustrum::flashload::sampler_patterns() {
        let peek = |a: u16| bytes.get(a as usize).copied().unwrap_or(0);
        let mut shapes = Vec::new();
        let mut at = 0u16;
        while (at as usize) < bytes.len() {
            let insn = zx_rustrum::disasm::disasm(&peek, at);
            shapes.push(read::shape(&read::canonical(&insn.text)));
            at += insn.len.max(1) as u16;
        }
        let (_, pattern) = lineage::LOADERS
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("no shape written for {name:?}"));
        assert!(
            shapes.windows(pattern.len()).any(|w| w
                .iter()
                .zip(pattern.iter())
                .all(|(a, b)| *b == "*" || a == b)),
            "{name}: {pattern:?} is not in what its bytes decode as, {shapes:?}"
        );
    }
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

// Within a year the games are only in title order, which says nothing about
// which came out first; letting that order pick a parent made Gunfright
// Nightshade's parent because G comes before N, though by the usual accounts
// Nightshade came out first. Two games of a year share code with no arrow between them.
#[test]
fn games_of_the_same_year_are_relatives_and_neither_hangs_off_the_other() {
    let tree = Lineage::new(vec![
        game("Beta", 1985, "Bob", (1..61).chain(200..240)),
        game("Alpha", 1985, "Ann", 1..101),
    ]);
    assert_eq!(
        tree.parent,
        vec![None, None],
        "title order must not make Alpha Beta's parent"
    );
    assert_eq!(
        tree.kin.len(),
        1,
        "the pair should be relatives: {:?}",
        tree.links
    );
    let link = &tree.links[tree.kin[0]];
    assert!(
        link.same_year && !link.ordered,
        "a same-year link has no direction: {link:?}"
    );
}

// A year that is not known says nothing about order either.
#[test]
fn a_game_with_no_year_hangs_off_nothing() {
    let mut undated = game("Undated", 1984, "Bob", 1..61);
    undated.game.year = None;
    let tree = Lineage::new(vec![game("Dated", 1983, "Ann", 1..101), undated]);
    assert_eq!(tree.parent, vec![None, None], "no year, no direction");
    assert_eq!(tree.kin.len(), 1, "but they are related: {:?}", tree.links);
}

// The first game of a family used to show none of its code anywhere, because
// only earlier games were looked in. A block a later game holds is found there,
// and an earlier game holding it still wins, since that can say where it came
// from.
#[test]
fn a_routine_is_found_in_a_later_game_when_no_earlier_one_holds_it() {
    let tree = Lineage::new(vec![
        game("First", 1983, "Alice", 1..61),
        game("Second", 1984, "Bob", 1..101),
    ]);
    let origins = tree.origins(0);
    assert_eq!(
        origins.len(),
        1,
        "First's routine is in Second: {origins:?}"
    );
    assert_eq!(origins[0].from, 1, "found in the later game");
    assert!(tree.came_before(0, 1) && !tree.came_before(1, 0));

    let tree = Lineage::new(vec![
        game("Middle", 1984, "Bob", 1..61),
        game("Early", 1983, "Alice", 1..41),
        game("Late", 1985, "Carol", 1..61),
    ]);
    let middle = tree.index_of("Middle").unwrap();
    let origins = tree.origins(middle);
    assert_eq!(origins.len(), 1, "{origins:?}");
    assert_eq!(
        tree.games[origins[0].from].game.title, "Early",
        "an earlier game holding half beats a later one holding all"
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

// A disassembly that cannot be read must say so: a game missing from the tree
// reads as a game with no relatives.
#[test]
fn a_disassembly_that_cannot_be_read_is_named_not_left_out_quietly() {
    if std::fs::read("roms/48.rom").is_err() {
        eprintln!("need roms/48.rom; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("zxrs-lineage-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Good (1984)(Somebody).asm"),
        routine("$8000", "$B7", "plot"),
    )
    .unwrap();
    std::fs::write(
        dir.join("Bad (1985)(Somebody).asm"),
        "        ORG $8000\n        FROB A\n",
    )
    .unwrap();
    let mut server = Server::new();
    let tree = call(
        &mut server,
        "lineage",
        Json::obj([("dir", Json::str(dir.to_str().unwrap()))]),
    );
    std::fs::remove_dir_all(&dir).ok();
    let tree = tree.unwrap();
    assert!(
        tree.contains("Bad (1985)(Somebody).asm was not read") && tree.contains("line 2"),
        "the unreadable file and why should be named:\n{tree}"
    );
}

// The pair that showed the fault: Gunfright and Nightshade are both 1985, and
// Nightshade is usually said to have come out first; the old title order made
// Gunfright the parent all the same.
#[test]
fn nightshade_and_gunfright_are_relatives_with_neither_drawn_as_the_parent() {
    let needed = [
        "roms/48.rom",
        "disassembly/Gunfright (1985)(Ultimate Play The Game).asm",
        "disassembly/Nightshade (1985)(Ultimate Play The Game).asm",
    ];
    if needed.iter().any(|f| std::fs::metadata(f).is_err()) {
        eprintln!("need {needed:?}; skipping");
        return;
    }
    let mut server = Server::new();
    let tree = call(&mut server, "lineage", Json::obj([])).unwrap();
    let wrong: Vec<&str> = tree
        .lines()
        .filter(|l| {
            (l.contains("Nightshade —") && l.contains("of its code is in Gunfright"))
                || (l.contains("Gunfright —") && l.contains("of its code is in Nightshade"))
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "a same-year game drawn as a parent: {wrong:?}"
    );
    assert!(
        tree.lines()
            .any(|l| l.contains("shares code with 1985 Nightshade")),
        "Gunfright should list Nightshade as a same-year relative:\n{tree}"
    );
}

// The end-to-end check against games whose relationship is known: Alien 8 is
// built on Knight Lore's Filmation engine, and Jetpac, a year earlier, is not.
#[test]
fn alien_8_hangs_off_knight_lore_in_the_disassembly_folder() {
    let needed = [
        "roms/48.rom",
        "disassembly/Knight Lore (1984)(Ultimate Play The Game).asm",
        "disassembly/Alien 8 (1985)(Ultimate Play The Game).asm",
    ];
    if needed.iter().any(|f| std::fs::metadata(f).is_err()) {
        eprintln!("need {needed:?}; skipping");
        return;
    }
    let mut server = Server::new();
    let tree = call(&mut server, "lineage", Json::obj([])).unwrap();
    let line = tree
        .lines()
        .find(|l| l.contains("Alien 8") && l.contains("of its code is in"))
        .unwrap_or_else(|| panic!("Alien 8 hangs off nothing in:\n{tree}"));
    assert!(
        line.contains("Knight Lore"),
        "Alien 8 should hang off Knight Lore: {line}"
    );
    let share: f64 = line
        .split("— ")
        .nth(1)
        .and_then(|s| s.split('%').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| panic!("no share in {line:?}"));
    assert!(
        share >= 50.0,
        "Knight Lore should hold most of Alien 8, held {share}%"
    );

    let page = std::env::temp_dir().join(format!("zxrs-lineage-{}.html", std::process::id()));
    call(
        &mut server,
        "export_lineage",
        Json::obj([("path", Json::str(page.to_str().unwrap()))]),
    )
    .unwrap();
    let html = std::fs::read_to_string(&page).unwrap();
    std::fs::remove_file(&page).ok();
    assert!(
        html.contains("<title>Game Lineage</title>") && html.contains("Alien 8"),
        "the page names the games"
    );
}
