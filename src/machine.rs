//! ZX Spectrum 48K and 128K: memory map and paging, ULA timing, contention,
//! sound, keyboard, and the debugging hooks that hang off the bus.

use crate::audio::Audio;
use crate::profiler::Profiler;
use crate::tape::Tape;
use crate::tracker::{Tracker, SCREEN_END, SCREEN_START};
use crate::z80::{Bus, Z80};

/// Which machine is being emulated. The two differ in clock speed, frame
/// length, memory layout and sound hardware.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Model {
    Spectrum48,
    Spectrum128,
    /// +2A: the 128K reworked with four ROMs and the second paging port.
    Plus2A,
    /// +3: a +2A with a disk interface.
    Plus3,
}

impl Model {
    pub fn name(&self) -> &'static str {
        match self {
            Model::Spectrum48 => "48K",
            Model::Spectrum128 => "128K",
            Model::Plus2A => "+2A",
            Model::Plus3 => "+3",
        }
    }
    /// T-states in one frame.
    pub fn frame_t(&self) -> u32 {
        match self {
            Model::Spectrum48 => 69888,
            _ => 70908,
        }
    }
    /// T-state at which the ULA fetches the first pixel of the display.
    pub fn first_pixel_t(&self) -> u32 {
        match self {
            Model::Spectrum48 => 14335,
            _ => 14361,
        }
    }
    pub fn t_per_line(&self) -> u32 {
        match self {
            Model::Spectrum48 => 224,
            _ => 228,
        }
    }
    pub fn cpu_hz(&self) -> f64 {
        match self {
            Model::Spectrum48 => 3_500_000.0,
            _ => 3_546_900.0,
        }
    }
    pub fn rom_size(&self) -> usize {
        match self {
            Model::Spectrum48 => 0x4000,
            Model::Spectrum128 => 0x8000,
            // +2A/+3 have four 16K ROMs.
            Model::Plus2A | Model::Plus3 => 0x10000,
        }
    }
    pub fn has_ay(&self) -> bool {
        !matches!(self, Model::Spectrum48)
    }
    /// True for machines with the $7FFD paging latch.
    pub fn has_paging(&self) -> bool {
        !matches!(self, Model::Spectrum48)
    }
    /// True for the +2A/+3, which add port $1FFD and its all-RAM modes.
    pub fn has_plus3_paging(&self) -> bool {
        matches!(self, Model::Plus2A | Model::Plus3)
    }
    pub fn has_disk(&self) -> bool {
        matches!(self, Model::Plus3)
    }
    /// The +2A/+3 ULA delays the CPU on a different phase of the fetch cycle.
    pub fn contention_pattern(&self) -> [u8; 8] {
        if self.has_plus3_paging() {
            [1, 0, 7, 6, 5, 4, 3, 2]
        } else {
            [6, 5, 4, 3, 2, 1, 0, 0]
        }
    }
    /// Which RAM banks the ULA shares with the CPU: the odd ones on a 48K/128K,
    /// the top four on a +2A/+3.
    pub fn bank_is_contended(&self, bank: usize) -> bool {
        if self.has_plus3_paging() {
            bank >= 4
        } else {
            bank & 1 == 1
        }
    }
    /// The +2A/+3 gate array drives the bus high instead of leaving it
    /// floating, so there is no floating-bus trick to emulate.
    pub fn has_floating_bus(&self) -> bool {
        !self.has_plus3_paging()
    }
}

/// T-states in a 48K frame. Handy as a default run budget.
pub const FRAME_T: u32 = 69888;
pub const FRAME_T_128: u32 = 70908;
pub const PIXEL_LINES: u32 = 192;
/// How long the ULA holds /INT low at the top of the frame.
pub const IRQ_LEN: u32 = 32;
/// Ceiling on border changes recorded per frame. A whole frame of the tightest
/// border routine is comfortably under this.
pub const BORDER_EVENT_CAP: usize = 24_000;
pub const CPU_HZ: f64 = 3_500_000.0;

const TABLE_LEN: usize = (FRAME_T_128 + 512) as usize;

/// Slow-motion drawing: stop the CPU after a fixed number of writes to the
/// watched area so the screen visibly fills in over several host frames.
#[derive(Clone)]
pub struct SlowDraw {
    pub enabled: bool,
    /// Writes allowed per host frame before the CPU is parked.
    pub writes_per_slice: u32,
    pub watch_screen: bool,
    pub watch_back_buffer: bool,
    pub budget_left: u32,
    pub hit: bool,
    /// Writes to watched memory during the last host frame, for the UI.
    pub last_slice_writes: u32,
}

impl Default for SlowDraw {
    fn default() -> Self {
        SlowDraw {
            enabled: false,
            writes_per_slice: 8,
            watch_screen: true,
            watch_back_buffer: true,
            budget_left: 8,
            hit: false,
            last_slice_writes: 0,
        }
    }
}

impl SlowDraw {
    pub fn begin_slice(&mut self) {
        self.last_slice_writes = self.writes_per_slice.saturating_sub(self.budget_left);
        self.budget_left = self.writes_per_slice.max(1);
        self.hit = false;
    }
}

/// What a 16K slot of the address space currently points at.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Slot {
    Rom(usize),
    Ram(usize),
}

#[derive(Clone)]
pub struct SpectrumBus {
    pub model: Model,
    /// 16K (48K machine) or 32K (128K machine) of ROM.
    pub rom: Vec<u8>,
    /// Eight 16K RAM banks. A 48K machine uses banks 5, 2 and 0.
    pub ram: Vec<u8>,
    /// Last value written to port $7FFD.
    pub page_reg: u8,
    /// Last value written to port $1FFD (+2A/+3 only).
    pub page_reg_1ffd: u8,
    /// What is plugged into the back: which peripherals are fitted, and the
    /// state of the ones that do something.
    pub hardware: crate::hardware::Hardware,
    /// The Interface 1, when one is fitted: its shadow ROM and the microdrives
    /// on the chain behind it.
    pub if1: Option<crate::if1::If1>,
    /// A Multiface's button has been pressed and the NMI not taken yet.
    pub nmi_pending: bool,
    /// The Multifaces that are fitted. All three can be on the back at once,
    /// and one button serves them: the hardware daisy-chains through.
    pub multifaces: Vec<crate::multiface::Multiface>,
    /// The Currah µSpeech, if one is on the back.
    pub uspeech: Option<crate::uspeech::Uspeech>,
    /// The joystick, and which interface it is plugged into.
    pub joystick: crate::joystick::Joystick,
    /// Listening to MIC while a blank tape is in the deck, so what the machine
    /// saves goes onto it.
    pub recorder: Option<crate::recorder::Recorder>,
    /// Answers only while the Kempston mouse is fitted.
    pub mouse: crate::mouse::KempstonMouse,
    /// The ZX Printer or the Alphacom 32, whichever is fitted: to the machine
    /// they are the same thing on the same port.
    pub printer: Option<crate::printer::ZxPrinter>,
    /// The AMX mouse, when fitted: its PIO interrupts for every step.
    pub amx: Option<crate::mouse::AmxMouse>,
    /// The vector a PIO is putting on the bus while its interrupt is taken.
    pio_vector: Option<u8>,
    /// The +3's disk controller. Present on every model, since a bus that
    /// changes shape with the machine is a bus that has to be rebuilt to swap
    /// one; the ports are only decoded on a machine that has the hardware.
    pub fdc: crate::fdc::Fdc,
    /// Once the 128K locks paging, only a reset can undo it.
    pub paging_locked: bool,
    /// Later 48K machines run the display one T-state later relative to the
    /// interrupt. Both variants existed; HALT2INT tells them apart.
    pub late_timing: bool,
    slots: [Slot; 4],

    pub tracker: Tracker,

    /// What to put back to undo the instruction being executed: the address
    /// and the byte that was there before each write. Only collected while
    /// somebody is stepping by hand, since a running machine writes millions
    /// of bytes a second and none of them is going to be stepped back over.
    pub undo: Option<Vec<(u16, u8)>>,

    /// When the interrupt line went down, in T-states since the machine
    /// started, while a recording is playing. The ULA lets it go again after a
    /// few dozen, and a recording's frame boundary is not the T-state frame's,
    /// so there it has to be counted from the moment it was asked for.
    pub irq_raised: u64,

    /// The recording being made, if one is. A frame of an RZX is a number of
    /// opcode fetches and the bytes every IN in that stretch gave back, and
    /// neither can be worked out after the fact.
    pub capture: Option<crate::rzx::Capture>,

    /// T-states elapsed in the current frame.
    ///
    /// The ULA's clock, always: the frame is 69,888 of these on a 48K whatever
    /// the CPU is doing, and every table indexed by it — the contention
    /// pattern, the floating bus, the beam — means what it always did. What a
    /// faster CPU changes is how many of its own cycles fit into one of these,
    /// not how many of these there are.
    pub tstates: u32,
    pub frame: u64,
    /// True while the ULA is asserting /INT for this frame.
    pub irq_pending: bool,
    /// How many times the CPU's clock the machine's own: 1 is the machine as
    /// built, and 2, 4 or 8 an accelerator. See `docs/cpu-turbo.md`.
    pub turbo: u32,
    /// CPU cycles run but not yet paid for in ULA time.
    ///
    /// A cycle costs `1 / turbo` of a T-state, and a T-state is the smallest
    /// thing the ULA has: the remainder is carried from one instruction to the
    /// next rather than rounded away, so a million cycles at 8× cost exactly
    /// an eighth of a million T-states.
    cpu_debt: u32,

    pub border: u8,
    /// Border colour at the start of the frame, plus every mid-frame change,
    /// so the renderer can reproduce raster bands.
    pub border_start: u8,
    /// Every mid-frame border change, as (T-state, colour). Border art writes
    /// the port hundreds of times per line, so this has to be roomy.
    pub border_events: Vec<(u32, u8)>,
    /// The same for the frame just finished. The part of the screen the ULA
    /// has not reached yet still shows the previous frame, so a render taken
    /// mid-frame needs both.
    pub border_prev: Vec<(u32, u8)>,
    pub border_prev_start: u8,
    /// The display file as it stood at the end of the last frame, so the part
    /// of the screen the beam has not reached yet can show what is still on
    /// the glass rather than what the CPU has since written.
    pub screen_prev: Vec<u8>,
    /// The frame as the ULA has painted it so far: each line copied out of the
    /// display file at the moment the beam reached it.
    ///
    /// A game that races the beam writes to a line just after the beam has
    /// passed it and puts it back before the beam comes round again, so the
    /// display file at any one moment is not what a television showed. Reading
    /// it at whatever moment the window repaints makes such a line blink.
    pub painted: Vec<u8>,
    /// How each of this frame's cells was disturbed, if it was: [`SNOW`] or
    /// [`DOUBLE`], one byte a cell.
    ///
    /// Empty until something makes the screen snow, which is rare — a program
    /// has to point I into the screen's own RAM — and cleared as it is used.
    snow: Vec<u8>,
    /// How many marks are in `snow`, so a frame with none costs nothing.
    snow_marks: u32,
    /// For a cell lost to snow, the byte that replaced the low half of the
    /// address the ULA read from — the R register as it stood.
    snow_r: Vec<u8>,
    /// How many character cells of this frame have been copied into `painted`,
    /// counting across each line and then down.
    ///
    /// A cell rather than a whole line: the ULA fetches a cell every four
    /// T-states, and a game racing the beam writes to a cell the moment that
    /// cell has been fetched — often several times within one line. Copying a
    /// whole line the moment the beam entered it took the version from before
    /// any of those writes, which is a frame late for every sprite drawn
    /// behind the beam within a line.
    painted_cells: u32,
    /// Keyboard matrix: one byte per half-row, bit clear = key down.
    pub keys: [u8; 8],
    pub ear: bool,
    pub speaker: bool,
    pub mic: bool,

    pub audio: Audio,

    /// The last byte written to port $FE, which the EAR input hears through
    /// the loudspeaker when no tape is playing.
    pub last_fe: u8,
    /// Whether the MIC bit feeds back as well as the speaker's, which is what
    /// an issue 2 board does. On by default: a tape protection that listens
    /// for the line to be alive hears nothing on a dead one, and the tape it
    /// is listening to is still rolling on a real machine long after its last
    /// block — which no tape file has anything to say about.
    pub issue2: bool,
    /// Cassette player. Its EAR output is read through port $FE bit 6.
    pub tape: Option<Tape>,
    /// Run faster while the tape is playing, so loading does not take the
    /// same four minutes it did in 1983.
    pub tape_boost: bool,
    /// Hand whole blocks to the ROM's loader instead of playing them, so a
    /// tape loads in the time it takes to copy it. See [`crate::flashload`].
    pub tape_flash: bool,

    /// Scratch space for tape edges on their way to the mixer.
    tape_edge_scratch: Vec<(u64, bool)>,

    pub slow: SlowDraw,
    /// The bytes an RZX recording says the ports gave back, while one is
    /// playing. Every IN takes the next one instead of reading the hardware,
    /// which is what makes the program follow the path it followed then.
    pub playback: Option<Playback>,
    /// Opcode fetches since the machine started, which is how a recording
    /// measures the length of a frame.
    pub fetches: u32,
    /// Which parts of the picture were written behind the beam and which
    /// ahead of it, while anybody is watching. `None` is not watching, which
    /// is every case but Race the Beam: marking every write costs a branch and
    /// a couple of stores on the busiest path there is.
    pub tints: Option<Tints>,
    /// What each routine is seen to do, while it is switched on.
    pub observer: crate::observe::Observer,
    /// What the debugger is watching for, and what it caught. The bus is
    /// where all four things happen, so it is the bus that notices them.
    pub breaks: Breaks,
    pub break_hit: Option<Event>,
    /// Writes to video RAM in the frame just finished, for the UI readout.
    pub screen_writes: u32,
    screen_writes_acc: u32,

    contention: Vec<u8>,
}

impl Default for SpectrumBus {
    fn default() -> Self {
        Self::new(Model::Spectrum48)
    }
}

impl SpectrumBus {
    pub fn new(model: Model) -> Self {
        let mut bus = SpectrumBus {
            model,
            rom: vec![0xff; model.rom_size()],
            ram: vec![0; 8 * 0x4000],
            page_reg: 0,
            page_reg_1ffd: 0,
            hardware: crate::hardware::Hardware::default(),
            if1: None,
            nmi_pending: false,
            multifaces: Vec::new(),
            uspeech: None,
            joystick: crate::joystick::Joystick::default(),
            recorder: None,
            mouse: crate::mouse::KempstonMouse::default(),
            printer: None,
            amx: None,
            pio_vector: None,
            fdc: crate::fdc::Fdc::new(),
            paging_locked: false,
            late_timing: false,
            slots: [Slot::Rom(0), Slot::Ram(5), Slot::Ram(2), Slot::Ram(0)],
            tracker: Tracker::new(),
            snow: Vec::new(),
            snow_r: Vec::new(),
            snow_marks: 0,
            tints: None,
            undo: None,
            capture: None,
            irq_raised: 0,
            tstates: 0,
            frame: 0,
            irq_pending: false,
            turbo: 1,
            cpu_debt: 0,
            border: 7,
            border_start: 7,
            border_events: Vec::with_capacity(4096),
            border_prev: Vec::with_capacity(4096),
            border_prev_start: 7,
            screen_prev: vec![0; 6912],
            painted: vec![0; 6912],
            painted_cells: 0,
            keys: [0xff; 8],
            ear: false,
            speaker: false,
            mic: false,
            audio: Audio::new(model.cpu_hz()),
            tape: None,
            last_fe: 0,
            issue2: true,
            tape_boost: true,
            tape_flash: false,
            tape_edge_scratch: Vec::new(),
            slow: SlowDraw::default(),
            screen_writes: 0,
            playback: None,
            fetches: 0,
            observer: crate::observe::Observer::new(),
            breaks: Breaks::default(),
            break_hit: None,
            screen_writes_acc: 0,
            contention: vec![0; TABLE_LEN],
        };
        bus.audio.ay_present = model.has_ay();
        bus.build_contention_table();
        bus.apply_paging();
        bus
    }

    /// T-state of the first pixel fetch, including the late-timing shift.
    #[inline]
    pub fn first_pixel_t(&self) -> u32 {
        self.model.first_pixel_t() + self.late_timing as u32
    }

    /// Rebuild the contention table; call after changing the timing model.
    pub fn set_late_timing(&mut self, late: bool) {
        if self.late_timing != late {
            self.late_timing = late;
            self.build_contention_table();
        }
    }

    fn build_contention_table(&mut self) {
        let pattern = self.model.contention_pattern();
        self.contention.iter_mut().for_each(|v| *v = 0);
        let first = self.first_pixel_t();
        let per_line = self.model.t_per_line();
        for line in 0..PIXEL_LINES {
            let line_start = first + line * per_line;
            for px in 0..128u32 {
                self.contention[(line_start + px) as usize] = pattern[(px % 8) as usize];
            }
        }
    }

    #[inline]
    pub fn frame_t(&self) -> u32 {
        self.model.frame_t()
    }

    // ---- memory paging -----------------------------------------------------

    /// The four all-RAM layouts the +2A/+3 can select with $1FFD.
    pub const SPECIAL_CONFIGS: [[usize; 4]; 4] =
        [[0, 1, 2, 3], [4, 5, 6, 7], [4, 5, 6, 3], [4, 7, 6, 3]];

    /// Recompute the four 16K slots from the paging registers.
    pub(crate) fn apply_paging(&mut self) {
        if !self.model.has_paging() {
            self.slots = [Slot::Rom(0), Slot::Ram(5), Slot::Ram(2), Slot::Ram(0)];
            return;
        }

        // +2A/+3 special mode: no ROM at all, four RAM banks instead.
        if self.model.has_plus3_paging() && self.page_reg_1ffd & 0x01 != 0 {
            let config = ((self.page_reg_1ffd >> 1) & 0x03) as usize;
            let banks = Self::SPECIAL_CONFIGS[config];
            self.slots = [
                Slot::Ram(banks[0]),
                Slot::Ram(banks[1]),
                Slot::Ram(banks[2]),
                Slot::Ram(banks[3]),
            ];
            return;
        }

        // Normal mode. On a +2A/+3 the ROM number is two bits, the low one
        // from $7FFD and the high one from $1FFD.
        let rom = if self.model.has_plus3_paging() {
            (((self.page_reg_1ffd >> 1) & 0x02) | ((self.page_reg >> 4) & 0x01)) as usize
        } else {
            ((self.page_reg >> 4) & 0x01) as usize
        };
        let bank = (self.page_reg & 0x07) as usize;
        self.slots = [Slot::Rom(rom), Slot::Ram(5), Slot::Ram(2), Slot::Ram(bank)];
    }

    /// Write to port $7FFD.
    pub fn write_paging(&mut self, value: u8) {
        if !self.model.has_paging() || self.paging_locked {
            return;
        }
        self.page_reg = value;
        if value & 0x20 != 0 {
            self.paging_locked = true;
        }
        self.apply_paging();
    }

    /// Write to port $1FFD (+2A/+3).
    pub fn write_paging_1ffd(&mut self, value: u8) {
        if !self.model.has_plus3_paging() || self.paging_locked {
            return;
        }
        self.page_reg_1ffd = value;
        self.apply_paging();
    }

    /// True while the +3 is in one of its all-RAM configurations.
    pub fn special_paging(&self) -> bool {
        self.model.has_plus3_paging() && self.page_reg_1ffd & 0x01 != 0
    }

    /// +3 disk motor bit. The controller is told, because a drive whose motor
    /// is off is not ready and that is how +3DOS knows to wait.
    pub fn disk_motor(&self) -> bool {
        self.model.has_disk() && self.page_reg_1ffd & 0x08 != 0
    }

    /// Bank the ULA is displaying: 5 normally, 7 for the shadow screen.
    #[inline]
    pub fn screen_bank(&self) -> usize {
        if self.model.has_paging() && self.page_reg & 0x08 != 0 {
            7
        } else {
            5
        }
    }

    #[inline]
    pub fn slot_of(&self, addr: u16) -> Slot {
        self.slots[(addr >> 14) as usize]
    }

    #[inline]
    pub fn mem(&self, addr: u16) -> u8 {
        // The Interface 1's ROM sits over the bottom 8K of the machine's while
        // it is paged in, which is how a microdrive command runs code the
        // machine has no room for.
        if let Some(if1) = &self.if1 {
            if let Some(byte) = if1.rom_byte(addr) {
                return byte;
            }
        }
        // A Multiface with its button pressed is over the bottom 16K: its ROM
        // under the machine's, and its own RAM where the machine's ROM is not.
        for mf in &self.multifaces {
            if let Some(byte) = mf.mem(addr) {
                return byte;
            }
        }
        // The µSpeech takes the bottom 16K while it is paged in: its ROM, the
        // speech chip, and no machine ROM behind either.
        if let Some(uspeech) = &self.uspeech {
            let busy = self.audio.speech.as_ref().is_some_and(|chip| chip.busy());
            if let Some(byte) = uspeech.mem(addr, busy) {
                return byte;
            }
        }
        let off = (addr & 0x3fff) as usize;
        match self.slot_of(addr) {
            Slot::Rom(page) => {
                let i = page * 0x4000 + off;
                self.rom.get(i).copied().unwrap_or(0xff)
            }
            Slot::Ram(bank) => self.ram[bank * 0x4000 + off],
        }
    }

    #[inline]
    pub fn poke(&mut self, addr: u16, v: u8) {
        for mf in &mut self.multifaces {
            if mf.poke(addr, v) {
                return;
            }
        }
        if let Some(uspeech) = &mut self.uspeech {
            if let Some(told) = uspeech.poke(addr, v) {
                self.tell_speech(told);
                return;
            }
        }
        let off = (addr & 0x3fff) as usize;
        if let Slot::Ram(bank) = self.slot_of(addr) {
            self.ram[bank * 0x4000 + off] = v;
        }
    }

    /// A byte of the display file as it was at the end of the last frame.
    #[inline]
    pub fn video_prev(&self, offset: u16) -> u8 {
        self.screen_prev
            .get(offset as usize & 0x1fff)
            .copied()
            .unwrap_or(0)
    }

    /// Put what the display file holds on the screen at once, rather than
    /// waiting for the beam to come round to it.
    ///
    /// The picture is what the ULA painted, so a screen poked straight into
    /// memory — a `.scr` loaded, or a snapshot restored while the machine is
    /// stopped — would not be seen until the machine had run a frame, and a
    /// stopped machine never does.
    pub fn show_screen_now(&mut self) {
        let from = self.screen_bank() * 0x4000;
        self.painted.copy_from_slice(&self.ram[from..from + 6912]);
        self.screen_prev.copy_from_slice(&self.painted);
    }

    /// Copy out every line the beam has passed since this was last asked.
    ///
    /// Called before anything writes to the screen, and again at the end of the
    /// frame: between those two, nothing can change a line without the version
    /// the beam saw having been kept first.
    pub fn catch_up_painting(&mut self) {
        let reached = self.cells_reached();
        self.paint_cells_to(reached);
    }

    /// Copy cells into the painted frame up to, but not including, `upto`.
    fn paint_cells_to(&mut self, upto: u32) {
        let snowing = self.snow_marks > 0;
        while self.painted_cells < upto {
            let line = (self.painted_cells / 32) as u16;
            let cell = (self.painted_cells % 32) as u16;
            // The display file's thirds-and-rows order, and the attribute
            // that goes with the cell.
            let from = ((line & 0xc0) << 5) | ((line & 0x07) << 8) | ((line & 0x38) << 2);
            let attr = 0x1800 + (line / 8) * 32;
            // How the ULA's fetch went. Nearly every frame has nothing wrong
            // with it, and this runs six thousand times a frame, so the empty
            // case does no work.
            let disturbed = if snowing {
                self.snow
                    .get(self.painted_cells as usize)
                    .copied()
                    .unwrap_or(0)
            } else {
                0
            };
            let (bitmap, attribute) = match disturbed {
                // Read from the wrong address: bits 6..0 of R stand in for
                // the low seven of it, so the byte comes from somewhere else
                // in the same part of the screen — which is why snow is made
                // of the program's own graphics rather than of noise. The
                // coincidence is with one fetch, and that fetch is a pixel
                // one; the attribute is read a T-state later and is fine.
                SNOW => {
                    let r = self.snow_r[self.painted_cells as usize] as u16;
                    let wrong = ((from + cell) & 0xff80) | (r & 0x7f);
                    (self.video(wrong), self.video(attr + cell))
                }
                // Not fetched at all: what went out was the cell before it,
                // which is why it shows as a repeated bar.
                DOUBLE => (
                    self.painted[(from + cell - 1) as usize],
                    self.painted[(attr + cell - 1) as usize],
                ),
                _ => (self.video(from + cell), self.video(attr + cell)),
            };
            self.painted[(from + cell) as usize] = bitmap;
            self.painted[(attr + cell) as usize] = attribute;
            // The beam has now put this byte out, so whatever was written
            // there has been shown and there is nothing left to say about it.
            if let Some(tints) = self.tints.as_mut() {
                tints.clear(from + cell);
                if line.is_multiple_of(8) {
                    tints.clear(attr + cell);
                }
            }
            self.painted_cells += 1;
        }
    }

    /// The CPU has put `addr` on the bus for a refresh.
    ///
    /// The ULA shares the lower RAM with the CPU and tells the two apart by
    /// watching the address bus. A refresh address in that RAM — which is I in
    /// $40..$7F on a 48K, and also $C0..$FF on a 128K with a contended page
    /// banked at $C000 — arrives once per instruction and disturbs the fetch
    /// the ULA is making. Which way it is disturbed depends on where in the
    /// ULA's eight-T-state cycle the last T-state of the M1 falls:
    ///
    /// - on the third, where the first cell of the pair is fetched, that fetch
    ///   is made from the wrong address: bits 6..0 of R are picked up into the
    ///   low byte of it. That is the snow, and it is made of the program's own
    ///   graphics — the same 256 bytes of the screen, the wrong one of them.
    /// - on the fifth, where the second cell is fetched, it is not fetched at
    ///   all and the first cell goes out again in its place. That is the
    ///   "double effect", and it shows as an eight-pixel bar repeated.
    ///
    /// The reference counts the ULA's cycle from two T-states before the first
    /// fetch of the pair, so its third and fifth are the two fetches this
    /// emulator counts as the first and third T-states of the eight.
    ///
    /// Only where there is a shared bus to be confused about: the +2A and +3
    /// gate array drives it itself and does neither.
    pub fn refresh(&mut self, addr: u16) {
        if !self.model.has_floating_bus() || !self.contended_addr(addr) {
            return;
        }
        // The M1 cycle has already been counted, so its last T-state — the one
        // that has to coincide with the ULA's — is one back from here.
        let at = self.tstates.wrapping_sub(1);
        let Some((cell, kind)) = self.disturbed_cell(at) else {
            return;
        };
        // A cell the beam has already been over cannot be disturbed: the ULA
        // read it before the CPU got here.
        if cell < self.painted_cells {
            return;
        }
        if self.snow.is_empty() {
            self.snow = vec![0; (192 * 32) as usize];
            self.snow_r = vec![0; (192 * 32) as usize];
        }
        if self.snow[cell as usize] == 0 {
            self.snow_marks += 1;
        }
        self.snow[cell as usize] = kind;
        // The low byte of the address the ULA reads from, when it is snow.
        self.snow_r[cell as usize] = addr as u8;
    }

    /// How many of each kind, for tests: (snow, double).
    pub fn snow_kinds(&self) -> (u32, u32) {
        let snow = self.snow.iter().filter(|k| **k == SNOW).count() as u32;
        let double = self.snow.iter().filter(|k| **k == DOUBLE).count() as u32;
        (snow, double)
    }

    /// How many of this frame's fetches have been disturbed, for tests and for
    /// anybody wondering why the picture looks like that.
    pub fn snow_marks(&self) -> u32 {
        self.snow_marks
    }

    /// Which cell the ULA is fetching at T-state `at`, and how a refresh
    /// landing there disturbs it.
    ///
    /// The ULA works in pairs of cells over eight T-states: the first cell's
    /// bitmap and attribute, then the second's, then four T-states with the
    /// bus left alone. Counting from one as the reference does, the third
    /// T-state is where the second cell's bitmap is fetched and the fifth is
    /// the first idle one.
    fn disturbed_cell(&self, at: u32) -> Option<(u32, u8)> {
        let first = self.first_pixel_t();
        let since = at.checked_sub(first + 1)?;
        let per_line = self.model.t_per_line();
        let line = since / per_line;
        if line >= 192 {
            return None;
        }
        let along = since % per_line;
        if along >= 128 {
            return None;
        }
        let pair = (along / 8) * 2;
        match along % 8 {
            2 => Some((line * 32 + pair + 1, SNOW)),
            4 => Some((line * 32 + pair + 1, DOUBLE)),
            _ => None,
        }
    }

    /// Has the beam already put this byte of the display file out this frame?
    ///
    /// An attribute byte covers eight lines and is fetched again on every one
    /// of them, so it counts as passed once the beam has started the row: a
    /// write after that point is late for at least part of what it governs.
    fn beam_has_passed(&self, offset: u16) -> bool {
        let cell = (offset & 31) as u32;
        let line = if offset < 0x1800 {
            // Undo the display file's thirds-and-rows order.
            ((offset >> 8) & 0x07) | ((offset >> 2) & 0x38) | ((offset >> 5) & 0xc0)
        } else {
            (offset - 0x1800) / 32 * 8
        } as u32;
        self.painted_cells > line * 32 + cell
    }

    /// How many character cells of the picture the ULA has fetched.
    ///
    /// One every four T-states along a line, thirty-two to a line, and the
    /// fetch runs two T-states ahead of the pixels it puts out.
    fn cells_reached(&self) -> u32 {
        let first = self.first_pixel_t();
        if self.tstates + 2 < first {
            return 0;
        }
        let since = self.tstates + 2 - first;
        let per_line = self.model.t_per_line();
        let line = since / per_line;
        if line >= 192 {
            return 192 * 32;
        }
        let along = (since % per_line) / 4;
        (line * 32 + along.min(32)).min(192 * 32)
    }

    /// Read a byte of the frame the ULA is painting now.
    ///
    /// Up to where the beam has reached this is this frame; past it, it is
    /// still the frame before, because the buffer is painted over rather than
    /// cleared. That is what racing the beam wants behind the cursor: the
    /// raster effect as it is being built, running into what it looked like
    /// last time round.
    pub fn video_painting(&self, offset: u16) -> u8 {
        self.painted
            .get(offset as usize & 0x1fff)
            .copied()
            .unwrap_or(0)
    }

    /// Read a byte of the frame as the ULA painted it.
    pub fn video_painted(&self, offset: u16) -> u8 {
        self.screen_prev
            .get(offset as usize & 0x1fff)
            .copied()
            .unwrap_or(0)
    }

    /// Read a byte of the displayed screen, wherever it is banked.
    #[inline]
    pub fn video(&self, offset: u16) -> u8 {
        let bank = self.screen_bank();
        self.ram[bank * 0x4000 + (offset as usize & 0x3fff)]
    }

    /// Where an address lives in physical memory, which is what the access
    /// tracker records against so a bank keeps its history while paged out.
    #[inline]
    pub fn phys_index(&self, addr: u16) -> usize {
        match self.slot_of(addr) {
            Slot::Ram(bank) => crate::tracker::ram_phys(bank, addr),
            Slot::Rom(page) => crate::tracker::rom_phys(page, addr),
        }
    }

    /// RAM banks worth showing, in address order. A 48K machine only ever
    /// sees three of them.
    pub fn visible_banks(&self) -> Vec<usize> {
        if self.model.has_paging() {
            (0..8).collect()
        } else {
            vec![5, 2, 0]
        }
    }

    /// ROM pages this machine has.
    pub fn rom_pages(&self) -> usize {
        self.rom.len() / 0x4000
    }

    /// Which slot, if any, a RAM bank is currently paged into.
    pub fn ram_bank_slot(&self, bank: usize) -> Option<usize> {
        (0..4).find(|&slot| self.slots[slot] == Slot::Ram(bank))
    }

    /// Which ROM page is paged in at $0000, which is the one whose routines a
    /// program calling into the ROM will reach.
    ///
    /// The 128K's ROM image is two 16K ROMs and the +3's is four; each is
    /// addressed $0000-$3FFF in its own right, so a name for an address only
    /// means anything alongside which of them is in.
    pub fn rom_in_use(&self) -> usize {
        (0..self.rom_pages())
            .find(|page| self.rom_page_slot(*page) == Some(0))
            .unwrap_or(0)
    }

    /// Which slot, if any, a ROM page is currently paged into.
    pub fn rom_page_slot(&self, page: usize) -> Option<usize> {
        (0..4).find(|&slot| self.slots[slot] == Slot::Rom(page))
    }

    /// Read a byte of a specific RAM bank, for the debugger.
    pub fn bank_byte(&self, bank: usize, offset: u16) -> u8 {
        self.ram[(bank & 7) * 0x4000 + (offset as usize & 0x3fff)]
    }

    /// Untracked read for renderers and the debugger.
    #[inline]
    pub fn peek_raw(&self, addr: u16) -> u8 {
        self.mem(addr)
    }

    /// A page is contended when it holds an odd-numbered RAM bank: on a 48K
    /// that is only bank 5 at $4000, on a 128K also banks 1, 3 and 7 wherever
    /// they are paged in.
    #[inline]
    fn contended_addr(&self, addr: u16) -> bool {
        // An accelerated machine is not sharing the ULA's bus on the ULA's
        // terms any more, and the switch exists to get work done rather than
        // to reproduce a stall. At 1× every delay is exactly what it was.
        if self.turbo > 1 {
            return false;
        }
        match self.slot_of(addr) {
            Slot::Ram(bank) => self.model.bank_is_contended(bank),
            Slot::Rom(_) => false,
        }
    }

    /// Charge the ULA's clock for cycles of the CPU's.
    ///
    /// At 1× they are the same clock and this adds what it is given. Above it,
    /// the cycles are divided and what does not divide is carried: the ULA has
    /// nothing smaller than a T-state, and a fraction of one thrown away every
    /// instruction is a machine running slower than it says it does.
    #[inline]
    fn cpu_cycles(&mut self, cycles: u32) {
        if self.turbo <= 1 {
            self.tstates += cycles;
            return;
        }
        self.cpu_debt += cycles;
        self.tstates += self.cpu_debt / self.turbo;
        self.cpu_debt %= self.turbo;
    }

    // ---- timing ------------------------------------------------------------

    /// A whole memory cycle: the ULA stalls the CPU once, at the start, then
    /// the access takes its usual `t` T-states.
    #[inline]
    fn access(&mut self, addr: u16, t: u32) {
        if self.contended_addr(addr) {
            self.tstates += self.delay() as u32;
        }
        self.cpu_cycles(t);
    }

    /// Internal cycles: the address stays on the bus, so contention is
    /// re-evaluated for every single T-state.
    #[inline]
    fn contend_addr(&mut self, addr: u16, times: u32) {
        if self.contended_addr(addr) {
            for _ in 0..times {
                // The stall is the ULA's and the cycle is the CPU's, so they
                // are charged to their own clocks rather than added together.
                self.tstates += self.delay() as u32;
                self.cpu_cycles(1);
            }
        } else {
            self.cpu_cycles(times);
        }
    }

    #[inline]
    fn delay(&self) -> u8 {
        let t = self.tstates as usize;
        if t < self.contention.len() {
            self.contention[t]
        } else if self.contention.is_empty() {
            0
        } else {
            // Past the end of the frame, which only happens while a recording
            // is playing and its frame is running long. The ULA has gone round
            // again and is contending the next frame's display, so the pattern
            // repeats rather than stopping.
            self.contention[t % self.contention.len()]
        }
    }

    /// The I/O contention pattern, which depends on both the port's high byte
    /// and whether it is a ULA port (A0 low).
    ///
    /// Returns the T-state at which the IORQ cycle begins, which is when the
    /// ULA's data is on the bus and therefore what a floating-bus read sees.
    fn contend_io(&mut self, port: u16) -> u32 {
        let high_contended = port & 0xc000 == 0x4000;
        let ula = port & 1 == 0;
        match (high_contended, ula) {
            // C:1, C:3
            (true, true) => {
                self.io_stall();
                let sampled = self.tstates;
                self.cpu_cycles(1);
                self.io_stall();
                self.cpu_cycles(3);
                sampled
            }
            // C:1, C:1, C:1, C:1
            (true, false) => {
                self.io_stall();
                let sampled = self.tstates;
                self.cpu_cycles(1);
                for _ in 0..3 {
                    self.io_stall();
                    self.cpu_cycles(1);
                }
                sampled
            }
            // N:1, C:3 — the ULA stalls the CPU even for an uncontended page.
            (false, true) => {
                self.cpu_cycles(1);
                self.io_stall();
                let sampled = self.tstates;
                self.cpu_cycles(3);
                sampled
            }
            // N:4
            (false, false) => {
                let sampled = self.tstates;
                self.cpu_cycles(4);
                sampled
            }
        }
    }

    #[inline]
    fn io_stall(&mut self) {
        // The same switch as the memory contention: nothing above 1×.
        if self.turbo > 1 {
            return;
        }
        self.tstates += self.delay() as u32;
    }

    // ---- sound -------------------------------------------------------------

    /// Generate any samples due up to now. Called whenever the sound output
    /// changes and once per frame.
    ///
    /// The tape is always advanced first: the mixer must never run past the
    /// tape, or the tape's edges would arrive with timestamps already in the
    /// past and be collapsed into silence.
    pub fn audio_sync(&mut self) {
        if self.tape_playing() {
            self.tape_advance();
        }
        let now = self.total_t();
        self.audio.advance_to(now);
    }

    fn update_beeper(&mut self) {
        // The speaker bit dominates; MIC and the EAR input are audible on real
        // hardware too, which is why you can hear a tape loading.
        let level = 0.55 * self.speaker as u8 as f32
            + 0.08 * self.mic as u8 as f32
            + 0.08 * self.ear as u8 as f32;
        self.audio.beeper = level;
    }

    // ---- tape --------------------------------------------------------------

    /// T-states since power-on, which is the timebase for tape and sound.
    #[inline]
    pub fn total_t(&self) -> u64 {
        self.frame * self.model.frame_t() as u64 + self.tstates as u64
    }

    /// Current EAR input level, advancing the tape to the present moment.
    ///
    /// Every edge the tape produced along the way is mixed in at its own
    /// T-state. Sampling the level only when the CPU reads the port would
    /// collapse a whole burst of pilot tone into one transition, which is
    /// what makes a tape sound like noise instead of a tone.
    pub fn tape_level(&mut self) -> bool {
        self.tape_advance()
    }

    /// The EAR level as it stood at T-state `at`, which is where the ULA put
    /// it on the bus rather than where the instruction ended.
    pub fn tape_level_at(&mut self, at: u64) -> bool {
        self.tape_advance_to(at)
    }

    /// Advance the tape to the present, mixing in every edge it produced at
    /// the T-state it happened, and return the resulting EAR level.
    fn tape_advance(&mut self) -> bool {
        self.tape_advance_to(self.total_t())
    }

    /// The same, at a given T-state rather than at the present one.
    fn tape_advance_to(&mut self, now: u64) -> bool {
        if self.tape.is_none() {
            return self.ear;
        }

        let mut edges = std::mem::take(&mut self.tape_edge_scratch);
        edges.clear();
        let level = {
            let tape = self.tape.as_mut().expect("checked above");
            let level = tape.level_at(now);
            tape.take_pending_edges(&mut edges);
            level
        };

        for (at, ear) in edges.drain(..) {
            self.audio.advance_to(at);
            self.ear = ear;
            self.update_beeper();
        }
        self.tape_edge_scratch = edges;

        if self.ear != level {
            self.audio.advance_to(now);
            self.ear = level;
            self.update_beeper();
        }
        level
    }

    /// Keep the tape running even when the CPU is not polling the port, so the
    /// oscilloscope and block position stay live.
    pub fn tape_tick(&mut self) {
        // Also while the tape is merely paused, if the head is still on it:
        // the hiss goes on, and the scope has nothing to draw unless somebody
        // keeps asking the deck what it can hear.
        if self.tape_playing() || self.tape.as_ref().is_some_and(|t| t.head_down) {
            self.tape_advance();
        }
        // The hiss goes to the loudspeaker as a level rather than as edges: a
        // quiet one never crosses the reader's threshold, so through a gap
        // there would be nothing to hear, and a tape with the volume up hisses
        // through its gaps.
        self.audio.tape_hiss = self.tape.as_ref().map_or(0.0, |t| t.audible_hiss());
    }

    pub fn tape_playing(&self) -> bool {
        self.tape.as_ref().is_some_and(|t| t.playing)
    }

    /// Border colour at every T-state of the frame as it would appear on
    /// screen right now: the current frame up to where the ULA has got to,
    /// and the previous frame beyond that.
    pub fn border_raster(&self) -> Vec<u8> {
        self.border_raster_at(self.tstates)
    }

    /// The same, but splitting this frame from the last at `split` rather than
    /// at wherever the emulator has got to.
    pub fn border_raster_at(&self, split: u32) -> Vec<u8> {
        let frame_t = self.frame_t() as usize;
        let mut out = vec![0u8; frame_t];
        let now = (split as usize).min(frame_t);

        let mut colour = self.border_start;
        let mut ev = self.border_events.iter().peekable();
        for (t, slot) in out.iter_mut().enumerate().take(now) {
            while ev.peek().is_some_and(|(at, _)| *at as usize <= t) {
                colour = ev.next().unwrap().1;
            }
            *slot = colour;
        }

        let mut colour = self.border_prev_start;
        let mut ev = self.border_prev.iter().peekable();
        // Catch up to the current position before filling the rest.
        while ev.peek().is_some_and(|(at, _)| (*at as usize) < now) {
            colour = ev.next().unwrap().1;
        }
        for (t, slot) in out.iter_mut().enumerate().skip(now) {
            while ev.peek().is_some_and(|(at, _)| *at as usize <= t) {
                colour = ev.next().unwrap().1;
            }
            *slot = colour;
        }
        out
    }

    /// Border colour in effect at T-state `t` of the current frame.
    pub fn border_at(&self, t: u32) -> u8 {
        match self.border_events.partition_point(|&(at, _)| at <= t) {
            0 => self.border_start,
            i => self.border_events[i - 1].1,
        }
    }

    fn watched(&self, addr: u16) -> bool {
        if self.slow.watch_screen && (SCREEN_START..SCREEN_END).contains(&addr) {
            return true;
        }
        if self.slow.watch_back_buffer {
            if let Some(r) = self.tracker.back_buffer() {
                return r.contains(addr);
            }
        }
        false
    }

    /// Keyboard: 0xFE reads return the AND of every selected half-row.
    fn keyboard(&self, port: u16, ear: bool) -> u8 {
        // A Sinclair or Cursor interface is wired to five keys, so it pulls
        // the same lines a finger would and a game reading the keyboard cannot
        // tell the difference. That was the whole idea of it.
        let stick = self.joystick.matrix();
        let mut result = 0x1f;
        for (row, held) in stick.iter().enumerate() {
            if port & (1 << (8 + row)) == 0 {
                result &= self.keys[row] & held & 0x1f;
            }
        }
        let mut v = result | 0xa0;
        if ear {
            v |= 0x40;
        }
        v
    }

    /// What bit 6 reads back when no tape is playing.
    ///
    /// The EAR input is not dead with the tape stopped: the machine hears its
    /// own loudspeaker. On an issue 3 board bit 6 follows bit 4 of the last
    /// write to $FE, and on an issue 2 it follows bit 3 as well — which is
    /// what a loader is asking about when it writes to the port and reads
    /// straight back.
    fn ear_feedback(&self) -> bool {
        let mask = if self.issue2 { 0x18 } else { 0x10 };
        self.last_fe & mask != 0
    }

    /// The floating bus: what the ULA had on the bus at T-state `t`.
    /// The +2A/+3 has none, so it reads back as $FF.
    fn floating_bus(&self, t: u32) -> u8 {
        if !self.model.has_floating_bus() {
            return 0xff;
        }
        let first = self.first_pixel_t();
        let per_line = self.model.t_per_line();
        if t < first || t >= first + PIXEL_LINES * per_line {
            return 0xff;
        }
        let rel = t - first;
        let line = rel / per_line;
        let col_t = rel % per_line;
        if col_t >= 128 {
            return 0xff;
        }
        // The ULA fetches two cells in every eight T-states — bitmap, attribute,
        // bitmap, attribute — and leaves the bus alone for the other four, when
        // it reads back as $FF.
        let group = col_t / 8;
        let cell = (group * 2) as u16;
        let offset = match col_t % 8 {
            0 => screen_bitmap_offset(line as u16, cell),
            1 => screen_attr_offset(line as u16, cell),
            2 => screen_bitmap_offset(line as u16, cell + 1),
            3 => screen_attr_offset(line as u16, cell + 1),
            // The four T-states in eight when the ULA is not fetching. The bus
            // is not driven then, and what is on it is the last byte the ULA
            // put there — the attribute of the second cell of the pair. It
            // does not read back as $FF: an IO read is stalled to a free slot
            // before it samples, so every such read landed in here, and a game
            // that waits for a particular byte to come back waited for ever.
            _ => screen_attr_offset(line as u16, cell + 1),
        };
        self.video(offset)
    }

    /// End-of-frame bookkeeping: flush sound, re-arm the interrupt.
    /// Put the interrupt line down, and note when: the ULA lets it go again
    /// after a few dozen T-states whether anything took it or not.
    pub fn raise_interrupt(&mut self) {
        self.irq_pending = true;
        self.irq_raised = self.total_t();
    }

    /// Every access to $0038 turns the µSpeech over, whichever kind of cycle
    /// it is: a read, a write, an `IN`, an `OUT` or an opcode fetch. This is
    /// the read and write side of that; the fetch is in `fetch_op` and the
    /// ports in the I/O handlers.
    #[inline]
    fn uspeech_touch(&mut self, addr: u16) {
        if addr != 0x0038 {
            return;
        }
        if let Some(uspeech) = &mut self.uspeech {
            uspeech.touch(addr);
        }
    }

    /// Pass on what the interface was told: the chip lives with the mixer,
    /// because it makes sound at its own rate rather than the machine's.
    pub fn tell_speech(&mut self, told: crate::uspeech::Told) {
        match told {
            crate::uspeech::Told::Say(allophone) => {
                if let Some(chip) = self.audio.speech.as_mut() {
                    chip.speak(allophone);
                }
            }
            crate::uspeech::Told::Pitch(high) => self.audio.set_speech_pitch(high),
            crate::uspeech::Told::Nothing => {}
        }
    }

    /// The Multiface's red button, which is one button for all of them: the
    /// hardware chains through, and the last one on the back that is ready
    /// takes it.
    pub fn press_red_button(&mut self) -> bool {
        for mf in self.multifaces.iter_mut().rev() {
            if mf.press() {
                self.nmi_pending = true;
                return true;
            }
        }
        false
    }

    /// End the video frame here, wherever the T-state count has got to.
    ///
    /// For a recording, whose frames are counted in opcode fetches: on the
    /// machine it was made on, that boundary *was* the start of a video frame,
    /// because that is where the ULA's interrupt came from. Letting the
    /// T-state frame run on its own beside it lets the two drift apart, and
    /// then everything timed against the picture — which is most of what a
    /// game does with the border and the display file — happens at the wrong
    /// place on screen.
    pub fn end_frame_here(&mut self) {
        // Whatever is left of this T-state frame is the start of the next one.
        self.tstates = 0;
        self.finish_frame();
    }

    pub fn end_frame(&mut self) {
        self.tstates -= self.model.frame_t();
        self.finish_frame();
    }

    /// The housekeeping a finished frame needs, however it ended.
    fn finish_frame(&mut self) {
        // Whatever the beam had left to paint, so the frame handed on is a
        // whole one.
        self.paint_cells_to(192 * 32);
        // Snow belongs to the frame it happened in.
        if self.snow_marks > 0 {
            self.snow.iter_mut().for_each(|mark| *mark = 0);
            self.snow_marks = 0;
        }
        self.frame += 1;
        // A block the machine has finished saving goes onto the tape in the
        // deck, once MIC has been quiet long enough for it to be over. It is
        // done after the frame count has moved on: before it, `tstates` has
        // already had the frame taken off and the clock reads a frame early,
        // which the recorder takes for a reset and drops the block.
        if self.recorder.as_ref().is_some_and(|r| r.pending()) {
            let now = self.total_t();
            if let Some(block) = self.recorder.as_mut().and_then(|r| r.finish_if_quiet(now)) {
                if let Some(tape) = self.tape.as_mut() {
                    tape.blocks.push(block);
                }
            }
        }
        // While a recording is playing, the frame boundary is where the
        // recording says it is — an instruction count, not a T-state count —
        // so the interrupt is raised there instead of here.
        self.irq_pending = self.playback.is_none();
        if let Some(capture) = &mut self.capture {
            capture.end_frame(self.fetches);
        }
        self.screen_writes = self.screen_writes_acc;
        self.screen_writes_acc = 0;
        // Keep the finished frame: what the ULA painted, line by line, rather
        // than what the display file holds now. A game that races the beam has
        // already rubbed out the lines it drew before the beam reached them.
        self.screen_prev.copy_from_slice(&self.painted);
        self.painted_cells = 0;
        std::mem::swap(&mut self.border_events, &mut self.border_prev);
        self.border_prev_start = self.border_start;
        self.border_start = self.border;
        self.border_events.clear();
        self.audio_sync();
        self.observer.end_frame();
    }

    pub fn frame_visuals(&mut self) {
        self.tracker.fade();
        self.tracker.tick_detector();
        self.fdc.fade();
    }
}

/// Offset of a pixel byte within the 6912-byte display file.
pub fn screen_bitmap_offset(line: u16, cell: u16) -> u16 {
    let y = line;
    ((y & 0xc0) << 5) | ((y & 0x07) << 8) | ((y & 0x38) << 2) | (cell & 0x1f)
}

/// Offset of an attribute byte within the display file.
pub fn screen_attr_offset(line: u16, cell: u16) -> u16 {
    0x1800 + (line / 8) * 32 + (cell & 0x1f)
}

pub fn screen_bitmap_addr(line: u16, cell: u16) -> u16 {
    0x4000 | screen_bitmap_offset(line, cell)
}

pub fn screen_attr_addr(line: u16, cell: u16) -> u16 {
    0x4000 | screen_attr_offset(line, cell)
}

impl Bus for SpectrumBus {
    fn int_vector(&mut self) -> u8 {
        self.pio_vector.unwrap_or(0xFF)
    }

    fn refresh(&mut self, addr: u16) {
        SpectrumBus::refresh(self, addr);
    }

    fn fetch_op(&mut self, addr: u16) -> u8 {
        if self.if1.is_some() {
            let now = self.total_t();
            if let Some(if1) = &mut self.if1 {
                if1.at(now);
                if1.on_fetch(addr);
            }
        }
        // A Multiface pages itself in at the fetch from $0066: the button
        // pulled /NMI, and this is where the machine lands.
        for mf in &mut self.multifaces {
            mf.on_fetch(addr);
        }
        // And the µSpeech turns over at $0038, before the byte is read: that
        // is how the interrupt runs its handler and then the machine's.
        if let Some(uspeech) = &mut self.uspeech {
            uspeech.touch(addr);
        }
        // Counted for RZX playback, which measures a frame in opcode fetches:
        // a prefixed instruction is two or more of them, so counting whole
        // instructions instead runs past the end of every frame.
        self.fetches = self.fetches.wrapping_add(1);
        self.observer.on_fetch(addr);
        self.access(addr, 4);
        let phys = self.phys_index(addr);
        self.tracker.on_exec(phys, addr);
        let byte = self.mem(addr);
        // The interface pages its ROM out *after* the byte at $0700 has been
        // read: that byte is the RET which takes the machine back to its own
        // ROM, and it has to come from the shadow.
        if let Some(if1) = &mut self.if1 {
            if1.after_fetch(addr);
        }
        byte
    }

    fn read(&mut self, addr: u16) -> u8 {
        self.access(addr, 3);
        self.uspeech_touch(addr);
        let phys = self.phys_index(addr);
        self.tracker.on_read(phys, addr);
        self.observer.on_read(addr);
        self.mem(addr)
    }

    fn read_operand(&mut self, addr: u16) -> u8 {
        // Timed exactly as a read, because that is what the Z80 does; counted
        // as code, because that is what it is.
        self.access(addr, 3);
        let phys = self.phys_index(addr);
        self.tracker.on_read(phys, addr);
        self.observer.on_fetch(addr);
        self.mem(addr)
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.access(addr, 3);
        self.uspeech_touch(addr);
        // A watch on an address costs a comparison on every write, which is
        // why it is an Option and not a list: None is one test, and nothing
        // is paid for a watch nobody set.
        if let Some((low, high)) = self.breaks.write_range {
            if addr >= low && addr <= high {
                self.break_hit.get_or_insert(Event::Wrote(addr, value));
            }
        }
        let phys = self.phys_index(addr);
        self.tracker.on_write(phys, addr);
        // Only RAM can change, and a write to ROM is not a change. A +3 with
        // its all-RAM paging is the one machine this misses below $4000.
        if self.observer.enabled && addr >= 0x4000 {
            let old = self.peek_raw(addr);
            self.observer.on_change(addr, old, value);
        }
        self.observer.on_write(addr, value);
        if self.undo.is_some() {
            // What was there before, so it can be put back. Read before the
            // write rather than worked out afterwards, because afterwards it
            // is gone. A write into ROM changes nothing and undoes to the same
            // nothing, so it needs no special case.
            let was = self.mem(addr);
            if let Some(undo) = self.undo.as_mut() {
                undo.push((addr, was));
            }
        }
        if (SCREEN_START..SCREEN_END).contains(&addr) {
            // Whatever the beam has already put out is kept before this write
            // can change it: a game racing the beam rubs a line out the moment
            // it has been painted.
            self.catch_up_painting();
            if self.tints.is_some() {
                let offset = addr - SCREEN_START;
                let kind = if self.beam_has_passed(offset) {
                    Tint::Late
                } else {
                    Tint::Early
                };
                let now = self.total_t();
                if let Some(tints) = self.tints.as_mut() {
                    tints.mark(offset, kind, now);
                }
            }
            self.screen_writes_acc += 1;
            if self.breaks.screen {
                self.break_hit.get_or_insert(Event::Screen(addr));
            }
        }
        if self.slow.enabled && self.watched(addr) {
            if self.slow.budget_left == 0 {
                self.slow.hit = true;
            } else {
                self.slow.budget_left -= 1;
                if self.slow.budget_left == 0 {
                    self.slow.hit = true;
                }
            }
        }
        self.poke(addr, value);
    }

    fn contend(&mut self, addr: u16, times: u32) {
        self.contend_addr(addr, times);
    }

    fn io_read(&mut self, port: u16) -> u8 {
        let byte = self.io_read_uncaptured(port);
        if let Some(capture) = &mut self.capture {
            // Every byte an IN gave back, in order: that is what playing the
            // recording hands out again, in place of the hardware.
            capture.inputs.push(byte);
        }
        byte
    }

    fn io_write(&mut self, port: u16, value: u8) {
        let sampled = self.contend_io(port);
        self.observer.on_port(port, true);
        if self.breaks.port_out {
            self.break_hit.get_or_insert(Event::Out(port, value));
        }

        if port & 1 == 0 {
            self.last_fe = value;
            let new = value & 7;
            if new != self.border && self.border_events.len() < BORDER_EVENT_CAP {
                // Timed at the start of the IORQ cycle, which is when the ULA
                // sees the write.
                self.border_events.push((sampled, new));
            }
            self.border = new;
            let speaker = value & 0x10 != 0;
            let mic = value & 0x08 != 0;
            if speaker != self.speaker || mic != self.mic {
                if self.breaks.beeper {
                    self.break_hit.get_or_insert(Event::Beeper);
                }
                self.audio_sync();
                if mic != self.mic {
                    let now = self.total_t();
                    if let Some(recorder) = &mut self.recorder {
                        recorder.edge(now);
                    }
                }
                self.speaker = speaker;
                self.mic = mic;
                self.update_beeper();
            }
        }

        // The Fuller Audio Box: a sound chip of its own, register select
        // at $3F and data at $5F. Its ports are decoded on the low byte,
        // as the box does.
        if self.hardware.fitted(crate::hardware::Peripheral::Fuller) {
            if port & 0xFF == 0x3F {
                self.hardware.fuller_register = value & 0x0F;
            } else if port & 0xFF == 0x5F {
                let register = self.hardware.fuller_register;
                if let Some(ay) = self.audio.extra_ay.as_mut() {
                    ay.selected = register;
                    ay.write(value);
                }
            }
        }
        // The SpecDrum: an eight-bit converter and nothing else. A byte
        // written to $DF is a sample, and a program feeds it drum sounds
        // out of memory as fast as it can.
        if self.hardware.fitted(crate::hardware::Peripheral::SpecDrum) && port & 0xFF == 0xDF {
            self.audio_sync();
            // Centred on nothing, so silence is silence: the converter
            // idles at half scale.
            self.audio.dac = (value as f32 - 128.0) / 128.0 * 0.4;
        }
        if let Some(amx) = &mut self.amx {
            amx.io_write(port, value);
        }
        // The printer: the stylus and the motor, on $FB.
        if crate::printer::ZxPrinter::decodes(port) && self.printer.is_some() {
            let (now, frame_t) = (self.total_t(), self.model.frame_t() as u64);
            if let Some(printer) = &mut self.printer {
                printer.write(now, frame_t, value);
            }
        }
        // The Interface 1's data and control registers.
        if self.if1.is_some() {
            let now = self.total_t();
            if port & 0x0018 == 0x0000 {
                if let Some(if1) = &mut self.if1 {
                    if1.at(now);
                    if1.write_data(value);
                }
            }
            if port & 0x0018 == 0x0008 {
                if let Some(if1) = &mut self.if1 {
                    if1.at(now);
                    if1.write_control(value);
                }
            }
        }

        for mf in &mut self.multifaces {
            mf.io_write(port, value);
        }
        if self.uspeech.is_some() {
            let told = self
                .uspeech
                .as_mut()
                .and_then(|uspeech| uspeech.io_write(port, value));
            if let Some(told) = told {
                self.tell_speech(told);
            }
        }

        if self.model.has_paging() {
            // $7FFD: memory paging. The +2A/+3 decode it more strictly than
            // the 128K, which decodes only A15 and A1.
            let is_7ffd = if self.model.has_plus3_paging() {
                port & 0xc002 == 0x4000
            } else {
                port & 0x8002 == 0
            };
            if is_7ffd {
                self.write_paging(value);
            }
            // $1FFD: the +2A/+3's second paging port.
            if self.model.has_plus3_paging() && port & 0xf002 == 0x1000 {
                self.write_paging_1ffd(value);
                let now = self.total_t();
                self.fdc.motor = self.disk_motor();
                self.fdc.at(now);
            }
            // $3FFD: the controller's data register. The status register at
            // $2FFD is read-only, so nothing is written there.
            if self.model.has_disk() && port & 0xf002 == 0x3000 {
                let now = self.total_t();
                self.fdc.at(now);
                self.fdc.write(value);
            }
            // $FFFD: AY register select, $BFFD: AY data.
            if port & 0xc002 == 0xc000 {
                if self.breaks.ay {
                    self.break_hit.get_or_insert(Event::Ay);
                }
                self.audio.ay.selected = value & 0x0f;
            } else if port & 0xc002 == 0x8000 {
                if self.breaks.ay {
                    self.break_hit.get_or_insert(Event::Ay);
                }
                self.audio_sync();
                self.audio.ay.write(value);
            }
        }
    }

    fn peek(&self, addr: u16) -> u8 {
        self.mem(addr)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stop {
    /// Ran out of the requested T-state budget.
    Budget,
    /// Slow-draw mode used up its write allowance.
    SlowDraw,
    Breakpoint(u16),
    /// A single-step request completed.
    Stepped,
    /// Something the debugger was watching for happened, at this address.
    ///
    /// For an interrupt that is the first instruction of the handler, which
    /// has not run yet. For the others it is the instruction that did it,
    /// which has: a write or an OUT takes effect part-way through an
    /// instruction and the CPU is only stoppable between them.
    Watched(Event, u16),
}

/// What the machine can be told to stop on besides reaching an address.
///
/// These are the things a program does that are hard to find by address:
/// where it draws, where it makes a noise, and where the frame interrupt takes
/// it. Each is only tested when it is switched on, so nothing is paid for a
/// watch that is off.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Breaks {
    /// A write anywhere in the display file.
    pub screen: bool,
    /// The speaker or MIC bit of port $FE changing state.
    pub beeper: bool,
    /// Any access to the sound chip, read or write.
    pub ay: bool,
    /// The frame interrupt being accepted by the CPU.
    pub interrupt: bool,
    /// Code entering the ROM from outside it: a program calling a ROM routine.
    /// Moving about within the ROM is not entering it, so a ROM routine
    /// calling another one does not count.
    pub rom: bool,
    /// Any IN at all: a program reading a port, whichever port it is.
    pub port_in: bool,
    /// Any OUT at all.
    pub port_out: bool,
    /// A write anywhere in this range of addresses, inclusive. The one watch
    /// that is about a place rather than a kind of thing: "what writes to the
    /// lives counter" is the question a debugger is for, and the address is
    /// usually all that is known.
    pub write_range: Option<(u16, u16)>,
}

impl Breaks {
    /// Whether anything at all is being watched.
    pub fn any(&self) -> bool {
        self.write_range.is_some()
            || self.screen
            || self.beeper
            || self.ay
            || self.interrupt
            || self.rom
            || self.port_in
            || self.port_out
    }
}

/// One frame's worth of recorded input, being handed out.
#[derive(Clone, Debug, Default)]
pub struct Playback {
    pub inputs: Vec<u8>,
    pub cursor: usize,
    /// How many times a frame has asked for more input than was recorded.
    /// Never zero on a recording that has come adrift from the machine, so it
    /// is worth telling the user about rather than playing on regardless.
    pub short: u32,
}

impl Playback {
    /// The next recorded byte. A frame that reads more than was recorded is
    /// out of step; the last byte is repeated rather than inventing one, and
    /// the shortfall is counted.
    fn next(&mut self) -> u8 {
        match self.inputs.get(self.cursor) {
            Some(byte) => {
                self.cursor += 1;
                *byte
            }
            None => {
                self.short += 1;
                self.inputs.last().copied().unwrap_or(0xFF)
            }
        }
    }
}

/// Which of those happened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    Screen(u16),
    Beeper,
    Ay,
    Interrupt,
    /// Went into the ROM from this address.
    Rom(u16),
    /// Read a port, and which.
    In(u16),
    /// Wrote to a port, and what.
    Out(u16, u8),
    /// Wrote to a watched address, and what was written.
    Wrote(u16, u8),
}

impl Event {
    /// What to tell the user the machine stopped for.
    pub fn describe(&self) -> String {
        match self {
            Event::Screen(addr) => format!("Wrote to the screen at ${addr:04X}"),
            Event::Wrote(addr, value) => format!("Wrote ${value:02X} to ${addr:04X}"),
            Event::Beeper => "Toggled the beeper".to_string(),
            Event::Ay => "Used the sound chip".to_string(),
            Event::Interrupt => "Took the frame interrupt".to_string(),
            Event::Rom(from) => format!("Went into the ROM from ${from:04X}"),
            Event::In(port) => format!("Read port ${port:04X}"),
            Event::Out(port, value) => format!("Wrote ${value:02X} to port ${port:04X}"),
        }
    }
}

/// Everything needed to put the machine back the way it was before one
/// instruction.
///
/// Not a copy of the machine: that would be sixty-four kilobytes of RAM a
/// step, plus the tape, the audio and everything the observer has watched. An
/// instruction writes a byte or two, so what it changed is small even when the
/// machine is not.
///
/// What is not put back: the sound already played, where the tape has reached,
/// and anything the ULA has already painted. Those are outside the machine's
/// memory and cannot be recalled; the picture catches up on the next frame.
#[derive(Clone)]
pub struct Undo {
    pub cpu: Z80,
    /// Each address written, and the byte that was there before.
    pub writes: Vec<(u16, u8)>,
    pub tstates: u32,
    pub border: u8,
    pub page_reg: u8,
    pub page_reg_1ffd: u8,
    pub frames_completed: u32,
}

#[derive(Clone)]
pub struct Spectrum {
    pub cpu: Z80,
    pub bus: SpectrumBus,
    pub breakpoints: Vec<u16>,
    /// Temporary breakpoint used by "step over" / "run to cursor".
    pub temp_bp: Option<u16>,
    /// Whole frames completed since the last visual update.
    pub frames_completed: u32,
    /// Call profiler; does nothing until a run is started.
    pub profiler: Profiler,
}

impl Default for Spectrum {
    fn default() -> Self {
        Self::new()
    }
}

/// When a byte of the display file was written, relative to the beam.
///
/// Writing to a part of the picture the ULA has already put out means the
/// change will not be seen until the next frame; writing to a part it has not
/// reached yet means it will be seen in this one. Which of the two a game is
/// doing is the difference between a sprite that appears and a sprite that
/// flickers, and neither shows up in a finished picture.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tint {
    /// Nothing has been written here, or the beam has since been over it.
    #[default]
    None,
    /// Written after the beam had passed: too late for this frame.
    Late,
    /// Written before the beam got there: it will be shown this frame.
    Early,
}

/// The marks on the display file, one per byte of it.
#[derive(Clone, Debug)]
pub struct Tints {
    kind: Vec<Tint>,
    /// When each was written, in T-states since the machine started.
    when: Vec<u64>,
}

impl Default for Tints {
    fn default() -> Self {
        Tints {
            kind: vec![Tint::None; 0x1b00],
            when: vec![0; 0x1b00],
        }
    }
}

impl Tints {
    /// The mark on a byte of the display file, and when it was made.
    pub fn at(&self, offset: u16) -> (Tint, u64) {
        let at = offset as usize & 0x1fff;
        match (self.kind.get(at), self.when.get(at)) {
            (Some(kind), Some(when)) => (*kind, *when),
            _ => (Tint::None, 0),
        }
    }

    fn mark(&mut self, offset: u16, kind: Tint, when: u64) {
        let at = offset as usize & 0x1fff;
        if at < self.kind.len() {
            self.kind[at] = kind;
            self.when[at] = when;
        }
    }

    fn clear(&mut self, offset: u16) {
        let at = offset as usize & 0x1fff;
        if at < self.kind.len() {
            self.kind[at] = Tint::None;
        }
    }
}

/// A fetch made from the wrong address, the low half of it the R register.
pub const SNOW: u8 = 1;
/// A fetch not made at all, the cell before it put out again instead.
pub const DOUBLE: u8 = 2;

/// Where the ROM ends and RAM begins, on every machine here.
const ROM_END: u16 = 0x4000;

/// Read a byte without disturbing anything, for looking at the stack.
fn raw_peek(bus: &SpectrumBus, addr: u16) -> u8 {
    bus.peek_raw(addr)
}

impl Spectrum {
    pub fn new() -> Self {
        Spectrum::with_model(Model::Spectrum48)
    }

    pub fn with_model(model: Model) -> Self {
        Spectrum {
            cpu: Z80::new(),
            bus: SpectrumBus::new(model),
            breakpoints: Vec::new(),
            temp_bp: None,
            frames_completed: 0,
            profiler: Profiler::new(),
        }
    }

    pub fn load_rom(&mut self, data: &[u8]) {
        let n = data.len().min(self.bus.rom.len());
        self.bus.rom[..n].copy_from_slice(&data[..n]);
    }

    /// Switch machine, keeping the tape and audio device attached.
    pub fn set_model(&mut self, model: Model, rom: &[u8]) {
        let tape = self.bus.tape.take();
        let audio = std::mem::replace(&mut self.bus.audio, crate::audio::Audio::new(1.0));
        let tape_boost = self.bus.tape_boost;
        let turbo = self.bus.turbo;
        let tape_flash = self.bus.tape_flash;
        let slow_enabled = self.bus.slow.enabled;
        let late = self.bus.late_timing;
        // What is plugged into the back stays plugged in: changing the machine
        // is unplugging one and plugging in another, not unscrewing the
        // Interface 1 from the back of it.
        let hardware = std::mem::take(&mut self.bus.hardware);
        let if1 = self.bus.if1.take();
        let multifaces = std::mem::take(&mut self.bus.multifaces);
        let uspeech = self.bus.uspeech.take();
        let joystick = std::mem::take(&mut self.bus.joystick);
        let recorder = self.bus.recorder.take();
        let mouse = self.bus.mouse;
        let printer = self.bus.printer.take();
        let amx = self.bus.amx.take();

        self.bus = SpectrumBus::new(model);
        self.bus.hardware = hardware;
        self.bus.if1 = if1;
        self.bus.multifaces = multifaces;
        self.bus.uspeech = uspeech;
        self.bus.joystick = joystick;
        self.bus.recorder = recorder;
        self.bus.mouse = mouse;
        self.bus.printer = printer;
        self.bus.amx = amx;
        self.bus.set_late_timing(late);
        self.bus.audio = audio;
        self.bus.audio.set_cpu_hz(model.cpu_hz());
        self.bus.audio.ay_present = model.has_ay();
        self.bus.audio.ay.reset();
        self.bus.tape = tape;
        self.bus.tape_boost = tape_boost;
        self.bus.turbo = turbo;
        self.bus.tape_flash = tape_flash;
        self.bus.slow.enabled = slow_enabled;
        self.load_rom(rom);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.cpu.reset();
        self.bus.tstates = 0;
        self.bus.frame = 0;
        self.bus.irq_pending = false;
        self.bus.border = 7;
        self.bus.ram.iter_mut().for_each(|b| *b = 0);
        self.bus.page_reg = 0;
        self.bus.page_reg_1ffd = 0;
        self.bus.paging_locked = false;
        self.bus.apply_paging();
        self.bus.tracker.reset();
        // The reset line goes to the disk controller too: it comes back
        // waiting for a command, with whatever is in the drives still in them.
        self.bus.fdc.reset();
        if let Some(if1) = &mut self.bus.if1 {
            if1.reset();
        }
        for mf in &mut self.bus.multifaces {
            mf.reset();
        }
        if let Some(uspeech) = &mut self.bus.uspeech {
            uspeech.reset();
        }
        // A direction held across a reset would be held for ever, the same way
        // a shift clicked in the keyboard window used to be.
        self.bus.joystick.release();
        if let Some(printer) = &mut self.bus.printer {
            printer.halt();
        }
        if let Some(amx) = &mut self.bus.amx {
            amx.reset();
        }
        if let Some(chip) = self.bus.audio.speech.as_mut() {
            chip.reset();
        }
        self.bus.audio.ay.reset();
        self.bus.audio.rebase(0);
        self.bus.speaker = false;
        self.bus.mic = false;
        self.bus.ear = false;
        self.bus.audio.beeper = 0.0;
        // The tape's timebase is absolute, so it has to go back to the start
        // along with the frame counter.
        if let Some(tape) = &mut self.bus.tape {
            tape.stop();
            tape.rewind();
            tape.edges.clear();
        }
    }

    fn check_interrupt(&mut self) {
        // The red button pulls /NMI, which is taken between instructions
        // whatever the program has said about interrupts: that is the whole
        // point of it, and why a Multiface can stop a game that has them off.
        if self.bus.nmi_pending {
            self.bus.nmi_pending = false;
            self.cpu.nmi(&mut self.bus);
        }
        if self.bus.irq_pending {
            // The ULA holds the interrupt line down for thirty-odd T-states
            // and then lets it go, so a program with interrupts disabled
            // across the top of the frame misses that one entirely.
            //
            // Measured against the frame while the machine runs on its own,
            // which is where the ULA measures it from. A recording's frames
            // are counted in opcode fetches and wander away from the T-state
            // frame, so there the window runs from the moment the recording
            // asked for the interrupt.
            //
            // Two rules rather than one clock for both. Timing the ordinary
            // case from a stored T-state as well left that number behind after
            // a reset — which is what loading a tape does — with the counter
            // back at zero and the stored moment in the future, so the
            // subtraction saturated, nothing was ever missed, and every
            // interrupt was taken wherever in the frame the program happened
            // to enable them.
            let missed = match self.bus.playback {
                None => self.bus.tstates >= IRQ_LEN,
                Some(_) => self.bus.total_t().saturating_sub(self.bus.irq_raised) >= IRQ_LEN as u64,
            };
            if missed {
                self.bus.irq_pending = false;
            } else if self.cpu.interrupt(&mut self.bus) {
                self.bus.irq_pending = false;
                if self.bus.breaks.interrupt {
                    self.bus.break_hit.get_or_insert(Event::Interrupt);
                }
                if self.bus.observer.enabled {
                    let registers = crate::observe::Registers {
                        af: self.cpu.af(),
                        bc: self.cpu.bc(),
                        de: self.cpu.de(),
                        hl: self.cpu.hl(),
                    };
                    let (pc, sp) = (self.cpu.pc, self.cpu.sp);
                    self.bus.observer.on_interrupt(pc, sp, registers);
                }
                // The handler is profiled like any other call.
                if self.profiler.running {
                    let now = self.bus.total_t();
                    self.profiler.on_call(self.cpu.pc, self.cpu.sp, now);
                }
            }
        }
        // The AMX mouse's PIO holds /INT down until the CPU takes it, rather
        // than for the ULA's thirty-odd T-states, and puts its own vector on
        // the bus when it is acknowledged.
        if self.bus.amx.is_some() && !self.bus.irq_pending {
            let now = self.bus.total_t();
            let wants = self
                .bus
                .amx
                .as_ref()
                .and_then(|amx| amx.wants_interrupt(now));
            if let Some((channel, vector)) = wants {
                self.bus.pio_vector = Some(vector);
                if self.cpu.interrupt(&mut self.bus) {
                    if let Some(amx) = self.bus.amx.as_mut() {
                        amx.acknowledged(channel, now);
                    }
                }
                self.bus.pio_vector = None;
            }
        }
    }

    /// Execute exactly one instruction (after any pending interrupt).
    /// One instruction, with what it took to get there kept so it can be
    /// undone. Only used while somebody is stepping by hand.
    pub fn step_recording(&mut self) -> Undo {
        let before = Undo {
            cpu: self.cpu.clone(),
            writes: Vec::new(),
            tstates: self.bus.tstates,
            border: self.bus.border,
            page_reg: self.bus.page_reg,
            page_reg_1ffd: self.bus.page_reg_1ffd,
            frames_completed: self.frames_completed,
        };
        self.bus.undo = Some(Vec::new());
        self.step_instruction();
        let writes = self.bus.undo.take().unwrap_or_default();
        Undo { writes, ..before }
    }

    /// Put the machine back as it was before that instruction.
    ///
    /// The paging registers go back first, so the addresses that were written
    /// mean the same thing again before anything is written to them.
    pub fn undo_step(&mut self, undo: &Undo) {
        self.bus.page_reg = undo.page_reg;
        self.bus.page_reg_1ffd = undo.page_reg_1ffd;
        if self.bus.model.has_paging() {
            self.bus.apply_paging();
        }
        for (addr, was) in undo.writes.iter().rev() {
            self.bus.poke(*addr, *was);
        }
        self.cpu = undo.cpu.clone();
        self.bus.tstates = undo.tstates;
        self.bus.border = undo.border;
        self.frames_completed = undo.frames_completed;
    }

    pub fn step_instruction(&mut self) {
        // Answering the ROM's loader is done in place of the instruction at
        // its first address, so the routine never runs at all. One comparison
        // when the switch is off, and it is only on while somebody is loading
        // a tape in a hurry.
        if self.bus.tape_flash
            && self.cpu.pc == crate::flashload::LD_BYTES
            && crate::flashload::load_block(self) != crate::flashload::Loaded::NotOurs
        {
            return;
        }
        self.check_interrupt();

        let watching = self.profiler.running || self.bus.observer.enabled || self.bus.breaks.rom;
        let (pc0, sp0, t0) = if watching {
            (self.cpu.pc, self.cpu.sp, self.bus.total_t())
        } else {
            (0, 0, 0)
        };

        // Where the code was before this instruction, for the ROM watch: only
        // needed while it is on, and it is one comparison when it is not.
        let was_outside_rom = self.bus.breaks.rom && self.cpu.pc >= ROM_END;

        if self.bus.observer.enabled {
            // Before the instruction runs: a port is touched part-way through
            // one, and the observer needs to know which instruction that was.
            self.bus.observer.executing = pc0;
        }
        self.cpu.step(&mut self.bus);

        // Going into the ROM from outside it is a program calling a ROM
        // routine. Moving about inside the ROM is not, so a ROM routine
        // calling another one is left alone.
        if was_outside_rom && self.cpu.pc < ROM_END {
            self.bus.break_hit.get_or_insert(Event::Rom(pc0));
        }

        // A recording's frame is the video frame, so while one is playing the
        // frame ends where the recording says and nowhere else. Ending it on
        // the T-state count as well paints the screen twice inside one
        // recorded frame whenever the machine takes longer over the recorded
        // instructions than the machine that recorded them did — which is
        // what the flicker in Space Harrier's recording was.
        if self.bus.playback.is_none() && self.bus.tstates >= self.bus.frame_t() {
            self.bus.end_frame();
            self.frames_completed += 1;
        }

        if watching {
            let now = self.bus.total_t();
            let elapsed = now.saturating_sub(t0);
            let (pc1, sp1) = (self.cpu.pc, self.cpu.sp);
            if self.profiler.running {
                let bus = &self.bus;
                self.profiler
                    .on_instruction(pc0, sp0, pc1, sp1, now, elapsed, |a| {
                        u16::from_le_bytes([bus.peek_raw(a), bus.peek_raw(a.wrapping_add(1))])
                    });
            }
            if self.bus.observer.enabled {
                let t = self.bus.tstates;
                self.bus.observer.set_frame_t(t);
                let registers = crate::observe::Registers {
                    af: self.cpu.af(),
                    bc: self.cpu.bc(),
                    de: self.cpu.de(),
                    hl: self.cpu.hl(),
                };
                // The observer lives on the bus, which is also what has to be
                // read to see what was pushed, so the memory is copied out
                // first rather than borrowing both at once.
                let stack_word = |a: u16| {
                    u16::from_le_bytes([
                        raw_peek(&self.bus, a),
                        raw_peek(&self.bus, a.wrapping_add(1)),
                    ])
                };
                let pushed = stack_word(sp1);
                let popped = stack_word(sp0);
                // The first two bytes of the instruction that ran: enough to
                // tell an unconditional jump from a conditional one, which the
                // stack pointer cannot say since neither touches it.
                let opcode = [
                    self.bus.peek_raw(pc0),
                    self.bus.peek_raw(pc0.wrapping_add(1)),
                ];
                self.bus.observer.flow.note(pc0, opcode, pc1);
                self.bus
                    .observer
                    .on_instruction(pc0, sp0, pc1, sp1, registers, opcode, |a| {
                        if a == sp1 {
                            pushed
                        } else {
                            popped
                        }
                    });
            }
        }
    }

    /// Run until `budget` T-states have been consumed, a breakpoint is hit, or
    /// slow-draw mode parks the CPU.
    pub fn run(&mut self, budget: u32) -> Stop {
        let frame_t = self.bus.frame_t();
        let mut spent = 0u32;
        while spent < budget {
            let before = self.bus.tstates;

            // The interrupt is taken before the next instruction is fetched,
            // so a watch on it stops with the handler's first instruction
            // still to run rather than after it.
            self.check_interrupt();
            if let Some(event) = self.bus.break_hit.take() {
                return Stop::Watched(event, self.cpu.pc);
            }

            let at = self.cpu.pc;
            self.step_instruction();
            let after = self.bus.tstates;
            spent += if after >= before {
                after - before
            } else {
                after + frame_t - before
            };

            if let Some(event) = self.bus.break_hit.take() {
                return Stop::Watched(event, at);
            }
            if self.bus.slow.enabled && self.bus.slow.hit {
                return Stop::SlowDraw;
            }
            let pc = self.cpu.pc;
            if self.temp_bp == Some(pc) {
                self.temp_bp = None;
                return Stop::Breakpoint(pc);
            }
            if self.breakpoints.contains(&pc) {
                return Stop::Breakpoint(pc);
            }
        }
        Stop::Budget
    }

    /// Run exactly `fetches` instructions, or until something stops it.
    ///
    /// A recording is measured in instructions, not in T-states, so playing
    /// one back means running the number it says and no more. Returns how many
    /// were actually run, so a stop part-way through a frame can be picked up
    /// where it left off.
    pub fn run_fetches(&mut self, fetches: u32) -> (Stop, u32) {
        let start = self.bus.fetches;
        let done_now = |bus: &SpectrumBus| bus.fetches.wrapping_sub(start);
        while done_now(&self.bus) < fetches {
            let done = done_now(&self.bus);
            self.check_interrupt();
            if let Some(event) = self.bus.break_hit.take() {
                return (Stop::Watched(event, self.cpu.pc), done);
            }
            let at = self.cpu.pc;
            self.step_instruction();
            let done = done_now(&self.bus);

            if let Some(event) = self.bus.break_hit.take() {
                return (Stop::Watched(event, at), done);
            }
            if self.bus.slow.enabled && self.bus.slow.hit {
                return (Stop::SlowDraw, done);
            }
            let pc = self.cpu.pc;
            if self.temp_bp == Some(pc) {
                self.temp_bp = None;
                return (Stop::Breakpoint(pc), done);
            }
            if self.breakpoints.contains(&pc) {
                return (Stop::Breakpoint(pc), done);
            }
        }
        // The true count, which can be more than was asked for: instructions
        // are run whole, and a prefixed one is two fetches or more. Reporting
        // the budget instead loses the overshoot, and a caller running a
        // recorded frame in several goes then thinks the frame has further to
        // run than it has — so it runs on and reads input that was never
        // recorded.
        (Stop::Budget, done_now(&self.bus))
    }

    /// True when the instruction at `pc` is one that "step over" should run to
    /// completion rather than enter.
    pub fn is_step_over_target(&self, pc: u16) -> bool {
        let op = self.bus.peek(pc);
        match op {
            // CALL nn, CALL cc,nn, RST n
            0xcd | 0xc4 | 0xcc | 0xd4 | 0xdc | 0xe4 | 0xec | 0xf4 | 0xfc => true,
            0xc7 | 0xcf | 0xd7 | 0xdf | 0xe7 | 0xef | 0xf7 | 0xff => true,
            // The repeating block instructions.
            0xed => matches!(
                self.bus.peek(pc.wrapping_add(1)),
                0xb0 | 0xb1 | 0xb2 | 0xb3 | 0xb8 | 0xb9 | 0xba | 0xbb
            ),
            _ => false,
        }
    }
}

impl SpectrumBus {
    /// Plug the stick in. There is one stick, so one joystick interface at a
    /// time: a choice that needs one fits it and takes the other off, and a
    /// key-wired choice — Interface 2, a cursor interface — takes both off.
    /// The Hardware window, the Input window, the preferences and the MCP
    /// server all come through here, so the two windows cannot disagree.
    pub fn set_joystick(&mut self, kind: crate::joystick::Kind) {
        use crate::hardware::Peripheral;
        let wanted = kind.interface();
        for interface in [
            Peripheral::KempstonJoystick,
            Peripheral::DkTronicsJoystick,
            Peripheral::DkTronicsProgrammable,
        ] {
            self.hardware.fit(interface, wanted == Some(interface));
        }
        if kind != self.joystick.kind {
            // Whatever was over is let go: a direction held on an interface
            // nobody is reading any more would be held for ever.
            self.joystick.release();
        }
        self.joystick.kind = kind;
    }

    /// What an IN gives back, before the recording is told about it.
    fn io_read_uncaptured(&mut self, port: u16) -> u8 {
        let sampled = self.contend_io(port);
        self.observer.on_port(port, false);
        if self.breaks.port_in {
            self.break_hit.get_or_insert(Event::In(port));
        }
        // A recording replaces the hardware, not just the keyboard: the
        // floating bus, the tape and the sound chip all read back what they
        // read back on the day.
        if let Some(playback) = &mut self.playback {
            let byte = playback.next();
            if self.breaks.ay && self.model.has_ay() && port & 0xc002 == 0xc000 {
                self.break_hit.get_or_insert(Event::Ay);
            }
            return byte;
        }
        // A joystick on a port answers before anything else looks: an
        // unattached read gives the floating bus, and a stick would never be
        // seen through it.
        // The AMX mouse's PIO answers ahead of a joystick: a Kempston stick at
        // $1F would share its port, and whoever fitted the mouse is using it.
        if let Some(byte) = self.amx.as_ref().and_then(|amx| amx.io_read(port)) {
            return byte;
        }
        if let Some(byte) = self.joystick.io_read(port) {
            return byte;
        }
        if self
            .hardware
            .fitted(crate::hardware::Peripheral::KempstonMouse)
        {
            if let Some(byte) = self.mouse.io_read(port) {
                return byte;
            }
        }
        if crate::printer::ZxPrinter::decodes(port) {
            if let Some(printer) = &self.printer {
                return printer.read(self.total_t(), self.model.frame_t() as u64);
            }
        }
        // The µSpeech decodes the address bus and does not care that this is
        // an I/O cycle: $0038 turns it over, and its registers answer.
        if self.uspeech.is_some() {
            let busy = self.audio.speech.as_ref().is_some_and(|chip| chip.busy());
            if let Some(uspeech) = &mut self.uspeech {
                if let Some(byte) = uspeech.io_read(port, busy) {
                    return byte;
                }
            }
        }
        // A Multiface pages itself in and out on a read of its own port, so
        // this is asked before anything else that decodes the low bits.
        for mf in &mut self.multifaces {
            if let Some(byte) = mf.io_read(port) {
                return byte;
            }
        }
        // The Interface 1: $E7 is the microdrive's data register and $EF its
        // control and status one. Decoded on the low bits, as the interface
        // does — it watches A0-A4 and nothing else.
        if self.if1.is_some() {
            let now = self.total_t();
            if port & 0x0018 == 0x0000 {
                if let Some(if1) = &mut self.if1 {
                    if1.at(now);
                    return if1.read_data();
                }
            }
            if port & 0x0018 == 0x0008 {
                if let Some(if1) = &mut self.if1 {
                    if1.at(now);
                    return if1.read_status();
                }
            }
        }
        // The disk controller: $2FFD is its status register and $3FFD its
        // data register. Both are read; only the data register is written.
        if self.model.has_disk() && port & 0xf002 == 0x2000 {
            let now = self.total_t();
            self.fdc.at(now);
            return self.fdc.status();
        }
        if self.model.has_disk() && port & 0xf002 == 0x3000 {
            let now = self.total_t();
            self.fdc.at(now);
            return self.fdc.read();
        }
        // AY register read: $FFFD.
        if self.model.has_ay() && port & 0xc002 == 0xc000 {
            if self.breaks.ay {
                self.break_hit.get_or_insert(Event::Ay);
            }
            return self.audio.ay.read();
        }
        if port & 1 == 0 {
            let playing = self.tape.as_ref().is_some_and(|t| t.playing);
            // At the T-state the ULA put the byte on the bus, not at the end
            // of the instruction: the contention stall comes before that
            // point, so reading the tape afterwards samples it late by however
            // much the ULA happened to stall this particular read.
            let at = self.frame * self.model.frame_t() as u64 + sampled as u64;
            let ear = if playing {
                self.tape_level_at(at)
            } else {
                self.ear_feedback()
            };
            self.keyboard(port, ear)
        } else {
            self.floating_bus(sampled)
        }
    }
}
