//! Per-address access tracking: the data behind the RAM heat map and the
//! back-buffer detector.

pub const PAGE_SIZE: usize = 256;
pub const PAGES: usize = 256;

/// Heat is recorded per physical location rather than per address, so a RAM
/// bank keeps its history while it is paged out. The physical space is the
/// eight 16K RAM banks followed by up to four 16K ROM pages.
pub const PHYS_RAM_BANKS: usize = 8;
pub const PHYS_ROM_PAGES: usize = 4;
pub const BANK_SIZE: usize = 0x4000;
pub const PHYS_ROM_BASE: usize = PHYS_RAM_BANKS * BANK_SIZE;
pub const PHYS_LEN: usize = (PHYS_RAM_BANKS + PHYS_ROM_PAGES) * BANK_SIZE;

/// Physical index of a byte in RAM bank `bank`.
pub fn ram_phys(bank: usize, offset: u16) -> usize {
    (bank & 7) * BANK_SIZE + (offset as usize & 0x3fff)
}

/// Physical index of a byte in ROM page `page`.
pub fn rom_phys(page: usize, offset: u16) -> usize {
    PHYS_ROM_BASE + (page & 3) * BANK_SIZE + (offset as usize & 0x3fff)
}

/// Main video RAM of a 48K Spectrum: 6144 bytes of bitmap + 768 of attributes.
pub const SCREEN_START: u16 = 0x4000;
pub const SCREEN_END: u16 = 0x5b00;
pub const SCREEN_LEN: u16 = SCREEN_END - SCREEN_START;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Region {
    pub start: u16,
    pub len: u16,
}

impl Region {
    pub fn contains(&self, addr: u16) -> bool {
        let end = self.start as u32 + self.len as u32;
        (addr as u32) >= self.start as u32 && (addr as u32) < end
    }
    pub fn screen() -> Region {
        Region {
            start: SCREEN_START,
            len: SCREEN_LEN,
        }
    }
}

#[derive(Clone)]
pub struct Tracker {
    /// 0..=255 intensity, refreshed to 255 on access and faded every frame,
    /// indexed by physical location (see [`ram_phys`] and [`rom_phys`]).
    pub read_heat: Box<[u8; PHYS_LEN]>,
    pub write_heat: Box<[u8; PHYS_LEN]>,
    pub exec_heat: Box<[u8; PHYS_LEN]>,
    /// Where an opcode has ever been fetched since the last reset: code that
    /// has run, as against bytes that have only been read or written. Kept
    /// rather than faded like the heat, for the debugger's Disassemble mode.
    pub executed: Box<[bool; PHYS_LEN]>,

    /// Total accesses since reset, for the "coldest/hottest" statistics.
    pub read_count: Box<[u32; PHYS_LEN]>,
    pub write_count: Box<[u32; PHYS_LEN]>,

    pub fade_read: u8,
    pub fade_write: u8,
    pub fade_exec: u8,

    /// Writes per 256-byte page during the current detector window.
    win_writes: [u32; PAGES],
    /// Evidence that a page is the source of a block copy into video RAM.
    copy_score: [u32; PAGES],
    /// Address of the most recent read, used to spot `LDIR`-style copies.
    last_read: u16,
    window_frames: u32,

    /// Best guess at a back buffer, or None while there is no evidence.
    pub detected: Option<Region>,
    pub detected_confidence: f32,
    /// User override; when set the detector's result is ignored.
    pub manual: Option<Region>,
    pub detect_enabled: bool,
}

impl Default for Tracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Tracker {
    pub fn new() -> Self {
        Tracker {
            read_heat: Box::new([0; PHYS_LEN]),
            write_heat: Box::new([0; PHYS_LEN]),
            exec_heat: Box::new([0; PHYS_LEN]),
            executed: Box::new([false; PHYS_LEN]),
            read_count: Box::new([0; PHYS_LEN]),
            write_count: Box::new([0; PHYS_LEN]),
            fade_read: 12,
            fade_write: 8,
            fade_exec: 20,
            win_writes: [0; PAGES],
            copy_score: [0; PAGES],
            last_read: 0,
            window_frames: 0,
            detected: None,
            detected_confidence: 0.0,
            manual: None,
            detect_enabled: true,
        }
    }

    pub fn reset(&mut self) {
        self.read_heat.fill(0);
        self.write_heat.fill(0);
        self.exec_heat.fill(0);
        self.executed.fill(false);
        self.read_count.fill(0);
        self.write_count.fill(0);
        self.win_writes = [0; PAGES];
        self.copy_score = [0; PAGES];
        self.detected = None;
        self.detected_confidence = 0.0;
    }

    #[inline]
    pub fn on_read(&mut self, phys: usize, addr: u16) {
        self.read_heat[phys] = 255;
        self.read_count[phys] = self.read_count[phys].saturating_add(1);
        self.last_read = addr;
    }

    #[inline]
    pub fn on_exec(&mut self, phys: usize, _addr: u16) {
        self.exec_heat[phys] = 255;
        self.executed[phys] = true;
        self.read_count[phys] = self.read_count[phys].saturating_add(1);
    }

    #[inline]
    pub fn on_write(&mut self, phys: usize, addr: u16) {
        self.write_heat[phys] = 255;
        self.write_count[phys] = self.write_count[phys].saturating_add(1);
        self.win_writes[(addr as usize) >> 8] += 1;

        if self.detect_enabled && (SCREEN_START..SCREEN_END).contains(&addr) {
            // A write into video RAM whose data came from somewhere else is
            // the signature of a buffer flip (LDIR, stack blit, unrolled LD).
            let src = self.last_read;
            if !(SCREEN_START..SCREEN_END).contains(&src) && src >= 0x4000 {
                self.copy_score[(src as usize) >> 8] += 1;
            }
        }
    }

    /// Fade every heat map one step. Called once per rendered frame.
    /// Whether any heat is left to fade, which is what a paused window keeps
    /// repainting for.
    pub fn fade(&mut self) -> bool {
        let read = fade_buf(&mut self.read_heat, self.fade_read);
        let write = fade_buf(&mut self.write_heat, self.fade_write);
        let exec = fade_buf(&mut self.exec_heat, self.fade_exec);
        read || write || exec
    }

    /// Re-evaluate the back-buffer guess. Called once per rendered frame.
    pub fn tick_detector(&mut self) {
        if !self.detect_enabled {
            return;
        }
        self.window_frames += 1;
        if self.window_frames < 25 {
            return;
        }
        self.window_frames = 0;

        let result = self.best_candidate();
        match result {
            Some((region, conf)) => {
                // Only replace an existing guess with a clearly better one.
                if conf > self.detected_confidence * 0.75 {
                    self.detected = Some(region);
                    self.detected_confidence = conf;
                }
            }
            None => {
                self.detected_confidence *= 0.5;
                if self.detected_confidence < 0.05 {
                    self.detected = None;
                    self.detected_confidence = 0.0;
                }
            }
        }

        for i in 0..PAGES {
            self.win_writes[i] /= 2;
            self.copy_score[i] /= 2;
        }
    }

    /// Look for a run of at least 24 pages (6144 bytes) outside video RAM that
    /// is written heavily, preferring runs that also feed copies into the
    /// screen.
    fn best_candidate(&self) -> Option<(Region, f32)> {
        const MIN_PAGES: usize = 24; // one bitmap's worth
        const SPAN_PAGES: usize = 27; // bitmap + attributes, rounded up

        let outside = |page: usize| {
            let addr = (page << 8) as u16;
            addr >= 0x4000 && !(SCREEN_START..SCREEN_END).contains(&addr)
        };

        let mut best: Option<(Region, f32)> = None;
        let mut page = 0usize;
        while page + MIN_PAGES <= PAGES {
            if !outside(page) || self.win_writes[page] == 0 {
                page += 1;
                continue;
            }
            // Grow a run of consecutively written pages.
            let mut end = page;
            while end < PAGES && outside(end) && self.win_writes[end] > 0 {
                end += 1;
            }
            let run = end - page;
            if run >= MIN_PAGES {
                let span = run.min(SPAN_PAGES);
                let writes: u32 = self.win_writes[page..page + span].iter().sum();
                let copies: u32 = self.copy_score[page..page + span].iter().sum();
                // A copy into the screen is worth far more than a plain write.
                let score = writes as f32 + copies as f32 * 32.0;
                let conf = (score / 20_000.0).min(1.0);
                let region = Region {
                    start: (page << 8) as u16,
                    len: (span * PAGE_SIZE) as u16,
                };
                if best.is_none_or(|(_, c)| conf > c) {
                    best = Some((region, conf));
                }
            }
            page = end.max(page + 1);
        }

        // A strong copy signal on its own is enough, even without a long run.
        if best.is_none() {
            let (top_page, top) = self
                .copy_score
                .iter()
                .enumerate()
                .filter(|(p, _)| outside(*p))
                .max_by_key(|(_, v)| **v)
                .map(|(p, v)| (p, *v))?;
            if top > 200 {
                // Walk back to the start of the copied block.
                let mut start = top_page;
                while start > 0 && outside(start - 1) && self.copy_score[start - 1] > top / 8 {
                    start -= 1;
                }
                let len = (SPAN_PAGES.min(PAGES - start) * PAGE_SIZE) as u16;
                return Some((
                    Region {
                        start: (start << 8) as u16,
                        len,
                    },
                    (top as f32 / 6144.0).min(1.0),
                ));
            }
        }
        best
    }

    /// The region the "watch drawing" mode should slow down on.
    pub fn back_buffer(&self) -> Option<Region> {
        self.manual.or(self.detected)
    }
}

/// Whether anything is still lit and will go on fading: at a step of 0 nothing
/// changes, however much is lit.
fn fade_buf(buf: &mut [u8; PHYS_LEN], step: u8) -> bool {
    if step == 0 {
        return false;
    }
    let mut lit = 0;
    for v in buf.iter_mut() {
        *v = v.saturating_sub(step);
        lit |= *v;
    }
    lit != 0
}
