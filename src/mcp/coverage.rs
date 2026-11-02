//! The edge of what has been seen running, and the jumps the code does not name.

use crate::coverage::report;
use crate::disasm::disasm;
use crate::mcp::json::Json;
use crate::mcp::tools::{addr, count, Session};

const NOTHING_WATCHED: &str =
    "nothing has been watched yet: call watch_routines, then run the machine";

fn range(args: &Json) -> Result<(u16, u16), String> {
    let from = match args.get("from") {
        Some(_) => addr(args, "from")?,
        None => 0x5B00,
    };
    let to = match args.get("to") {
        Some(_) => addr(args, "to")?,
        None => 0xFFFF,
    };
    if to <= from {
        return Err("to should be above from".into());
    }
    Ok((from, to))
}

/// An address, and its name if it has one.
fn named(session: &Session, at: u16) -> String {
    let label = session.notes.label(at);
    if label.is_empty() {
        format!("${at:04X}")
    } else {
        format!("${at:04X} {label}")
    }
}

pub fn coverage(session: &mut Session, args: &Json) -> Result<String, String> {
    let (from, to) = range(args)?;
    let limit = count(args, "limit", 60)? as usize;
    let observer = &session.spec.bus.observer;
    if observer.code_runs().is_empty() {
        return Err(NOTHING_WATCHED.into());
    }
    let bus = &session.spec.bus;
    let peek = |a: u16| bus.peek_raw(a);
    let r = report(&peek, observer, from, to);
    let share = |n: u32| n as f64 * 100.0 / r.bytes as f64;
    let mut out = format!(
        "${from:04X}-${to:04X}, {} bytes, over {} frames watched:\n  \
         {:>6} run ({:.1}%)\n  \
         {:>6} could be reached, decoded from what has run, and has not run ({:.1}%)\n  \
         {:>6} read, never run: data, or a program moved by its loader ({:.1}%)\n  \
         {:>6} touched by nothing here ({:.1}%)\n",
        r.bytes,
        observer.frames,
        r.run,
        share(r.run),
        r.reachable,
        share(r.reachable),
        r.data,
        share(r.data),
        r.untouched,
        share(r.untouched),
    );
    out.push_str(&format!(
        "\nCode not reached yet: {} stretches, widest first. Decoded rather than \
         run, so a stretch may run on into something that is not code.\n",
        r.unreached.len()
    ));
    let mut widest = r.unreached.clone();
    widest.sort_by_key(|u| std::cmp::Reverse(u.high - u.low));
    for u in widest.iter().take(limit) {
        out.push_str(&format!(
            "  ${:04X}-${:04X}  {:>5} bytes, led to by {}\n",
            u.low,
            u.high,
            u.high as u32 - u.low as u32 + 1,
            named(session, u.named_by)
        ));
    }
    out.push_str(&format!(
        "\nConditionals that have only gone one way: {}. Recorded as they ran, so \
         \"never taken\" means this instruction never went there, whatever else did.\n",
        r.one_sided.len()
    ));
    for (at, sides) in r.one_sided.iter().take(limit) {
        let text = disasm(&peek, *at).text;
        let said = if sides.taken == 0 {
            format!("never taken, fell through {} times", sides.not_taken)
        } else {
            format!("always taken, {} times", sides.taken)
        };
        out.push_str(&format!("  ${at:04X}  {text:<16} {said}\n"));
    }
    if r.one_sided.len() > limit {
        out.push_str(&format!("  ({} more)\n", r.one_sided.len() - limit));
    }
    Ok(out)
}

pub fn indirect_jumps(session: &mut Session, args: &Json) -> Result<String, String> {
    let (from, to) = range(args)?;
    let observer = &session.spec.bus.observer;
    if observer.code_runs().is_empty() {
        return Err(NOTHING_WATCHED.into());
    }
    let flow = &observer.flow;
    if flow.indirect.is_empty() {
        return Ok(
            "No JP (HL), JP (IX) or JP (IY) has run while watching. A PUSH then RET \
                   used as a jump would not show here."
                .into(),
        );
    }
    let mut out = format!(
        "{} indirect jumps ran. A PUSH then RET used as a jump does not show here.\n",
        flow.indirect.len()
    );
    for (at, site) in &flow.indirect {
        out.push_str(&format!(
            "\n{}, {} targets, reached from {}:\n",
            named(session, *at),
            site.targets.len(),
            site.from
                .iter()
                .map(|f| format!("${f:04X}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for (target, times) in &site.targets {
            out.push_str(&format!(
                "  -> {}  {times} times\n",
                named(session, *target)
            ));
        }
    }
    let bus = &session.spec.bus;
    let peek = |a: u16| bus.peek_raw(a);
    let found = flow.tables(&peek, from, to);
    out.push_str(&format!(
        "\nThose targets side by side in memory, between ${from:04X} and ${to:04X}: {}. \
         Where one ends is a guess, so it may take in a word or two too many.\n",
        found.len()
    ));
    for table in &found {
        let seen = table.entries.iter().filter(|(_, seen)| *seen).count();
        out.push_str(&format!(
            "  ${:04X}  {} words, {seen} jumped to so far:\n",
            table.at,
            table.entries.len()
        ));
        for (i, (word, seen)) in table.entries.iter().enumerate() {
            out.push_str(&format!(
                "    [{i:>2}] {}{}\n",
                named(session, *word),
                if *seen { "" } else { "   not jumped to yet" }
            ));
        }
    }
    Ok(out)
}
