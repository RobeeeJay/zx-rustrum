//! Placing every disassembled game in a family tree: which earlier games hold
//! its code, how much, and whose code that was. The measuring is in
//! `src/lineage.rs`; this is the tools and the page that draws the result.

use std::path::{Path, PathBuf};

use crate::lineage::{self, Fingerprint, Game, Lineage};
use crate::machine::Model;
use crate::mcp::json::Json;
use crate::mcp::tools::{text, Session};

/// Where `export_asm`'s files are kept, and so where the games are read from.
pub const DISASSEMBLIES: &str = "disassembly";

/// The game a disassembly is of: what its file name says, then what ZXDB
/// says, with a word on what ZXDB made of it when that was nothing.
fn game_for(file_name: &str, zxdb: Option<&str>) -> (Game, Option<String>) {
    let mut game = lineage::from_file_name(file_name);
    let Some(list) = zxdb else {
        return (game, None);
    };
    match lineage::find_in_zxdb(list, &game) {
        Ok(found) => {
            game.title = found.title;
            game.year = found.year.or(game.year);
            if !found.publisher.is_empty() {
                game.publisher = found.publisher;
            }
            game.authors = found.authors;
            game.zxdb = found.zxdb;
            (game, None)
        }
        Err(why) => (game, Some(why)),
    }
}

fn corpus_dir(args: &Json) -> Result<PathBuf, String> {
    match args.get("dir") {
        Some(_) => Ok(PathBuf::from(text(args, "dir")?)),
        None => Ok(PathBuf::from(DISASSEMBLIES)),
    }
}

/// Every disassembly in a directory, fingerprinted, and what could not be
/// said about any of them. A file that cannot be read is named rather than
/// left out quietly, since a game missing from the tree reads as a game with
/// no relatives.
pub fn read_corpus(
    session: &Session,
    dir: &Path,
) -> Result<(Vec<Fingerprint>, Vec<String>), String> {
    let rom = session.rom_for(Model::Spectrum48).map_err(|_| {
        "the 48K ROM is needed, to take the ROM's own routines out of every game: put it in roms/"
            .to_string()
    })?;
    let rom = lineage::rom_grams(&|a: u16| rom.get(a as usize).copied().unwrap_or(0));
    let zxdb = crate::prefs::config_dir()
        .and_then(|d| std::fs::read_to_string(d.join(lineage::ZXDB_FILE)).ok());
    let mut notes = Vec::new();
    if zxdb.is_none() {
        notes.push(format!(
            "No {} in the preferences directory, so no authors: tools/zxdb-games.py writes it.",
            lineage::ZXDB_FILE
        ));
    }
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.to_ascii_lowercase().ends_with(".asm"))
        .collect();
    names.sort();
    let mut games = Vec::new();
    for name in names {
        let text = std::fs::read_to_string(dir.join(&name)).map_err(|e| format!("{name}: {e}"))?;
        let disassembly = match lineage::read::read_asm(&text) {
            Ok(d) => d,
            Err(why) => {
                notes.push(format!("{name} was not read: {why}."));
                continue;
            }
        };
        let (game, why) = game_for(&name, zxdb.as_deref());
        if let Some(why) = why {
            notes.push(format!("{name}: {why}, so its authors are not known."));
        }
        games.push(Fingerprint {
            game,
            source: name,
            ..lineage::from_disassembly(&disassembly, &rom, &[])
        });
    }
    if games.is_empty() {
        return Err(format!(
            "no disassembly could be read in {}: export_asm writes one, as <tape name>.asm",
            dir.display()
        ));
    }
    Ok((games, notes))
}

fn year(game: &Game) -> String {
    game.year
        .map(|y| y.to_string())
        .unwrap_or_else(|| "????".into())
}

fn percent(share: f64) -> String {
    let p = share * 100.0;
    if p > 0.0 && p < 1.0 {
        "<1%".into()
    } else {
        format!("{p:.0}%")
    }
}

fn credit(game: &Game) -> String {
    let coders = game.coders();
    if coders.is_empty() {
        "code: not known".into()
    } else {
        format!("code: {}", coders.join(", "))
    }
}

fn children(tree: &Lineage) -> Vec<Vec<usize>> {
    let mut kids = vec![Vec::new(); tree.games.len()];
    for game in 0..tree.games.len() {
        if let Some(parent) = tree.parent_game(game) {
            kids[parent].push(game);
        }
    }
    kids
}

/// How much of each game's code the other holds, `game`'s first.
fn shares(link: &lineage::Link, game: usize) -> (f64, f64) {
    if link.later == game {
        (link.of_later, link.of_earlier)
    } else {
        (link.of_earlier, link.of_later)
    }
}

/// Why a game hangs where it does, in words: from an earlier game the share of
/// its code there; from a later or same-year one, the fragments shared both
/// ways, since that link says nothing about which way the code went.
fn tree_reason(tree: &Lineage, game: usize) -> Option<String> {
    let link = &tree.links[tree.parent[game]?];
    let above = tree.parent_game(game)?;
    let title = &tree.games[above].game.title;
    let (mine, theirs) = shares(link, game);
    let mut words = if tree.came_before(above, game) {
        format!(
            "{} of its code is in {title} ({} fragments)",
            percent(mine),
            link.shared
        )
    } else {
        format!(
            "shares {} fragments with {title}, {}: {} of its code is there, {} of {title}'s is here{}",
            link.shared,
            match relation(tree, game, above) {
                "later" => "a later game",
                "same year" => "of the same year",
                _ => "whose year is not known",
            },
            percent(mine),
            percent(theirs),
            if tree.came_before(game, above) {
                ""
            } else {
                " (which came first is not known)"
            }
        )
    };
    if tree.weak(link) {
        words.push_str("; a weak link, which may be no more than shared idioms");
    }
    Some(words)
}

/// Where `other` stands in time against `game`, in words.
fn relation(tree: &Lineage, game: usize, other: usize) -> &'static str {
    if tree.came_before(other, game) {
        "earlier"
    } else if tree.came_before(game, other) {
        "later"
    } else if tree.games[game].game.year.is_some()
        && tree.games[game].game.year == tree.games[other].game.year
    {
        "same year"
    } else {
        "year not known"
    }
}

/// The relatives of `game` with no direction to them, each as the other game
/// and how much of each one's code the other holds, this game's first.
fn kin_of(tree: &Lineage, game: usize) -> Vec<(usize, f64, f64, &lineage::Link)> {
    let mut out: Vec<_> = tree
        .kin
        .iter()
        .map(|i| &tree.links[*i])
        .filter_map(|l| {
            if l.later == game {
                Some((l.earlier, l.of_later, l.of_earlier, l))
            } else if l.earlier == game {
                Some((l.later, l.of_earlier, l.of_later, l))
            } else {
                None
            }
        })
        .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out
}

fn draw_tree(
    tree: &Lineage,
    kids: &[Vec<usize>],
    game: usize,
    prefix: &str,
    last: bool,
    top: bool,
    out: &mut String,
) {
    let print = &tree.games[game];
    let branch = if top {
        ""
    } else if last {
        "└─ "
    } else {
        "├─ "
    };
    let why = match tree_reason(tree, game) {
        Some(words) => format!(" — {words}"),
        None => format!(" — {} — {}", print.game.publisher, credit(&print.game)),
    };
    out.push_str(&format!(
        "{prefix}{branch}{} {}{why}\n",
        year(&print.game),
        print.game.title
    ));
    let next = if top {
        prefix.to_string()
    } else {
        format!("{prefix}{}", if last { "   " } else { "│  " })
    };
    let bar = if kids[game].is_empty() {
        "   "
    } else {
        "│  "
    };
    for (other, mine, theirs, _) in kin_of(tree, game) {
        let o = &tree.games[other].game;
        out.push_str(&format!(
            "{next}{bar}· shares code with {} {}: {} of its code is there, {} of {}'s is here ({}: which came first is not known)\n",
            year(o),
            o.title,
            percent(mine),
            percent(theirs),
            o.title,
            relation(tree, game, other)
        ));
    }
    for (i, child) in kids[game].iter().enumerate() {
        draw_tree(
            tree,
            kids,
            *child,
            &next,
            i + 1 == kids[game].len(),
            false,
            out,
        );
    }
}

pub fn lineage(session: &mut Session, args: &Json) -> Result<String, String> {
    let dir = corpus_dir(args)?;
    let (games, notes) = read_corpus(session, &dir)?;
    let tree = Lineage::new(games);
    let mut out = format!(
        "{} games disassembled in {}, oldest first. A share is how much of the later game's \
         code is also in the earlier one: fragments of {} instructions, a fragment that many \
         games hold counting for less. Each game is joined to the game it shares the most \
         fragments with, and each family hangs from its oldest game. Where the game above is \
         of the same year, the link says nothing about which way the code went, and \
         says so; a link under 5% and 12 fragments is marked weak. Other games of the same \
         year sharing code are listed under each game.\n\n",
        tree.games.len(),
        dir.display(),
        lineage::GRAM
    );
    for note in &notes {
        out.push_str(&format!("{note}\n"));
    }
    if !notes.is_empty() {
        out.push('\n');
    }
    let kids = children(&tree);
    for root in (0..tree.games.len()).filter(|g| tree.parent[*g].is_none()) {
        draw_tree(&tree, &kids, root, "", true, true, &mut out);
    }

    let focus =
        match args.get("game").and_then(|v| v.as_str()) {
            Some(title) => Some(tree.index_of(title).ok_or_else(|| {
                format!("no disassembly of a game called {title:?} in the folder")
            })?),
            None => None,
        };
    match focus {
        Some(game) => out.push_str(&about(&tree, game)),
        None => {
            out.push_str("\nStrongest links:\n");
            let mut links: Vec<_> = tree.links.iter().collect();
            links.sort_by(|a, b| b.of_later.total_cmp(&a.of_later));
            for link in links.iter().take(15) {
                out.push_str(&format!("  {}\n", link_line(&tree, link)));
            }
            out.push_str(
                "\nPass game to see one game's relatives and where its routines came from.",
            );
        }
    }
    Ok(out)
}

fn link_line(tree: &Lineage, link: &lineage::Link) -> String {
    format!(
        "{} {} holds {} of {} {}'s code, and it {} of the earlier's; {} fragments{}",
        year(&tree.games[link.earlier].game),
        tree.games[link.earlier].game.title,
        percent(link.of_later),
        year(&tree.games[link.later].game),
        tree.games[link.later].game.title,
        percent(link.of_earlier),
        link.shared,
        if link.same_year { ", same year" } else { "" }
    )
}

fn about(tree: &Lineage, game: usize) -> String {
    let print = &tree.games[game];
    let g = &print.game;
    let mut out = format!(
        "\n{} ({}, {})\nauthors: {}\n{} instructions in its disassembly, {} fragments.\n",
        g.title,
        year(g),
        g.publisher,
        if g.authors.is_empty() {
            "not known".into()
        } else {
            lineage::authors_text(&g.authors)
        },
        print.instructions(),
        print.grams().len()
    );
    out.push_str("\nRelatives:\n");
    let links = tree.links_of(game);
    if links.is_empty() {
        out.push_str("  none: no fragment of its code is in any other game fingerprinted\n");
    }
    for link in links.iter().take(12) {
        out.push_str(&format!("  {}\n", link_line(tree, link)));
        if !link.techniques.is_empty() {
            let words: Vec<String> = link
                .techniques
                .iter()
                .map(|t| lineage::describe(t))
                .collect();
            out.push_str(&format!("    both: {}\n", words.join("; ")));
        }
    }
    let origins = tree.origins(game);
    out.push_str(&format!(
        "\nRoutines found in another game ({} of {} runs that have fragments). An earlier game \
         holding one is named first: the earliest in this collection holding the most of it, \
         which is not always where it was first written. Otherwise it is a game of the same year \
         or a later one, which says the code was shared but not which way it went. A suspected \
         author is a guess, from who coded an earlier game, and is only given for one.\n",
        origins.len(),
        print.blocks.iter().filter(|b| !b.grams.is_empty()).count()
    ));
    for origin in origins.iter().take(40) {
        let block = &print.blocks[origin.block];
        let from = &tree.games[origin.from].game;
        let when = relation(tree, game, origin.from);
        let suspected = tree.suspected(game, origin.from);
        let who = if when != "earlier" {
            String::new()
        } else if suspected.is_empty() {
            " — the same coders".to_string()
        } else {
            format!(" — @suspected {}", suspected.join(", "))
        };
        out.push_str(&format!(
            "  ${:04X} {:<20} {}/{} fragments in {} ({}, {when}){who}\n",
            block.start,
            block.name(),
            origin.found,
            origin.of,
            from.title,
            year(from)
        ));
    }
    if origins.len() > 40 {
        out.push_str(&format!("  … and {} more\n", origins.len() - 40));
    }
    out
}

// ---- the page ---------------------------------------------------------------

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const CARD_W: f32 = 230.0;
const CARD_H: f32 = 58.0;
const ROW: f32 = 74.0;
const INDENT: f32 = 60.0;
/// Room on the left for nothing but the first branch's line. The years are on
/// the cards rather than down the side: rows go family by family, so a column
/// of years would run backwards wherever a new family starts.
const GUTTER: f32 = 8.0;

pub fn export_lineage(session: &mut Session, args: &Json) -> Result<String, String> {
    let path = PathBuf::from(text(args, "path")?);
    let dir = corpus_dir(args)?;
    let (games, notes) = read_corpus(session, &dir)?;
    let tree = Lineage::new(games);
    std::fs::write(&path, page(&tree)).map_err(|e| format!("{}: {e}", path.display()))?;
    let roots = tree.parent.iter().filter(|p| p.is_none()).count();
    let weak = tree
        .parent
        .iter()
        .flatten()
        .filter(|i| tree.weak(&tree.links[**i]))
        .count();
    Ok(format!(
        "Wrote {}: {} games, {} links drawn as branches ({weak} of them weak), {roots} {}, \
         {} other pairs of the same year drawn as sharing code.{}",
        path.display(),
        tree.games.len(),
        tree.games.len() - roots,
        if roots == 1 { "family" } else { "families" },
        tree.kin.len(),
        notes.iter().map(|n| format!("\n{n}")).collect::<String>()
    ))
}

/// The games in the order the tree is drawn: each family together, a game
/// below its parent, so the page reads oldest first within a family.
fn drawing_order(tree: &Lineage) -> Vec<(usize, usize)> {
    let kids = children(tree);
    let mut order = Vec::new();
    fn walk(game: usize, depth: usize, kids: &[Vec<usize>], order: &mut Vec<(usize, usize)>) {
        order.push((game, depth));
        for child in &kids[game] {
            walk(*child, depth + 1, kids, order);
        }
    }
    for root in (0..tree.games.len()).filter(|g| tree.parent[*g].is_none()) {
        walk(root, 0, &kids, &mut order);
    }
    order
}

pub fn page(tree: &Lineage) -> String {
    let order = drawing_order(tree);
    let row_of: std::collections::HashMap<usize, usize> = order
        .iter()
        .enumerate()
        .map(|(row, (game, _))| (*game, row))
        .collect();
    let depth_of: std::collections::HashMap<usize, usize> = order.iter().copied().collect();
    let deepest = order.iter().map(|(_, d)| *d).max().unwrap_or(0);
    let width = GUTTER + deepest as f32 * INDENT + CARD_W + 260.0;
    let height = order.len() as f32 * ROW + 20.0;
    let x_of = |game: usize| GUTTER + depth_of[&game] as f32 * INDENT;
    let y_of = |game: usize| 10.0 + row_of[&game] as f32 * ROW;

    let mut svg = format!(
        "<svg viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\" role=\"img\" \
         aria-label=\"Family tree of the games fingerprinted\">\n"
    );
    // The other links, the ones that did not become a branch, off to the right
    // of the cards so they do not cross them.
    let side = GUTTER + deepest as f32 * INDENT + CARD_W + 20.0;
    for (index, link) in tree.links.iter().enumerate() {
        let branch = tree.parent.contains(&Some(index));
        let kin = tree.kin.contains(&index);
        if branch
            || !kin
                && (!link.ordered
                    || link.of_later < lineage::MIN_SHARE
                    || link.shared < lineage::MIN_SHARED)
        {
            continue;
        }
        let (y1, y2) = (
            y_of(link.earlier) + CARD_H / 2.0,
            y_of(link.later) + CARD_H / 2.0,
        );
        let reach = side + 20.0 + (y2 - y1).abs().sqrt() * 6.0;
        let (class, words) = if kin {
            (
                "kin",
                format!(
                    "{} and {} share code, and which came first is not known: {} of {}'s is in {}, {} of {}'s in {}",
                    escape(&tree.games[link.earlier].game.title),
                    escape(&tree.games[link.later].game.title),
                    percent(link.of_later),
                    escape(&tree.games[link.later].game.title),
                    escape(&tree.games[link.earlier].game.title),
                    percent(link.of_earlier),
                    escape(&tree.games[link.earlier].game.title),
                    escape(&tree.games[link.later].game.title)
                ),
            )
        } else {
            (
                "other",
                format!(
                    "{} holds {} of {}'s code",
                    escape(&tree.games[link.earlier].game.title),
                    percent(link.of_later),
                    escape(&tree.games[link.later].game.title)
                ),
            )
        };
        svg.push_str(&format!(
            "<path class=\"{class}\" d=\"M{side} {y1} C{reach} {y1} {reach} {y2} {side} {y2}\" \
             stroke-width=\"{:.1}\"><title>{words}</title></path>\n",
            1.0 + link.of_later.max(link.of_earlier) * 4.0,
        ));
    }
    for (game, _) in &order {
        if let (Some(index), Some(above)) = (tree.parent[*game], tree.parent_game(*game)) {
            let link = &tree.links[index];
            let (mine, _) = shares(link, *game);
            let px = x_of(above) + 18.0;
            let py = y_of(above) + CARD_H;
            let cx = x_of(*game);
            let cy = y_of(*game) + CARD_H / 2.0;
            // A branch from an earlier game is solid; one from a game of the
            // same year says nothing about direction, and a
            // weak one may be no more than shared idioms.
            let class = if tree.weak(link) {
                "branch weak"
            } else if tree.came_before(above, *game) {
                "branch"
            } else {
                "branch level"
            };
            svg.push_str(&format!(
                "<path class=\"{class}\" d=\"M{px} {py} V{cy} H{cx}\" stroke-width=\"{:.1}\"><title>{}</title></path>\n\
                 <text class=\"share\" x=\"{}\" y=\"{}\">{}</text>\n",
                1.5 + mine * 6.0,
                escape(&format!(
                    "{}: {}",
                    tree.games[*game].game.title,
                    tree_reason(tree, *game).unwrap_or_default()
                )),
                px + 6.0,
                cy - 6.0,
                percent(mine)
            ));
        }
    }
    for (game, _) in &order {
        let g = &tree.games[*game].game;
        let (x, y) = (x_of(*game), y_of(*game));
        svg.push_str(&format!(
            "<a href=\"#g{game}\"><rect class=\"card\" x=\"{x}\" y=\"{y}\" width=\"{CARD_W}\" height=\"{CARD_H}\" rx=\"6\"/>\n\
             <text class=\"title\" x=\"{}\" y=\"{}\">{}</text>\n\
             <text class=\"meta\" x=\"{}\" y=\"{}\">{} · {}</text>\n\
             <text class=\"meta\" x=\"{}\" y=\"{}\">{}</text></a>\n",
            x + 10.0,
            y + 19.0,
            escape(&clip(&g.title, 30)),
            x + 10.0,
            y + 35.0,
            escape(&clip(&g.publisher, 24)),
            year(g),
            x + 10.0,
            y + 50.0,
            escape(&clip(&credit(g), 34)),
        ));
    }
    svg.push_str("</svg>\n");

    let mut details = String::new();
    for (game, _) in &order {
        details.push_str(&game_section(tree, *game));
    }
    format!(
        "<!doctype html>\n<html lang=\"en-GB\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>Game Lineage</title>\n<style>{STYLE}</style>\n</head>\n<body>\n<main>\n\
         <h1>Game lineage</h1>\n\
         <p class=\"lede\">{} games, oldest first within each family. Each game is joined to \
         the game it shares the most fragments with, and each family hangs from its oldest \
         game; a branch's number is how much of the lower game's code the upper one holds: fragments \
         of {} instructions from each game's disassembly, the ROM's own taken out, a fragment many games hold counting \
         for less. A solid branch comes from an earlier year. A dotted branch comes from a game \
         of the same year, and says nothing about which way the code went, \
         since within a year the order is by title. A faint dashed branch is weak, under 5% and \
         12 fragments. Dashed lines on the right are other earlier games holding at least {} of \
         a game; dotted ones join other games of the same year that share code. Hover over a \
         line for what it is. Under each game, a routine found in another game opens to show \
         the two side by side.</p>\n\
         <div class=\"tree\">{svg}</div>\n{details}</main>\n</body>\n</html>\n",
        tree.games.len(),
        lineage::GRAM,
        percent(lineage::MIN_SHARE)
    )
}

fn clip(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(most - 1).collect::<String>())
    }
}

fn game_section(tree: &Lineage, game: usize) -> String {
    let print = &tree.games[game];
    let g = &print.game;
    let mut out = format!(
        "<section id=\"g{game}\">\n<h2>{} <span>{} · {}</span></h2>\n<p>Authors: {}</p>\n\
         <p class=\"small\">{} instructions in its disassembly, {} fragments{}.</p>\n",
        escape(&g.title),
        escape(if g.publisher.is_empty() {
            "publisher not known"
        } else {
            &g.publisher
        }),
        year(g),
        escape(&if g.authors.is_empty() {
            "not known".into()
        } else {
            lineage::authors_text(&g.authors)
        }),
        print.instructions(),
        print.grams().len(),
        g.zxdb
            .map(|id| format!(", ZXDB entry {id}"))
            .unwrap_or_default()
    );
    let links = tree.links_of(game);
    if !links.is_empty() {
        out.push_str(
            "<table><thead><tr><th>Related game</th><th>Of the later's code</th>\
             <th>Of the earlier's</th><th>Fragments</th><th>Techniques both use</th></tr></thead><tbody>\n",
        );
        for link in links.iter().take(12) {
            let other = if link.earlier == game {
                link.later
            } else {
                link.earlier
            };
            let o = &tree.games[other].game;
            let words: Vec<String> = link
                .techniques
                .iter()
                .map(|t| lineage::describe(t))
                .collect();
            out.push_str(&format!(
                "<tr><td><a href=\"#g{other}\">{}</a> ({}){}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
                escape(&o.title),
                year(o),
                if link.same_year { " <em>same year</em>" } else { "" },
                percent(link.of_later),
                percent(link.of_earlier),
                link.shared,
                escape(&words.join("; "))
            ));
        }
        out.push_str("</tbody></table>\n");
    }
    let origins = tree.origins(game);
    if !origins.is_empty() {
        out.push_str(
            "<details><summary>Routines found in another game</summary>\n\
             <p class=\"small\">An earlier game holding most of the routine is named first: the \
             earliest in this collection, which is not always where it was first written. Otherwise \
             it is a game of the same year or a later one, which says the code was shared but not \
             which way it went. A suspected author is a guess from who coded an earlier game, and \
             is only given where they are not credited with this one.</p>\n\
             <table><thead><tr><th>Routine</th><th>Fragments found</th><th>Found in</th>\
             <th>Suspected author</th></tr></thead><tbody>\n",
        );
        for origin in &origins {
            let block = &print.blocks[origin.block];
            let from = &tree.games[origin.from].game;
            let when = relation(tree, game, origin.from);
            let suspected = tree.suspected(game, origin.from);
            out.push_str(&format!(
                "<tr><td><code>${:04X}</code> {}</td><td>{}/{}</td><td><a href=\"#g{}\">{}</a> ({}, {when})</td><td>{}</td></tr>\n",
                block.start,
                escape(&block.label),
                origin.found,
                origin.of,
                origin.from,
                escape(&from.title),
                year(from),
                if when != "earlier" {
                    "<span class=\"small\">not a source</span>".to_string()
                } else if suspected.is_empty() {
                    "<span class=\"small\">the same coders</span>".to_string()
                } else {
                    format!("<em>{}</em>", escape(&suspected.join(", ")))
                }
            ));
            out.push_str(&diff_row(tree, game, origin));
        }
        out.push_str("</tbody></table></details>\n");
    }
    if !print.techniques.is_empty() {
        let words: Vec<String> = print
            .techniques
            .keys()
            .map(|t| escape(&lineage::describe(t)))
            .collect();
        out.push_str(&format!(
            "<p class=\"small\">Techniques: {}.</p>\n",
            words.join("; ")
        ));
    }
    out.push_str("</section>\n");
    out
}

/// A routine beside the block of the other game holding the most of its
/// fragments, lined up by instruction shape, folded away under the routine's
/// row.
fn diff_row(tree: &Lineage, game: usize, origin: &lineage::Origin) -> String {
    let mine = &tree.games[game].blocks[origin.block];
    let Some(other) = tree.counterpart(game, origin.block, origin.from) else {
        return String::new();
    };
    let theirs = &tree.games[origin.from].blocks[other];
    let lines = lineage::diff(&mine.text, &theirs.text);
    let same = lines
        .iter()
        .filter(|l| matches!(l, lineage::Diff::Both(a, b) if a == b))
        .count();
    let mut out = format!(
        "<tr class=\"diffrow\"><td colspan=\"4\"><details><summary>Side by side: <code>${:04X}</code> {} \
         here, <code>${:04X}</code> {} in {}. {} of {} lines the same, {} the same but for their \
         numbers.</summary>\n<table class=\"diff\"><thead><tr><th>{}</th><th>{}</th></tr></thead><tbody>\n",
        mine.start,
        escape(&mine.name()),
        theirs.start,
        escape(&theirs.name()),
        escape(&tree.games[origin.from].game.title),
        same,
        lines.len(),
        lines
            .iter()
            .filter(|l| matches!(l, lineage::Diff::Both(a, b) if a != b))
            .count(),
        escape(&tree.games[game].game.title),
        escape(&tree.games[origin.from].game.title),
    );
    for line in &lines {
        let (class, left, right) = match line {
            lineage::Diff::Both(a, b) if a == b => ("same", a.as_str(), b.as_str()),
            lineage::Diff::Both(a, b) => ("alike", a.as_str(), b.as_str()),
            lineage::Diff::Left(a) => ("left", a.as_str(), ""),
            lineage::Diff::Right(b) => ("right", "", b.as_str()),
        };
        out.push_str(&format!(
            "<tr class=\"{class}\"><td>{}</td><td>{}</td></tr>\n",
            escape(left),
            escape(right)
        ));
    }
    out.push_str("</tbody></table></details></td></tr>\n");
    out
}

const STYLE: &str = "
:root { --bg: #fbfaf7; --fg: #1d1c1a; --muted: #6b675f; --card: #ffffff; --line: #d8d3c8;
  --branch: #2f6f8f; --other: #b5651d; --accent: #2f6f8f; }
@media (prefers-color-scheme: dark) { :root:not([data-theme=\"light\"]) {
  --bg: #161614; --fg: #ecebe6; --muted: #9c978c; --card: #22211e; --line: #3b3934;
  --branch: #6fb3d2; --other: #e09a52; --accent: #6fb3d2; } }
:root[data-theme=\"dark\"] { --bg: #161614; --fg: #ecebe6; --muted: #9c978c; --card: #22211e;
  --line: #3b3934; --branch: #6fb3d2; --other: #e09a52; --accent: #6fb3d2; }
body { margin: 0; background: var(--bg); color: var(--fg);
  font: 15px/1.5 system-ui, -apple-system, 'Segoe UI', sans-serif; }
main { max-width: 1100px; margin: 0 auto; padding: 24px 16px 64px; }
h1 { font-size: 1.6rem; margin: 0 0 8px; }
h2 { font-size: 1.15rem; margin: 0 0 4px; }
h2 span { color: var(--muted); font-weight: normal; font-size: 0.95rem; }
.lede, .small { color: var(--muted); }
.small { font-size: 0.85rem; }
.tree { overflow-x: auto; margin: 16px 0 32px; }
svg text { fill: var(--fg); font-family: inherit; }
.card { fill: var(--card); stroke: var(--line); }
a:hover .card { stroke: var(--accent); }
.title { font-weight: 600; font-size: 14px; }
.meta { fill: var(--muted); font-size: 12px; }
.share { fill: var(--branch); font-size: 11px; font-weight: 600; }
.branch { fill: none; stroke: var(--branch); }
.other { fill: none; stroke: var(--other); stroke-dasharray: 4 3; opacity: 0.8; }
.branch.level { stroke-dasharray: 2 3; }
.branch.weak { stroke-dasharray: 5 4; opacity: 0.5; }
.kin { fill: none; stroke: var(--branch); stroke-dasharray: 1 3; stroke-linecap: round; }
section { border-top: 1px solid var(--line); padding: 16px 0; }
table { border-collapse: collapse; width: 100%; font-size: 0.85rem; margin: 8px 0;
  display: block; overflow-x: auto; }
th, td { text-align: left; padding: 4px 10px 4px 0; border-bottom: 1px solid var(--line);
  vertical-align: top; }
th { color: var(--muted); font-weight: 600; }
a { color: var(--accent); }
summary { cursor: pointer; color: var(--accent); margin: 8px 0; }
.diffrow summary { margin: 0; font-size: 0.8rem; }
table.diff { font: 12px/1.4 ui-monospace, Menlo, monospace; width: auto; }
table.diff td { padding: 0 16px 0 4px; white-space: pre; border: none; }
table.diff .alike td { background: color-mix(in srgb, var(--accent) 12%, transparent); }
table.diff .left td:first-child { background: color-mix(in srgb, #c0392b 18%, transparent); }
table.diff .right td:last-child { background: color-mix(in srgb, #27ae60 18%, transparent); }
";
