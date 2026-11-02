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

        // A label of any kind starts a line of its own, whether it came from
        // this session, a symbol file or a guess.
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

        if ran || in_code_run {
            let insn = crate::disasm::disasm(&|a| session.spec.bus.peek_raw(a), at);
            let bytes: String = insn.bytes.iter().map(|b| format!("{b:02X} ")).collect();
            let comment = session.notes.comment(at);
            out.push_str(&format!(
                "${at:04X}  {bytes:<12} {:<24}{}\n",
                insn.text,
                if comment.is_empty() {
                    String::new()
                } else {
                    format!("; {comment}")
                }
            ));
            lines += 1;
            let step = insn.len.max(1) as u16;
            if at.checked_add(step).is_none() || at as u32 + step as u32 > to as u32 {
                break;
            }
            at = at.wrapping_add(step);
        } else {
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
                        || !session.notes.comment(next).is_empty())
                {
                    break;
                }
                bytes.push(session.spec.bus.peek_raw(next));
            }
            if bytes.is_empty() {
                break;
            }
            let comment = session.notes.comment(at);
            out.push_str(&format!(
                "${at:04X}  DEFB {}{}\n",
                bytes
                    .iter()
                    .map(|b| format!("${b:02X}"))
                    .collect::<Vec<_>>()
                    .join(","),
                if comment.is_empty() {
                    String::new()
                } else {
                    format!("  ; {comment}")
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
