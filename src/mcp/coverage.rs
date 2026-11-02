//! The edge of what has been seen running, and the jumps the code does not name.

use crate::coverage::report;
use crate::disasm::disasm;
use crate::machine::{Stop, FRAME_T};
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
    let forced: u32 = observer
        .forced
        .iter()
        .filter(|(low, high)| *high >= from && *low <= to)
        .map(|(low, high)| *high.min(&to) as u32 - *low.max(&from) as u32 + 1)
        .sum();
    if forced > 0 {
        out.push_str(&format!(
            "  ({forced} of what has run ran only because a branch was forced)\n"
        ));
    }
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

pub fn force_branch(session: &mut Session, args: &Json) -> Result<String, String> {
    let at = addr(args, "address")?;
    if !session.spec.bus.observer.enabled {
        return Err("force_branch keeps a record of what ran: call watch_routines first".into());
    }
    let opcode = session.spec.bus.peek_raw(at);
    let text = disasm(&|a| session.spec.bus.peek_raw(a), at).text;
    if !crate::coverage::is_conditional(opcode) {
        return Err(format!("${at:04X} is {text}, which has nothing to force"));
    }
    let seen = session
        .spec
        .bus
        .observer
        .flow
        .conditionals
        .get(&at)
        .copied()
        .unwrap_or_default();
    let take = match args.get("side").and_then(|s| s.as_str()) {
        Some("taken") => true,
        Some("not_taken") => false,
        Some(other) => return Err(format!("side {other:?}: it should be taken or not_taken")),
        None if seen.taken > 0 && seen.not_taken > 0 => {
            return Err(format!(
                "${at:04X} {text} has gone both ways already: say which side with side"
            ))
        }
        None => seen.taken == 0,
    };
    let reach = count(args, "reach_frames", 500)?;
    let frames = count(args, "frames", 100)?;

    // The fork is what runs and what is thrown away, so the real machine keeps
    // its hold on the sound device and comes back exactly as it was.
    let mut fork = session.spec.clone();
    fork.bus.audio.detach();
    let original = std::mem::replace(&mut session.spec, fork);
    let result = run_forced(session, args, at, opcode, take, reach, frames);
    let fork = std::mem::replace(&mut session.spec, original);
    let (pc, sp) = result?;

    let new = session.spec.bus.observer.absorb_forced(&fork.bus.observer);
    let side = if take { "taken" } else { "not taken" };
    let mut out = format!(
        "Forced ${at:04X} {text} {side}, then ran {frames} frames; it ended at PC ${pc:04X}, SP \
         ${sp:04X}. The machine is back as it was.\n"
    );
    if new.is_empty() {
        out.push_str("Nothing ran that had not run already.\n");
        return Ok(out);
    }
    let bytes: u32 = new.iter().map(|(l, h)| *h as u32 - *l as u32 + 1).sum();
    out.push_str(&format!(
        "{bytes} bytes ran that had not, marked as forced. Only the one flag was changed, so \
         the program may not be able to get here like this by itself:\n"
    ));
    for (low, high) in &new {
        out.push_str(&format!(
            "  ${low:04X}-${high:04X}  {}\n",
            named(session, *low)
        ));
    }
    Ok(out)
}

fn run_forced(
    session: &mut Session,
    args: &Json,
    at: u16,
    opcode: u8,
    take: bool,
    reach: u32,
    frames: u32,
) -> Result<(u16, u16), String> {
    if let Some(state) = args.get("state").and_then(|s| s.as_str()) {
        let data = session
            .states
            .get(state)
            .ok_or_else(|| format!("no state called {state:?}"))?
            .clone();
        crate::snapshot::load_sna(&mut session.spec, &data)?;
    }
    if session.spec.cpu.pc != at {
        match crate::mcp::control::run_to(session, &[at], reach)? {
            Stop::Breakpoint(pc) if pc == at => {}
            other => {
                return Err(format!(
                    "the machine did not get to ${at:04X} in {reach} frames ({other:?}); \
                     nothing was forced. Try from a saved state nearer to it."
                ))
            }
        }
    }
    match crate::coverage::condition(opcode) {
        Some((flag, wanted)) => {
            let set = wanted == take;
            let cpu = &mut session.spec.cpu;
            cpu.f = if set { cpu.f | flag } else { cpu.f & !flag };
        }
        // DJNZ decrements first: 1 falls through, and anything else jumps.
        None => session.spec.cpu.b = if take { 2 } else { 1 },
    }
    session.spec.breakpoints.clear();
    for _ in 0..frames {
        session.spec.run(FRAME_T);
    }
    Ok((session.spec.cpu.pc, session.spec.cpu.sp))
}
