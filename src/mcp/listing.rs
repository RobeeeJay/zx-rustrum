//! The annotated listing: what the whole exercise is building towards.

use crate::mcp::json::Json;
use crate::mcp::tools::{addr, Session};

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
    out.push_str(&format!(
        "; What is marked \"run\" was executed while the machine was watched over {watched} \
         frames.\n; Anything else may be data, or code that has not run yet.\n\n"
    ));

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
                note_above(session, row.at, &mut out);
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
            let beside = note_above(session, at, &mut out);
            let insn = crate::disasm::disasm(&|a| session.spec.bus.peek_raw(a), at);
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
            let beside = note_above(session, at, &mut out);
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
fn note_above(session: &Session, at: u16, out: &mut String) -> String {
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
    if comment.is_empty() {
        return String::new();
    }
    if label.is_empty() && comment.len() <= BESIDE && !comment.contains('\n') {
        return comment.to_string();
    }
    for line in wrapped(comment, 76) {
        out.push_str(&format!("; {line}\n"));
    }
    String::new()
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
