//! Writes that changed something, for the two questions a count of writes does
//! not answer: what is rewriting the program's own code, and who changed this
//! value and when.

use std::collections::{BTreeMap, VecDeque};

/// How many changes one watch keeps. A value changing every frame fills this in
/// about a minute of the machine's time, and the latest are what is wanted.
const KEPT: usize = 4096;

/// How many different values are remembered for a byte of code being rewritten.
/// A byte patched between two or three values is a switch; one that takes every
/// value is a variable that happens to live in an instruction.
const VALUES: usize = 8;

/// What has happened to one byte that has already run as code.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rewrite {
    pub times: u32,
    /// The instructions that did it, and how many times each.
    pub by: BTreeMap<u16, u32>,
    /// What it was set to, in the order first seen, up to `VALUES` of them.
    pub values: Vec<u8>,
}

/// One change to a watched byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub addr: u16,
    pub frame: u64,
    pub t: u32,
    /// The instruction that did it.
    pub pc: u16,
    pub old: u8,
    pub new: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValueWatch {
    pub low: u16,
    pub high: u16,
    pub log: VecDeque<Change>,
    /// How many older changes have been let go to stay within `KEPT`.
    pub dropped: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Changes {
    pub rewrites: BTreeMap<u16, Rewrite>,
    pub watches: Vec<ValueWatch>,
}

impl Changes {
    /// A byte has been written with a value it did not already hold.
    pub fn note(&mut self, change: Change, was_code: bool) {
        if was_code {
            let rewrite = self.rewrites.entry(change.addr).or_default();
            rewrite.times += 1;
            *rewrite.by.entry(change.pc).or_default() += 1;
            if rewrite.values.len() < VALUES && !rewrite.values.contains(&change.new) {
                rewrite.values.push(change.new);
            }
        }
        for watch in &mut self.watches {
            if change.addr >= watch.low && change.addr <= watch.high {
                if watch.log.len() == KEPT {
                    watch.log.pop_front();
                    watch.dropped += 1;
                }
                watch.log.push_back(change);
            }
        }
    }

    /// Runs of neighbouring bytes rewritten by the same instructions, since a
    /// patched address is two bytes and a loader moving a block is thousands.
    pub fn rewritten_runs(&self) -> Vec<(u16, u16)> {
        let mut runs: Vec<(u16, u16)> = Vec::new();
        for (addr, rewrite) in &self.rewrites {
            match runs.last_mut() {
                Some((_, high))
                    if *high as u32 + 1 == *addr as u32
                        && self
                            .rewrites
                            .get(high)
                            .is_some_and(|r| r.by.keys().eq(rewrite.by.keys())) =>
                {
                    *high = *addr
                }
                _ => runs.push((*addr, *addr)),
            }
        }
        runs
    }
}
