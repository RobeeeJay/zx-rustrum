//! Reading the program rather than running it: the disassembly, what the
//! machine was seen to do, and the notes that are written about it.
//!
//! The measurements come from `src/observe.rs`, which attributes every write,
//! read, port access and loop to the routine on top of the call stack. That is
//! why these tools say "watch_routines first": with the observer off, nothing
//! is being counted and the answers would be empty rather than wrong.

use crate::mcp::json::Json;
use crate::mcp::tools::{addr, count, flag, text, Session};

/// How many routines a listing hands back before it is cut off. A game makes a
/// few hundred; the first fifty by work done are the ones worth reading.
const MAX_ROUTINES: usize = 50;

pub fn disassemble(session: &mut Session, args: &Json) -> Result<String, String> {
    let start = match args.get("address") {
        Some(_) => addr(args, "address")?,
        None => session.spec.cpu.pc,
    };
    let lines = count(args, "count", 32)?.min(512);
    let comments = flag(args, "comments", true);
    let peek = |a: u16| session.spec.bus.peek_raw(a);

    let mut out = String::new();
    let mut at = start;
    for _ in 0..lines {
        let insn = crate::disasm::disasm(&peek, at);
        let bytes: String = insn
            .bytes
            .iter()
            .map(|b| format!("{b:02X} "))
            .collect::<String>();
        // A name typed in this session wins; a name from a symbol file fills
        // in where there is none, which is what makes a ROM call readable.
        let own = session.notes.label(at);
        let from_file = session.symbols.get(at);
        let label = if !own.is_empty() {
            own.to_string()
        } else {
            from_file
                .map(|(name, _)| name.to_string())
                .unwrap_or_default()
        };
        let comment = session.notes.comment(at);
        let executed = session.spec.bus.observer.was_executed(at);
        let data = session.spec.bus.observer.is_data(at);

        if comments && !label.is_empty() {
            out.push_str(&format!("{label}:\n"));
        }
        out.push_str(&format!("${at:04X}  {bytes:<12} {:<20}", insn.text));
        let mut notes = Vec::new();
        // Where the instruction names an address that has a name, say so on
        // the line: "CALL $0D6B" means nothing, "CALL $0D6B (rom_cls)" does.
        if comments {
            let mut named = Vec::new();
            crate::layout::replace_addresses(&insn.text, &mut |value| {
                if let Some(name) = crate::mcp::listing::name_of(session, value) {
                    named.push(format!("-> {name}"));
                }
                None
            });
            notes.extend(named);
            if let Some((_, said)) = session.notes.layout().name_displacement(at, &insn.text) {
                notes.push(said);
            }
        }
        if executed {
            notes.push("run".to_string());
        } else if data {
            notes.push("read as data".to_string());
        }
        if comments && !comment.is_empty() {
            notes.push(comment.to_string());
        }
        if !notes.is_empty() {
            out.push_str(&format!("; {}", notes.join(" — ")));
        }
        out.push('\n');
        at = at.wrapping_add(insn.len.max(1) as u16);
    }
    out.push_str(&format!("(next address ${at:04X})\n"));
    Ok(out)
}

pub fn watch_routines(session: &mut Session, args: &Json) -> Result<String, String> {
    let on = flag(args, "enabled", true);
    session.spec.bus.observer.enabled = on;
    if flag(args, "clear", false) {
        session.spec.bus.observer.clear();
        // What has run is part of what was seen: the export decodes code
        // wherever it says so, and an unpacker that ran there first and then
        // wrote the program over itself would otherwise leave its mark.
        session.spec.bus.tracker.executed.fill(false);
        session.ran_before.clear();
    }
    Ok(if on {
        "Watching. Every write, read, port access and loop is now attributed to \
         the routine making it; run the machine, then ask for routines. \
         Watching costs a branch on every access, so turn it off when done."
            .to_string()
    } else {
        "Not watching any more. What was already seen is kept; pass clear: true to forget it."
            .to_string()
    })
}

pub fn routines(session: &mut Session, args: &Json) -> Result<String, String> {
    let observer = &session.spec.bus.observer;
    if observer.routines.is_empty() {
        return Err(
            "nothing has been watched yet: call watch_routines, then run the machine".into(),
        );
    }
    let limit = count(args, "limit", MAX_ROUTINES as u32)? as usize;
    let mut list: Vec<_> = observer.routines.values().collect();
    match text(args, "sort")
        .unwrap_or_else(|_| "work".into())
        .as_str()
    {
        "calls" => list.sort_by_key(|r| std::cmp::Reverse(r.calls)),
        "address" => list.sort_by_key(|r| r.entry),
        "writes" => list.sort_by_key(|r| std::cmp::Reverse(r.inclusive.total())),
        _ => list.sort_by_key(|r| std::cmp::Reverse(r.instructions)),
    }
    let frames = observer.frames.max(1);

    let mut out = format!(
        "{} routines seen over {} frames. Columns: entry, calls, instructions, \
         writes as screen/attrs/other (per call, including what it calls), \
         reads, ports, size.\n",
        observer.routines.len(),
        observer.frames
    );
    for r in list.iter().take(limit) {
        let per_call = |v: u32| v as f64 / r.calls.max(1) as f64;
        let size = r
            .spans
            .map_or(0, |(low, high)| high as u32 - low as u32 + 1);
        let label = session.notes.label(r.entry).to_string();
        out.push_str(&format!(
            "${:04X} {label:<16} calls {:>6} ({:.1}/frame)  insns {:>9}  \
             writes {:.0}/{:.0}/{:.0}  reads {:.0}  ports {}in/{}out  size {size}\n",
            r.entry,
            r.calls,
            r.calls as f64 / frames as f64,
            r.instructions,
            per_call(r.inclusive.screen),
            per_call(r.inclusive.attrs),
            per_call(r.inclusive.other),
            per_call(r.inclusive_reads.total()),
            r.ports_in.len(),
            r.ports_out.len(),
        ));
    }
    if list.len() > limit {
        out.push_str(&format!("({} more not shown)\n", list.len() - limit));
    }
    Ok(out)
}

pub fn routine(session: &mut Session, args: &Json) -> Result<String, String> {
    let entry = addr(args, "address")?;
    let observed = session
        .spec
        .bus
        .observer
        .routines
        .get(&entry)
        .ok_or_else(|| format!("${entry:04X} has not been seen called while watching"))?;
    let frames = session.spec.bus.observer.frames.max(1);
    let per_call = |v: u32| v as f64 / observed.calls.max(1) as f64;

    let mut out = String::new();
    let label = session.notes.label(entry);
    out.push_str(&format!(
        "${entry:04X} {label}\ncalled {} times over {} frames ({:.2} a frame), \
         {} instructions, deepest nesting {}\n",
        observed.calls,
        session.spec.bus.observer.frames,
        observed.calls as f64 / frames as f64,
        observed.instructions,
        observed.max_depth
    ));
    match observed.spans {
        Some((low, high)) => out.push_str(&format!(
            "reaches ${low:04X}-${high:04X} ({} bytes) — where it got to, not a promise: \
             a routine that jumps over a table reaches past the table\n",
            high as u32 - low as u32 + 1
        )),
        None => out.push_str("extent not established\n"),
    }
    out.push_str(&format!(
        "writes per call: {:.1} to the display file, {:.1} to attributes, {:.1} elsewhere \
         (itself alone: {:.1}/{:.1}/{:.1})\n",
        per_call(observed.inclusive.screen),
        per_call(observed.inclusive.attrs),
        per_call(observed.inclusive.other),
        per_call(observed.writes.screen),
        per_call(observed.writes.attrs),
        per_call(observed.writes.other),
    ));
    out.push_str(&format!(
        "reads per call: {:.1} (itself alone {:.1})\n",
        per_call(observed.inclusive_reads.total()),
        per_call(observed.reads.total())
    ));
    if let Some((low, high)) = observed.wrote_between {
        out.push_str(&format!("wrote between ${low:04X} and ${high:04X}\n"));
    }
    if let Some(byte) = observed.filled_with {
        out.push_str(&format!("filled with ${byte:02X}\n"));
    }
    if !observed.ports_in.is_empty() || !observed.ports_out.is_empty() {
        out.push_str(&format!(
            "ports: in {} ({} reads), out {} ({} writes)\n",
            ports(&observed.ports_in),
            observed.port_reads,
            ports(&observed.ports_out),
            observed.port_writes
        ));
    }
    if !observed.loops.is_empty() {
        let mut loops: Vec<_> = observed.loops.iter().collect();
        loops.sort_by_key(|(_, times)| std::cmp::Reverse(**times));
        out.push_str("loops (address, times round): ");
        out.push_str(
            &loops
                .iter()
                .take(8)
                .map(|(at, times)| format!("${at:04X}×{times}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push('\n');
    }
    if !observed.exits.is_empty() {
        out.push_str(&format!("exits: {}\n", addresses(&observed.exits)));
    }
    if !observed.after.is_empty() {
        out.push_str(&format!("ends at: {}\n", addresses(&observed.after)));
    }
    if !observed.hot.is_empty() {
        out.push_str(&format!(
            "busiest addresses: {}\n",
            addresses(&observed.hot)
        ));
    }
    out.push_str(&format!(
        "entry registers seen: AF {} BC {} DE {} HL {}\n",
        seen(&observed.entry_af),
        seen(&observed.entry_bc),
        seen(&observed.entry_de),
        seen(&observed.entry_hl)
    ));
    Ok(out)
}

fn ports(list: &[u16]) -> String {
    if list.is_empty() {
        return "none".into();
    }
    list.iter()
        .take(8)
        .map(|p| format!("${p:04X}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn addresses(list: &[u16]) -> String {
    list.iter()
        .take(12)
        .map(|a| format!("${a:04X}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn seen(range: &crate::observe::Seen) -> String {
    if range.low == range.high {
        format!("${:04X}", range.low)
    } else {
        format!("${:04X}-${:04X}", range.low, range.high)
    }
}

pub fn call_graph(session: &mut Session, args: &Json) -> Result<String, String> {
    let observer = &session.spec.bus.observer;
    if observer.edges.is_empty() {
        return Err("no calls have been watched: call watch_routines, then run the machine".into());
    }
    let limit = count(args, "limit", 60)? as usize;
    let mut edges: Vec<_> = observer
        .edges
        .iter()
        .map(|((from, to), edge)| (*from, *to, edge.calls))
        .collect();
    edges.sort_by_key(|e| std::cmp::Reverse(e.2));
    let mut out = format!(
        "{} call edges between {} routines, the busiest first.\n",
        observer.edges.len(),
        observer.routines.len()
    );
    for (from, to, calls) in edges.iter().take(limit) {
        let from_label = session.notes.label(*from);
        let to_label = session.notes.label(*to);
        out.push_str(&format!(
            "${from:04X} {from_label:<14} -> ${to:04X} {to_label:<14} {calls} calls\n"
        ));
    }
    if edges.len() > limit {
        out.push_str(&format!("({} more not shown)\n", edges.len() - limit));
    }
    Ok(out)
}

pub fn code_map(session: &mut Session, args: &Json) -> Result<String, String> {
    let min = count(args, "min_length", 8)? as u16;
    let observer = &session.spec.bus.observer;
    let code = observer.code_runs();
    let data = observer.data_blocks(min);
    if code.is_empty() && data.is_empty() {
        return Err(
            "nothing has been watched yet: call watch_routines, then run the machine".into(),
        );
    }
    let mut out = format!(
        "What was executed, and what was only read. {} runs of code, {} blocks of data.\n\
         This is what the machine did while it was watched, not everything that exists.\n",
        code.len(),
        data.len()
    );
    out.push_str("code:\n");
    for (low, high) in code.iter().take(60) {
        out.push_str(&format!(
            "  ${low:04X}-${high:04X}  {} bytes\n",
            *high as u32 - *low as u32 + 1
        ));
    }
    out.push_str("data (read, never executed):\n");
    // data_blocks gives where each block starts and how long it is, longest first.
    for (low, length) in data.iter().take(60) {
        let length = *length as u32;
        let high = (*low as u32 + length - 1) as u16;
        let where_ = match low {
            0x4000..=0x57FF => " — in the display file",
            0x5800..=0x5AFF => " — in the attributes",
            _ => "",
        };
        out.push_str(&format!(
            "  ${low:04X}-${high:04X}  {length} bytes{where_}\n"
        ));
    }
    Ok(out)
}

pub fn autodoc(session: &mut Session, args: &Json) -> Result<String, String> {
    let mut entries: Vec<u16> = Vec::new();
    if let Some(list) = args.get("entries").and_then(|e| e.as_array()) {
        for item in list {
            entries.push(crate::mcp::tools::addr_of(item)?);
        }
    }
    if entries.is_empty() {
        // Whatever has been watched, busiest first: the routines a game
        // spends its time in are the ones worth a name.
        let mut seen: Vec<_> = session.spec.bus.observer.routines.values().collect();
        seen.sort_by_key(|r| std::cmp::Reverse(r.instructions));
        entries = seen.iter().take(40).map(|r| r.entry).collect();
    }
    if entries.is_empty() {
        entries.push(session.spec.cpu.pc);
    }
    let peek = |a: u16| session.spec.bus.peek_raw(a);
    let doc = crate::autodoc::analyse(&peek, &entries);
    if doc.is_empty() {
        return Ok(
            "AutoDoc found nothing it was willing to name. Code with no tell is \
                   left unnamed rather than guessed at."
                .to_string(),
        );
    }
    let mut out = String::from(
        "AutoDoc's guesses. These are guesses: it says so in the wording where the \
         evidence is thin, and a routine with no tell is left out.\n",
    );
    for entry in &entries {
        let label = doc.label(*entry);
        let comment = doc.comment(*entry);
        if label.is_empty() && comment.is_empty() {
            continue;
        }
        out.push_str(&format!("${entry:04X}  {label}"));
        if !comment.is_empty() {
            out.push_str(&format!("  — {comment}"));
        }
        out.push('\n');
    }
    Ok(out)
}

pub fn set_comment(session: &mut Session, args: &Json) -> Result<String, String> {
    let at = addr(args, "address")?;
    let mut did = Vec::new();
    if let Some(label) = args.get("label").and_then(|l| l.as_str()) {
        session.notes.set_label(at, label);
        did.push(format!("label {label:?}"));
    }
    if let Some(comment) = args.get("comment").and_then(|c| c.as_str()) {
        session.notes.set_comment(at, comment);
        did.push(format!("comment {comment:?}"));
    }
    if did.is_empty() {
        return Err("set_comment needs a label, a comment, or both".into());
    }
    Ok(format!(
        "${at:04X}: {}. Typed rather than guessed, so nothing will overwrite it.",
        did.join(", ")
    ))
}

pub fn comments(session: &mut Session, args: &Json) -> Result<String, String> {
    let from = match args.get("from") {
        Some(_) => addr(args, "from")?,
        None => 0,
    };
    let to = match args.get("to") {
        Some(_) => addr(args, "to")?,
        None => 0xFFFF,
    };
    let mut lines = Vec::new();
    for (at, label, auto) in session.notes.labelled() {
        if at < from || at > to {
            continue;
        }
        let comment = session.notes.comment(at);
        let mark = if auto { " (a guess)" } else { "" };
        lines.push(format!(
            "${at:04X}  {label}{mark}{}",
            if comment.is_empty() {
                String::new()
            } else {
                format!("  — {comment}")
            }
        ));
    }
    if lines.is_empty() {
        return Ok(format!(
            "Nothing written between ${from:04X} and ${to:04X}. There are {} notes altogether.",
            session.notes.len()
        ));
    }
    Ok(lines.join("\n"))
}

pub fn save_comments(session: &mut Session, _args: &Json) -> Result<String, String> {
    match session.notes.file() {
        None => Err(
            "the notes are not attached to a file: load a tape or a snapshot first, \
                     and they are written beside it"
                .into(),
        ),
        Some(path) => {
            let path = path.to_path_buf();
            match session.notes.save_if_dirty() {
                Ok(true) => Ok(format!("Written to {}", path.display())),
                Ok(false) => Ok(format!(
                    "Nothing had changed since the last write to {}",
                    path.display()
                )),
                Err(e) => Err(format!("{}: {e}", path.display())),
            }
        }
    }
}

/// Everything that refers to an address, both ways round.
///
/// Two kinds of answer, and they are worth telling apart. What the machine was
/// watched doing is fact: these routines called it, this one drew that cell,
/// these read that page. What the code says is a search: the bytes `CD 00 90`
/// are a call to $9000 wherever they appear, and some of them will be data
/// that happens to look like one. Both are given, labelled.
pub fn xrefs(session: &mut Session, args: &Json) -> Result<String, String> {
    let at = addr(args, "address")?;
    let mut out = format!("References to ${at:04X}\n");

    // ---- what was watched -------------------------------------------------
    let observer = &session.spec.bus.observer;
    let callers: Vec<(u16, u32)> = observer
        .edges
        .iter()
        .filter(|((_, to), _)| *to == at)
        .map(|((from, _), edge)| (*from, edge.calls))
        .collect();
    if !callers.is_empty() {
        out.push_str("called by (watched):\n");
        for (from, calls) in callers.iter().take(20) {
            out.push_str(&format!(
                "  ${from:04X} {:<16} {calls} times\n",
                session.notes.label(*from)
            ));
        }
    }
    let calls_out: Vec<(u16, u32)> = observer
        .edges
        .iter()
        .filter(|((from, _), _)| *from == at)
        .map(|((_, to), edge)| (*to, edge.calls))
        .collect();
    if !calls_out.is_empty() {
        out.push_str("calls (watched):\n");
        for (to, calls) in calls_out.iter().take(20) {
            out.push_str(&format!(
                "  ${to:04X} {:<16} {calls} times\n",
                session.notes.label(*to)
            ));
        }
    }
    let users = observer.users_of(at);
    if !users.is_empty() {
        out.push_str("routines that hammered this address (watched):\n");
        for entry in users.iter().take(20) {
            out.push_str(&format!("  ${entry:04X} {}\n", session.notes.label(*entry)));
        }
    }
    let readers = observer.readers_of((at >> 8) as u8);
    if !readers.is_empty() {
        out.push_str(&format!(
            "routines that read page ${:02X}xx (watched, commonest first):\n",
            at >> 8
        ));
        for entry in readers.iter().take(12) {
            out.push_str(&format!("  ${entry:04X} {}\n", session.notes.label(*entry)));
        }
    }
    if let Some(who) = observer.drew(at) {
        out.push_str(&format!(
            "the byte of screen at ${at:04X} was last drawn by ${who:04X} {}\n",
            session.notes.label(who)
        ));
    }

    // ---- what the code says ----------------------------------------------
    let from = match args.get("search_from") {
        Some(_) => addr(args, "search_from")?,
        None => 0x4000,
    };
    let to = match args.get("search_to") {
        Some(_) => addr(args, "search_to")?,
        None => 0xFFFF,
    };
    let target = at.to_le_bytes();
    let mut found = Vec::new();
    let mut a = from;
    while a < to {
        if session.spec.bus.peek_raw(a.wrapping_add(1)) == target[0]
            && session.spec.bus.peek_raw(a.wrapping_add(2)) == target[1]
        {
            let opcode = session.spec.bus.peek_raw(a);
            // The three-byte instructions that carry an address: CALL, JP,
            // their conditional forms, and the loads through (nn).
            let kind = match opcode {
                0xCD => Some("CALL"),
                0xC4 | 0xCC | 0xD4 | 0xDC | 0xE4 | 0xEC | 0xF4 | 0xFC => Some("CALL cc"),
                0xC3 => Some("JP"),
                0xC2 | 0xCA | 0xD2 | 0xDA | 0xE2 | 0xEA | 0xF2 | 0xFA => Some("JP cc"),
                0x21 | 0x01 | 0x11 | 0x31 => Some("LD rr,nn"),
                0x2A | 0x3A | 0x22 | 0x32 => Some("LD (nn)"),
                _ => None,
            };
            if let Some(kind) = kind {
                let insn = crate::disasm::disasm(&|x| session.spec.bus.peek_raw(x), a);
                found.push(format!("  ${a:04X}  {:<18} ({kind})", insn.text));
            }
        }
        a = a.wrapping_add(1);
        if a == 0 {
            break;
        }
    }
    if found.is_empty() {
        out.push_str(&format!(
            "nothing in ${from:04X}-${to:04X} holds the bytes of ${at:04X} as an instruction would.\n"
        ));
    } else {
        out.push_str(&format!(
            "instructions naming ${at:04X}, found by searching ${from:04X}-${to:04X} — some of \
             these will be data that happens to look like code:\n"
        ));
        for line in found.iter().take(40) {
            out.push_str(line);
            out.push('\n');
        }
        if found.len() > 40 {
            out.push_str(&format!("  ({} more)\n", found.len() - 40));
        }
    }
    Ok(out)
}

/// Where the beam is, and when in the frame the routines ran.
///
/// On this machine *when* is half the question. The ULA puts the picture out
/// as it goes, so a write above the beam is seen this frame and one below it
/// waits for the next: that is what a flickering sprite is. A routine that
/// always runs at the same point in the frame is doing its work to a
/// timetable; one that wanders is not.
pub fn frame_timing(session: &mut Session, args: &Json) -> Result<String, String> {
    let bus = &session.spec.bus;
    let model = bus.model;
    let first_pixel = bus.first_pixel_t();
    let per_line = model.t_per_line();
    let frame_t = model.frame_t();
    let display_end = first_pixel + 192 * per_line;

    let line_of = |t: u32| -> i64 {
        // Counted from the first line of the display, so the top border is
        // negative — which is where the beam is at the interrupt.
        (t as i64 - first_pixel as i64).div_euclid(per_line as i64)
    };
    let describe = |t: u32| -> String {
        let line = line_of(t);
        if t < first_pixel {
            format!("T {t}, the top border, {} lines before the display", -line)
        } else if t < display_end {
            format!("T {t}, display line {line} of 192")
        } else {
            format!("T {t}, below the display, {} lines past it", line - 192)
        }
    };

    let mut out = format!(
        "{}: a frame is {frame_t} T-states, a line {per_line}. The interrupt is at T 0; \
         the first pixel goes out at T {first_pixel} and the last at T {}.\n\
         The machine is at {} in frame {}.\n",
        model.name(),
        display_end - 1,
        describe(bus.tstates),
        bus.frame
    );

    if let Some(one) = args.get("address") {
        let entry = crate::mcp::tools::addr_of(one)?;
        let observed = session
            .spec
            .bus
            .observer
            .routines
            .get(&entry)
            .ok_or_else(|| format!("${entry:04X} has not been seen called while watching"))?;
        let frames_seen = session.spec.bus.observer.frames as u32;
        if observed.entered_at.is_empty() {
            out.push_str(&format!(
                "${entry:04X} has been called, but not while the frame position was being \
                 recorded.\n"
            ));
            return Ok(out);
        }
        out.push_str(&format!(
            "\n${entry:04X} {} was entered between {} and {}.\n",
            session.notes.label(entry),
            describe(observed.entered_at.low),
            describe(observed.entered_at.high)
        ));
        // The same T-state every time is a routine on a timetable. A line's
        // worth of slack, since an interrupt is taken between instructions and
        // the handler's own work varies by a few dozen T-states.
        if observed.entered_at.high - observed.entered_at.low < per_line {
            out.push_str(
                "Always at the same point in the frame: it is being run to a timetable, \
                 which usually means from the interrupt handler.\n",
            );
        }
        out.push_str(&format!(
            "It ran in {} of the {frames_seen} frames watched{}.\n",
            observed.frames,
            if observed.every_frame(frames_seen) {
                " — every frame, near enough"
            } else {
                ""
            }
        ));
        if observed.inclusive.screen > 0 {
            let above = observed.entered_at.high < first_pixel;
            let below = observed.entered_at.low > display_end;
            out.push_str(match (above, below) {
                (true, _) => {
                    "It writes to the display file before the beam reaches it, so \
                              what it draws is seen in the same frame.\n"
                }
                (_, true) => {
                    "It writes to the display file after the beam has passed, so \
                              what it draws is not seen until the next frame — which is \
                              what a flickering sprite is.\n"
                }
                _ => {
                    "It writes to the display file while the beam is crossing it, so some \
                      of what it draws is seen this frame and some next.\n"
                }
            });
        }
    }
    Ok(out)
}

/// The blocks a listing is divided into: what is code and what is data.
///
/// Worked out from what ran, and kept in the same notes file as the labels, so
/// a listing keeps its shape between sessions. `blocks::work_out` reads the
/// observer; anything already written down is kept, since a person who has
/// looked at the bytes knows better than a run that has not reached them.
pub fn blocks(session: &mut Session, args: &Json) -> Result<String, String> {
    if flag(args, "work_out", false) {
        let found = crate::blocks::work_out(&session.spec.bus.observer);
        let merged = crate::blocks::merge(session.notes.blocks(), &found);
        let added = merged.len().saturating_sub(session.notes.blocks().len());
        session.notes.set_blocks(merged);
        return Ok(format!(
            "Worked out {} blocks from what has run; {added} of them are new. \
             What was already written down was kept.",
            found.len()
        ));
    }
    if let (Some(from), Some(to)) = (args.get("from"), args.get("to")) {
        let from = crate::mcp::tools::addr_of(from)?;
        let to = crate::mcp::tools::addr_of(to)?;
        let kind = match crate::mcp::tools::text(args, "kind")
            .unwrap_or_else(|_| "code".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "code" => crate::blocks::Kind::Code,
            "data" => crate::blocks::Kind::Data,
            other => return Err(format!("a block is code or data, not {other:?}")),
        };
        let mut blocks = session.notes.blocks().to_vec();
        blocks.push(crate::blocks::Block { from, to, kind });
        blocks.sort();
        session.notes.set_blocks(blocks);
        return Ok(format!("${from:04X}-${to:04X} marked as {}.", kind.label()));
    }
    let blocks = session.notes.blocks();
    if blocks.is_empty() {
        return Ok(
            "No blocks written down. Pass work_out: true to take them from what has run, \
             or from and to with kind to mark one by hand."
                .into(),
        );
    }
    let mut out = format!("{} blocks:\n", blocks.len());
    for block in blocks.iter().take(100) {
        out.push_str(&format!(
            "  ${:04X}-${:04X}  {:<5} {} bytes\n",
            block.from,
            block.to,
            block.kind.label(),
            block.length()
        ));
    }
    Ok(out)
}

/// Where the time goes, which is a different question from what runs most
/// often: a routine called twice a frame that takes half of it matters more
/// than one called two hundred times that does not.
pub fn profile(session: &mut Session, args: &Json) -> Result<String, String> {
    let frames = count(args, "frames", 50)?.min(3000);
    let cpu_hz = session.spec.bus.model.cpu_hz();
    let now = session.spec.bus.total_t();
    session.spec.profiler.start(now, cpu_hz);
    for _ in 0..frames {
        session.spec.run(crate::machine::FRAME_T);
    }
    let now = session.spec.bus.total_t();
    session.spec.profiler.stop(now);

    let run = session
        .spec
        .profiler
        .runs
        .last()
        .ok_or("the profiler recorded nothing")?;
    let metric = crate::profiler::Metric::SelfTime;
    let ranked = run.ranked(metric);
    let total = run.total(metric).max(1);
    if ranked.is_empty() {
        return Ok(format!(
            "Nothing was profiled over {frames} frames. The profiler follows calls, so a \
             program whose main loop was never itself called has nothing to attribute."
        ));
    }
    let mut out = format!(
        "Where the time went over {frames} frames ({:.2}s of the machine's own time), by \
         time in the routine itself rather than in what it called:\n",
        run.seconds(run.emulated_t)
    );
    for stats in ranked.iter().take(count(args, "limit", 20)? as usize) {
        let share = stats.time(metric) as f64 / total as f64 * 100.0;
        out.push_str(&format!(
            "  ${:04X} {:<16} {share:5.1}%  {:>8} calls  {:.1}ms in itself, {:.1}ms including \
             what it called\n",
            stats.entry,
            session.notes.label(stats.entry),
            stats.calls,
            run.seconds(stats.self_t) * 1000.0,
            run.seconds(stats.incl_t) * 1000.0,
        ));
    }
    if run.unfinished > 0 {
        out.push_str(&format!(
            "{} routines had not returned when the run ended; their time is counted where \
             they were.\n",
            run.unfinished
        ));
    }
    Ok(out)
}

/// The program in RAM as assembly source that builds back into the same bytes.
pub fn export_asm(session: &mut Session, args: &Json) -> Result<String, String> {
    use crate::mcp::listing::{references, seen_line, used_by_line, with_marks};
    let path = crate::mcp::tools::text(args, "path")?;
    let from = match args.get("from") {
        Some(_) => addr(args, "from")?,
        None => crate::asmexport::FROM,
    };
    let to = match args.get("to") {
        Some(_) => addr(args, "to")?,
        None => 0xFFFF,
    };
    if to <= from {
        return Err("to should be above from".into());
    }
    let program = session
        .loaded
        .clone()
        .unwrap_or_else(|| "a machine with nothing loaded".into());
    let facts = flag(args, "facts", true);
    let used_by = if facts {
        // Callers count wherever they are, not only inside what is written.
        references(session, 0, 0xFFFF)
    } else {
        Default::default()
    };
    let session_ref: &Session = session;
    let above = |at: u16| -> Vec<String> {
        if !facts {
            return Vec::new();
        }
        let mut lines = Vec::new();
        if let Some(seen) = seen_line(session_ref, at) {
            lines.push(format!("seen: {seen}"));
        }
        if let Some(from) = used_by.get(&at) {
            let line = used_by_line(session_ref, at, from);
            if !line.is_empty() {
                lines.push(format!("used by: {line}"));
            }
        }
        lines
    };
    let marks = |at: u16, length: u8| -> String {
        if facts {
            with_marks(session_ref, at, length, String::new())
        } else {
            String::new()
        }
    };
    let out = crate::asmexport::from_machine_with(
        &session.spec,
        &session.notes,
        Some(&session.symbols),
        &program,
        crate::asmexport::Extras {
            ran_before: &session.ran_before,
            from,
            to,
            above: &above,
            marks: &marks,
        },
    );
    std::fs::write(&path, &out.text).map_err(|e| format!("{path}: {e}"))?;
    Ok(format!(
        "Wrote {path}: ${from:04X}-${to:04X}, {} instructions and {} bytes as data. Code is \
         only where it has been seen to run since this program was started, going back to its \
         saved states included, and not what only ran under force_branch; run the program \
         further to find more of it.",
        out.instructions, out.data_bytes
    ))
}
