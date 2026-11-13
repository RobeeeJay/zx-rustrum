//! The +3's disk controller: a µPD765A, as far as +3DOS can tell.
//!
//! The chip is driven through two ports — a status register the program polls
//! and a data register it pushes bytes at — and every command is the same
//! three phases: the program writes a command and its parameters, then the
//! data goes one way or the other, then the program reads the result bytes
//! back. What is here is that state machine and the commands +3DOS uses.
//!
//! Nothing is timed. A real controller makes the program wait while the head
//! steps and the disk turns, and reports "not ready" until the motor is up to
//! speed; this one answers at once. +3DOS polls rather than counting, so it
//! cannot tell — but a program that measures the wait could, and that is worth
//! knowing before trusting this with a protected disk.

use crate::disk::Disk;

/// How the drive behaves about time.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Speed {
    /// The waits a real drive makes the program sit through: the motor coming
    /// up to speed, the head stepping from track to track, and the sector
    /// coming round under it. A game that loads in eleven seconds on a real +3
    /// loads in eleven seconds here.
    Normal,
    /// No waits at all: every answer is ready the moment it is asked for.
    /// A disk load takes as long as the ROM's own code takes to run.
    #[default]
    Fastload,
}

/// What the waits are, in T-states of a 3.5469MHz +3.
///
/// A three-inch drive turns at 300rpm, so a revolution is 200ms and one of the
/// nine sectors comes round every 22ms. The motor takes about a second to come
/// up to speed from stopped, and the head steps at the rate the program asked
/// for in SPECIFY — 3ms a track by default, which is what +3DOS sets.
pub mod delay {
    /// A second, near enough, for the motor to reach speed.
    pub const MOTOR_UP: u64 = 3_546_900;
    /// One sector coming round under the head.
    pub const SECTOR: u64 = 3_546_900 / 45;
    /// One track of head movement.
    pub const STEP: u64 = 3_546_900 / 333;
    /// Settling after the head has finished moving.
    pub const SETTLE: u64 = 3_546_900 / 66;
}

/// Main status register bits, as the datasheet names them.
pub const RQM: u8 = 0x80;
pub const DIO: u8 = 0x40;
pub const EXM: u8 = 0x20;
pub const CB: u8 = 0x10;

/// A disk in a drive, and what may be done to it.
#[derive(Clone)]
pub struct Drive {
    pub disk: Disk,
    /// Where it came from, and where writes go. None for a disk that has never
    /// been saved.
    pub path: Option<std::path::PathBuf>,
    /// A disk mounted read-only takes writes as far as the controller is
    /// concerned and refuses them at the surface, which is what a write-
    /// protected disk does: the program is told, rather than the write quietly
    /// going nowhere.
    pub write_protected: bool,
}

impl Drive {
    pub fn new(disk: Disk, path: Option<std::path::PathBuf>, write_protected: bool) -> Drive {
        Drive {
            disk,
            path,
            write_protected,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Waiting for a command byte, or for the parameters of one.
    Command,
    /// Handing bytes to the program.
    Reading,
    /// Taking bytes from it.
    Writing,
    /// Taking the four bytes an ID field is made of, per sector, while
    /// formatting.
    Formatting,
    /// Handing back the seven bytes that say how it went.
    Result,
}

#[derive(Clone)]
pub struct Fdc {
    /// A: and B:. The +3 has one drive built in and a socket for another.
    pub drives: [Option<Drive>; 2],
    /// Whether the motor is turning, from bit 3 of port $1FFD.
    pub motor: bool,
    /// Whether the drive makes the program wait, and for how long.
    pub speed: Speed,
    /// The machine's clock, as the bus last told it, and the T-state before
    /// which the controller has nothing to say. The chip has no clock of its
    /// own here: it is handed the time on every port access, which is the only
    /// moment it can matter.
    pub now: u64,
    busy_until: u64,
    /// When the motor was last switched on, so the spin-up wait is only paid
    /// once rather than on every command.
    motor_since: Option<u64>,
    /// What the drive has been doing lately, per sector, for the window to
    /// draw: 0..=255, refreshed on access and faded every frame.
    pub reads: std::collections::BTreeMap<(u8, u8, u8), u8>,
    pub writes: std::collections::BTreeMap<(u8, u8, u8), u8>,
    /// The last sector touched, and whether it was written: what the light on
    /// the front of the drive is showing.
    pub last_access: Option<(u8, u8, u8, bool)>,
    /// T-state of that access, so the light goes out again.
    pub last_access_at: u64,
    /// Commands completed, and how many of them ended abnormally. Kept
    /// because "did the machine actually talk to the disk" is otherwise
    /// invisible from outside, and it is the first thing worth knowing when a
    /// disk will not load.
    pub commands: u32,
    pub errors: u32,
    /// The last command byte, for the same reason.
    pub last_command: Option<u8>,

    phase: Phase,
    /// The command and its parameters as they arrive.
    command: Vec<u8>,
    /// How many parameter bytes the command in hand still wants.
    wanted: usize,
    /// Bytes going to or from the program.
    buffer: Vec<u8>,
    at: usize,
    /// Where the heads are, per drive.
    pcn: [u8; 2],
    /// Which drive and head the command in hand is about.
    unit: usize,
    head: u8,
    /// Set by a seek or recalibrate, read back by SENSE INTERRUPT STATUS.
    seek_done: Option<u8>,
    /// While a read or write is going on: which sector is being transferred,
    /// and where it ends.
    sector: Option<(u8, u8, u8)>,
    last_sector: u8,
    /// The result bytes, once there is something to say.
    result: Vec<u8>,
    /// What the sector being transferred says about itself, to go in the
    /// result when it has been handed over.
    pending: (u8, u8),
    /// While formatting: the track being written and the sectors so far.
    formatting: Option<(u8, u8, u8, u8, u8)>,
    format_sectors: Vec<[u8; 4]>,
}

impl Default for Fdc {
    fn default() -> Self {
        Fdc::new()
    }
}

impl Fdc {
    pub fn new() -> Fdc {
        Fdc {
            drives: [None, None],
            motor: false,
            speed: Speed::default(),
            now: 0,
            busy_until: 0,
            motor_since: None,
            reads: std::collections::BTreeMap::new(),
            writes: std::collections::BTreeMap::new(),
            last_access: None,
            last_access_at: 0,
            commands: 0,
            errors: 0,
            last_command: None,
            phase: Phase::Command,
            command: Vec::new(),
            wanted: 0,
            buffer: Vec::new(),
            at: 0,
            pcn: [0, 0],
            unit: 0,
            head: 0,
            seek_done: None,
            sector: None,
            last_sector: 0,
            result: Vec::new(),
            pending: (0, 0),
            formatting: None,
            format_sectors: Vec::new(),
        }
    }

    /// What the reset line does to the controller.
    ///
    /// The disks stay in the drives and the speed stays as it was set; the
    /// state machine goes back to waiting for a command. Two things went wrong
    /// without this. A machine reset in the middle of a command left the
    /// controller handing over data nobody was going to take, so the ROM's
    /// next command found it talking rather than listening. And the machine's
    /// clock goes back to zero on a reset, so a wait timed against it — the
    /// motor coming up to speed, a sector coming round — was left ending
    /// millions of T-states in the future, and the controller reported itself
    /// busy for as long as it took the clock to catch up. Either way the ROM
    /// sat at $211A polling the status register until it gave up, which is
    /// twenty-two seconds of a machine that looks dead.
    pub fn reset(&mut self) {
        self.phase = Phase::Command;
        self.command.clear();
        self.wanted = 0;
        self.buffer.clear();
        self.at = 0;
        self.pcn = [0, 0];
        self.unit = 0;
        self.head = 0;
        self.seek_done = None;
        self.sector = None;
        self.last_sector = 0;
        self.result.clear();
        self.pending = (0, 0);
        self.formatting = None;
        self.format_sectors.clear();
        self.motor = false;
        self.motor_since = None;
        self.now = 0;
        self.busy_until = 0;
        self.last_access = None;
        self.last_access_at = 0;
        self.reads.clear();
        self.writes.clear();
    }

    /// Tell the controller what time it is. The bus does this on every port
    /// access, which is the only moment the time can matter.
    pub fn at(&mut self, now: u64) {
        // The machine's clock can go backwards — a reset puts it to zero, a
        // snapshot puts it wherever it was saved — and a wait timed against a
        // clock that has moved under it is not a wait at all.
        if now < self.now {
            self.busy_until = 0;
            self.motor_since = None;
        }
        self.now = now;
        if self.motor {
            self.motor_since.get_or_insert(now);
        } else {
            self.motor_since = None;
        }
    }

    /// Whether the drive is still working on the last thing it was asked.
    ///
    /// At Fastload nothing is ever busy. At Normal the program polls the
    /// status register until the wait is over, which is what it does on a real
    /// machine and why a disk load takes the time it takes.
    pub fn busy(&self) -> bool {
        self.speed == Speed::Normal && self.now < self.busy_until
    }

    /// Make the program wait, from now.
    fn wait(&mut self, t: u64) {
        if self.speed == Speed::Normal {
            self.busy_until = self.now.max(self.busy_until) + t;
        }
    }

    /// What is left of the motor coming up to speed, if it was started
    /// recently. Paid once rather than on every command.
    fn spin_up(&self) -> u64 {
        match self.motor_since {
            Some(since) => delay::MOTOR_UP.saturating_sub(self.now.saturating_sub(since)),
            None => 0,
        }
    }

    /// The status register the program polls: whether the controller wants a
    /// byte, which way the data is going, and whether it is busy.
    pub fn status(&self) -> u8 {
        // Still working: busy, and not asking for anything. The program polls
        // this until the drive has caught up.
        if self.busy() {
            return CB;
        }
        match self.phase {
            Phase::Command => RQM | if self.command.is_empty() { 0 } else { CB },
            Phase::Reading => RQM | DIO | EXM | CB,
            Phase::Writing | Phase::Formatting => RQM | EXM | CB,
            Phase::Result => RQM | DIO | CB,
        }
    }

    /// Whether a drive has a disk in it and the motor is turning. A drive with
    /// no disk is not ready, which is how +3DOS knows to ask for one.
    fn ready(&self, unit: usize) -> bool {
        self.motor && self.drives.get(unit).map(|d| d.is_some()).unwrap_or(false)
    }

    /// A byte written to the data register.
    pub fn write(&mut self, value: u8) {
        match self.phase {
            Phase::Command => self.take_command_byte(value),
            Phase::Writing => {
                // `at` holds how long the sector is; the buffer was made that
                // size when the write began.
                self.buffer.push(value);
                if self.buffer.len() == self.at {
                    self.finish_write();
                }
            }
            Phase::Formatting => {
                self.buffer.push(value);
                if self.buffer.len().is_multiple_of(4) {
                    let n = self.buffer.len();
                    let id = [
                        self.buffer[n - 4],
                        self.buffer[n - 3],
                        self.buffer[n - 2],
                        self.buffer[n - 1],
                    ];
                    self.format_sectors.push(id);
                    if let Some((_, _, sectors, _, _)) = self.formatting {
                        if self.format_sectors.len() >= sectors as usize {
                            self.finish_format();
                        }
                    }
                }
            }
            // A byte written while the controller is talking is thrown away,
            // which is what the chip does.
            Phase::Reading | Phase::Result => {}
        }
    }

    /// A byte read from the data register.
    pub fn read(&mut self) -> u8 {
        match self.phase {
            Phase::Reading => {
                let byte = self.buffer.get(self.at).copied().unwrap_or(0);
                self.at += 1;
                if self.at >= self.buffer.len() {
                    self.finish_read();
                }
                byte
            }
            Phase::Result => {
                let byte = self.result.first().copied().unwrap_or(0);
                if !self.result.is_empty() {
                    self.result.remove(0);
                }
                if self.result.is_empty() {
                    self.phase = Phase::Command;
                }
                byte
            }
            _ => 0,
        }
    }

    /// Note that a sector was read or written, for the drive light and the
    /// map of what the program has been touching.
    fn touched(&mut self, r: u8, written: bool) {
        let track = self.pcn[self.unit.min(1)];
        let key = (track, self.head, r);
        let heat = if written {
            &mut self.writes
        } else {
            &mut self.reads
        };
        heat.insert(key, 255);
        self.last_access = Some((track, self.head, r, written));
        self.last_access_at = self.now;
    }

    /// Fade the map, once a frame. What was touched a moment ago is bright and
    /// what was touched a while back is dim, which is what makes a load look
    /// like a load rather than like a list of sectors.
    /// Whether any sector is still lit, for a paused window to repaint for.
    pub fn fade(&mut self) -> bool {
        const STEP: u8 = 6;
        for heat in [&mut self.reads, &mut self.writes] {
            heat.retain(|_, v| {
                *v = v.saturating_sub(STEP);
                *v > 0
            });
        }
        !self.reads.is_empty() || !self.writes.is_empty()
    }

    /// Where the head is on a drive, which is the track the next read comes
    /// from.
    pub fn head_at(&self, unit: usize) -> u8 {
        self.pcn[unit.min(1)]
    }

    /// Whether the light on the front of the drive is lit: something has been
    /// read or written in the last little while.
    pub fn light(&self) -> bool {
        const LIT: u64 = 3_546_900 / 20;
        self.last_access.is_some() && self.now.saturating_sub(self.last_access_at) < LIT
    }

    /// How many parameter bytes each command takes, and whether it is one this
    /// controller knows.
    fn parameters(command: u8) -> Option<usize> {
        Some(match command & 0x1F {
            0x02 => 8, // READ TRACK
            0x03 => 2, // SPECIFY
            0x04 => 1, // SENSE DRIVE STATUS
            0x05 => 8, // WRITE DATA
            0x06 => 8, // READ DATA
            0x07 => 1, // RECALIBRATE
            0x08 => 0, // SENSE INTERRUPT STATUS
            0x09 => 8, // WRITE DELETED DATA
            0x0A => 1, // READ ID
            0x0C => 8, // READ DELETED DATA
            0x0D => 5, // FORMAT TRACK
            0x0F => 2, // SEEK
            0x11 => 8, // SCAN EQUAL
            0x19 => 8, // SCAN LOW OR EQUAL
            0x1D => 8, // SCAN HIGH OR EQUAL
            _ => return None,
        })
    }

    fn take_command_byte(&mut self, value: u8) {
        if self.command.is_empty() {
            match Fdc::parameters(value) {
                Some(wanted) => {
                    self.command.push(value);
                    self.wanted = wanted;
                    if wanted == 0 {
                        self.execute();
                    }
                }
                // An invalid command is answered with $80 in ST0, which is
                // what the chip does rather than ignoring it.
                None => {
                    self.result = vec![0x80];
                    self.phase = Phase::Result;
                }
            }
            return;
        }
        self.command.push(value);
        if self.command.len() > self.wanted {
            self.execute();
        }
    }

    fn execute(&mut self) {
        let command = self.command[0];
        self.commands += 1;
        self.last_command = Some(command);
        let opcode = command & 0x1F;
        match opcode {
            0x03 => self.done_without_result(), // SPECIFY
            0x04 => self.sense_drive_status(),
            0x07 => self.recalibrate(),
            0x08 => self.sense_interrupt(),
            0x0A => self.read_id(),
            0x0F => self.seek(),
            0x05 | 0x09 => self.start_write(),
            0x06 | 0x0C | 0x02 => self.start_read(),
            0x0D => self.start_format(),
            _ => {
                // Something known but not implemented: say the command
                // finished abnormally rather than hanging the program.
                self.result = vec![0x40 | self.unit as u8, 0, 0, 0, 0, 0, 0];
                self.phase = Phase::Result;
                self.command.clear();
            }
        }
    }

    fn done_without_result(&mut self) {
        self.command.clear();
        self.phase = Phase::Command;
    }

    fn unit_and_head(&mut self) {
        let byte = self.command.get(1).copied().unwrap_or(0);
        self.unit = (byte & 0x03) as usize;
        self.head = (byte >> 2) & 1;
    }

    /// ST0: the unit, the head, and how the command ended.
    fn st0(&self, end: u8) -> u8 {
        end | ((self.head & 1) << 2)
            | (self.unit as u8 & 3)
            | if self.ready(self.unit) { 0 } else { 0x08 }
    }

    fn sense_drive_status(&mut self) {
        self.unit_and_head();
        let ready = self.ready(self.unit);
        let protected = self
            .drives
            .get(self.unit)
            .and_then(|d| d.as_ref())
            .map(|d| d.write_protected)
            .unwrap_or(false);
        // ST3: unit, head, two-sided, track 0, ready, write protected.
        let mut st3 = (self.unit as u8 & 3) | ((self.head & 1) << 2);
        if self.pcn[self.unit.min(1)] == 0 {
            st3 |= 0x10;
        }
        if ready {
            st3 |= 0x20;
        }
        if protected {
            st3 |= 0x40;
        }
        self.result = vec![st3];
        self.phase = Phase::Result;
        self.command.clear();
    }

    fn recalibrate(&mut self) {
        self.unit_and_head();
        // Stepping out to track 0 from wherever the head is.
        let moved = self.pcn[self.unit.min(1)] as u64;
        self.wait(moved * delay::STEP + delay::SETTLE);
        self.pcn[self.unit.min(1)] = 0;
        // Seek End, and Equipment Check when there is no disk to find track 0
        // on: that is how the ROM knows the drive is empty.
        let end = if self.ready(self.unit) { 0x20 } else { 0x70 };
        self.seek_done = Some(self.st0(end));
        self.done_without_result();
    }

    fn seek(&mut self) {
        self.unit_and_head();
        let to = self.command.get(2).copied().unwrap_or(0);
        let from = self.pcn[self.unit.min(1)];
        let moved = to.abs_diff(from) as u64;
        self.wait(moved * delay::STEP + if moved > 0 { delay::SETTLE } else { 0 });
        self.pcn[self.unit.min(1)] = to;
        let end = if self.ready(self.unit) { 0x20 } else { 0x60 };
        self.seek_done = Some(self.st0(end));
        self.done_without_result();
    }

    fn sense_interrupt(&mut self) {
        match self.seek_done.take() {
            Some(st0) => {
                let pcn = self.pcn[self.unit.min(1)];
                self.result = vec![st0, pcn];
            }
            // Nothing to report: $80, invalid, which is how a polling loop
            // knows to stop asking.
            None => self.result = vec![0x80],
        }
        self.phase = Phase::Result;
        self.command.clear();
    }

    fn read_id(&mut self) {
        self.unit_and_head();
        let track = self.pcn[self.unit.min(1)];
        let head = self.head;
        let found = self
            .drives
            .get(self.unit)
            .and_then(|d| d.as_ref())
            .and_then(|d| d.disk.track(track, head))
            .and_then(|t| t.sectors.first())
            .map(|s| (s.c, s.h, s.r, s.n));
        match found {
            Some((c, h, r, n)) => {
                let st0 = self.st0(0);
                self.result = vec![st0, 0, 0, c, h, r, n];
            }
            None => {
                // Missing address mark: there is nothing readable there.
                let st0 = self.st0(0x40);
                self.result = vec![st0, 0x01, 0, track, head, 0, 0];
            }
        }
        self.phase = Phase::Result;
        self.command.clear();
    }

    /// The eight parameters a read or write carries: unit/head, C, H, R, N,
    /// EOT, GPL, DTL.
    fn transfer_parameters(&mut self) -> (u8, u8, u8, u8, u8) {
        self.unit_and_head();
        let c = self.command.get(2).copied().unwrap_or(0);
        let h = self.command.get(3).copied().unwrap_or(0);
        let r = self.command.get(4).copied().unwrap_or(0);
        let n = self.command.get(5).copied().unwrap_or(0);
        let eot = self.command.get(6).copied().unwrap_or(r);
        (c, h, r, n, eot)
    }

    fn start_read(&mut self) {
        let (c, h, r, _n, eot) = self.transfer_parameters();
        self.last_sector = eot;
        self.touched(r, false);
        // The sector has to come round under the head, and the motor has to be
        // up to speed before any of it can be read.
        let wait = self.spin_up() + delay::SECTOR;
        self.wait(wait);
        match self.sector_data(c, h, r) {
            Some(data) => {
                // A sector whose mark is not the one the command asked for —
                // a deleted one for READ DATA, an ordinary one for READ
                // DELETED DATA — is handed over anyway with the Control Mark
                // set, unless the command said to skip it. Half of the disk
                // protections there are turn on this.
                let (st1, st2) = self.sector_status(c, h, r);
                let deleted = st2 & 0x40 != 0;
                let wanted_deleted = self.command[0] & 0x1F == 0x0C;
                let skipping = self.command[0] & 0x20 != 0;
                if deleted != wanted_deleted && skipping {
                    // Skipped: on to the next one, or the end of the command.
                    if r < self.last_sector {
                        self.command[4] = r + 1;
                        return self.start_read();
                    }
                    self.transfer_result(st1, st2 & 0x40, c, h, r.wrapping_add(1), 2);
                    return;
                }
                self.pending = (
                    st1,
                    if deleted != wanted_deleted {
                        st2
                    } else {
                        st2 & !0x40
                    },
                );
                self.buffer = data;
                self.at = 0;
                self.sector = Some((c, h, r));
                self.phase = Phase::Reading;
            }
            None => self.transfer_failed(c, h, r),
        }
    }

    /// What the disk records against a sector: the status bytes a controller
    /// would give back after reading it.
    ///
    /// A preserved disk carries these — a deleted data mark, a CRC that does
    /// not check out — and they are the whole of some protections. Reporting
    /// a clean read of a sector the disk says is marked deleted is telling the
    /// program the disk is a copy.
    fn sector_status(&self, c: u8, h: u8, r: u8) -> (u8, u8) {
        self.sector_at(c, h, r)
            .map(|s| (s.st1, s.st2))
            .unwrap_or((0, 0))
    }

    fn sector_at(&self, c: u8, h: u8, r: u8) -> Option<&crate::disk::Sector> {
        let drive = self.drives.get(self.unit)?.as_ref()?;
        let track = drive.disk.track(self.pcn[self.unit.min(1)], self.head)?;
        track
            .sectors
            .iter()
            .find(|s| s.r == r && s.c == c && (s.h == h || h == 0))
    }

    fn sector_data(&self, c: u8, h: u8, r: u8) -> Option<Vec<u8>> {
        let drive = self.drives.get(self.unit)?.as_ref()?;
        let track = drive.disk.track(self.pcn[self.unit.min(1)], self.head)?;
        // Matched on the identity in the address mark, not on where the sector
        // sits: a disk that numbers its sectors oddly is read correctly, and
        // one that lies about its cylinder is not.
        let sector = track
            .sectors
            .iter()
            .find(|s| s.r == r && s.c == c && (s.h == h || h == 0))?;
        Some(sector.data.clone())
    }

    /// The seven result bytes a read or write ends with.
    fn transfer_result(&mut self, st1: u8, st2: u8, c: u8, h: u8, r: u8, n: u8) {
        let end = if st1 == 0 && st2 == 0 { 0 } else { 0x40 };
        if end != 0 {
            self.errors += 1;
        }
        let st0 = self.st0(end);
        self.result = vec![st0, st1, st2, c, h, r, n];
        self.phase = Phase::Result;
        self.command.clear();
        self.sector = None;
    }

    fn transfer_failed(&mut self, c: u8, h: u8, r: u8) {
        // No data: either the drive is empty or the sector is not there.
        let st1 = if self.ready(self.unit) { 0x04 } else { 0x00 };
        self.transfer_result(st1, 0, c, h, r, 2);
    }

    fn finish_read(&mut self) {
        let Some((c, h, r)) = self.sector else {
            self.phase = Phase::Command;
            return;
        };
        // A sector that was not the kind the command asked for, or whose CRC
        // is wrong, ends the command where it is: the controller stops rather
        // than reading on, and says why.
        let (st1, st2) = self.pending;
        if st1 != 0 || st2 != 0 {
            self.pending = (0, 0);
            self.transfer_result(st1, st2, c, h, r.wrapping_add(1), 2);
            return;
        }
        // A multi-sector read carries on to the next one until it reaches the
        // last the command named.
        if r < self.last_sector {
            let next = r + 1;
            if let Some(data) = self.sector_data(c, h, next) {
                let (st1, st2) = self.sector_status(c, h, next);
                let deleted = st2 & 0x40 != 0;
                let wanted_deleted = self.command[0] & 0x1F == 0x0C;
                self.pending = (
                    st1,
                    if deleted != wanted_deleted {
                        st2
                    } else {
                        st2 & !0x40
                    },
                );
                self.touched(next, false);
                self.buffer = data;
                self.at = 0;
                self.sector = Some((c, h, next));
                return;
            }
        }
        self.transfer_result(0, 0, c, h, r.wrapping_add(1), 2);
    }

    fn start_write(&mut self) {
        let (c, h, r, n, eot) = self.transfer_parameters();
        self.last_sector = eot;
        let protected = self
            .drives
            .get(self.unit)
            .and_then(|d| d.as_ref())
            .map(|d| d.write_protected)
            .unwrap_or(false);
        if protected {
            // ST1 bit 1 is Not Writable, which is what a write-protected disk
            // answers. The program is told rather than the write going
            // nowhere quietly.
            self.transfer_result(0x02, 0, c, h, r, n);
            return;
        }
        self.touched(r, true);
        let wait = self.spin_up() + delay::SECTOR;
        self.wait(wait);
        let length = self
            .sector_data(c, h, r)
            .map(|d| d.len())
            .unwrap_or(128usize << (n.min(6) as usize));
        if self.sector_data(c, h, r).is_none() {
            self.transfer_failed(c, h, r);
            return;
        }
        self.buffer = Vec::with_capacity(length);
        self.at = length;
        self.sector = Some((c, h, r));
        self.phase = Phase::Writing;
    }

    fn finish_write(&mut self) {
        let Some((c, h, r)) = self.sector else {
            self.phase = Phase::Command;
            return;
        };
        let track_no = self.pcn[self.unit.min(1)];
        let head = self.head;
        let bytes = std::mem::take(&mut self.buffer);
        if let Some(drive) = self.drives.get_mut(self.unit).and_then(|d| d.as_mut()) {
            let mut wrote = false;
            if let Some(track) = drive.disk.track_mut(track_no, head) {
                let filler = track.filler;
                if let Some(sector) = track
                    .sectors
                    .iter_mut()
                    .find(|s| s.r == r && s.c == c && (s.h == h || h == 0))
                {
                    // However many bytes the program handed over, the sector
                    // is the length it was: a short write leaves the rest of
                    // it as the disk was formatted.
                    let length = sector.data.len();
                    sector.data = bytes;
                    sector.data.resize(length, filler);
                    wrote = true;
                }
            }
            drive.disk.dirty |= wrote;
            if wrote {
                drive.disk.revision += 1;
            }
        }
        if r < self.last_sector {
            let next = r + 1;
            if let Some(data) = self.sector_data(c, h, next) {
                self.at = data.len();
                self.buffer = Vec::with_capacity(self.at);
                self.sector = Some((c, h, next));
                return;
            }
        }
        self.transfer_result(0, 0, c, h, r.wrapping_add(1), 2);
    }

    fn start_format(&mut self) {
        self.unit_and_head();
        let n = self.command.get(2).copied().unwrap_or(2);
        let sectors = self.command.get(3).copied().unwrap_or(9);
        let gap3 = self.command.get(4).copied().unwrap_or(0x4E);
        let filler = self.command.get(5).copied().unwrap_or(crate::disk::FILLER);
        let protected = self
            .drives
            .get(self.unit)
            .and_then(|d| d.as_ref())
            .map(|d| d.write_protected)
            .unwrap_or(false);
        if protected || !self.ready(self.unit) {
            let st1 = if protected { 0x02 } else { 0 };
            self.transfer_result(st1, 0, 0, 0, 0, n);
            return;
        }
        self.formatting = Some((n, sectors, sectors, gap3, filler));
        self.format_sectors.clear();
        self.buffer.clear();
        self.phase = Phase::Formatting;
    }

    fn finish_format(&mut self) {
        let Some((n, _, _, gap3, filler)) = self.formatting.take() else {
            self.phase = Phase::Command;
            return;
        };
        let track_no = self.pcn[self.unit.min(1)];
        let head = self.head;
        let ids = std::mem::take(&mut self.format_sectors);
        let length = 128usize << (n.min(6) as usize);
        if let Some(drive) = self.drives.get_mut(self.unit).and_then(|d| d.as_mut()) {
            let sectors: Vec<crate::disk::Sector> = ids
                .iter()
                .map(|id| crate::disk::Sector {
                    c: id[0],
                    h: id[1],
                    r: id[2],
                    n: id[3],
                    st1: 0,
                    st2: 0,
                    data: vec![filler; length],
                })
                .collect();
            match drive.disk.track_mut(track_no, head) {
                Some(track) => {
                    track.sector_size = n;
                    track.gap3 = gap3;
                    track.filler = filler;
                    track.sectors = sectors;
                }
                None => drive.disk.tracks.push(crate::disk::Track {
                    track: track_no,
                    side: head,
                    sector_size: n,
                    gap3,
                    filler,
                    sectors,
                }),
            }
            drive.disk.dirty = true;
            drive.disk.revision += 1;
        }
        self.buffer.clear();
        self.transfer_result(0, 0, track_no, head, 1, n);
    }
}
