//! Fingerprinting a game's code, and placing it among the games already
//! fingerprinted: which earlier ones hold its code, how much, and whose code
//! that was. The measuring is in `src/lineage.rs`; this is the tools and the
//! page that draws the result.

use std::path::{Path, PathBuf};

use crate::lineage::{self, Fingerprint, Game, Lineage};
use crate::mcp::json::Json;
use crate::mcp::tools::{addr_of, text, Session};

/// The file the machine was loaded from, if it was loaded from one.
fn loaded_path(session: &Session) -> Option<PathBuf> {
    let loaded = session.loaded.as_deref()?;
    let (_, path) = loaded.split_once(' ')?;
    Some(PathBuf::from(path))
}

fn sidecar(source: &Path) -> PathBuf {
    source.with_extension(lineage::EXTENSION)
}

/// `[["$8000", "$80FF"], ...]`: what is not the game.
fn ranges(args: &Json, key: &str) -> Result<Vec<(u16, u16)>, String> {
    let Some(Json::Arr(items)) = args.get(key) else {
        return Ok(Vec::new());
    };
    items
        .iter()
        .map(|item| match item {
            Json::Arr(pair) if pair.len() == 2 => Ok((addr_of(&pair[0])?, addr_of(&pair[1])?)),
            other => Err(format!(
                "{key} wants pairs like [\"$8000\", \"$80FF\"], not {other}"
            )),
        })
        .collect()
}

pub fn fingerprint(session: &mut Session, args: &Json) -> Result<String, String> {
    if !session.rom_loaded {
        return Err(
            "there is no ROM in the machine, so nothing it ran is a game: load one first".into(),
        );
    }
    let source = loaded_path(session);
    let path = match (args.get("path"), &source) {
        (Some(_), _) => PathBuf::from(text(args, "path")?),
        (None, Some(source)) => sidecar(source),
        (None, None) => {
            return Err("nothing was loaded from a file, so say where to write it with path".into())
        }
    };

    // What the file name says, then ZXDB, then whatever was given outright.
    let file_name = source
        .as_ref()
        .and_then(|s| s.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut game = lineage::from_file_name(&file_name);
    if let Some(title) = args.get("title").and_then(|v| v.as_str()) {
        game.title = title.to_string();
    }
    if let Some(year) = args.get("year").and_then(|v| v.as_i64()) {
        game.year = Some(year as u16);
    }
    if let Some(publisher) = args.get("publisher").and_then(|v| v.as_str()) {
        game.publisher = publisher.to_string();
    }
    if game.title.is_empty() {
        return Err("the game needs a title: give one with title".into());
    }
    let zxdb_note = match crate::prefs::config_dir().map(|d| d.join(lineage::ZXDB_FILE)) {
        Some(file) => match std::fs::read_to_string(&file) {
            Ok(list) => match lineage::find_in_zxdb(&list, &game) {
                Ok(found) => {
                    let note = format!("ZXDB entry {}", found.zxdb.unwrap_or(0));
                    merge(&mut game, found);
                    note
                }
                Err(why) => why,
            },
            Err(_) => format!(
                "no {} in the preferences directory, so no authors: tools/zxdb-games.py writes it",
                lineage::ZXDB_FILE
            ),
        },
        None => "no preferences directory, so no ZXDB".into(),
    };
    if let Some(authors) = args.get("authors").and_then(|v| v.as_str()) {
        game.authors = lineage::parse_authors(authors);
    }

    let exclude = ranges(args, "exclude")?;
    let bus = &session.spec.bus;
    let peek = |a: u16| bus.peek_raw(a);
    let ran = |a: u16| {
        let phys = bus.phys_index(a);
        bus.tracker.executed[phys] || session.ran_before.get(phys).copied().unwrap_or(false)
    };
    let label = |a: u16| {
        let own = session.notes.label(a);
        if !own.is_empty() {
            return Some(own.to_string());
        }
        session.symbols.get(a).map(|(name, _)| name.to_string())
    };
    let rom = lineage::rom_grams(&peek);
    let print = Fingerprint {
        game,
        source: file_name,
        ..lineage::take(&peek, &ran, &label, &rom, &exclude)
    };
    let instructions = print.instructions();
    if instructions == 0 {
        return Err(
            "nothing outside the ROM has run since the machine was loaded: run the game first"
                .into(),
        );
    }
    std::fs::write(&path, lineage::to_text(&print))
        .map_err(|e| format!("{}: {e}", path.display()))?;

    let game = &print.game;
    let mut out = format!(
        "Wrote {}.\n{} — {} — {}. {zxdb_note}.\n",
        path.display(),
        game.title,
        game.year
            .map(|y| y.to_string())
            .unwrap_or_else(|| "year not known".into()),
        if game.publisher.is_empty() {
            "publisher not known"
        } else {
            &game.publisher
        },
    );
    out.push_str(&format!(
        "Authors: {}\n",
        if game.authors.is_empty() {
            "not known".to_string()
        } else {
            lineage::authors_text(&game.authors)
        }
    ));
    out.push_str(&format!(
        "{instructions} instructions that ran, in {} runs, {} fragments once the ROM's own are taken out.\n",
        print.blocks.len(),
        print.grams().len()
    ));
    let left: usize = print.left_out.len();
    if left > 0 {
        out.push_str(&format!("{left} runs left out:\n"));
        for (low, high, why) in print.left_out.iter().take(10) {
            out.push_str(&format!("  ${low:04X}-${high:04X} {why}\n"));
        }
    }
    if !print.techniques.is_empty() {
        out.push_str("Techniques:\n");
        for (name, at) in &print.techniques {
            out.push_str(&format!("  ${at:04X} {}\n", lineage::describe(name)));
        }
    }
    out.push_str(
        "Only code that ran is in it, so a game played further holds more. Call lineage to place it.",
    );
    Ok(out)
}

/// Take what ZXDB says, keeping a title, year or publisher already given
/// where ZXDB has none.
fn merge(game: &mut Game, found: Game) {
    game.title = found.title;
    game.year = found.year.or(game.year);
    if !found.publisher.is_empty() {
        game.publisher = found.publisher;
    }
    game.authors = found.authors;
    game.zxdb = found.zxdb;
}

fn corpus_dir(session: &Session, args: &Json) -> Result<PathBuf, String> {
    if args.get("dir").is_some() {
        return Ok(PathBuf::from(text(args, "dir")?));
    }
    Ok(loaded_path(session)
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("tapes")))
}

/// Every fingerprint in a directory.
pub fn read_corpus(dir: &Path) -> Result<Vec<Fingerprint>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut games = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(&format!(".{}", lineage::EXTENSION)) {
            let text = std::fs::read_to_string(entry.path()).map_err(|e| format!("{name}: {e}"))?;
            games.push(lineage::parse(&text));
        }
    }
    if games.is_empty() {
        return Err(format!(
            "no fingerprints in {}: call fingerprint on each game first",
            dir.display()
        ));
    }
    Ok(games)
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
    for (game, parent) in tree.parent.iter().enumerate() {
        if let Some(link) = parent {
            kids[tree.links[*link].earlier].push(game);
        }
    }
    kids
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
    let why = match tree.parent[game] {
        Some(link) => {
            let link = &tree.links[link];
            format!(
                " — {} of its code is in {}{}",
                percent(link.of_later),
                tree.games[link.earlier].game.title,
                if link.same_year {
                    " (same year: which came first is not known)"
                } else {
                    ""
                }
            )
        }
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
    let dir = corpus_dir(session, args)?;
    let tree = Lineage::new(read_corpus(&dir)?);
    let mut out = format!(
        "{} games fingerprinted in {}, oldest first. A share is how much of the later game's \
         code is also in the earlier one: fragments of {} instructions, a fragment that many \
         games hold counting for less. Within a year the order is by title and says nothing \
         about which came first.\n\n",
        tree.games.len(),
        dir.display(),
        lineage::GRAM
    );
    let kids = children(&tree);
    for root in (0..tree.games.len()).filter(|g| tree.parent[*g].is_none()) {
        draw_tree(&tree, &kids, root, "", true, true, &mut out);
    }

    let focus = match args.get("game").and_then(|v| v.as_str()) {
        Some(title) => Some(
            tree.index_of(title)
                .ok_or_else(|| format!("no fingerprint of a game called {title:?}"))?,
        ),
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
        "\n{} ({}, {})\nauthors: {}\n{} instructions that ran, {} fragments.\n",
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
        "\nRoutines found in an earlier game ({} of {} runs that have fragments). The earliest \
         in this collection, which is not always where it was first written. A suspected author \
         is a guess, from who coded the earlier game.\n",
        origins.len(),
        print.blocks.iter().filter(|b| !b.grams.is_empty()).count()
    ));
    for origin in origins.iter().take(40) {
        let block = &print.blocks[origin.block];
        let from = &tree.games[origin.from].game;
        let suspected = tree.suspected(game, origin.from);
        let who = if suspected.is_empty() {
            "the same coders".to_string()
        } else {
            format!("@suspected {}", suspected.join(", "))
        };
        out.push_str(&format!(
            "  ${:04X} {:<20} {}/{} fragments in {} ({}) — {who}\n",
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
    let dir = corpus_dir(session, args)?;
    let tree = Lineage::new(read_corpus(&dir)?);
    std::fs::write(&path, page(&tree)).map_err(|e| format!("{}: {e}", path.display()))?;
    let roots = tree.parent.iter().filter(|p| p.is_none()).count();
    Ok(format!(
        "Wrote {}: {} games, {} links drawn as branches, {roots} with no earlier relative.",
        path.display(),
        tree.games.len(),
        tree.games.len() - roots
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
        let branch = tree.parent[link.later] == Some(index);
        if branch || link.of_later < lineage::MIN_SHARE || link.shared < lineage::MIN_SHARED {
            continue;
        }
        let (y1, y2) = (
            y_of(link.earlier) + CARD_H / 2.0,
            y_of(link.later) + CARD_H / 2.0,
        );
        let reach = side + 20.0 + (y2 - y1).abs().sqrt() * 6.0;
        svg.push_str(&format!(
            "<path class=\"other\" d=\"M{side} {y1} C{reach} {y1} {reach} {y2} {side} {y2}\" \
             stroke-width=\"{:.1}\"><title>{} holds {} of {}'s code</title></path>\n",
            1.0 + link.of_later * 4.0,
            escape(&tree.games[link.earlier].game.title),
            percent(link.of_later),
            escape(&tree.games[link.later].game.title)
        ));
    }
    for (game, _) in &order {
        if let Some(index) = tree.parent[*game] {
            let link = &tree.links[index];
            let px = x_of(link.earlier) + 18.0;
            let py = y_of(link.earlier) + CARD_H;
            let cx = x_of(*game);
            let cy = y_of(*game) + CARD_H / 2.0;
            svg.push_str(&format!(
                "<path class=\"branch\" d=\"M{px} {py} V{cy} H{cx}\" stroke-width=\"{:.1}\"/>\n\
                 <text class=\"share\" x=\"{}\" y=\"{}\">{}</text>\n",
                1.5 + link.of_later * 6.0,
                px + 6.0,
                cy - 6.0,
                percent(link.of_later)
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
         <p class=\"lede\">{} games, oldest first within each family. A branch goes to the \
         earlier game holding the most of a game's code, and its number is how much: fragments \
         of {} instructions that ran, the ROM's own taken out, a fragment many games hold counting \
         for less. Dashed lines on the right are other earlier games holding at least {} of it. \
         Within a year the order is by title and says nothing about which came first.</p>\n\
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
         <p class=\"small\">{} instructions that ran, {} fragments{}.</p>\n",
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
            "<details><summary>Routines found in an earlier game</summary>\n\
             <p class=\"small\">The earliest game in this collection holding most of the routine, \
             which is not always where it was first written. A suspected author is a guess from \
             who coded that game, and is only given where they are not credited with this one.</p>\n\
             <table><thead><tr><th>Routine</th><th>Fragments found</th><th>Earliest in</th>\
             <th>Suspected author</th></tr></thead><tbody>\n",
        );
        for origin in &origins {
            let block = &print.blocks[origin.block];
            let from = &tree.games[origin.from].game;
            let suspected = tree.suspected(game, origin.from);
            out.push_str(&format!(
                "<tr><td><code>${:04X}</code> {}</td><td>{}/{}</td><td><a href=\"#g{}\">{}</a> ({})</td><td>{}</td></tr>\n",
                block.start,
                escape(&block.label),
                origin.found,
                origin.of,
                origin.from,
                escape(&from.title),
                year(from),
                if suspected.is_empty() {
                    "<span class=\"small\">the same coders</span>".to_string()
                } else {
                    format!("<em>{}</em>", escape(&suspected.join(", ")))
                }
            ));
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
section { border-top: 1px solid var(--line); padding: 16px 0; }
table { border-collapse: collapse; width: 100%; font-size: 0.85rem; margin: 8px 0;
  display: block; overflow-x: auto; }
th, td { text-align: left; padding: 4px 10px 4px 0; border-bottom: 1px solid var(--line);
  vertical-align: top; }
th { color: var(--muted); font-weight: 600; }
a { color: var(--accent); }
summary { cursor: pointer; color: var(--accent); margin: 8px 0; }
";
