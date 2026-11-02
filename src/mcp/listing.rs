//! The annotated listing: what the whole exercise is building towards.

use crate::mcp::json::Json;
use crate::mcp::tools::{addr, count, flag, Session};
use std::collections::{BTreeMap, BTreeSet};

/// The whole annotated disassembly, written out.
///
/// This is the thing being built: the listing with every label and comment
/// against it, the data blocks left as bytes rather than disassembled into
/// nonsense, and a header saying what was known and how. Plain text, because
/// it is going to be read by a person in the end.
pub fn export_listing(session: &mut Session, args: &Json) -> Result<String, String> {
    let path = crate::mcp::tools::text(args, "path")?;
    let from = match args.get("from") {
        Some(_) => addr(args, "from")?,
        None => 0x4000,
    };
    let to = match args.get("to") {
        Some(_) => addr(args, "to")?,
        None => 0xFFFF,
    };
    if to <= from {
        return Err("to should be above from".into());
    }

    let observer = &session.spec.bus.observer;
    let watched = observer.frames;
    let executed: Vec<(u16, u16)> = observer.code_runs();
    let mut out = format!(
        "; {}\n; Disassembled from ${from:04X} to ${to:04X}.\n",
        session
            .loaded
            .clone()
            .unwrap_or_else(|| "a machine with nothing loaded".into())
    );
    let facts = flag(args, "facts", true);
    out.push_str(&format!(
        "; What is marked \"run\" was executed while the machine was watched over {watched} \
         frames.\n; Anything else may be data, or code that has not run yet.\n"
    ));
    if facts {
        out.push_str(
            "; \"seen:\" and \"used by:\" lines and [square brackets] are measured, not written \
             by\n; anybody: [never taken] is a conditional that only went one way, [forced] is \
             code\n; that ran only when force_branch made it, [rewritten] was changed after it \
             ran.\n",
        );
    }
    out.push('\n');
    let used_by = if facts {
        references(session, from, to)
    } else {
        Default::default()
    };

    let mut at = from;
    let mut lines = 0u32;
    let mut data_bytes = 0u32;
    loop {
        let ran = session.spec.bus.observer.was_executed(at);
        let in_code_run = executed.iter().any(|(low, high)| at >= *low && at <= *high);

        if let Some(typed) = session.notes.layout().data.get(&at).copied() {
            let peek = |a: u16| session.spec.bus.peek_raw(a);
            let rows = crate::layout::rows(&typed, &peek);
            for row in &rows {
                note_above(session, row.at, &used_by, &mut out);
                let beside = if row.beside.is_empty() {
                    String::new()
                } else {
                    format!("  ; {}", row.beside)
                };
                out.push_str(&format!(
                    "${:04X}  {}{beside}\n",
                    row.at,
                    named_text(session, &row.directive)
                ));
                // A note on a byte a row does not start at still has to appear
                // somewhere, or it is lost from the file without a word.
                for inner in 1..row.len {
                    let there = row.at.wrapping_add(inner);
                    let comment = session.notes.comment(there);
                    let label = session.notes.label(there);
                    if !comment.is_empty() || !label.is_empty() {
                        out.push_str(&format!("        ; ${there:04X} {label} {comment}\n"));
                    }
                }
            }
            data_bytes += typed.length as u32;
            if at as u32 + typed.length as u32 > to as u32 {
                break;
            }
            at = at.wrapping_add(typed.length);
        } else if ran || in_code_run {
            let beside = note_above(session, at, &used_by, &mut out);
            let insn = crate::disasm::disasm(&|a| session.spec.bus.peek_raw(a), at);
            let beside = if facts {
                with_marks(session, at, insn.len, beside)
            } else {
                beside
            };
            let bytes: String = insn.bytes.iter().map(|b| format!("{b:02X} ")).collect();
            let shown = match session.notes.layout().name_displacement(at, &insn.text) {
                Some((named, _)) => named,
                None => insn.text.clone(),
            };
            out.push_str(&format!(
                "${at:04X}  {bytes:<12} {:<24}{}\n",
                named_text(session, &shown),
                if beside.is_empty() {
                    String::new()
                } else {
                    format!("; {beside}")
                }
            ));
            lines += 1;
            let step = insn.len.max(1) as u16;
            if at.checked_add(step).is_none() || at as u32 + step as u32 > to as u32 {
                break;
            }
            at = at.wrapping_add(step);
        } else {
            let beside = note_above(session, at, &used_by, &mut out);
            // Data: sixteen bytes a line, with the printable ones beside them,
            // rather than disassembled into instructions nobody executes.
            let mut bytes = Vec::new();
            for i in 0..16u16 {
                if at as u32 + i as u32 > to as u32 {
                    break;
                }
                // A row stops where code starts, or it runs over the first
                // instructions of whatever comes next; and at a note, which
                // is only written against the address a row starts at.
                let next = at.wrapping_add(i);
                if i > 0
                    && (session.spec.bus.observer.was_executed(next)
                        || !session.notes.label(next).is_empty()
                        || session.symbols.get(next).is_some()
                        || !session.notes.comment(next).is_empty()
                        || session.notes.layout().data.contains_key(&next))
                {
                    break;
                }
                bytes.push(session.spec.bus.peek_raw(next));
            }
            if bytes.is_empty() {
                break;
            }
            out.push_str(&format!(
                "${at:04X}  DEFB {}{}\n",
                bytes
                    .iter()
                    .map(|b| format!("${b:02X}"))
                    .collect::<Vec<_>>()
                    .join(","),
                if beside.is_empty() {
                    String::new()
                } else {
                    format!("  ; {beside}")
                }
            ));
            data_bytes += bytes.len() as u32;
            let step = bytes.len() as u16;
            if at as u32 + step as u32 > to as u32 {
                break;
            }
            at = at.wrapping_add(step);
        }
        if at >= to {
            break;
        }
    }

    std::fs::write(&path, &out).map_err(|e| format!("{path}: {e}"))?;
    Ok(format!(
        "Written to {path}: {lines} instructions and {data_bytes} bytes of data between \
         ${from:04X} and ${to:04X}. What was disassembled is what ran while watching, plus \
         anything inside a run of it; the rest is left as bytes rather than turned into \
         instructions nobody executed.",
    ))
}

/// How long a comment can be and still sit beside its instruction.
const BESIDE: usize = 52;

/// What an address is called: a label, then a place in a record, then a name
/// from a symbol file.
pub fn name_of(session: &Session, at: u16) -> Option<String> {
    let own = session.notes.label(at);
    if !own.is_empty() {
        return Some(own.to_string());
    }
    let label = |a: u16| session.notes.label(a).to_string();
    session
        .notes
        .layout()
        .field_name(at, &label)
        .or_else(|| session.symbols.get(at).map(|(name, _)| name.to_string()))
}

/// A line of assembly with its addresses named. An immediate below the RAM is
/// more likely a number than an address, and `LD BC,$0250` should not read as
/// whatever the ROM has at $0250.
pub fn named_text(session: &Session, text: &str) -> String {
    let an_address = text.contains('(')
        || ["CALL", "JP", "JR", "DJNZ", "DEFW"]
            .iter()
            .any(|op| text.starts_with(op));
    crate::layout::replace_addresses(text, &mut |value| {
        if value < 0x4000 && !an_address {
            return None;
        }
        name_of(session, value)
    })
}

/// Everything that goes above a line, and what is left to go beside it. A
/// routine's description belongs above it, and so does anything too long to
/// read at the end of a line of code.
fn note_above(
    session: &Session,
    at: u16,
    used_by: &BTreeMap<u16, Vec<u16>>,
    out: &mut String,
) -> String {
    let own = session.notes.label(at);
    let label = if own.is_empty() {
        session
            .symbols
            .get(at)
            .map(|(name, _)| name.to_string())
            .unwrap_or_default()
    } else {
        own.to_string()
    };
    if !label.is_empty() {
        out.push_str(&format!("\n{label}:\n"));
    }
    let comment = session.notes.comment(at);
    let beside = label.is_empty() && comment.len() <= BESIDE && !comment.contains('\n');
    if !comment.is_empty() && !beside {
        for line in wrapped(comment, 76) {
            out.push_str(&format!("; {line}\n"));
        }
    }
    if !label.is_empty() {
        if let Some(seen) = seen_line(session, at) {
            out.push_str(&format!("; seen: {seen}\n"));
        }
        if let Some(from) = used_by.get(&at) {
            let line = used_by_line(session, at, from);
            if !line.is_empty() {
                out.push_str(&format!("; used by: {line}\n"));
            }
        }
    }
    if beside {
        comment.to_string()
    } else {
        String::new()
    }
}

/// Which addresses each piece of run code in the range names.
fn references(session: &Session, from: u16, to: u16) -> BTreeMap<u16, Vec<u16>> {
    let mut found: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
    let mut at = from as u32;
    while at <= to as u32 {
        let here = at as u16;
        if !session.spec.bus.observer.was_executed(here) {
            at += 1;
            continue;
        }
        let insn = crate::disasm::disasm(&|a| session.spec.bus.peek_raw(a), here);
        crate::layout::replace_addresses(&insn.text, &mut |value| {
            found.entry(value).or_default().push(here);
            None
        });
        at += insn.len.max(1) as u32;
    }
    found
}

/// The name of what an address is in: the nearest label at or below it.
fn enclosing(session: &Session, at: u16) -> String {
    session
        .notes
        .labelled()
        .take_while(|(a, _, _)| *a <= at)
        .last()
        .map_or_else(|| format!("${at:04X}"), |(_, name, _)| name.to_string())
}

/// What else mentions this address, by routine rather than by instruction. The
/// routine's own jumps back into itself say nothing about what it is for.
fn used_by_line(session: &Session, at: u16, from: &[u16]) -> String {
    let own = enclosing(session, at);
    let names: BTreeSet<String> = from
        .iter()
        .map(|a| enclosing(session, *a))
        .filter(|name| *name != own)
        .collect();
    let shown: Vec<&str> = names.iter().take(8).map(String::as_str).collect();
    let more = names.len().saturating_sub(shown.len());
    if more > 0 {
        format!("{}, and {more} more", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

/// What the observer measured about a routine, in a line.
fn seen_line(session: &Session, at: u16) -> Option<String> {
    let observer = &session.spec.bus.observer;
    let routine = observer.routines.get(&at)?;
    if routine.calls == 0 {
        return None;
    }
    let calls = routine.calls as f64;
    let frames = observer.frames.max(1) as f64;
    let mut line = format!(
        "called {} times, {:.2} a frame, {:.0} instructions a call",
        routine.calls,
        calls / frames,
        routine.instructions as f64 / calls
    );
    let per_call = |n: u32| n as f64 / calls;
    let written = [
        ("screen", per_call(routine.inclusive.screen)),
        ("attributes", per_call(routine.inclusive.attrs)),
        ("elsewhere", per_call(routine.inclusive.other)),
    ];
    let wrote: Vec<String> = written
        .iter()
        .filter(|(_, n)| *n >= 0.5)
        .map(|(what, n)| format!("{n:.0} {what}"))
        .collect();
    if !wrote.is_empty() {
        line.push_str(&format!("; writes {} a call", wrote.join(", ")));
    }
    let ports = |set: &[u16]| -> String {
        set.iter()
            .map(|p| format!("${p:04X}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    if !routine.ports_in.is_empty() {
        line.push_str(&format!("; reads ports {}", ports(&routine.ports_in)));
    }
    if !routine.ports_out.is_empty() {
        line.push_str(&format!("; writes ports {}", ports(&routine.ports_out)));
    }
    Some(line)
}

/// What was measured about an instruction, added to what was said about it.
fn with_marks(session: &Session, at: u16, length: u8, said: String) -> String {
    let observer = &session.spec.bus.observer;
    let mut marks = Vec::new();
    if let Some(sides) = observer.flow.conditionals.get(&at) {
        if sides.taken == 0 {
            marks.push("[never taken]".to_string());
        } else if sides.not_taken == 0 {
            marks.push("[always taken]".to_string());
        }
    }
    if observer.was_forced(at) {
        marks.push("[forced]".to_string());
    }
    let mut by: BTreeSet<String> = BTreeSet::new();
    let mut values: Vec<u8> = Vec::new();
    for byte in 0..length.max(1) as u16 {
        if let Some(rewrite) = observer.changes.rewrites.get(&at.wrapping_add(byte)) {
            by.extend(rewrite.by.keys().map(|pc| enclosing(session, *pc)));
            values.extend(rewrite.values.iter());
        }
    }
    if !by.is_empty() {
        let values: Vec<String> = values.iter().map(|v| format!("${v:02X}")).collect();
        marks.push(format!(
            "[rewritten by {} with {}]",
            by.into_iter().collect::<Vec<_>>().join(", "),
            values.join(",")
        ));
    }
    if marks.is_empty() {
        return said;
    }
    let marks = marks.join(" ");
    if said.is_empty() {
        marks
    } else {
        format!("{said}  {marks}")
    }
}

/// Lines no longer than `width`, keeping the line breaks that were written.
fn wrapped(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.len() + 1 + word.len() > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

/// What has been read and not explained yet, biggest first.
pub fn unexplained(session: &mut Session, args: &Json) -> Result<String, String> {
    let from = match args.get("from") {
        Some(_) => addr(args, "from")?,
        None => 0x5B00,
    };
    let to = match args.get("to") {
        Some(_) => addr(args, "to")?,
        None => 0xFFFF,
    };
    let limit = count(args, "limit", 30)? as usize;
    struct Piece {
        at: u16,
        instructions: u32,
        explained: u32,
        described: bool,
    }
    let mut pieces: Vec<Piece> = Vec::new();
    let mut pointer = from as u32;
    while pointer <= to as u32 {
        let here = pointer as u16;
        if !session.spec.bus.observer.was_executed(here) {
            pointer += 1;
            continue;
        }
        let insn = crate::disasm::disasm(&|a| session.spec.bus.peek_raw(a), here);
        let labelled = !session.notes.label(here).is_empty();
        let commented = !session.notes.comment(here).is_empty();
        if labelled || pieces.is_empty() {
            pieces.push(Piece {
                at: here,
                instructions: 0,
                explained: 0,
                described: labelled && commented,
            });
        }
        let piece = pieces.last_mut().expect("one was just pushed");
        piece.instructions += 1;
        // A description above a routine says what it is for, and is not a
        // comment on its first instruction.
        if commented && !labelled {
            piece.explained += 1;
        }
        pointer += insn.len.max(1) as u32;
    }
    if pieces.is_empty() {
        return Err("nothing in that range has run: call watch_routines and run it".into());
    }
    let instructions: u32 = pieces.iter().map(|p| p.instructions).sum();
    let explained: u32 = pieces.iter().map(|p| p.explained).sum();
    let described = pieces.iter().filter(|p| p.described).count();
    let mut out = format!(
        "${from:04X}-${to:04X}: {} of {} routines and labelled stretches say what they do; \
         {explained} of {instructions} instructions ({:.0}%) have a comment of their own.\n\
         Nothing here judges the comments: it counts them. Biggest gap first:\n",
        described,
        pieces.len(),
        explained as f64 * 100.0 / instructions as f64,
    );
    pieces.sort_by_key(|p| {
        std::cmp::Reverse(p.instructions.saturating_sub(p.explained) + u32::from(!p.described) * 8)
    });
    for piece in pieces.iter().take(limit) {
        let name = session.notes.label(piece.at);
        out.push_str(&format!(
            "  ${:04X} {:<24} {:>4} instructions, {:>3} commented{}\n",
            piece.at,
            if name.is_empty() { "(no label)" } else { name },
            piece.instructions,
            piece.explained,
            if piece.described {
                ""
            } else {
                ", no description"
            },
        ));
    }
    let observer = &session.spec.bus.observer;
    let unnamed: Vec<u16> = observer
        .routines
        .keys()
        .copied()
        .filter(|a| *a >= from && *a <= to && session.notes.label(*a).is_empty())
        .collect();
    if !unnamed.is_empty() {
        let shown: Vec<String> = unnamed
            .iter()
            .take(limit)
            .map(|a| format!("${a:04X}"))
            .collect();
        out.push_str(&format!(
            "Called but not named: {} routines: {}\n",
            unnamed.len(),
            shown.join(", ")
        ));
    }
    Ok(out)
}
