//! What is rewriting the program, and who changed a value. See `crate::changes`.

use crate::changes::ValueWatch;
use crate::mcp::json::Json;
use crate::mcp::tools::{addr, count, flag, Session};

/// An instruction's address and the name of what it is in.
fn where_it_is(session: &Session, at: u16) -> String {
    let notes = &session.notes;
    match notes.labelled().take_while(|(a, _, _)| *a <= at).last() {
        Some((a, name, _)) if a == at => format!("${at:04X} {name}"),
        Some((a, name, _)) => format!("${at:04X} {name}+{}", at - a),
        None => format!("${at:04X}"),
    }
}

pub fn self_modifying(session: &mut Session, args: &Json) -> Result<String, String> {
    let from = match args.get("from") {
        Some(_) => addr(args, "from")?,
        None => 0x5B00,
    };
    let to = match args.get("to") {
        Some(_) => addr(args, "to")?,
        None => 0xFFFF,
    };
    let limit = count(args, "limit", 40)? as usize;
    let changes = &session.spec.bus.observer.changes;
    let runs: Vec<(u16, u16)> = changes
        .rewritten_runs()
        .into_iter()
        .filter(|(low, high)| *high >= from && *low <= to)
        .collect();
    if runs.is_empty() {
        return Ok(
            "Nothing has rewritten code that had already run, while it was being watched. \
             Code written before it first ran doesn't count, nor do the writes before \
             watch_routines was called."
                .to_string(),
        );
    }
    let mut out = format!(
        "{} runs of code rewritten after it had run. A row of values is a switch being \
         thrown; a loader moving a block will show one instruction and many bytes.\n",
        runs.len()
    );
    for (low, high) in runs.iter().take(limit) {
        let mut times = 0;
        let mut values: Vec<u8> = Vec::new();
        let mut by: Vec<u16> = Vec::new();
        for a in *low..=*high {
            if let Some(r) = changes.rewrites.get(&a) {
                times += r.times;
                for v in &r.values {
                    if values.len() < 8 && !values.contains(v) {
                        values.push(*v);
                    }
                }
                for pc in r.by.keys() {
                    if !by.contains(pc) {
                        by.push(*pc);
                    }
                }
            }
        }
        let values: Vec<String> = values.iter().map(|v| format!("${v:02X}")).collect();
        let by: Vec<String> = by.iter().map(|pc| where_it_is(session, *pc)).collect();
        out.push_str(&format!(
            "  ${low:04X}-${high:04X} ({}) rewritten {times} times by {}; values {}\n",
            where_it_is(session, *low),
            by.join(", "),
            values.join(",")
        ));
    }
    if runs.len() > limit {
        out.push_str(&format!("  and {} more\n", runs.len() - limit));
    }
    Ok(out)
}

pub fn watch_value(session: &mut Session, args: &Json) -> Result<String, String> {
    let observer = &mut session.spec.bus.observer;
    if flag(args, "clear", false) {
        let had = observer.changes.watches.len();
        observer.changes.watches.clear();
        return Ok(format!("Stopped {had} watches."));
    }
    let low = addr(args, "address")?;
    let length = count(args, "length", 1)?.max(1);
    let high = low.saturating_add(length.min(0xFFFF) as u16 - 1);
    let was_on = observer.enabled;
    observer.enabled = true;
    observer.changes.watches.push(ValueWatch {
        low,
        high,
        ..Default::default()
    });
    Ok(format!(
        "Logging changes to ${low:04X}-${high:04X}.{}",
        if was_on {
            ""
        } else {
            " The observer was off and has been turned on: the routines tools will start counting too."
        }
    ))
}

pub fn value_log(session: &mut Session, args: &Json) -> Result<String, String> {
    let only = match args.get("address") {
        Some(_) => Some(addr(args, "address")?),
        None => None,
    };
    let limit = count(args, "limit", 40)? as usize;
    let observer = &session.spec.bus.observer;
    if observer.changes.watches.is_empty() {
        return Err("nothing is being watched: call watch_value first".into());
    }
    let mut out = String::new();
    for watch in &observer.changes.watches {
        if only.is_some_and(|a| a < watch.low || a > watch.high) {
            continue;
        }
        let changes: Vec<_> = watch
            .log
            .iter()
            .filter(|c| only.is_none_or(|a| c.addr == a))
            .collect();
        out.push_str(&format!(
            "${:04X}-${:04X}: {} changes kept",
            watch.low,
            watch.high,
            changes.len()
        ));
        if watch.dropped > 0 {
            out.push_str(&format!(", and {} older ones let go", watch.dropped));
        }
        out.push('\n');
        let mut by: std::collections::BTreeMap<u16, u32> = Default::default();
        for c in &changes {
            *by.entry(c.pc).or_default() += 1;
        }
        for (pc, times) in &by {
            out.push_str(&format!("  {times:>6} by {}\n", where_it_is(session, *pc)));
        }
        for c in changes.iter().skip(changes.len().saturating_sub(limit)) {
            out.push_str(&format!(
                "  frame {} T {:>5}  ${:04X}  ${:02X} -> ${:02X}  {}\n",
                c.frame,
                c.t,
                c.addr,
                c.old,
                c.new,
                where_it_is(session, c.pc)
            ));
        }
    }
    if out.is_empty() {
        return Err("no watch covers that address".into());
    }
    Ok(out)
}
