//! What running a program has and has not reached, and what it jumped to that
//! the code does not name.
//!
//! Code is what has run, so a disassembly built from watching stops at whatever
//! was never played. This says where that edge is: what has run, what the code
//! that has run could reach, which branches have only ever gone one way. And a
//! `JP (HL)` names nothing, so the jump table behind one cannot be found by
//! reading: `Flow` writes down where each one actually went, and `tables` looks
//! for those addresses side by side in memory.
//!
//! What a conditional did is recorded rather than worked out afterwards. "Its
//! target ran" does not mean it was ever taken, since something else may have
//! gone there.
//!
//! A `PUSH HL : RET` used as a jump is not seen as one: nothing about a RET says
//! which it is.

use std::collections::{BTreeMap, BTreeSet};

use crate::disasm::disasm;
use crate::observe::Observer;

/// How often a conditional went each way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sides {
    pub taken: u32,
    pub not_taken: u32,
}

/// One indirect jump: where it went, and what ran just before it. A jump to a
/// one-byte `JP (HL)` elsewhere is how a program without a spare register for
/// it dispatches, and then the instruction before is the one worth reading.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Site {
    pub targets: BTreeMap<u16, u32>,
    pub from: BTreeSet<u16>,
}

/// Enough to stop a jump through a table of random words filling memory.
const MOST_TARGETS: usize = 512;
const MOST_FROM: usize = 32;

/// What the machine was seen to do at each place it had a choice.
#[derive(Clone, Debug, Default)]
pub struct Flow {
    pub conditionals: BTreeMap<u16, Sides>,
    pub indirect: BTreeMap<u16, Site>,
    previous: u16,
}

/// How long a conditional instruction is, if this opcode is one. The length is
/// what says which way it went: it fell through if it ended up just after
/// itself.
fn conditional_length(opcode: u8) -> Option<u16> {
    match opcode {
        0x10 | 0x20 | 0x28 | 0x30 | 0x38 => Some(2),
        op if op & 0xC7 == 0xC2 || op & 0xC7 == 0xC4 => Some(3),
        op if op & 0xC7 == 0xC0 => Some(1),
        _ => None,
    }
}

/// Whether an opcode is a conditional jump, call or return, or a DJNZ.
pub fn is_conditional(opcode: u8) -> bool {
    conditional_length(opcode).is_some()
}

/// Which flag a conditional tests and whether it wants it set, from the opcode.
/// None for DJNZ, which tests B and not a flag.
pub fn condition(opcode: u8) -> Option<(u8, bool)> {
    const Z: u8 = 0x40;
    const C: u8 = 0x01;
    const PV: u8 = 0x04;
    const S: u8 = 0x80;
    if opcode == 0x10 {
        return None;
    }
    // JR cc has its two conditions in bits 3-4, four along from the others.
    let index = (opcode >> 3) & 7;
    let index = if opcode & 0xE7 == 0x20 {
        index - 4
    } else {
        index
    };
    Some(match index {
        0 => (Z, false),
        1 => (Z, true),
        2 => (C, false),
        3 => (C, true),
        4 => (PV, false),
        5 => (PV, true),
        6 => (S, false),
        _ => (S, true),
    })
}

impl Flow {
    /// One instruction has run, from `pc` to `next`.
    pub fn note(&mut self, pc: u16, opcode: [u8; 2], next: u16) {
        if let Some(length) = conditional_length(opcode[0]) {
            let sides = self.conditionals.entry(pc).or_default();
            if next == pc.wrapping_add(length) {
                sides.not_taken += 1;
            } else {
                sides.taken += 1;
            }
        } else if opcode[0] == 0xE9 || (matches!(opcode[0], 0xDD | 0xFD) && opcode[1] == 0xE9) {
            let site = self.indirect.entry(pc).or_default();
            if site.targets.len() < MOST_TARGETS || site.targets.contains_key(&next) {
                *site.targets.entry(next).or_insert(0) += 1;
            }
            if site.from.len() < MOST_FROM {
                site.from.insert(self.previous);
            }
        }
        self.previous = pc;
    }

    /// The tables behind the indirect jumps. Each jump is looked for on its
    /// own: the ROM's calculator and a game's dispatcher together span nearly
    /// all of memory, and would find a "table" in anything.
    pub fn tables(&self, peek: &impl Fn(u16) -> u8, from: u16, to: u16) -> Vec<Table> {
        let mut found: Vec<Table> = Vec::new();
        for site in self.indirect.values() {
            let targets: BTreeSet<u16> = site.targets.keys().copied().collect();
            for table in tables(peek, &targets, from, to) {
                if !found.iter().any(|t| t.at == table.at) {
                    found.push(table);
                }
            }
        }
        found.sort_by_key(|t| t.at);
        found
    }
}

/// Where control can go after an instruction, read from the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    Next,
    Jump(u16),
    /// Either to here or on: a conditional jump, or any call.
    Either(u16),
    Stop,
    /// A conditional return: on, or back to wherever it came from.
    MaybeReturn,
}

/// An instruction's length, and where it can go.
pub fn exit_of(peek: &impl Fn(u16) -> u8, at: u16) -> (u16, Exit) {
    let length = disasm(peek, at).len.max(1) as u16;
    let op = peek(at);
    let word = || u16::from_le_bytes([peek(at.wrapping_add(1)), peek(at.wrapping_add(2))]);
    let relative = || {
        at.wrapping_add(2)
            .wrapping_add(peek(at.wrapping_add(1)) as i8 as u16)
    };
    let exit = match op {
        0xC3 => Exit::Jump(word()),
        0x18 => Exit::Jump(relative()),
        0x10 | 0x20 | 0x28 | 0x30 | 0x38 => Exit::Either(relative()),
        0xCD => Exit::Either(word()),
        0xC9 | 0xE9 => Exit::Stop,
        0xDD | 0xFD if peek(at.wrapping_add(1)) == 0xE9 => Exit::Stop,
        // RETN and RETI, and the undocumented copies of them.
        0xED if peek(at.wrapping_add(1)) & 0xC7 == 0x45 => Exit::Stop,
        op if op & 0xC7 == 0xC2 || op & 0xC7 == 0xC4 => Exit::Either(word()),
        op if op & 0xC7 == 0xC0 => Exit::MaybeReturn,
        op if op & 0xC7 == 0xC7 => Exit::Either((op & 0x38) as u16),
        _ => Exit::Next,
    };
    (length, exit)
}

/// A place that looks like a jump table: words side by side, at least two of
/// them seen as the target of an indirect jump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub at: u16,
    /// Each word, and whether it has been jumped to.
    pub entries: Vec<(u16, bool)>,
}

/// How far outside the targets an entry not jumped to yet may point. Handlers
/// sit together, and Jetpac's reach $0300 either side of the ones it had run.
const TABLE_MARGIN: u16 = 0x0400;

/// Look for the words an indirect jump went to, side by side in memory. A table
/// holds handlers that have not been jumped to yet as well as those that have,
/// so it runs on either way for as long as the words point near the targets.
/// That is a guess at where it ends, and may take in a word or two that is not
/// part of it.
pub fn tables(
    peek: &impl Fn(u16) -> u8,
    targets: &BTreeSet<u16>,
    from: u16,
    to: u16,
) -> Vec<Table> {
    let (Some(low), Some(high)) = (targets.first(), targets.last()) else {
        return Vec::new();
    };
    let (low, high) = (
        low.saturating_sub(TABLE_MARGIN),
        high.saturating_add(TABLE_MARGIN),
    );
    let word = |a: u16| u16::from_le_bytes([peek(a), peek(a.wrapping_add(1))]);
    let plausible = |a: u16| (low..=high).contains(&word(a));
    let mut found = Vec::new();
    let mut at = from as u32;
    while at < to as u32 {
        let a = at as u16;
        // Two different hits side by side is what starts one: a lone word
        // matching is an accident, and so is a repeated one, which is what a
        // row of a sprite looks like.
        let (first, second) = (word(a), word(a.wrapping_add(2)));
        if first == second || !(targets.contains(&first) && targets.contains(&second)) {
            at += 1;
            continue;
        }
        let mut start = a;
        while start >= from.saturating_add(2) && plausible(start - 2) {
            start -= 2;
        }
        let mut end = a.wrapping_add(4);
        while (end as u32) < to as u32 && plausible(end) {
            end = end.wrapping_add(2);
        }
        let entries: Vec<(u16, bool)> = (start..end)
            .step_by(2)
            .map(|e| (word(e), targets.contains(&word(e))))
            .collect();
        found.push(Table { at: start, entries });
        at = end as u32;
    }
    found
}

/// A stretch that decodes as code, could be reached, and has not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unreached {
    pub low: u16,
    pub high: u16,
    /// What leads there: an instruction that has run, or a table's word.
    pub named_by: u16,
}

/// How a range of memory stands.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub bytes: u32,
    pub run: u32,
    pub reachable: u32,
    pub data: u32,
    pub untouched: u32,
    pub unreached: Vec<Unreached>,
    /// Conditionals that have only ever gone one way, and how.
    pub one_sided: Vec<(u16, Sides)>,
}

/// Where an instruction can lead, added to what is still to be looked at.
fn lead_on(pending: &mut Vec<(u16, u16)>, at: u16, length: u16, exit: Exit, by: u16) {
    let on = at.wrapping_add(length);
    match exit {
        Exit::Next | Exit::MaybeReturn => pending.push((on, by)),
        Exit::Jump(t) => pending.push((t, by)),
        Exit::Either(t) => {
            pending.push((t, by));
            pending.push((on, by));
        }
        Exit::Stop => {}
    }
}

/// Follow the code outward from what has run, without running it.
pub fn report(peek: &impl Fn(u16) -> u8, observer: &Observer, from: u16, to: u16) -> Report {
    let inside = |a: u16| a >= from && a <= to;
    let mut pending: Vec<(u16, u16)> = Vec::new();
    for (low, high) in observer.code_runs() {
        let mut at = low;
        while at <= high {
            let (length, exit) = exit_of(peek, at);
            lead_on(&mut pending, at, length, exit, at);
            match at.checked_add(length) {
                Some(next) => at = next,
                None => break,
            }
        }
    }
    let flow = &observer.flow;
    // All of RAM, whatever range was asked about: a table need not be anywhere
    // near the code it points to.
    for table in flow.tables(peek, 0x4000, 0xFFFF) {
        for (i, (word, _)) in table.entries.iter().enumerate() {
            pending.push((*word, table.at.wrapping_add(2 * i as u16)));
        }
    }

    // Who leads to each byte reached. Having been read does not stop it: a
    // loader moving the program reads every byte of its code.
    let mut reached: BTreeMap<u16, u16> = BTreeMap::new();
    while let Some((at, by)) = pending.pop() {
        if !inside(at) || reached.contains_key(&at) || observer.was_executed(at) {
            continue;
        }
        let (length, exit) = exit_of(peek, at);
        for i in 0..length {
            reached.entry(at.wrapping_add(i)).or_insert(by);
        }
        lead_on(&mut pending, at, length, exit, by);
    }

    let mut out = Report::default();
    for at in from as u32..=to as u32 {
        let a = at as u16;
        out.bytes += 1;
        if observer.was_executed(a) {
            out.run += 1;
        } else if reached.contains_key(&a) {
            out.reachable += 1;
        } else if observer.is_data(a) {
            out.data += 1;
        } else {
            out.untouched += 1;
        }
    }
    for (&at, &by) in &reached {
        if !inside(at) || observer.was_executed(at) {
            continue;
        }
        match out.unreached.last_mut() {
            Some(last) if last.high.wrapping_add(1) == at => last.high = at,
            _ => out.unreached.push(Unreached {
                low: at,
                high: at,
                named_by: by,
            }),
        }
    }
    out.one_sided = flow
        .conditionals
        .iter()
        .filter(|(at, sides)| inside(**at) && (sides.taken == 0 || sides.not_taken == 0))
        .map(|(at, sides)| (*at, *sides))
        .collect();
    out
}
