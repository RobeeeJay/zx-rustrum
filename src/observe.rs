//! What each routine actually did, measured while it ran.
//!
//! Reading the code can only say what a routine might do. This watches what it
//! does: which parts of memory it writes to and how much, which ports it
//! touches, how often it is called and by whom, what its registers held on the
//! way in, and how many times its loops went round. A routine that writes 6144
//! bytes into the display file once a frame is a screen blit whatever its
//! instructions look like.
//!
//! Everything here is attributed to the innermost routine on the call stack,
//! which is worked out from what the CPU did — see [`crate::flow`].

use std::collections::BTreeMap;

use crate::flow::{always_jumps, always_returns, classify, Flow};

/// Where a write landed. The Spectrum's memory map makes these worth counting
/// separately: they are the difference between drawing, colouring and thinking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Writes {
    /// The display file, $4000-$57FF.
    pub screen: u32,
    /// The attribute file, $5800-$5AFF.
    pub attrs: u32,
    /// Anywhere else in RAM.
    pub other: u32,
}

impl Writes {
    pub fn total(&self) -> u32 {
        self.screen + self.attrs + self.other
    }

    fn add(&mut self, addr: u16) {
        match addr {
            0x4000..=0x57FF => self.screen += 1,
            0x5800..=0x5AFF => self.attrs += 1,
            _ => self.other += 1,
        }
    }
}

/// The range a register was seen to hold on the way into a routine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    pub low: u16,
    pub high: u16,
}

/// The range of T-states within a frame a routine was seen to start at.
///
/// Its own type rather than [`Seen`], which holds registers and cannot hold a
/// T-state: a 48K frame is 69,888 of them and a `u16` stops at 65,535, so
/// anything entered in the last four and a half thousand — the bottom two
/// character rows and the border below them — was recorded as having happened
/// at 65,535. Every routine that ran down there looked as though it also ran
/// at the very end of the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub low: u32,
    pub high: u32,
    seen: bool,
}

impl Span {
    fn note(&mut self, at: u32) {
        if self.seen {
            self.low = self.low.min(at);
            self.high = self.high.max(at);
        } else {
            self.low = at;
            self.high = at;
            self.seen = true;
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.seen
    }
}

impl Seen {
    fn note(&mut self, value: u16) {
        self.low = self.low.min(value);
        self.high = self.high.max(value);
    }

    /// Whether it was the same every time, which makes it a constant rather
    /// than an argument.
    pub fn constant(&self) -> bool {
        self.low == self.high
    }
}

impl Default for Seen {
    fn default() -> Self {
        Seen {
            low: u16::MAX,
            high: 0,
        }
    }
}

/// What one routine was seen to do.
#[derive(Clone, Debug, Default)]
pub struct Observed {
    pub entry: u16,
    pub calls: u32,
    /// Instructions run inside it, its callees excluded.
    pub instructions: u64,
    pub writes: Writes,
    /// Bytes of memory it read, split the same way. What a routine reads says
    /// as much about it as what it writes: a routine that reads a screenful
    /// and writes a screenful is copying one, and one that reads a table and
    /// writes the display file is drawing from data.
    pub reads: Writes,
    /// The same, counting what the routines it calls read as well.
    pub inclusive_reads: Writes,
    /// The same, counting what the routines it calls wrote as well.
    ///
    /// A routine whose whole job is to call the drawing routine writes nothing
    /// itself, and looked in the measurements like a routine that thinks
    /// rather than draws — which is exactly the wrong thing to tell anybody
    /// about DRAWHG.
    pub inclusive: Writes,
    /// The lowest and highest address it wrote to.
    pub wrote_between: Option<(u16, u16)>,
    /// The one value it has written everywhere, while there is one. A routine
    /// that fills memory with a single byte is clearing it; one that writes
    /// different bytes is drawing something. Nothing else tells those apart:
    /// both write a lot into the display file, quickly, every frame.
    pub filled_with: Option<u8>,
    /// Whether it has written more than one distinct value, which is what
    /// makes `filled_with` empty rather than nothing having been written yet.
    pub mixed_values: bool,
    /// The lowest and highest address executed while it was the innermost
    /// routine: where the routine actually is, as against where it starts.
    /// What is between them is not necessarily all its own — a routine that
    /// jumps over a table of data reaches past the table — but it is where to
    /// start looking.
    pub spans: Option<(u16, u16)>,
    /// The addresses it returned or jumped away from, which is where a
    /// routine ends. A few of them: a routine with several exits has several,
    /// and one with dozens is not telling us anything more by the twentieth.
    pub exits: Vec<u16>,
    /// Where the byte after each of those instructions is — which is not one
    /// past the exit, since `RET` is one byte and `JP nn` is three. It is
    /// where the next routine begins when one falls straight after another,
    /// so it has to be the end of the instruction rather than the start.
    ///
    /// Only the exits that always end the routine are here. A taken `RET Z`
    /// ends the call it is in and nothing more: the next call through may fall
    /// straight past it, so it is not where the routine stops. Counting those
    /// drew every guarded routine as far as its first test.
    pub after: Vec<u16>,
    pub ports_in: Vec<u16>,
    pub ports_out: Vec<u16>,
    /// How many times, which is what tells a beeper routine hammering port
    /// $FE from a routine setting the border once.
    pub port_reads: u32,
    pub port_writes: u32,
    /// A few of the register sets it was actually handed, rather than only
    /// the range they spanned. What a routine is called *with* is half of
    /// what it is for, and a range does not show a caller passing $5E00 one
    /// time and $5F00 the next.
    pub examples: Vec<Registers>,
    /// What its registers held on the way in.
    pub entry_af: Seen,
    pub entry_bc: Seen,
    pub entry_de: Seen,
    pub entry_hl: Seen,
    /// How many times each loop in it went round, by the address jumped back
    /// to, totalled over every call.
    ///
    /// A total rather than the longest unbroken run: with one loop inside
    /// another, the inner one interrupts the outer one's run every time it
    /// goes round, and the outer loop reads as one iteration. Divided by the
    /// number of calls, this gives what a reader wants — how many times round
    /// per call.
    pub loops: BTreeMap<u16, u32>,
    /// Deepest it was seen nested, which spots recursion.
    pub max_depth: u32,
    /// When in the frame it was entered, in T-states: what it is doing
    /// relative to the beam. A routine that runs while the picture is being
    /// painted is timed against it; one that runs in the border above the
    /// picture is getting ready for the frame.
    pub entered_at: Span,
    /// The handful of addresses it writes to, when there are few enough to be
    /// worth naming. A routine that always writes the same three bytes is
    /// keeping something.
    pub hot: Vec<u16>,
    /// Frames in which it ran at least once.
    pub frames: u32,
    /// The last frame it was seen in, so `frames` counts frames not calls.
    last_frame: u64,
}

impl Observed {
    /// The most times round its longest loop, per call. On this machine the
    /// number is diagnostic: 192 is the pixel rows of the screen, 24 or 32 the
    /// characters across or down, 8 the rows of one character.
    pub fn longest_loop(&self) -> u32 {
        self.loops.values().copied().max().unwrap_or(0) / self.calls.max(1)
    }

    /// How many times a particular loop went round per call.
    pub fn loop_trips(&self) -> BTreeMap<u16, u32> {
        self.loops
            .iter()
            .map(|(at, total)| (*at, total / self.calls.max(1)))
            .collect()
    }

    /// Whether it runs about once per frame, which is what the work of a game
    /// looks like as against its setting up.
    pub fn every_frame(&self, frames_seen: u32) -> bool {
        frames_seen > 4 && self.frames * 4 >= frames_seen * 3
    }
}

/// One entry in the call graph.
#[derive(Clone, Copy, Debug, Default)]
pub struct Edge {
    pub calls: u32,
}

#[derive(Clone, Copy, Debug)]
struct Frame {
    entry: u16,
    sp: u16,
    /// Instructions run in this frame, its callees excluded.
    instructions: u64,
    /// The lowest and highest address executed while this was the innermost
    /// routine: how far the routine reaches. Kept on the frame rather than
    /// looked up per instruction, which would cost a map lookup on every one.
    low: u16,
    high: u16,
}

/// The observer itself.
#[derive(Clone, Debug, Default)]
pub struct Observer {
    pub enabled: bool,
    stack: Vec<Frame>,
    pub routines: BTreeMap<u16, Observed>,
    /// Who called whom, and how often.
    pub edges: BTreeMap<(u16, u16), Edge>,
    /// Frames watched, so "runs every frame" means something.
    pub frames: u64,
    /// Where in the frame the machine is, in T-states, as last told.
    frame_t: u32,
    /// The last few frames' worth of entering and leaving, oldest first. A
    /// ring rather than a log: a game makes a few hundred calls a frame and
    /// nobody wants the whole of a twenty-minute recording.
    steps: std::collections::VecDeque<Step>,
    /// How deep to follow before giving up on a runaway stack.
    max_depth: usize,
    /// Addresses that have been executed: what is code, as against what is
    /// only read. One bit each, so the whole address space costs 8K.
    executed: Vec<u64>,
    /// Addresses that have been read but not executed: candidate data.
    read: Vec<u64>,
    /// Which routine read each 256-byte page, and how often. Per page rather
    /// than per byte: a table of 65,536 counters would cost more than it says.
    readers: BTreeMap<(u8, u16), u32>,
    /// Where each IN and OUT is, by the address of the instruction doing it.
    /// A routine can hold several, and one of them can be the interesting one:
    /// per routine, three separate reads of the keyboard in one loop are a
    /// single line saying "this routine reads ports".
    pub port_sites: BTreeMap<u16, Site>,
    /// What each conditional and indirect jump was seen to do.
    pub flow: crate::coverage::Flow,
    /// Writes that changed something: to code that had run, and to whatever
    /// has been asked to be watched.
    pub changes: crate::changes::Changes,
    /// What has run only because a branch was forced by force_branch, which
    /// the machine may never do by itself.
    pub forced: Vec<(u16, u16)>,
    /// The address of the instruction being executed, so a port access can be
    /// attributed to the instruction making it. Set by the machine before each
    /// instruction; a port is read part-way through one, which is before
    /// anything else here is told about it.
    pub executing: u16,
    /// Which routine last wrote each byte of the display and attribute files.
    ///
    /// Per byte, unlike the readers, because this is the one place where being
    /// able to point at a thing on screen and say what put it there is worth
    /// 14K of memory. It is the difference between inferring that a routine
    /// draws and watching it draw a particular sprite.
    drew: Vec<u16>,
    /// Whether anything has been written there at all, since routine $0000 is
    /// a real address.
    drew_any: Vec<u64>,
}

impl Observer {
    pub fn new() -> Observer {
        Observer {
            max_depth: 64,
            executed: vec![0; 1024],
            read: vec![0; 1024],
            drew: vec![0; SCREEN_BYTES],
            drew_any: vec![0; SCREEN_BYTES.div_ceil(64)],
            ..Default::default()
        }
    }

    pub fn clear(&mut self) {
        let enabled = self.enabled;
        *self = Observer::new();
        self.enabled = enabled;
    }

    /// The routine the machine is in at the moment, if any.
    pub fn innermost(&self) -> Option<u16> {
        self.stack.last().map(|f| f.entry)
    }

    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Everything recorded, oldest first.
    pub fn steps(&self) -> impl Iterator<Item = &Step> {
        self.steps.iter()
    }

    /// Everything that happened in one frame, in order.
    pub fn frame_steps(&self, frame: u32) -> Vec<Step> {
        self.steps
            .iter()
            .copied()
            .filter(|s| s.frame == frame)
            .collect()
    }

    /// The most recent frame that was recorded from beginning to end. The one
    /// in progress is half a frame and would read as a program that stops
    /// half way through its work.
    pub fn last_whole_frame(&self) -> Option<u32> {
        // The newest frame is the one in progress, and the frame before it may
        // have been pushed out of the ring: what is wanted is the newest frame
        // that still has all of itself in here, which is the newest one that
        // is neither the first nor the last present.
        let newest = self.steps.back()?.frame;
        let oldest = self.steps.front()?.frame;
        self.steps
            .iter()
            .rev()
            .map(|step| step.frame)
            .find(|frame| *frame < newest && *frame > oldest)
    }

    fn remember(&mut self, entry: u16, depth: u8, enter: bool) {
        if self.steps.len() >= MAX_STEPS {
            self.steps.pop_front();
        }
        self.steps.push_back(Step {
            frame: self.frames as u32,
            t: self.frame_t,
            entry,
            depth,
            enter,
        });
    }

    /// Note the end of a frame, so what runs every frame can be told from what
    /// ran once.
    pub fn end_frame(&mut self) {
        if !self.enabled {
            return;
        }
        self.frames += 1;
        // What the routines still on the stack have reached so far. A main
        // loop never returns, so waiting for it to would leave the one routine
        // whose extent matters most with none at all — the same trap the
        // inclusive write counts fell into.
        let reached: Vec<(u16, u16, u16)> = self
            .stack
            .iter()
            .map(|frame| (frame.entry, frame.low, frame.high))
            .collect();
        for (entry, low, high) in reached {
            let stats = self.stats(entry);
            stats.spans = Some(match stats.spans {
                Some((was_low, was_high)) => (was_low.min(low), was_high.max(high)),
                None => (low, high),
            });
        }
    }

    pub fn was_executed(&self, addr: u16) -> bool {
        bit(&self.executed, addr)
    }

    /// Addresses read but never executed: data, as far as anything can tell.
    pub fn is_data(&self, addr: u16) -> bool {
        bit(&self.read, addr) && !bit(&self.executed, addr)
    }

    /// Runs of addresses that were executed, in address order.
    ///
    /// Every byte of an instruction counts, operands included: what the CPU
    /// read as part of an instruction is code however it was reached, and a
    /// routine nobody was seen to call is still code.
    pub fn code_runs(&self) -> Vec<(u16, u16)> {
        let mut runs = Vec::new();
        let mut start: Option<u16> = None;
        for addr in 0..=u16::MAX {
            match (self.was_executed(addr), start) {
                (true, None) => start = Some(addr),
                (false, Some(from)) => {
                    runs.push((from, addr - 1));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(from) = start {
            runs.push((from, u16::MAX));
        }
        runs
    }

    /// Runs of data, longest first, for whatever wants to look at them.
    pub fn data_blocks(&self, min_length: u16) -> Vec<(u16, u16)> {
        let mut blocks = Vec::new();
        let mut start: Option<u16> = None;
        for addr in 0..=u16::MAX {
            if self.is_data(addr) {
                start.get_or_insert(addr);
            } else if let Some(from) = start.take() {
                if addr - from >= min_length {
                    blocks.push((from, addr - from));
                }
            }
        }
        if let Some(from) = start {
            let length = u16::MAX - from;
            if length >= min_length {
                blocks.push((from, length));
            }
        }
        blocks.sort_by_key(|(_, length)| std::cmp::Reverse(*length));
        blocks
    }

    /// What the routine at the top of the stack should be credited with.
    fn current(&mut self) -> Option<u16> {
        self.stack.last().map(|f| f.entry)
    }

    fn stats(&mut self, entry: u16) -> &mut Observed {
        self.routines.entry(entry).or_insert(Observed {
            entry,
            ..Default::default()
        })
    }

    /// An opcode was fetched here: that address is code.
    pub fn on_fetch(&mut self, addr: u16) {
        if self.enabled {
            set(&mut self.executed, addr);
        }
    }

    /// A byte was read from here without being executed: candidate data.
    pub fn on_read(&mut self, addr: u16) {
        if !self.enabled {
            return;
        }
        set(&mut self.read, addr);
        let Some(entry) = self.stack.last().map(|f| f.entry) else {
            return;
        };
        let page = (addr >> 8) as u8;
        if self.readers.len() < 4096 {
            *self.readers.entry((page, entry)).or_insert(0) += 1;
        }
        // Counted the same way as the writes, and for the same reason: a
        // routine that hands the reading to something else has still caused it.
        let ancestors: Vec<u16> = self.stack.iter().map(|frame| frame.entry).collect();
        for ancestor in ancestors {
            self.stats(ancestor).inclusive_reads.add(addr);
        }
        self.stats(entry).reads.add(addr);
    }

    /// Which routine last wrote this byte of the screen, if anything has.
    ///
    /// This is the whole of the evidence for what drew something: the bus
    /// watched it happen, so pointing at a sprite and being told which routine
    /// put it there is not a guess at all.
    pub fn drew(&self, addr: u16) -> Option<u16> {
        let offset = screen_offset(addr)?;
        let written = self.drew_any[offset >> 6] & (1 << (offset & 63)) != 0;
        written.then(|| self.drew[offset])
    }

    /// Everything that drew any part of a character cell, commonest first: the
    /// eight pixel rows of a cell are rarely all one routine's work.
    pub fn drew_cell(&self, column: usize, row: usize) -> Vec<(u16, u32)> {
        let mut who: BTreeMap<u16, u32> = BTreeMap::new();
        for line in 0..8 {
            // The display file's thirds and rows, which is why this is not
            // simply row * 32.
            let third = row / 8;
            let y = (row % 8) * 8 + line;
            let addr = 0x4000 + (third << 11) + (y << 5) + column;
            if let Some(entry) = self.drew(addr as u16) {
                *who.entry(entry).or_insert(0) += 1;
            }
        }
        if let Some(entry) = self.drew(0x5800 + (row * 32 + column) as u16) {
            *who.entry(entry).or_insert(0) += 1;
        }
        let mut who: Vec<(u16, u32)> = who.into_iter().collect();
        who.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        who
    }

    /// Who read a page, most often first.
    pub fn readers_of(&self, page: u8) -> Vec<u16> {
        let mut who: Vec<(u16, u32)> = self
            .readers
            .iter()
            .filter(|((p, _), _)| *p == page)
            .map(|((_, entry), count)| (*entry, *count))
            .collect();
        who.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        who.into_iter().map(|(entry, _)| entry).collect()
    }

    /// Which pages a routine read, and how often. The other way round from
    /// [`Self::readers_of`], for asking what one routine looked at rather than
    /// who looked at one page.
    pub fn pages_read_by(&self, entry: u16) -> Vec<(u8, u32)> {
        let mut pages: Vec<(u8, u32)> = self
            .readers
            .iter()
            .filter(|((_, who), _)| *who == entry)
            .map(|((page, _), count)| (*page, *count))
            .collect();
        pages.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        pages
    }

    /// The blocks of data found, longest first, each with a guess at what it
    /// is taken from what the routines that read it went on to do.
    pub fn blocks(&self, min_length: u16) -> Vec<Block> {
        self.data_blocks(min_length)
            .into_iter()
            .map(|(at, length)| {
                let readers = self.readers_of((at >> 8) as u8);
                let kind = self.classify(&readers, length);
                Block {
                    at,
                    length,
                    kind,
                    readers,
                }
            })
            .collect()
    }

    fn classify(&self, readers: &[u16], length: u16) -> DataKind {
        let reader_stats: Vec<&Observed> = readers
            .iter()
            .filter_map(|e| self.routines.get(e))
            .collect();
        if reader_stats.iter().any(|r| r.writes.screen > 32) {
            return DataKind::Graphics;
        }
        if reader_stats.iter().any(|r| r.writes.attrs > 32) || length == 768 {
            return DataKind::Colours;
        }
        if reader_stats
            .iter()
            .any(|r| r.writes.other > 256 && r.longest_loop() > 8)
        {
            return DataKind::Packed;
        }
        DataKind::Unknown
    }

    /// A byte was written here by whatever routine is running.
    pub fn on_write(&mut self, addr: u16, value: u8) {
        if !self.enabled {
            return;
        }
        let Some(entry) = self.current() else { return };
        if let Some(offset) = screen_offset(addr) {
            self.drew[offset] = entry;
            self.drew_any[offset >> 6] |= 1 << (offset & 63);
        }
        // Credited to the routine itself, and to everything that called it:
        // what a routine causes to happen is as much a fact about it as what
        // it does with its own instructions.
        //
        // Counted here rather than totted up when a routine returns, because a
        // main loop does not return: the one routine whose inclusive figure
        // matters most would have been the one left at zero.
        let ancestors: Vec<u16> = self.stack.iter().map(|frame| frame.entry).collect();
        for ancestor in ancestors {
            self.stats(ancestor).inclusive.add(addr);
        }
        let stats = self.stats(entry);
        stats.writes.add(addr);
        match stats.filled_with {
            Some(seen) if seen != value => {
                stats.filled_with = None;
                stats.mixed_values = true;
            }
            None if !stats.mixed_values => stats.filled_with = Some(value),
            _ => {}
        }
        // Only worth keeping while there are few of them: a routine writing
        // half the screen is not keeping a variable, and the list is dropped
        // once it stops being a short one.
        if stats.hot.len() < 8 && !stats.hot.contains(&addr) {
            stats.hot.push(addr);
        }
        stats.wrote_between = Some(match stats.wrote_between {
            Some((low, high)) => (low.min(addr), high.max(addr)),
            None => (addr, addr),
        });
    }

    /// A write is about to replace `old` with `value`.
    pub fn on_change(&mut self, addr: u16, old: u8, value: u8) {
        if old == value {
            return;
        }
        let change = crate::changes::Change {
            addr,
            frame: self.frames,
            t: self.frame_t,
            pc: self.executing,
            old,
            new: value,
        };
        let was_code = bit(&self.executed, addr);
        self.changes.note(change, was_code);
    }

    /// What ran on a machine forked from this one, taken as run and written
    /// down as having needed forcing. Everything else the fork measured is
    /// left behind, so it does not get counted as what this program does.
    pub fn absorb_forced(&mut self, fork: &Observer) -> Vec<(u16, u16)> {
        let mut new: Vec<(u16, u16)> = Vec::new();
        for addr in 0..=0xFFFFu16 {
            if bit(&fork.executed, addr) && !bit(&self.executed, addr) {
                set(&mut self.executed, addr);
                match new.last_mut() {
                    Some((_, high)) if *high as u32 + 1 == addr as u32 => *high = addr,
                    _ => new.push((addr, addr)),
                }
            }
        }
        self.forced.extend(new.iter().copied());
        new
    }

    /// Whether this byte has run only under a forced branch.
    pub fn was_forced(&self, addr: u16) -> bool {
        self.forced
            .iter()
            .any(|(low, high)| addr >= *low && addr <= *high)
    }

    /// A port was read or written by whatever routine is running.
    pub fn on_port(&mut self, port: u16, write: bool) {
        if !self.enabled {
            return;
        }

        // Recorded against the instruction first, and against the routine
        // second. A program reads the keyboard from wherever it likes,
        // including from code nothing was ever seen to call — in which case
        // there is no routine to credit, and dropping the access on the floor
        // is how a game's own key handling went unnoticed.
        let at = self.executing;
        let routine = self.stack.last().map(|frame| frame.entry);
        let site = self.port_sites.entry(at).or_insert(Site {
            at,
            routine,
            ports: Vec::new(),
            reads: 0,
            writes: 0,
            frames: 0,
            last_frame: u64::MAX,
        });
        if write {
            site.writes += 1;
        } else {
            site.reads += 1;
        }
        if site.ports.len() < 8 && !site.ports.contains(&port) {
            site.ports.push(port);
        }
        if site.last_frame != self.frames {
            site.frames += 1;
            site.last_frame = self.frames;
        }

        let Some(entry) = self.current() else { return };
        let stats = self.stats(entry);
        if write {
            stats.port_writes += 1;
        } else {
            stats.port_reads += 1;
        }
        let ports = if write {
            &mut stats.ports_out
        } else {
            &mut stats.ports_in
        };
        // A handful of ports each; a set would cost more than it saved.
        if ports.len() < 8 && !ports.contains(&port) {
            ports.push(port);
        }
    }

    /// Offer one executed instruction.
    #[allow(clippy::too_many_arguments)]
    pub fn on_instruction(
        &mut self,
        pc_before: u16,
        sp_before: u16,
        pc_after: u16,
        sp_after: u16,
        registers: Registers,
        // The first two bytes of the instruction, for the one thing the stack
        // cannot answer: whether a jump was conditional.
        opcode: [u8; 2],
        peek: impl Fn(u16) -> u16,
    ) {
        if !self.enabled {
            return;
        }
        if let Some(frame) = self.stack.last_mut() {
            frame.instructions += 1;
            frame.low = frame.low.min(pc_before);
            frame.high = frame.high.max(pc_before);
        }

        // A jump backwards is a loop going round again. Only another
        // back-jump interrupts the count: the instructions of the loop body
        // are forward motion and would otherwise reset it every time round.
        if pc_after < pc_before && pc_before.wrapping_sub(pc_after) < 256 {
            if let Some(entry) = self.current() {
                let stats = self.stats(entry);
                if stats.loops.len() < 32 {
                    *stats.loops.entry(pc_after).or_insert(0) += 1;
                }
            }
        }

        // A jump that always jumps ends the routine it is in and starts
        // another where it lands — the tail call a Z80 program writes instead
        // of CALL followed by RET. A conditional jump is an early way out and
        // leaves the routine where it is.
        if always_jumps(opcode) {
            self.tail_jump(pc_before, pc_after, registers, opcode);
        }

        match classify(pc_before, sp_before, pc_after, sp_after, peek) {
            Flow::Call { entry, sp } => self.enter(entry, sp, registers),
            Flow::Return { sp_before, .. } => {
                // Where a routine ends is where it returned from, which is a
                // fact about the run rather than something to be worked out by
                // decoding forwards and hoping to meet a RET.
                if let Some(entry) = self.current() {
                    // RET is one byte; RETI and RETN carry the ED prefix.
                    let length = if opcode[0] == 0xED { 2 } else { 1 };
                    let ends_it = always_returns(opcode);
                    let stats = self.stats(entry);
                    note_exit(stats, pc_before, pc_before.wrapping_add(length), ends_it);
                }
                self.leave(sp_before)
            }
            Flow::Straight => {}
        }
    }

    /// An unconditional jump out of the routine it was in.
    ///
    /// Where it lands is the entry of another routine, at the same depth and
    /// with the same stack: whatever eventually returns goes back to whoever
    /// called the first one, which is what a tail call means. A jump back into
    /// the routine is a loop going round, not an ending, so only a jump
    /// outside what the routine has run so far counts.
    fn tail_jump(&mut self, from: u16, to: u16, registers: Registers, opcode: [u8; 2]) {
        let Some(frame) = self.stack.last().copied() else {
            return;
        };
        if to >= frame.low && to <= frame.high {
            return;
        }

        self.stack.pop();
        let depth = self.stack.len() as u8 + 1;
        self.remember(frame.entry, depth, false);
        let stats = self.stats(frame.entry);
        stats.instructions += frame.instructions;
        stats.spans = Some(match stats.spans {
            Some((low, high)) => (low.min(frame.low), high.max(frame.high)),
            None => (frame.low, frame.high),
        });
        // JP nn is three bytes, JP (IX) two, JP (HL) one.
        let length = match opcode[0] {
            0xC3 => 3,
            0xDD | 0xFD => 2,
            _ => 1,
        };
        note_exit(stats, from, from.wrapping_add(length), true);

        // Entered like anything else, so it is counted, timed and joined to
        // whoever called the routine it jumped out of.
        self.enter(to, frame.sp, registers);
    }

    /// Entered directly, which is what an interrupt does.
    pub fn on_interrupt(&mut self, entry: u16, sp: u16, registers: Registers) {
        if self.enabled {
            self.enter(entry, sp, registers);
        }
    }

    /// Where the beam is, so a routine can be timed against the picture.
    pub fn set_frame_t(&mut self, t: u32) {
        self.frame_t = t;
    }

    /// Everything that writes to an address, and everything that reads its
    /// page: what a variable is used by.
    pub fn users_of(&self, addr: u16) -> Vec<u16> {
        let mut who: Vec<u16> = self
            .routines
            .values()
            .filter(|r| r.hot.contains(&addr))
            .map(|r| r.entry)
            .collect();
        who.sort_unstable();
        who
    }

    fn enter(&mut self, entry: u16, sp: u16, registers: Registers) {
        if self.stack.len() >= self.max_depth {
            return;
        }
        let caller = self.current();
        let depth = self.stack.len() as u32 + 1;
        let frame = self.frames;
        let frame_t_now = self.frame_t;
        {
            let stats = self.stats(entry);
            stats.calls += 1;
            stats.max_depth = stats.max_depth.max(depth);
            stats.entry_af.note(registers.af);
            stats.entry_bc.note(registers.bc);
            stats.entry_de.note(registers.de);
            stats.entry_hl.note(registers.hl);
            if stats.examples.len() < 4
                && !stats.examples.iter().any(|seen| seen.hl == registers.hl)
            {
                stats.examples.push(registers);
            }
            stats.entered_at.note(frame_t_now);
            if stats.frames == 0 || stats.last_frame != frame {
                stats.frames += 1;
                stats.last_frame = frame;
            }
        }
        if let Some(caller) = caller {
            self.edges.entry((caller, entry)).or_default().calls += 1;
        }
        self.remember(entry, depth as u8, true);
        self.stack.push(Frame {
            entry,
            sp,
            instructions: 0,
            low: entry,
            high: entry,
        });
    }

    fn leave(&mut self, sp_before: u16) {
        while let Some(frame) = self.stack.last().copied() {
            if frame.sp > sp_before {
                break;
            }
            self.stack.pop();
            let depth = self.stack.len() as u8 + 1;
            self.remember(frame.entry, depth, false);
            let stats = self.stats(frame.entry);
            stats.instructions += frame.instructions;
            stats.spans = Some(match stats.spans {
                Some((low, high)) => (low.min(frame.low), high.max(frame.high)),
                None => (frame.low, frame.high),
            });
            if frame.sp == sp_before {
                break;
            }
        }
    }
}

/// What a block of data appears to be, judged by who read it and what they
/// did with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataKind {
    /// Read by something that then wrote to the display file.
    Graphics,
    /// The size of an attribute map, or read by something colouring.
    Colours,
    /// Read by something that shifts bits about: packed.
    Packed,
    /// Read, but nothing about how gives it away.
    Unknown,
}

impl DataKind {
    pub fn label(&self) -> &'static str {
        match self {
            DataKind::Graphics => "graphics",
            DataKind::Colours => "colours",
            DataKind::Packed => "packed",
            DataKind::Unknown => "data",
        }
    }
}

/// Note where a routine ended and where the byte after that instruction is.
///
/// `ends_it` says whether the instruction always ends the routine — a plain
/// `RET`, or a jump that always jumps — as against a conditional one, which
/// ended *this* call and leaves the routine carrying on underneath. Both are
/// exits and both are worth knowing; only the first is a boundary, or a
/// routine would be drawn as far as its first test and no further.
fn note_exit(stats: &mut Observed, at: u16, after: u16, ends_it: bool) {
    if stats.exits.len() >= 8 || stats.exits.contains(&at) {
        return;
    }
    stats.exits.push(at);
    if ends_it {
        stats.after.push(after);
    }
}

/// One IN or OUT instruction, and what it has been seen doing.
///
/// Kept per instruction rather than per routine because that is the grain a
/// reader works at: stopping on every IN shows a handful of instructions, and
/// a detector that answers with the routine they are all in is answering a
/// coarser question than the one asked.
#[derive(Clone, Debug, Default)]
pub struct Site {
    pub at: u16,
    /// The routine it is in, when anything was seen to call one. Code the
    /// machine was already running when watching started has none.
    pub routine: Option<u16>,
    /// The ports it has touched, up to a handful.
    pub ports: Vec<u16>,
    pub reads: u32,
    pub writes: u32,
    /// Frames in which it ran at all, so "every frame" means something.
    pub frames: u32,
    last_frame: u64,
}

/// A run of bytes that was read but never executed.
#[derive(Clone, Debug)]
pub struct Block {
    pub at: u16,
    pub length: u16,
    pub kind: DataKind,
    /// The routines that read it, most first.
    pub readers: Vec<u16>,
}

/// One routine being entered or left, and when in the frame.
///
/// Totals say what a routine does; only a sequence says in what order, which
/// is the question somebody reading a game's main loop is actually asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// Which frame it happened in, counted from when watching started.
    pub frame: u32,
    /// Where in the frame, in T-states: the x-axis of everything drawn from
    /// this, and on this machine an absolute position rather than a relative
    /// one, because the beam is somewhere definite at that moment.
    pub t: u32,
    pub entry: u16,
    pub depth: u8,
    /// Going in, as against coming back out.
    pub enter: bool,
}

/// The registers on the way into a routine: what it was handed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Registers {
    pub af: u16,
    pub bc: u16,
    pub de: u16,
    pub hl: u16,
}

/// How many enterings and leavings to keep. Finding the loops in a program
/// means watching it for minutes, not seconds: a game making a few hundred
/// calls a frame fills this in about ten minutes, and it costs 32MB.
const MAX_STEPS: usize = 4_000_000;

/// The display and attribute files, as one run of bytes.
const SCREEN_BYTES: usize = 0x1B00;

/// Where an address sits in that run, if it is in it at all.
fn screen_offset(addr: u16) -> Option<usize> {
    (0x4000..0x5B00)
        .contains(&addr)
        .then(|| addr as usize - 0x4000)
}

fn bit(bits: &[u64], addr: u16) -> bool {
    bits[addr as usize >> 6] & (1 << (addr & 63)) != 0
}

fn set(bits: &mut [u64], addr: u16) {
    bits[addr as usize >> 6] |= 1 << (addr & 63);
}
