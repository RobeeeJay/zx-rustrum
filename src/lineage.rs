//! Which games share code with which, and which of them had it first.
//!
//! A fingerprint is taken of the code a game was seen to run. Each run of
//! instructions has its sixteen-bit operands blanked, so a routine moved to a
//! different address still reads the same, and is cut into overlapping
//! fragments of `GRAM` instructions which are hashed. Two games holding the
//! same fragment hold the same few instructions in the same order; a game
//! holding a great many of another's holds its code.
//!
//! What is compared is code that ran, not memory: data and dead code would
//! match on nothing in particular. The ROM's own fragments are taken out, since
//! every game that copies a ROM routine would otherwise look related to every
//! other, and so is any run beside a tape loader's sampling loop, since a loader
//! belongs to whoever protected the tape rather than whoever wrote the game.
//! See [`docs/lineage.md`](../docs/lineage.md).

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::disasm::{disasm, Insn};

pub const EXTENSION: &str = "lineage.txt";

/// Instructions in a fragment. Five is long enough that two programmers
/// writing the same thing independently rarely produce it byte for byte, and
/// short enough that a routine of a dozen instructions still yields some.
pub const GRAM: usize = 5;

/// How much of a game has to be found in an earlier one before that game is
/// drawn as where it came from: a twentieth of its code, and at least this many
/// fragments, so a handful of shared idioms is not a family.
pub const MIN_SHARE: f64 = 0.05;
pub const MIN_SHARED: usize = 12;

/// How far apart two runs of a loader's code may be and still both be the
/// loader: the tables and the bytes a loader keeps between its routines.
const LOADER_GAP: u16 = 0x40;

/// What each technique the fingerprint looks for is, in words.
pub const TECHNIQUES: &[(&str, &str)] = &[
    ("im2", "interrupt mode 2: a handler of its own every frame"),
    (
        "refresh-register",
        "reads R, for randomness or as a decryption key",
    ),
    (
        "stack-blit",
        "writes through the stack: LD SP, then a run of PUSHes",
    ),
    ("xor-draw", "draws by XOR onto what is there"),
    ("or-draw", "draws by OR onto what is there"),
    ("masked-draw", "draws through a mask: AND, then OR"),
    (
        "pixel-address",
        "works out a screen address from pixel coordinates",
    ),
    ("inc-h-rows", "steps down a character's rows with INC H"),
    ("unrolled-ldi", "copies with an unrolled run of LDI"),
    (
        "beeper",
        "toggles the beeper itself, XOR $10 then OUT ($FE)",
    ),
    ("ay", "plays the AY through $FFFD"),
    ("kempston", "reads a Kempston joystick at $1F"),
    ("halt-sync", "waits for the frame with HALT"),
    (
        "ix-records",
        "keeps its objects in records reached through IX or IY",
    ),
    ("rom-print", "prints through the ROM's RST $10"),
];

pub fn describe(technique: &str) -> String {
    if let Some(name) = technique.strip_prefix("loader:") {
        return format!("loads with {name}");
    }
    TECHNIQUES
        .iter()
        .find(|(id, _)| *id == technique)
        .map(|(_, words)| words.to_string())
        .unwrap_or_else(|| technique.to_string())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Author {
    pub name: String,
    pub roles: Vec<String>,
}

impl Author {
    /// Whether they may have written the code: credited with it, or credited
    /// with nothing in particular, which is how ZXDB has the one programmer of
    /// many early games.
    pub fn may_have_coded(&self) -> bool {
        self.roles.is_empty() || self.roles.iter().any(|r| r == "Code")
    }
}

/// `Name (Code, Music); Name`, as ZXDB's list is written out.
pub fn parse_authors(text: &str) -> Vec<Author> {
    text.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|one| match one.split_once(" (") {
            Some((name, roles)) => Author {
                name: name.trim().to_string(),
                roles: roles
                    .trim_end_matches(')')
                    .split(',')
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty())
                    .collect(),
            },
            None => Author {
                name: one.to_string(),
                roles: Vec::new(),
            },
        })
        .collect()
}

pub fn authors_text(authors: &[Author]) -> String {
    authors
        .iter()
        .map(|a| {
            if a.roles.is_empty() {
                a.name.clone()
            } else {
                format!("{} ({})", a.name, a.roles.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Game {
    pub title: String,
    pub year: Option<u16>,
    pub publisher: String,
    pub authors: Vec<Author>,
    pub zxdb: Option<u32>,
}

impl Game {
    /// The people who may have written its code.
    pub fn coders(&self) -> Vec<&str> {
        self.authors
            .iter()
            .filter(|a| a.may_have_coded())
            .map(|a| a.name.as_str())
            .collect()
    }
}

/// One run of code that ran, from its first byte to its last.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    pub start: u16,
    pub end: u16,
    pub label: String,
    pub instructions: usize,
    pub grams: Vec<u64>,
}

impl Block {
    pub fn name(&self) -> String {
        if self.label.is_empty() {
            format!("${:04X}", self.start)
        } else {
            self.label.clone()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fingerprint {
    pub game: Game,
    /// The file it was taken from, by name.
    pub source: String,
    pub blocks: Vec<Block>,
    /// Each technique seen, with the first address it was seen at.
    pub techniques: BTreeMap<String, u16>,
    /// Runs that were left out, and why.
    pub left_out: Vec<(u16, u16, String)>,
}

impl Fingerprint {
    pub fn instructions(&self) -> usize {
        self.blocks.iter().map(|b| b.instructions).sum()
    }

    pub fn grams(&self) -> HashSet<u64> {
        self.blocks
            .iter()
            .flat_map(|b| b.grams.iter().copied())
            .collect()
    }
}

fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The instruction with every number taken out of it: `LD H,$B7` is `LD H,n`.
///
/// Blanking only the sixteen-bit operands was tried first, and it lost code
/// that had been moved: a routine relocated with its tables holds them at a
/// different page, and `LD H,$B7` becomes `LD H,$C2`. Blanking the eight-bit
/// ones too raised every link between games known to share code — Saboteur's
/// to its sequel from 8% to 15% — and left the unrelated pairs where they were,
/// at a couple of per cent.
fn token(insn: &Insn) -> u64 {
    let mut shape = Vec::with_capacity(insn.text.len());
    let mut in_number = false;
    for c in insn.text.bytes() {
        if c == b'$' {
            in_number = true;
            shape.push(b'n');
        } else if !(in_number && c.is_ascii_hexdigit()) {
            in_number = false;
            shape.push(c);
        }
    }
    fnv(shape)
}

fn grams_of(run: &[(u16, Insn)]) -> Vec<u64> {
    let tokens: Vec<u64> = run.iter().map(|(_, insn)| token(insn)).collect();
    tokens
        .windows(GRAM)
        .map(|w| fnv(w.iter().flat_map(|t| t.to_le_bytes())))
        .collect()
}

/// Whether control cannot fall through this instruction to the next. A `JR`
/// is left out: it mostly hops over a few bytes inside the routine it is in.
fn ends_run(text: &str) -> bool {
    matches!(text, "RET" | "RETI" | "RETN") || (text.starts_with("JP ") && !text.contains(','))
}

/// The runs of instructions that ran, from `from` to the top of memory.
fn runs<F: Fn(u16) -> u8>(
    peek: &F,
    ran: &dyn Fn(u16) -> bool,
    from: u16,
    to: u16,
) -> Vec<Vec<(u16, Insn)>> {
    let mut out = Vec::new();
    let mut run: Vec<(u16, Insn)> = Vec::new();
    let mut at = from as u32;
    while at <= to as u32 {
        let here = at as u16;
        if !ran(here) {
            if !run.is_empty() {
                out.push(std::mem::take(&mut run));
            }
            at += 1;
            continue;
        }
        let insn = disasm(peek, here);
        let len = insn.len.max(1) as u32;
        let ends = ends_run(&insn.text);
        run.push((here, insn));
        if ends {
            out.push(std::mem::take(&mut run));
        }
        at += len;
    }
    if !run.is_empty() {
        out.push(run);
    }
    out
}

/// Every fragment the ROM holds, read straight through from $0000. Its
/// tables decode as nonsense, which matches nothing and costs nothing.
pub fn rom_grams<F: Fn(u16) -> u8>(peek: &F) -> HashSet<u64> {
    runs(peek, &|_| true, 0, 0x3FFF)
        .iter()
        .flat_map(|run| grams_of(run))
        .collect()
}

/// What can be told about how a run of code does things, from its
/// instructions alone.
fn techniques(run: &[(u16, Insn)], found: &mut BTreeMap<String, u16>) {
    let text = |i: usize| run.get(i).map(|(_, insn)| insn.text.as_str()).unwrap_or("");
    let mut note = |name: &str, at: u16| {
        found.entry(name.to_string()).or_insert(at);
    };
    let has = |wanted: &str| run.iter().any(|(_, insn)| insn.text == wanted);
    let mut pushes = 0;
    let mut ldis = 0;
    let mut indexed = 0;
    for (i, (at, insn)) in run.iter().enumerate() {
        let t = insn.text.as_str();
        match t {
            "IM 2" => note("im2", *at),
            "LD A,R" => note("refresh-register", *at),
            "HALT" => note("halt-sync", *at),
            "RST $10" => note("rom-print", *at),
            "IN A,($1F)" => note("kempston", *at),
            _ => {}
        }
        let stores = (1..=2).any(|k| text(i + k) == "LD (HL),A");
        if t == "XOR (HL)" && stores {
            note("xor-draw", *at);
        }
        if t == "OR (HL)" && stores {
            note("or-draw", *at);
        }
        if t == "AND (HL)" && (1..=2).any(|k| text(i + k).starts_with("OR ")) {
            note("masked-draw", *at);
        }
        if t == "INC H" && (1..=3).any(|k| text(i + k) == "LD A,H") {
            note("inc-h-rows", *at);
        }
        pushes = if t.starts_with("PUSH ") {
            pushes + 1
        } else {
            0
        };
        if pushes == 6 && has_prefix(run, "LD SP,") {
            note("stack-blit", *at);
        }
        ldis = if t == "LDI" { ldis + 1 } else { 0 };
        if ldis == 8 {
            note("unrolled-ldi", *at);
        }
        if t.contains("(IX+") || t.contains("(IY+") || t.contains("(IX-") || t.contains("(IY-") {
            indexed += 1;
            if indexed == 8 {
                note("ix-records", *at);
            }
        }
    }
    let start = run[0].0;
    if has("AND $07") && has("AND $18") && has("OR $40") {
        note("pixel-address", start);
    }
    if has("XOR $10") && has("OUT ($FE),A") {
        note("beeper", start);
    }
    if (has("LD BC,$FFFD") || has("LD BC,$BFFD"))
        && run.iter().any(|(_, i)| i.text.starts_with("OUT (C),"))
    {
        note("ay", start);
    }
}

fn has_prefix(run: &[(u16, Insn)], prefix: &str) -> bool {
    run.iter().any(|(_, insn)| insn.text.starts_with(prefix))
}

/// Take the fingerprint of the code that ran: `ran` says which addresses an
/// instruction was fetched from, `label` names a block where the notes do,
/// `rom` is what `rom_grams` gave for the ROM in the machine, and `exclude` is
/// what somebody has said is not the game. The game and the file it came
/// from are left for the caller to say.
pub fn take<F: Fn(u16) -> u8>(
    peek: &F,
    ran: &dyn Fn(u16) -> bool,
    label: &dyn Fn(u16) -> Option<String>,
    rom: &HashSet<u64>,
    exclude: &[(u16, u16)],
) -> Fingerprint {
    let runs = runs(peek, ran, 0x4000, 0xFFFF);
    let mut found = BTreeMap::new();
    let mut samplers = Vec::new();
    for run in &runs {
        for (at, _) in run {
            if let Some(name) = crate::flashload::sampler_at(peek, *at) {
                samplers.push((*at, name));
                found.entry(format!("loader:{name}")).or_insert(*at);
            }
        }
    }
    let extent = |run: &[(u16, Insn)]| {
        let last = run.last().unwrap();
        (run[0].0, last.0.wrapping_add(last.1.len.max(1) as u16 - 1))
    };
    // A loader is a few hundred bytes of code packed together round its
    // sampler, so its runs are taken from the sampler outward for as long as
    // each is close to the last. A fixed reach either side was tried first and
    // left the end of Speedlock in Head over Heels and RoboCop, $145 bytes
    // above its sampler, where it made two games by different people look 17%
    // related.
    let mut loader = vec![None; runs.len()];
    for (sampler, name) in &samplers {
        let Some(seed) = runs.iter().position(|run| {
            let (start, end) = extent(run);
            start <= *sampler && *sampler <= end
        }) else {
            continue;
        };
        loader[seed] = Some(*name);
        for step in [-1isize, 1] {
            let mut at = seed as isize;
            loop {
                let next = at + step;
                if next < 0 || next as usize >= runs.len() {
                    break;
                }
                let (near, far) = if step > 0 {
                    (extent(&runs[at as usize]).1, extent(&runs[next as usize]).0)
                } else {
                    (extent(&runs[next as usize]).1, extent(&runs[at as usize]).0)
                };
                if far.saturating_sub(near) > LOADER_GAP {
                    break;
                }
                loader[next as usize] = Some(*name);
                at = next;
            }
        }
    }
    let mut blocks = Vec::new();
    let mut left_out = Vec::new();
    for (index, run) in runs.into_iter().enumerate() {
        let (start, end) = extent(&run);
        if let Some(name) = loader[index] {
            left_out.push((start, end, format!("part of the loader round {name}")));
            continue;
        }
        if exclude
            .iter()
            .any(|(low, high)| start <= *high && end >= *low)
        {
            left_out.push((start, end, "excluded when the fingerprint was taken".into()));
            continue;
        }
        techniques(&run, &mut found);
        let mut seen = HashSet::new();
        let grams: Vec<u64> = grams_of(&run)
            .into_iter()
            .filter(|g| !rom.contains(g) && seen.insert(*g))
            .collect();
        blocks.push(Block {
            start,
            end,
            label: label(start).unwrap_or_default(),
            instructions: run.len(),
            grams,
        });
    }
    Fingerprint {
        blocks,
        techniques: found,
        left_out,
        ..Fingerprint::default()
    }
}

// ---- the file --------------------------------------------------------------

pub fn to_text(print: &Fingerprint) -> String {
    let game = &print.game;
    let mut out = String::from(
        "; ZX-Rustrum lineage fingerprint: the code a game was seen to run, in fragments.\n\
         ; The game's details can be edited; everything below them was measured.\n",
    );
    out.push_str(&format!("title {}\n", game.title));
    if let Some(year) = game.year {
        out.push_str(&format!("year {year}\n"));
    }
    if !game.publisher.is_empty() {
        out.push_str(&format!("publisher {}\n", game.publisher));
    }
    if !game.authors.is_empty() {
        out.push_str(&format!("authors {}\n", authors_text(&game.authors)));
    }
    if let Some(id) = game.zxdb {
        out.push_str(&format!("zxdb {id}\n"));
    }
    out.push_str(&format!("source {}\n", print.source));
    for (name, at) in &print.techniques {
        out.push_str(&format!("technique ${at:04X} {name}\n"));
    }
    for (low, high, why) in &print.left_out {
        out.push_str(&format!("left ${low:04X} ${high:04X} {why}\n"));
    }
    for block in &print.blocks {
        out.push_str(&format!(
            "block ${:04X} ${:04X} {} {}\n",
            block.start, block.end, block.instructions, block.label
        ));
        if !block.grams.is_empty() {
            let grams: Vec<String> = block.grams.iter().map(|g| format!("{g:016x}")).collect();
            out.push_str(&format!("grams {}\n", grams.join(" ")));
        }
    }
    out
}

fn hex(word: &str) -> Option<u16> {
    u16::from_str_radix(word.trim_start_matches('$'), 16).ok()
}

/// Read a fingerprint back. A line that cannot be read is skipped rather than
/// losing the file, as the notes are.
pub fn parse(text: &str) -> Fingerprint {
    let mut print = Fingerprint::default();
    for line in text.lines() {
        let line = line.trim_end();
        if line.starts_with(';') || line.is_empty() {
            continue;
        }
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "title" => print.game.title = rest.to_string(),
            "year" => print.game.year = rest.trim().parse().ok(),
            "publisher" => print.game.publisher = rest.to_string(),
            "authors" => print.game.authors = parse_authors(rest),
            "zxdb" => print.game.zxdb = rest.trim().parse().ok(),
            "source" => print.source = rest.to_string(),
            "technique" => {
                if let Some((at, name)) = rest.split_once(' ') {
                    if let Some(at) = hex(at) {
                        print.techniques.insert(name.to_string(), at);
                    }
                }
            }
            "left" => {
                let mut words = rest.splitn(3, ' ');
                if let (Some(Some(low)), Some(Some(high))) =
                    (words.next().map(hex), words.next().map(hex))
                {
                    print
                        .left_out
                        .push((low, high, words.next().unwrap_or("").to_string()));
                }
            }
            "block" => {
                let mut words = rest.splitn(4, ' ');
                let start = words.next().and_then(hex);
                let end = words.next().and_then(hex);
                let count = words.next().and_then(|w| w.parse().ok());
                if let (Some(start), Some(end), Some(instructions)) = (start, end, count) {
                    print.blocks.push(Block {
                        start,
                        end,
                        label: words.next().unwrap_or("").to_string(),
                        instructions,
                        grams: Vec::new(),
                    });
                }
            }
            "grams" => {
                if let Some(block) = print.blocks.last_mut() {
                    block.grams = rest
                        .split_whitespace()
                        .filter_map(|g| u64::from_str_radix(g, 16).ok())
                        .collect();
                }
            }
            _ => {}
        }
    }
    print
}

/// The title, year and publisher a TOSEC-style name gives:
/// `Jetpac (1983)(Ultimate Play The Game)[16K].tzx`.
pub fn from_file_name(name: &str) -> Game {
    let stem = name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name);
    let mut game = Game::default();
    let Some(open) = stem.find(" (") else {
        game.title = stem.trim().to_string();
        return game;
    };
    game.title = stem[..open].trim().to_string();
    let parts: Vec<&str> = stem[open..]
        .split(['(', ')'])
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('['))
        .collect();
    if let Some(year) = parts.first() {
        game.year = year.get(..4).and_then(|y| y.parse().ok());
    }
    if let Some(publisher) = parts.get(1) {
        game.publisher = publisher.to_string();
    }
    game
}

// ---- ZXDB -------------------------------------------------------------------

pub const ZXDB_FILE: &str = "zxdb-games.txt";

fn simple(text: &str) -> String {
    let lower = text.to_lowercase();
    let lower = lower.strip_prefix("the ").unwrap_or(&lower);
    lower.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Find the game in what `tools/zxdb-games.py` wrote. ZXDB has many titles
/// several times over — a remake, a re-release, a type-in of the same name —
/// so the year and the publisher choose between them, and with neither to go
/// on a title that is there more than once is not guessed at.
pub fn find_in_zxdb(list: &str, wanted: &Game) -> Result<Game, String> {
    let found = find_title(list, wanted, &wanted.title);
    // TOSEC names carry a subtitle, or a part, after " - " that ZXDB's titles
    // mostly leave out: "Saboteur II - Avenging Angel" is "Saboteur II" there.
    match (&found, wanted.title.split_once(" - ")) {
        (Err(_), Some((short, _))) => {
            find_title(list, wanted, short).map_err(|_| found.unwrap_err())
        }
        _ => found,
    }
}

fn find_title(list: &str, wanted: &Game, title: &str) -> Result<Game, String> {
    let title = simple(title);
    let publisher = simple(&wanted.publisher);
    let mut candidates = Vec::new();
    for line in list.lines() {
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 5 || simple(fields[1]) != title {
            continue;
        }
        let year: Option<u16> = fields[2].parse().ok();
        let mut score = 0;
        if wanted.year.is_some() && year == wanted.year {
            score += 2;
        }
        let theirs = simple(fields[3]);
        if !publisher.is_empty()
            && !theirs.is_empty()
            && (theirs.contains(&publisher) || publisher.contains(&theirs))
        {
            score += 1;
        }
        candidates.push((
            score,
            Game {
                title: fields[1].to_string(),
                year,
                publisher: fields[3].to_string(),
                authors: parse_authors(fields[4]),
                zxdb: fields[0].parse().ok(),
            },
        ));
    }
    candidates.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    match candidates.as_slice() {
        [] => Err(format!("ZXDB has no title {:?}", wanted.title)),
        [(_, only)] => Ok(only.clone()),
        [(best, game), (next, _), ..] if best > next => Ok(game.clone()),
        _ => Err(format!(
            "ZXDB has {} titles {:?} and nothing to choose between them: give the year or the publisher",
            candidates.len(),
            wanted.title
        )),
    }
}

// ---- comparing -------------------------------------------------------------

/// How much one game holds of an earlier one. "Earlier" is by year and then by
/// title, so within a year it says nothing about which came first.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub earlier: usize,
    pub later: usize,
    /// Fragments the two have in common.
    pub shared: usize,
    /// The share of the later game's code found in the earlier, weighted.
    pub of_later: f64,
    /// The same the other way round.
    pub of_earlier: f64,
    pub same_year: bool,
    /// Techniques both use.
    pub techniques: Vec<String>,
}

/// Where a block of one game is found first among the games before it.
#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    pub block: usize,
    pub from: usize,
    pub found: usize,
    pub of: usize,
}

pub struct Lineage {
    /// The games, oldest first.
    pub games: Vec<Fingerprint>,
    /// Every pair holding a fragment in common.
    pub links: Vec<Link>,
    /// For each game, the link to the earlier game it holds most of, if it
    /// holds enough of one to count.
    pub parent: Vec<Option<usize>>,
    sets: Vec<HashSet<u64>>,
    weight: HashMap<u64, f64>,
}

/// A fragment two games hold says those two are related. One that half the
/// corpus holds says much less about any pair of them — it is an idiom, or a
/// routine everybody copied out of the same magazine — so its evidence is
/// shared out: a fragment in `d` games counts for `2/d`.
fn weight(df: usize) -> f64 {
    if df <= 2 {
        1.0
    } else {
        2.0 / df as f64
    }
}

impl Lineage {
    pub fn new(mut games: Vec<Fingerprint>) -> Lineage {
        games.sort_by(|a, b| {
            (a.game.year.unwrap_or(u16::MAX), &a.game.title)
                .cmp(&(b.game.year.unwrap_or(u16::MAX), &b.game.title))
        });
        let sets: Vec<HashSet<u64>> = games.iter().map(Fingerprint::grams).collect();
        let mut df: HashMap<u64, usize> = HashMap::new();
        for set in &sets {
            for g in set {
                *df.entry(*g).or_default() += 1;
            }
        }
        let weight: HashMap<u64, f64> = df.into_iter().map(|(g, d)| (g, self::weight(d))).collect();
        let mass = |set: &HashSet<u64>| set.iter().map(|g| weight[g]).sum::<f64>();
        let masses: Vec<f64> = sets.iter().map(mass).collect();

        let mut links = Vec::new();
        for later in 0..games.len() {
            for earlier in 0..later {
                let common: Vec<u64> = sets[later].intersection(&sets[earlier]).copied().collect();
                if common.is_empty() {
                    continue;
                }
                let shared_mass: f64 = common.iter().map(|g| weight[g]).sum();
                let techniques = games[later]
                    .techniques
                    .keys()
                    .filter(|t| games[earlier].techniques.contains_key(*t))
                    .cloned()
                    .collect();
                links.push(Link {
                    earlier,
                    later,
                    shared: common.len(),
                    of_later: shared_mass / masses[later].max(1e-9),
                    of_earlier: shared_mass / masses[earlier].max(1e-9),
                    same_year: games[earlier].game.year == games[later].game.year,
                    techniques,
                });
            }
        }
        let parent = (0..games.len())
            .map(|game| {
                links
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| {
                        l.later == game && l.of_later >= MIN_SHARE && l.shared >= MIN_SHARED
                    })
                    .max_by(|(_, a), (_, b)| a.of_later.total_cmp(&b.of_later))
                    .map(|(i, _)| i)
            })
            .collect();
        Lineage {
            games,
            links,
            parent,
            sets,
            weight,
        }
    }

    pub fn index_of(&self, title: &str) -> Option<usize> {
        let wanted = simple(title);
        self.games
            .iter()
            .position(|g| simple(&g.game.title) == wanted)
            .or_else(|| {
                self.games
                    .iter()
                    .position(|g| simple(&g.game.title).contains(&wanted))
            })
    }

    /// The links touching one game, strongest first by how much of the later
    /// one they account for.
    pub fn links_of(&self, game: usize) -> Vec<&Link> {
        let mut links: Vec<&Link> = self
            .links
            .iter()
            .filter(|l| l.earlier == game || l.later == game)
            .collect();
        links.sort_by(|a, b| {
            b.of_later
                .max(b.of_earlier)
                .total_cmp(&a.of_later.max(a.of_earlier))
        });
        links
    }

    /// The weight of one fragment, for whatever wants to explain a number.
    pub fn weight_of(&self, gram: u64) -> f64 {
        self.weight.get(&gram).copied().unwrap_or(0.0)
    }

    /// For each block of a game with most of its fragments in an earlier game,
    /// the earliest game holding the most of them. Earliest in the corpus, which
    /// is not the same as where it was first written.
    pub fn origins(&self, game: usize) -> Vec<Origin> {
        let mut out = Vec::new();
        for (index, block) in self.games[game].blocks.iter().enumerate() {
            if block.grams.is_empty() {
                continue;
            }
            let best = (0..game)
                .map(|from| {
                    (
                        from,
                        block
                            .grams
                            .iter()
                            .filter(|g| self.sets[from].contains(g))
                            .count(),
                    )
                })
                .filter(|(_, found)| *found > 0)
                .max_by(|(a, x), (b, y)| x.cmp(y).then(b.cmp(a)));
            if let Some((from, found)) = best {
                if found * 2 >= block.grams.len() {
                    out.push(Origin {
                        block: index,
                        from,
                        found,
                        of: block.grams.len(),
                    });
                }
            }
        }
        out
    }

    /// Who is suspected of having written code that came from `from` into
    /// `game`: the people who may have coded the earlier one and are not
    /// credited with coding this one. Empty where they are the same people.
    pub fn suspected(&self, game: usize, from: usize) -> Vec<String> {
        let own = self.games[game].game.coders();
        self.games[from]
            .game
            .coders()
            .into_iter()
            .filter(|c| !own.contains(c))
            .map(str::to_string)
            .collect()
    }
}
