//! One routine as a flow chart: what it does in the order it does it, with the
//! decisions it makes and how often each way was taken.
//!
//! Drawn top to bottom in address order, the way the listing reads, rather than
//! laid out as a graph: a fall-through is always the next thing down, so only
//! jumps need lines of their own, forward ones down the right and loops back up
//! the left.

use std::collections::{BTreeMap, BTreeSet};

use crate::disasm::disasm;
use crate::mcp::json::Json;
use crate::mcp::tools::{addr, count, text, Session};

/// How far from its start a routine is followed before a jump counts as leaving.
const REACH: u16 = 0x0600;

/// How a block ends.
#[derive(Clone, Debug, PartialEq)]
pub enum Exit {
    /// On into the next block.
    Fall(u16),
    /// Always to here.
    Jump(u16),
    /// To `taken` if the condition holds, otherwise on to `fall`.
    Branch {
        condition: String,
        taken: u16,
        fall: u16,
    },
    /// Returns if the condition holds, otherwise on.
    ReturnIf {
        condition: String,
        fall: u16,
    },
    Return,
    /// Through a register: JP (HL) and the like.
    Computed(String),
    /// Through a register to something that returns to `then`, its address
    /// pushed first: a dispatch through a table, which is a call in all but name.
    Dispatch {
        via: String,
        then: u16,
    },
}

#[derive(Clone, Debug)]
pub struct Block {
    pub start: u16,
    pub instructions: Vec<(u16, String)>,
    pub exit: Exit,
}

/// The four-digit addresses an instruction names.
fn targets(text: &str) -> Vec<u16> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('$') {
        let hex = rest.get(i + 1..i + 5).unwrap_or("");
        if hex.len() == 4 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            out.push(u16::from_str_radix(hex, 16).expect("four hex digits"));
        }
        rest = &rest[i + 1..];
    }
    out
}

/// The condition of a conditional jump or return, or None.
fn condition(text: &str) -> Option<String> {
    if text.starts_with("DJNZ") {
        return Some("B not 0 after one off".into());
    }
    let words: Vec<&str> = text.splitn(2, ' ').collect();
    let (op, rest) = (words[0], words.get(1).copied().unwrap_or(""));
    match op {
        "JR" | "JP" if rest.contains(',') => Some(rest.split(',').next().unwrap_or("").into()),
        "RET" if !rest.is_empty() => Some(rest.into()),
        _ => None,
    }
}

/// The blocks of the routine at `entry`, followed through the code that ran.
/// `outside` says whether a jump to an address leaves the routine.
pub fn blocks(
    peek: &dyn Fn(u16) -> u8,
    ran: &dyn Fn(u16) -> bool,
    entry: u16,
    outside: &dyn Fn(u16) -> bool,
) -> Vec<Block> {
    // First every instruction reachable inside, and where blocks must start.
    let mut seen: BTreeMap<u16, (String, u8)> = BTreeMap::new();
    // JP (HL)s whose return address was pushed just before, and where they return.
    let mut returns: BTreeMap<u16, u16> = BTreeMap::new();
    let mut leaders: BTreeSet<u16> = BTreeSet::from([entry]);
    let mut todo = vec![entry];
    let inside =
        |a: u16| a == entry || (a >= entry && a < entry.wrapping_add(REACH) && !outside(a));
    while let Some(start) = todo.pop() {
        let mut at = start;
        let mut loaded: Option<(String, u16)> = None;
        let mut pushed: Option<u16> = None;
        loop {
            if seen.contains_key(&at) || !ran(at) {
                break;
            }
            let insn = disasm(&|a| peek(a), at);
            let len = insn.len.max(1);
            let next = at.wrapping_add(len as u16);
            seen.insert(at, (insn.text.clone(), len));
            let t = insn.text.as_str();
            let jump = t.starts_with("JR ") || t.starts_with("JP ") || t.starts_with("DJNZ ");
            if jump && !t.contains('(') {
                for target in targets(t) {
                    if inside(target) {
                        leaders.insert(target);
                        todo.push(target);
                    }
                }
                if condition(t).is_some() {
                    leaders.insert(next);
                    at = next;
                    continue;
                }
                break;
            }
            // LD HL,$nnnn then PUSH HL: an address for something to return to.
            if let Some(rest) = t.strip_prefix("LD ") {
                if let Some((register, value)) = rest.split_once(",$") {
                    if let Ok(v) = u16::from_str_radix(value, 16) {
                        loaded = Some((register.to_string(), v));
                    }
                }
            }
            if let (Some(register), Some((held, value))) = (t.strip_prefix("PUSH "), &loaded) {
                if register == held {
                    pushed = Some(*value);
                }
            }
            if t.starts_with("JP (") {
                if let Some(then) = pushed.filter(|a| inside(*a)) {
                    returns.insert(at, then);
                    leaders.insert(then);
                    todo.push(then);
                }
                break;
            }
            if t == "RET" || t.starts_with("RETI") || t.starts_with("RETN") {
                break;
            }
            if t.starts_with("RET ") {
                leaders.insert(next);
            }
            at = next;
        }
    }
    // Then cut the instructions into blocks at the leaders.
    let mut out: Vec<Block> = Vec::new();
    for &start in &leaders {
        if !seen.contains_key(&start) {
            continue;
        }
        let mut instructions = Vec::new();
        let mut at = start;
        let exit = loop {
            let Some((t, len)) = seen.get(&at).cloned() else {
                break Exit::Fall(at);
            };
            instructions.push((at, t.clone()));
            let next = at.wrapping_add(len as u16);
            let jumps = t.starts_with("JR ") || t.starts_with("JP ") || t.starts_with("DJNZ ");
            if jumps && !t.contains('(') {
                let target = targets(&t).first().copied().unwrap_or(next);
                break match condition(&t) {
                    Some(condition) => Exit::Branch {
                        condition,
                        taken: target,
                        fall: next,
                    },
                    None => Exit::Jump(target),
                };
            }
            if t.starts_with("JP (") {
                break match returns.get(&at) {
                    Some(&then) => Exit::Dispatch {
                        via: t.clone(),
                        then,
                    },
                    None => Exit::Computed(t.clone()),
                };
            }
            if t == "RET" || t.starts_with("RETI") || t.starts_with("RETN") {
                break Exit::Return;
            }
            if let (true, Some(condition)) = (t.starts_with("RET "), condition(&t)) {
                break Exit::ReturnIf {
                    condition,
                    fall: next,
                };
            }
            if leaders.contains(&next) {
                break Exit::Fall(next);
            }
            at = next;
        };
        out.push(Block {
            start,
            instructions,
            exit,
        });
    }
    out
}

const BOX_W: f32 = 360.0;
const LINE: f32 = 14.0;
const GAP: f32 = 26.0;
const DIAMOND_H: f32 = 46.0;
const DIAMOND_W: f32 = 240.0;
const LANE: f32 = 11.0;
const CHARS: usize = 50;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn first_sentence(comment: &str) -> String {
    let comment = comment.lines().next().unwrap_or("");
    match comment.find(". ") {
        Some(end) => comment[..=end].to_string(),
        None => comment.to_string(),
    }
}

/// A conditional jump's or return's condition as words.
fn plain(condition: &str) -> String {
    match condition {
        "Z" => "zero (Z)".into(),
        "NZ" => "not zero (NZ)".into(),
        "C" => "carry (C)".into(),
        "NC" => "no carry (NC)".into(),
        "M" => "minus (M)".into(),
        "P" => "plus (P)".into(),
        "PE" => "parity even (PE)".into(),
        "PO" => "parity odd (PO)".into(),
        other => other.into(),
    }
}

pub fn export_flowchart(session: &mut Session, args: &Json) -> Result<String, String> {
    let entry = addr(args, "address")?;
    let path = text(args, "path")?;
    let most = count(args, "max_blocks", 80)?.clamp(1, 400) as usize;
    let observer = &session.spec.bus.observer;
    if !observer.was_executed(entry) {
        return Err(format!(
            "${entry:04X} has not run while watched: a flow chart is drawn from the code that ran"
        ));
    }
    let bus = &session.spec.bus;
    let peek = |a: u16| bus.peek_raw(a);
    let ran = |a: u16| observer.was_executed(a);
    // A jump leaves the routine when it goes to something called from elsewhere,
    // or to anything before the start.
    let outside = |a: u16| a < entry || (a != entry && observer.routines.contains_key(&a));
    let mut found = blocks(&peek, &ran, entry, &outside);
    let cut = found.len() > most;
    found.truncate(most);
    let name_of =
        |a: u16| crate::mcp::listing::name_of(session, a).unwrap_or_else(|| format!("${a:04X}"));
    let index: BTreeMap<u16, usize> = found
        .iter()
        .enumerate()
        .map(|(i, b)| (b.start, i))
        .collect();

    // Each block's lines: its name, then what it calls and what is said about it.
    let lines: Vec<Vec<(String, bool)>> = found
        .iter()
        .map(|b| {
            let mut out = Vec::new();
            let label = session.notes.label(b.start);
            if !label.is_empty() {
                out.push((label.to_string(), true));
                let said = first_sentence(session.notes.comment(b.start));
                for line in crate::asmexport::wrap(&said, CHARS) {
                    if !line.is_empty() {
                        out.push((line, false));
                    }
                }
            }
            for (at, t) in &b.instructions {
                if *at != b.start || label.is_empty() {
                    let said = session.notes.comment(*at);
                    if !said.is_empty() {
                        for line in crate::asmexport::wrap(&first_sentence(said), CHARS) {
                            out.push((line, false));
                        }
                    }
                }
                if t.starts_with("CALL") {
                    let callee = targets(t).first().map(|a| name_of(*a)).unwrap_or_default();
                    let when =
                        condition(&t.replace("CALL ", "JP ")).map(|c| format!(" if {}", plain(&c)));
                    out.push((format!("call {callee}{}", when.unwrap_or_default()), false));
                }
            }
            if out.is_empty() || out.iter().all(|(_, title)| *title) {
                let n = b.instructions.len();
                out.push((
                    format!("{n} instruction{}", if n == 1 { "" } else { "s" }),
                    false,
                ));
            }
            out
        })
        .collect();

    // Down the page: each block, then its decision if it has one.
    let mut tops = Vec::new();
    let mut y = 88.0;
    for (i, b) in found.iter().enumerate() {
        let h = 12.0 + LINE * lines[i].len() as f32;
        let decision = matches!(b.exit, Exit::Branch { .. } | Exit::ReturnIf { .. });
        tops.push((y, h, decision));
        y += h + if decision { GAP + DIAMOND_H } else { 0.0 } + GAP;
        if matches!(
            b.exit,
            Exit::Return | Exit::Computed(_) | Exit::Dispatch { .. }
        ) {
            y += 24.0;
        }
    }
    let height = y + 40.0;

    // The jumps that need lines, with which side they run down.
    struct Wire {
        from_y: f32,
        to_y: f32,
        back: bool,
        label: String,
        never: bool,
        out: Option<String>,
    }
    let conditional = &observer.flow.conditionals;
    let mut wires: Vec<Wire> = Vec::new();
    for (i, b) in found.iter().enumerate() {
        let (top, h, _) = tops[i];
        let (from_y, target, label, never) = match &b.exit {
            Exit::Jump(t) => (top + h - 6.0, *t, String::new(), false),
            Exit::Dispatch { then, .. } => (top + h - 6.0, *then, String::new(), false),
            Exit::Branch { taken, .. } => {
                let last = b.instructions.last().map(|(a, _)| *a).unwrap_or(b.start);
                let sides = conditional.get(&last).copied().unwrap_or_default();
                (
                    top + h + GAP + DIAMOND_H / 2.0,
                    *taken,
                    format!("yes ×{}", sides.taken),
                    sides.taken == 0,
                )
            }
            _ => continue,
        };
        match index.get(&target) {
            Some(&j) => wires.push(Wire {
                from_y,
                to_y: tops[j].0 + 10.0,
                back: j <= i,
                label,
                never,
                out: None,
            }),
            None => wires.push(Wire {
                from_y,
                to_y: from_y,
                back: false,
                label,
                never,
                // Inside the routine but never run: there is nothing to draw it
                // from, and saying so is better than leaving the line dangling.
                out: Some(if observer.was_executed(target) {
                    name_of(target)
                } else {
                    format!("{}, never run", name_of(target))
                }),
            }),
        }
    }
    // Lanes: a wire takes the first lane on its side that nothing it overlaps uses.
    let mut lanes: Vec<usize> = Vec::new();
    for (i, w) in wires.iter().enumerate() {
        let (lo, hi) = (w.from_y.min(w.to_y), w.from_y.max(w.to_y));
        let mut lane = 0;
        while wires[..i].iter().zip(&lanes).any(|(o, l)| {
            *l == lane && o.back == w.back && o.out.is_none() == w.out.is_none() && {
                let (olo, ohi) = (o.from_y.min(o.to_y), o.from_y.max(o.to_y));
                olo <= hi && lo <= ohi
            }
        }) {
            lane += 1;
        }
        lanes.push(lane);
    }
    let left_lanes = wires
        .iter()
        .zip(&lanes)
        .filter(|(w, _)| w.back)
        .map(|(_, l)| *l + 1)
        .max()
        .unwrap_or(0);
    let right_lanes = wires
        .iter()
        .zip(&lanes)
        .filter(|(w, _)| !w.back && w.out.is_none())
        .map(|(_, l)| *l + 1)
        .max()
        .unwrap_or(0);
    let x0 = 24.0 + left_lanes as f32 * LANE + 20.0;
    let right = x0 + BOX_W + 20.0 + right_lanes as f32 * LANE;
    let width = right + 260.0;
    let centre = x0 + BOX_W / 2.0;

    let title = name_of(entry);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
         viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"Menlo, Consolas, monospace\">\n\
         <defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"7\" \
         markerHeight=\"7\" orient=\"auto-start-reverse\"><path d=\"M0,0 L10,5 L0,10 z\" fill=\"#3a6ea5\"/>\
         </marker></defs>\n\
         <rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n\
         <text x=\"24\" y=\"26\" font-size=\"16\" font-weight=\"bold\">{} (${entry:04X})</text>\n",
        escape(&title),
    );
    let said = format!(
        "In address order, from the code that ran. Jumps forward run down the right, loops \
         back up the left; the counts are how often each way was taken while watched, and a \
         way never taken is dashed.{}",
        if cut {
            " Cut short: more blocks than asked for."
        } else {
            ""
        }
    );
    for (i, line) in crate::asmexport::wrap(&said, ((width - 48.0) / 6.6) as usize)
        .iter()
        .enumerate()
    {
        svg.push_str(&format!(
            "<text x=\"24\" y=\"{:.0}\" font-size=\"11\" fill=\"#555\">{}</text>\n",
            44.0 + 14.0 * i as f32,
            escape(line)
        ));
    }
    for (i, b) in found.iter().enumerate() {
        let (top, h, decision) = tops[i];
        svg.push_str(&format!(
            "<rect class=\"block\" x=\"{x0:.1}\" y=\"{top:.1}\" width=\"{BOX_W}\" height=\"{h:.1}\" \
             rx=\"5\" fill=\"#eef2fb\" stroke=\"#33335a\"/>\n\
             <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"9\" fill=\"#777\" text-anchor=\"end\">${:04X}</text>\n",
            x0 + BOX_W - 6.0,
            top + 11.0,
            b.start
        ));
        for (k, (line, bold)) in lines[i].iter().enumerate() {
            svg.push_str(&format!(
                "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"11\"{}>{}</text>\n",
                x0 + 8.0,
                top + 16.0 + LINE * k as f32,
                if *bold { " font-weight=\"bold\"" } else { "" },
                escape(line)
            ));
        }
        let below = top + h;
        let next_top = tops.get(i + 1).map(|t| t.0);
        let down = |svg: &mut String, from: f32, to: f32, label: &str, never: bool| {
            svg.push_str(&format!(
                "<path d=\"M{centre:.1},{from:.1} L{centre:.1},{:.1}\" stroke=\"{}\" stroke-width=\"1.4\" \
                 fill=\"none\" marker-end=\"url(#arrow)\"{}/>\n",
                to - 1.0,
                if never { "#aaa" } else { "#3a6ea5" },
                if never { " stroke-dasharray=\"4,3\"" } else { "" }
            ));
            if !label.is_empty() {
                svg.push_str(&format!(
                    "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"9\" fill=\"#555\">{label}</text>\n",
                    centre + 6.0,
                    from + 13.0
                ));
            }
        };
        if decision {
            let dy = below + GAP;
            let (question, last) = match &b.exit {
                Exit::Branch { condition, .. } => (format!("if {}", plain(condition)), true),
                Exit::ReturnIf { condition, .. } => {
                    (format!("return if {}", plain(condition)), false)
                }
                _ => unreachable!(),
            };
            let at = b.instructions.last().map(|(a, _)| *a).unwrap_or(b.start);
            let sides = conditional.get(&at).copied().unwrap_or_default();
            down(&mut svg, below, dy, "", false);
            svg.push_str(&format!(
                "<path class=\"decision\" d=\"M{centre:.1},{dy:.1} L{:.1},{:.1} L{centre:.1},{:.1} L{:.1},{:.1} z\" \
                 fill=\"#fff6dc\" stroke=\"#8a6d1d\"/>\n\
                 <text x=\"{centre:.1}\" y=\"{:.1}\" font-size=\"11\" text-anchor=\"middle\">{}</text>\n",
                centre + DIAMOND_W / 2.0,
                dy + DIAMOND_H / 2.0,
                dy + DIAMOND_H,
                centre - DIAMOND_W / 2.0,
                dy + DIAMOND_H / 2.0,
                dy + DIAMOND_H / 2.0 + 4.0,
                escape(&question)
            ));
            if !last {
                // A conditional return: yes goes off to the side and ends.
                let x = centre + DIAMOND_W / 2.0;
                svg.push_str(&format!(
                    "<path d=\"M{x:.1},{:.1} L{:.1},{:.1}\" stroke=\"{}\" fill=\"none\" marker-end=\"url(#arrow)\"{}/>\n\
                     <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"11\">return</text>\n\
                     <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"9\" fill=\"#555\">yes ×{}</text>\n",
                    dy + DIAMOND_H / 2.0,
                    x + 40.0,
                    dy + DIAMOND_H / 2.0,
                    if sides.taken == 0 { "#aaa" } else { "#3a6ea5" },
                    if sides.taken == 0 { " stroke-dasharray=\"4,3\"" } else { "" },
                    x + 46.0,
                    dy + DIAMOND_H / 2.0 + 4.0,
                    x + 4.0,
                    dy + DIAMOND_H / 2.0 - 5.0,
                    sides.taken
                ));
            }
            if let Some(next) = next_top {
                down(
                    &mut svg,
                    dy + DIAMOND_H,
                    next,
                    &format!("no ×{}", sides.not_taken),
                    sides.not_taken == 0,
                );
            }
        } else {
            match &b.exit {
                Exit::Fall(_) => {
                    if let Some(next) = next_top {
                        down(&mut svg, below, next, "", false);
                    }
                }
                Exit::Return | Exit::Computed(_) | Exit::Dispatch { .. } => {
                    let what = match &b.exit {
                        Exit::Computed(t) => format!("on through {}", t.trim_start_matches("JP ")),
                        Exit::Dispatch { via, then } => format!(
                            "on through {}, which returns to {}",
                            via.trim_start_matches("JP "),
                            name_of(*then)
                        ),
                        _ => "return".into(),
                    };
                    svg.push_str(&format!(
                        "<path d=\"M{centre:.1},{below:.1} L{centre:.1},{:.1}\" stroke=\"#3a6ea5\" \
                         marker-end=\"url(#arrow)\"/>\n\
                         <text x=\"{centre:.1}\" y=\"{:.1}\" font-size=\"11\" text-anchor=\"middle\" \
                         font-weight=\"bold\">{}</text>\n",
                        below + 12.0,
                        below + 24.0,
                        escape(&what)
                    ));
                }
                _ => {}
            }
        }
    }
    for (w, lane) in wires.iter().zip(&lanes) {
        let colour = if w.never { "#aaa" } else { "#3a6ea5" };
        let dash = if w.never {
            " stroke-dasharray=\"4,3\""
        } else {
            ""
        };
        let side = if w.from_y > 0.0 && w.label.is_empty() {
            x0 + BOX_W
        } else {
            centre + DIAMOND_W / 2.0
        };
        if let Some(name) = &w.out {
            svg.push_str(&format!(
                "<path d=\"M{side:.1},{y:.1} L{:.1},{y:.1}\" stroke=\"{colour}\" fill=\"none\" \
                 marker-end=\"url(#arrow)\"{dash}/>\n\
                 <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"11\">to {}</text>\n\
                 <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"9\" fill=\"#555\">{}</text>\n",
                right + 10.0,
                right + 16.0,
                w.from_y + 4.0,
                escape(name),
                side + 4.0,
                w.from_y - 5.0,
                w.label,
                y = w.from_y
            ));
            continue;
        }
        let lx = if w.back {
            x0 - 20.0 - *lane as f32 * LANE
        } else {
            x0 + BOX_W + 20.0 + *lane as f32 * LANE
        };
        let entry_x = if w.back { x0 } else { x0 + BOX_W };
        let start_x = if w.back {
            if w.label.is_empty() {
                x0
            } else {
                centre - DIAMOND_W / 2.0
            }
        } else {
            side
        };
        svg.push_str(&format!(
            "<path d=\"M{start_x:.1},{:.1} L{lx:.1},{:.1} L{lx:.1},{:.1} L{entry_x:.1},{:.1}\" stroke=\"{colour}\" \
             stroke-width=\"1.4\" fill=\"none\" marker-end=\"url(#arrow)\"{dash}/>\n\
             <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"9\" fill=\"#555\"{}>{}</text>\n",
            w.from_y,
            w.from_y,
            w.to_y,
            w.to_y,
            if w.back { start_x - 4.0 } else { start_x + 4.0 },
            w.from_y - 5.0,
            if w.back { " text-anchor=\"end\"" } else { "" },
            w.label
        ));
    }
    svg.push_str("</svg>\n");
    std::fs::write(&path, &svg).map_err(|e| format!("{path}: {e}"))?;
    let decisions = found
        .iter()
        .filter(|b| matches!(b.exit, Exit::Branch { .. } | Exit::ReturnIf { .. }))
        .count();
    Ok(format!(
        "Wrote {path}: {} blocks and {decisions} decisions of {title}{}.",
        found.len(),
        if cut { ", cut short at max_blocks" } else { "" }
    ))
}
