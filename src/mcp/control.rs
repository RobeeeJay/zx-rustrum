//! Making the machine go: stepping, running, and stopping it again.
//!
//! Everything here goes through `Spectrum::run`, which stops for a breakpoint
//! or a watched event of its own accord, so a tool's job is to decide the
//! budget and then say what happened.

use crate::machine::{Stop, FRAME_T};
use crate::mcp::json::Json;
use crate::mcp::tools::{addr_of, count, describe_stop, flag, Session, RUN_LIMIT_FRAMES};

pub fn step(session: &mut Session, args: &Json) -> Result<String, String> {
    let count = count(args, "count", 1)?.min(10_000);
    let over = flag(args, "over", false);
    let mut lines = Vec::new();
    for _ in 0..count {
        let pc = session.spec.cpu.pc;
        let insn = crate::disasm::disasm(&|a| session.spec.bus.peek_raw(a), pc);
        if over && session.spec.is_step_over_target(pc) {
            let after = pc.wrapping_add(insn.len as u16);
            let stop = run_to(session, &[after], RUN_LIMIT_FRAMES)?;
            lines.push(format!("${pc:04X}  {:<20} (over)", insn.text));
            if !matches!(stop, Stop::Breakpoint(_)) {
                lines.push(format!("  did not come back: {}", describe_stop(stop)));
                break;
            }
        } else {
            lines.push(format!("${pc:04X}  {}", insn.text));
            session.spec.step_instruction();
        }
    }
    lines.push(session.registers());
    Ok(lines.join("\n"))
}

pub fn run_frames(session: &mut Session, args: &Json) -> Result<String, String> {
    let frames = count(args, "frames", 1)?.min(RUN_LIMIT_FRAMES * 10);
    let note = session.let_go_of_recording();
    let before = session.spec.bus.frame;
    let mut stopped = None;
    for _ in 0..frames {
        let stop = session.spec.run(FRAME_T);
        if !matches!(stop, Stop::Budget) {
            stopped = Some(stop);
            break;
        }
    }
    Ok(note.to_string() + &after_running(session, session.spec.bus.frame - before, stopped))
}

pub fn run_tstates(session: &mut Session, args: &Json) -> Result<String, String> {
    let want = count(args, "tstates", 1)?;
    let note = session.let_go_of_recording();
    let before = session.spec.bus.total_t();
    let mut left = want;
    let mut stopped = None;
    while left > 0 {
        let slice = left.min(FRAME_T);
        let stop = session.spec.run(slice);
        let done = (session.spec.bus.total_t() - before) as u32;
        if !matches!(stop, Stop::Budget) {
            stopped = Some(stop);
            break;
        }
        left = want.saturating_sub(done);
    }
    let ran = session.spec.bus.total_t() - before;
    let mut out = format!("{note}Ran {ran} T-states. {}", session.registers());
    if let Some(stop) = stopped {
        out.push_str(&format!("\nStopped: {}", describe_stop(stop)));
    }
    Ok(out)
}

pub fn run_until(session: &mut Session, args: &Json) -> Result<String, String> {
    let mut wanted = Vec::new();
    if let Some(one) = args.get("address") {
        wanted.push(addr_of(one)?);
    }
    if let Some(list) = args.get("addresses").and_then(|a| a.as_array()) {
        for item in list {
            wanted.push(addr_of(item)?);
        }
    }
    if wanted.is_empty() {
        return Err("run_until needs an address, or addresses".into());
    }
    let limit = count(args, "max_frames", RUN_LIMIT_FRAMES)?;
    let note = session.let_go_of_recording();
    let before = session.spec.bus.frame;
    let stop = run_to(session, &wanted, limit)?;
    Ok(note.to_string() + &after_running(session, session.spec.bus.frame - before, Some(stop)))
}

/// Run with temporary breakpoints on `wanted`, putting back whatever
/// breakpoints were there before.
pub fn run_to(session: &mut Session, wanted: &[u16], limit_frames: u32) -> Result<Stop, String> {
    let kept = std::mem::take(&mut session.spec.breakpoints);
    session.spec.breakpoints = wanted.to_vec();
    let mut stop = Stop::Budget;
    for _ in 0..limit_frames {
        stop = session.spec.run(FRAME_T);
        if !matches!(stop, Stop::Budget) {
            break;
        }
    }
    session.spec.breakpoints = kept;
    Ok(stop)
}

pub fn watch_events(session: &mut Session, args: &Json) -> Result<String, String> {
    let breaks = &mut session.spec.bus.breaks;
    for (key, field) in [
        ("screen", 0),
        ("beeper", 1),
        ("ay", 2),
        ("interrupt", 3),
        ("rom", 4),
        ("port_in", 5),
        ("port_out", 6),
    ] {
        if let Some(on) = args.get(key).and_then(|v| v.as_bool()) {
            match field {
                0 => breaks.screen = on,
                1 => breaks.beeper = on,
                2 => breaks.ay = on,
                3 => breaks.interrupt = on,
                4 => breaks.rom = on,
                5 => breaks.port_in = on,
                _ => breaks.port_out = on,
            }
        }
    }
    // A write to an address, or to a range of them. The one watch that is
    // about a place rather than a kind of thing.
    if let Some(value) = args.get("write_to") {
        if value.is_null() {
            session.spec.bus.breaks.write_range = None;
        } else {
            let low = addr_of(value)?;
            let high = match args.get("write_to_end") {
                Some(end) => addr_of(end)?,
                None => low,
            };
            session.spec.bus.breaks.write_range = Some((low.min(high), low.max(high)));
        }
    }

    let breaks = session.spec.bus.breaks;
    let mut on: Vec<String> = Vec::new();
    if let Some((low, high)) = breaks.write_range {
        on.push(if low == high {
            format!("writes to ${low:04X}")
        } else {
            format!("writes to ${low:04X}-${high:04X}")
        });
    }
    for (name, set) in [
        ("screen", breaks.screen),
        ("beeper", breaks.beeper),
        ("ay", breaks.ay),
        ("interrupt", breaks.interrupt),
        ("rom", breaks.rom),
        ("port_in", breaks.port_in),
        ("port_out", breaks.port_out),
    ] {
        if set {
            on.push(name.to_string());
        }
    }
    let watching = if on.is_empty() {
        "nothing".to_string()
    } else {
        on.join(", ")
    };
    if flag(args, "run", false) {
        let frames = count(args, "max_frames", RUN_LIMIT_FRAMES)?;
        let mut stop = Stop::Budget;
        for _ in 0..frames {
            stop = session.spec.run(FRAME_T);
            if !matches!(stop, Stop::Budget) {
                break;
            }
        }
        return Ok(format!(
            "Watching {watching}. Stopped: {}\n{}",
            describe_stop(stop),
            session.registers()
        ));
    }
    Ok(format!(
        "Watching {watching}. Add run: true to run until one of them happens."
    ))
}

fn after_running(session: &Session, frames: u64, stop: Option<Stop>) -> String {
    let mut out = format!("Ran {frames} frames. {}", session.registers());
    if let Some(stop) = stop {
        out.push_str(&format!("\nStopped: {}", describe_stop(stop)));
    }
    out
}

/// How many steps can be taken back. Twenty is the debugger's own number: far
/// enough to see how the machine got where it is, and short of being a
/// recording, which there is a format for already.
pub const REWIND: usize = 20;

/// Step forward, keeping what it would take to undo it.
///
/// A step taken this way costs a list of the writes it made, so it is only
/// worth doing while somebody is stepping by hand — which is exactly when the
/// question "what did that just do?" gets asked one instruction too late.
pub fn step_recording(session: &mut Session, args: &Json) -> Result<String, String> {
    let count = count(args, "count", 1)?.min(REWIND as u32);
    let mut lines = Vec::new();
    for _ in 0..count {
        let pc = session.spec.cpu.pc;
        let insn = crate::disasm::disasm(&|a| session.spec.bus.peek_raw(a), pc);
        let undo = session.spec.step_recording();
        session.history.push(undo);
        if session.history.len() > REWIND {
            session.history.remove(0);
        }
        lines.push(format!("${pc:04X}  {}", insn.text));
    }
    lines.push(format!(
        "{} steps can be taken back.\n{}",
        session.history.len(),
        session.registers()
    ));
    Ok(lines.join("\n"))
}

/// Put the machine back the way it was before the last stepped instruction.
///
/// Only instructions stepped with step_back's own forward tool can be undone:
/// running does not keep the writes, because keeping them for a frame of a
/// game would cost more than the frame.
pub fn step_back(session: &mut Session, args: &Json) -> Result<String, String> {
    let want = count(args, "count", 1)? as usize;
    if session.history.is_empty() {
        return Err(
            "nothing to go back over. step_forward keeps what it would take to undo an \
             instruction; plain step and the run tools do not, since keeping the writes \
             of a whole frame would cost more than the frame."
                .into(),
        );
    }
    let mut undone = 0;
    while undone < want {
        let Some(undo) = session.history.pop() else {
            break;
        };
        session.spec.undo_step(&undo);
        undone += 1;
    }
    Ok(format!(
        "Went back {undone} instruction{}. {} left.\n{}",
        if undone == 1 { "" } else { "s" },
        session.history.len(),
        session.registers()
    ))
}

/// Which ROM and which RAM bank the 128K has paged in, and switching them.
///
/// A 128K program moves its memory about under itself; a disassembly that does
/// not say which bank was in at $C000 is a disassembly of somewhere else.
pub fn paging(session: &mut Session, args: &Json) -> Result<String, String> {
    if !session.spec.bus.model.has_paging() {
        return Err(format!(
            "a {} has no paging: $0000-$3FFF is ROM and the rest is the only RAM there is",
            session.spec.bus.model.name()
        ));
    }
    if let Some(value) = args.get("page_register") {
        let byte = value
            .as_i64()
            .filter(|v| (0..=255).contains(v))
            .ok_or("page_register should be a byte")? as u8;
        // Through the port, so the machine does what it would have done: the
        // paging lock bit is part of that, and setting the register directly
        // would step around it.
        use crate::z80::Bus;
        session.spec.bus.io_write(0x7FFD, byte);
    }
    let bus = &session.spec.bus;
    Ok(format!(
        "Page register ${:02X}{}: RAM bank {} at $C000, ROM {} at $0000, screen from bank \
         {}. Banks in the address space now: {:?}.\n\
         read_memory takes a bank of its own, so a bank that is paged out can still be read.",
        bus.page_reg,
        if bus.paging_locked {
            " (locked — the program has shut the door behind it)"
        } else {
            ""
        },
        bus.page_reg & 7,
        bus.rom_in_use(),
        if bus.page_reg & 0x08 != 0 { 7 } else { 5 },
        bus.visible_banks()
    ))
}
