//! TZX, TAP and ZX81 (.p/.81/.p81) tape loading and playback.
//!
//! TZX is a pulse-level format, so the player does not decode bytes: it turns
//! blocks into a stream of pulse lengths in T-states and drives the EAR bit,
//! exactly like a real tape feeding the ULA. That means ordinary ROM loading,
//! turbo loaders and custom pulse schemes all work through the same path.

use std::collections::VecDeque;
use std::path::Path;

/// ROM loader pulse lengths, in T-states.
pub const PILOT_PULSE: u16 = 2168;
pub const SYNC1_PULSE: u16 = 667;
pub const SYNC2_PULSE: u16 = 735;
pub const ZERO_PULSE: u16 = 855;
pub const ONE_PULSE: u16 = 1710;
/// Pilot tone length: longer for headers (flag < 128) than for data blocks.
pub const HEADER_PILOT_PULSES: u16 = 8063;
pub const DATA_PILOT_PULSES: u16 = 3223;

const T_PER_MS: u32 = 3500;

/// ZX81 tape timings, in ZX81 T-states (its clock is 3.25 MHz, so these are
/// not interchangeable with the Spectrum figures above — but a ZX81 block only
/// ever plays into a ZX81).
///
/// Taken from the ROM's own SAVE routine at $031E: it computes the pulse count
/// per bit with `AND $05 / ADD A,$04`, giving nine pulses for a 1 and four for
/// a 0, then delays roughly 150 µs between level changes and about 1300 µs
/// after the last pulse of each bit.
pub const ZX81_HALF_PULSE: u16 = 488; // 150 µs
pub const ZX81_BIT_GAP: u16 = 4225; // 1300 µs
pub const ZX81_ZERO_PULSES: u8 = 4;
pub const ZX81_ONE_PULSES: u8 = 9;

#[derive(Clone, Debug)]
pub enum Block {
    /// ID $10, and every block of a .tap file.
    Standard {
        pause_ms: u16,
        data: Vec<u8>,
    },
    /// ID $11.
    Turbo {
        pilot: u16,
        sync1: u16,
        sync2: u16,
        zero: u16,
        one: u16,
        pilot_pulses: u16,
        used_bits: u8,
        pause_ms: u16,
        data: Vec<u8>,
    },
    /// ID $12.
    PureTone {
        len: u16,
        count: u16,
    },
    /// ID $13.
    Pulses(Vec<u16>),
    /// ID $14.
    PureData {
        zero: u16,
        one: u16,
        used_bits: u8,
        pause_ms: u16,
        data: Vec<u8>,
    },
    /// ID $15.
    Direct {
        t_per_sample: u16,
        pause_ms: u16,
        used_bits: u8,
        data: Vec<u8>,
    },
    /// ID $20. A pause of 0 means "stop the tape".
    Pause(u16),
    /// ID $21 / $22.
    GroupStart(String),
    GroupEnd,
    /// ID $23.
    Jump(i16),
    /// ID $24 / $25.
    LoopStart(u16),
    LoopEnd,
    /// ID $26 / $27.
    CallSequence(Vec<i16>),
    Return,
    /// ID $2A.
    StopIf48k,
    /// ID $2B.
    SetLevel(bool),
    /// A ZX81 file: the name in ZX81 character codes (the last one with bit 7
    /// set) followed by RAM from $4009 up. Both parts are one continuous bit
    /// stream on tape, so they are one block here; `name` is only for display.
    Zx81 {
        name: String,
        data: Vec<u8>,
        pause_ms: u16,
    },
    /// Informational blocks: $30, $31, $32, $33, $35, $5A.
    Info(String),
}

/// How a block's playing time divides up, in T-states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Segments {
    pub pilot: u64,
    pub sync: u64,
    pub data: u64,
    pub pause: u64,
}

impl Segments {
    pub fn total(&self) -> u64 {
        self.pilot + self.sync + self.data + self.pause
    }
}

impl Block {
    /// One-line description for the tape window.
    pub fn describe(&self) -> String {
        match self {
            Block::Standard { data, pause_ms } => {
                format!("Standard  {:5} bytes  {}", data.len(), header_summary(data))
                    + &format!("  (pause {pause_ms}ms)")
            }
            Block::Turbo { data, pilot, .. } => {
                format!("Turbo     {:5} bytes  pilot {pilot}T", data.len())
            }
            Block::PureTone { len, count } => format!("Pure tone {count} x {len}T"),
            Block::Pulses(p) => format!("Pulses    {} pulses", p.len()),
            Block::PureData { data, .. } => format!("Pure data {:5} bytes", data.len()),
            Block::Direct { data, .. } => format!("Direct    {:5} bytes", data.len()),
            Block::Pause(ms) => {
                if *ms == 0 {
                    "Stop the tape".into()
                } else {
                    format!("Pause     {ms}ms")
                }
            }
            Block::GroupStart(s) => format!("Group: {s}"),
            Block::GroupEnd => "Group end".into(),
            Block::Jump(n) => format!("Jump {n:+}"),
            Block::LoopStart(n) => format!("Loop start x{n}"),
            Block::LoopEnd => "Loop end".into(),
            Block::CallSequence(v) => format!("Call sequence ({})", v.len()),
            Block::Return => "Return".into(),
            Block::StopIf48k => "Stop if 48K".into(),
            Block::SetLevel(l) => format!("Set signal level {}", *l as u8),
            Block::Zx81 { name, data, .. } => {
                format!("ZX81      {:5} bytes  \"{}\"", data.len(), name)
            }
            Block::Info(s) => format!("Info: {s}"),
        }
    }

    /// How long the block takes to play, in T-states, split into the pilot
    /// tone, the sync pulses, the data itself and the pause that follows.
    ///
    /// The data figure assumes an even mix of 0 and 1 bits, which is what a
    /// progress bar needs; it is not used for playback, which times every
    /// pulse individually.
    pub fn segment_times(&self) -> Segments {
        let bits = |data: &[u8], used_bits: u8| -> u64 {
            let full = data.len().saturating_sub(1) as u64 * 8;
            let last = if (1..8).contains(&used_bits) {
                used_bits as u64
            } else {
                8
            };
            full + if data.is_empty() { 0 } else { last }
        };
        let pause = |ms: u16| ms as u64 * T_PER_MS as u64;

        match self {
            Block::Standard { data, pause_ms } => {
                let flag = data.first().copied().unwrap_or(0xff);
                let pilot_pulses = if flag < 0x80 {
                    HEADER_PILOT_PULSES
                } else {
                    DATA_PILOT_PULSES
                } as u64;
                Segments {
                    pilot: pilot_pulses * PILOT_PULSE as u64,
                    sync: SYNC1_PULSE as u64 + SYNC2_PULSE as u64,
                    data: bits(data, 8) * (ZERO_PULSE as u64 + ONE_PULSE as u64),
                    pause: pause(*pause_ms),
                }
            }
            Block::Turbo {
                pilot,
                sync1,
                sync2,
                zero,
                one,
                pilot_pulses,
                used_bits,
                pause_ms,
                data,
            } => Segments {
                pilot: *pilot_pulses as u64 * *pilot as u64,
                sync: *sync1 as u64 + *sync2 as u64,
                data: bits(data, *used_bits) * (*zero as u64 + *one as u64),
                pause: pause(*pause_ms),
            },
            Block::PureTone { len, count } => Segments {
                pilot: *count as u64 * *len as u64,
                sync: 0,
                data: 0,
                pause: 0,
            },
            Block::Pulses(p) => Segments {
                pilot: p.iter().map(|l| *l as u64).sum(),
                sync: 0,
                data: 0,
                pause: 0,
            },
            Block::PureData {
                zero,
                one,
                used_bits,
                pause_ms,
                data,
            } => Segments {
                pilot: 0,
                sync: 0,
                data: bits(data, *used_bits) * (*zero as u64 + *one as u64),
                pause: pause(*pause_ms),
            },
            Block::Direct {
                t_per_sample,
                pause_ms,
                used_bits,
                data,
            } => Segments {
                pilot: 0,
                sync: 0,
                data: bits(data, *used_bits) * *t_per_sample as u64,
                pause: pause(*pause_ms),
            },
            Block::Zx81 { data, pause_ms, .. } => {
                // Every bit is a burst of pulses then a gap. Assuming an even
                // mix of 0s and 1s, as the progress bar wants, that averages
                // 6.5 pulses; each pulse is two half-pulses.
                let bits = data.len() as u64 * 8;
                let per_bit = (ZX81_ZERO_PULSES + ZX81_ONE_PULSES) as u64
                    * ZX81_HALF_PULSE as u64 // 2 halves x average of the two counts
                    + ZX81_BIT_GAP as u64;
                Segments {
                    pilot: 0,
                    sync: 0,
                    data: bits * per_bit,
                    pause: pause(*pause_ms),
                }
            }
            Block::Pause(ms) => Segments {
                pilot: 0,
                sync: 0,
                data: 0,
                pause: pause(*ms),
            },
            _ => Segments::default(),
        }
    }

    /// Total time the block takes, in T-states.
    pub fn duration_t(&self) -> u64 {
        self.segment_times().total()
    }

    /// True for blocks that actually produce sound.
    pub fn is_data(&self) -> bool {
        matches!(
            self,
            Block::Standard { .. }
                | Block::Turbo { .. }
                | Block::PureTone { .. }
                | Block::Pulses(_)
                | Block::PureData { .. }
                | Block::Direct { .. }
                | Block::Zx81 { .. }
        )
    }
}

/// The ZX81 character set, for the eleven codes that matter here: a name is
/// only ever letters, digits and spaces.
fn zx81_char(c: char) -> Option<u8> {
    match c.to_ascii_uppercase() {
        ' ' => Some(0x00),
        c @ '0'..='9' => Some(0x1c + (c as u8 - b'0')),
        c @ 'A'..='Z' => Some(0x26 + (c as u8 - b'A')),
        _ => None,
    }
}

fn zx81_char_back(code: u8) -> char {
    match code & 0x7f {
        0x00 => ' ',
        c @ 0x1c..=0x25 => (b'0' + (c - 0x1c)) as char,
        c @ 0x26..=0x3f => (b'A' + (c - 0x26)) as char,
        _ => '?',
    }
}

/// Turn a host file name into ZX81 name bytes, with bit 7 set on the last one
/// as the ROM's loader expects. Unrepresentable characters are dropped, and a
/// name that comes out empty becomes "L" — `LOAD ""` takes whatever it finds
/// first, so the name only has to exist, not match.
pub fn zx81_name(stem: &str) -> Vec<u8> {
    let mut name: Vec<u8> = stem.chars().filter_map(zx81_char).take(127).collect();
    while name.last() == Some(&0x00) {
        name.pop(); // trailing spaces would be saved as part of the name
    }
    if name.is_empty() {
        name.push(zx81_char('L').expect("L is in the character set"));
    }
    *name.last_mut().expect("not empty") |= 0x80;
    name
}

/// Build the single block a ZX81 file plays as: the name, then the program.
pub fn zx81_block(name: &[u8], program: &[u8]) -> Block {
    let mut data = name.to_vec();
    data.extend_from_slice(program);
    Block::Zx81 {
        name: name.iter().map(|&c| zx81_char_back(c)).collect(),
        data,
        pause_ms: 1000,
    }
}

/// A .p81 already starts with the name, terminated by a byte with bit 7 set.
fn parse_p81(data: &[u8]) -> Result<Vec<Block>, String> {
    let end = data
        .iter()
        .take(128)
        .position(|b| b & 0x80 != 0)
        .ok_or("not a .p81: no end-of-name byte in the first 128 bytes")?;
    let (name, program) = data.split_at(end + 1);
    if program.is_empty() {
        return Err("not a .p81: a name but no program".into());
    }
    Ok(vec![zx81_block(name, program)])
}

/// Decode the 17-byte ZX header inside a standard block, if that is what it is.
fn header_summary(data: &[u8]) -> String {
    if data.len() != 19 || data[0] != 0x00 {
        return if data.first() == Some(&0xff) {
            "data".into()
        } else {
            String::new()
        };
    }
    let kind = match data[1] {
        0 => "Program",
        1 => "Number array",
        2 => "Char array",
        3 => "Bytes",
        _ => "?",
    };
    let name: String = data[2..12]
        .iter()
        .map(|&b| {
            if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                ' '
            }
        })
        .collect();
    format!("{kind} \"{}\"", name.trim_end())
}

// ---------------------------------------------------------------------------
// parsing
// ---------------------------------------------------------------------------

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn left(&self) -> usize {
        self.d.len().saturating_sub(self.p)
    }
    fn u8(&mut self) -> Result<u8, String> {
        let v = *self.d.get(self.p).ok_or("unexpected end of file")?;
        self.p += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes([self.u8()?, self.u8()?]))
    }
    fn i16(&mut self) -> Result<i16, String> {
        Ok(self.u16()? as i16)
    }
    fn u24(&mut self) -> Result<u32, String> {
        let (a, b, c) = (self.u8()? as u32, self.u8()? as u32, self.u8()? as u32);
        Ok(a | (b << 8) | (c << 16))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes([
            self.u8()?,
            self.u8()?,
            self.u8()?,
            self.u8()?,
        ]))
    }
    fn bytes(&mut self, n: usize) -> Result<Vec<u8>, String> {
        if self.left() < n {
            return Err(format!(
                "block claims {n} bytes but only {} remain",
                self.left()
            ));
        }
        let v = self.d[self.p..self.p + n].to_vec();
        self.p += n;
        Ok(v)
    }
    fn text(&mut self, n: usize) -> Result<String, String> {
        Ok(String::from_utf8_lossy(&self.bytes(n)?).trim().to_string())
    }
}

pub fn parse_tzx(data: &[u8]) -> Result<Vec<Block>, String> {
    if data.len() < 10 || &data[0..8] != b"ZXTape!\x1a" {
        return Err("not a TZX file (bad signature)".into());
    }
    let version = (data[8], data[9]);
    let mut r = Reader { d: data, p: 10 };
    let mut blocks = Vec::new();

    while r.left() > 0 {
        let id = r.u8()?;
        match id {
            0x10 => {
                let pause_ms = r.u16()?;
                let len = r.u16()? as usize;
                blocks.push(Block::Standard {
                    pause_ms,
                    data: r.bytes(len)?,
                });
            }
            0x11 => {
                let pilot = r.u16()?;
                let sync1 = r.u16()?;
                let sync2 = r.u16()?;
                let zero = r.u16()?;
                let one = r.u16()?;
                let pilot_pulses = r.u16()?;
                let used_bits = r.u8()?;
                let pause_ms = r.u16()?;
                let len = r.u24()? as usize;
                blocks.push(Block::Turbo {
                    pilot,
                    sync1,
                    sync2,
                    zero,
                    one,
                    pilot_pulses,
                    used_bits,
                    pause_ms,
                    data: r.bytes(len)?,
                });
            }
            0x12 => {
                let len = r.u16()?;
                let count = r.u16()?;
                blocks.push(Block::PureTone { len, count });
            }
            0x13 => {
                let count = r.u8()? as usize;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(r.u16()?);
                }
                blocks.push(Block::Pulses(v));
            }
            0x14 => {
                let zero = r.u16()?;
                let one = r.u16()?;
                let used_bits = r.u8()?;
                let pause_ms = r.u16()?;
                let len = r.u24()? as usize;
                blocks.push(Block::PureData {
                    zero,
                    one,
                    used_bits,
                    pause_ms,
                    data: r.bytes(len)?,
                });
            }
            0x15 => {
                let t_per_sample = r.u16()?;
                let pause_ms = r.u16()?;
                let used_bits = r.u8()?;
                let len = r.u24()? as usize;
                blocks.push(Block::Direct {
                    t_per_sample,
                    pause_ms,
                    used_bits,
                    data: r.bytes(len)?,
                });
            }
            0x18 | 0x19 => {
                // CSW / generalized data: skippable via their length field.
                let len = r.u32()? as usize;
                r.bytes(len)?;
                blocks.push(Block::Info(format!(
                    "unsupported block ${id:02X} ({len} bytes) skipped"
                )));
            }
            0x20 => blocks.push(Block::Pause(r.u16()?)),
            0x21 => {
                let n = r.u8()? as usize;
                blocks.push(Block::GroupStart(r.text(n)?));
            }
            0x22 => blocks.push(Block::GroupEnd),
            0x23 => blocks.push(Block::Jump(r.i16()?)),
            0x24 => blocks.push(Block::LoopStart(r.u16()?)),
            0x25 => blocks.push(Block::LoopEnd),
            0x26 => {
                let count = r.u16()? as usize;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(r.i16()?);
                }
                blocks.push(Block::CallSequence(v));
            }
            0x27 => blocks.push(Block::Return),
            0x28 => {
                let len = r.u16()? as usize;
                r.bytes(len)?;
                blocks.push(Block::Info("select block (ignored)".into()));
            }
            0x2a => {
                r.u32()?;
                blocks.push(Block::StopIf48k);
            }
            0x2b => {
                r.u32()?;
                blocks.push(Block::SetLevel(r.u8()? != 0));
            }
            0x30 => {
                let n = r.u8()? as usize;
                blocks.push(Block::Info(r.text(n)?));
            }
            0x31 => {
                r.u8()?;
                let n = r.u8()? as usize;
                blocks.push(Block::Info(r.text(n)?));
            }
            0x32 => {
                let len = r.u16()? as usize;
                let body = r.bytes(len)?;
                blocks.push(Block::Info(archive_info(&body)));
            }
            0x33 => {
                let count = r.u8()? as usize;
                r.bytes(count * 3)?;
                blocks.push(Block::Info("hardware type".into()));
            }
            0x35 => {
                // Sixteen bytes of name, then the length. Reading ten of them
                // took the length from the last four characters of the name,
                // which are spaces: a block claiming 0x20202020 bytes, and a
                // tape that would not load.
                let id = r.text(16)?;
                let len = r.u32()? as usize;
                r.bytes(len)?;
                blocks.push(Block::Info(format!("custom info: {}", id.trim())));
            }
            0x5a => {
                r.bytes(9)?;
                blocks.push(Block::Info("glue block".into()));
            }
            other => {
                return Err(format!(
                    "unknown TZX block ${other:02X} at offset {} (file version {}.{})",
                    r.p - 1,
                    version.0,
                    version.1
                ))
            }
        }
    }
    Ok(blocks)
}

/// Pull the title out of an archive-info block ($32) if it has one.
fn archive_info(body: &[u8]) -> String {
    if body.is_empty() {
        return "archive info".into();
    }
    let mut p = 1;
    for _ in 0..body[0] {
        if p + 1 >= body.len() {
            break;
        }
        let id = body[p];
        let len = body[p + 1] as usize;
        p += 2;
        if p + len > body.len() {
            break;
        }
        if id == 0x00 {
            return format!(
                "title: {}",
                String::from_utf8_lossy(&body[p..p + len]).trim()
            );
        }
        p += len;
    }
    "archive info".into()
}

/// A .tap file is just a sequence of length-prefixed standard blocks.
pub fn parse_tap(data: &[u8]) -> Result<Vec<Block>, String> {
    let mut blocks = Vec::new();
    let mut p = 0usize;
    while p + 2 <= data.len() {
        let len = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
        p += 2;
        if len == 0 || p + len > data.len() {
            return Err("truncated .tap block".into());
        }
        blocks.push(Block::Standard {
            pause_ms: 1000,
            data: data[p..p + len].to_vec(),
        });
        p += len;
    }
    if blocks.is_empty() {
        return Err("no blocks found".into());
    }
    Ok(blocks)
}

// ---------------------------------------------------------------------------
// playback
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Phase {
    /// Nothing loaded into the pulse generator yet.
    Enter,
    Pilot {
        left: u32,
    },
    Sync1,
    Sync2,
    Data {
        byte: usize,
        bit: u8,
        second: bool,
    },
    Tone {
        left: u32,
    },
    PulseList {
        idx: usize,
    },
    Direct {
        byte: usize,
        bit: u8,
    },
    /// A ZX81 bit: which pulse of the burst, and which half of that pulse.
    Zx81 {
        byte: usize,
        bit: u8,
        pulse: u8,
        second: bool,
    },
    /// The millisecond that finishes the block's last pulse, before the
    /// silence proper.
    PauseEdge {
        ms: u16,
    },
    /// Silence at the end of a block.
    BlockPause {
        ms: u16,
    },
    Next,
    Finished,
}

/// One pulse: how long it lasts and what the EAR line does.
struct Pulse {
    len: u32,
    /// `None` toggles the level, `Some(l)` forces it.
    level: Option<bool>,
    /// Whether this is the deck running tape past the head with nothing on it.
    /// A silence is no signal, which is nothing volts rather than a signal
    /// held all the way down, so the line sits in the middle of the scope and
    /// the hiss sits on top of it.
    silent: bool,
}

/// How well the tape and the deck are behaving.
///
/// A cassette is not a perfect medium and a deck is not a perfect reader. The
/// motor runs a little fast and a little slow — wow over a turn of the
/// capstan, flutter above it — and a head that is not square to the tape reads
/// the top of the track a moment before the bottom, so the two cancel each
/// other the shorter the wavelength gets. That is a low-pass filter whose
/// corner comes down as the head goes further out of square, and it is why a
/// misaligned deck loses the quick loaders first and ordinary ROM blocks last.
///
/// Nothing here is random. Both wobbles are sine waves read off the clock, so
/// a given moment always gets the same treatment and a load can be repeated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quality {
    /// Whether the motor wavers at all.
    pub wobble: bool,
    /// Wow: the slow one, a couple of turns of the reel, as a fraction of the
    /// right speed.
    pub wow: f32,
    /// Flutter: the quick one, over the top of the wow and by the same
    /// measure.
    pub flutter: f32,
    /// Whether the head is out of square with the tape.
    pub alignment: bool,
    /// How far out, from square at 0 to hopeless at 1. What it sets is where
    /// the filter's corner sits: see [`Quality::cutoff`].
    pub alignment_offset: f32,
    /// How much that corner wanders up and down as the tape runs, as a
    /// fraction of itself.
    pub alignment_wobble: f32,
    /// Whether the tape hisses.
    pub noise: bool,
    /// How loudly, against a full-strength signal. Past the reader's
    /// threshold it stops being a hiss behind the music and starts being
    /// edges the machine can hear.
    pub noise_level: f32,
}

/// Where the corner sits at each end of the slider.
///
/// The top end is only just above the quickest loaders — around 4.5 kHz — so
/// that the whole of the slider does something: at 30 kHz the first half of it
/// was a dead run, since nothing on any tape is anywhere near that. The bottom
/// end is below the pilot tone, where even a ROM block has had it.
const CUTOFF_SQUARE: f32 = 10_000.0;
const CUTOFF_WORST: f32 = 250.0;

/// How much of the signal the reader wants before it believes the line has
/// changed. A comparator has to have some of this or it would chatter on the
/// noise, and it is what decides that a rolled-off signal has become too
/// small to read: the filtered wave swings less and less as the corner comes
/// down, and when it stops reaching this the edges stop arriving.
const THRESHOLD: f32 = 0.45;

/// How much the signal's own strength varies from pulse to pulse.
///
/// A tape is oxide on plastic and its output is never quite the same twice. It
/// matters where the head has rolled the signal down to about what the reader
/// needs, which is where a real tape becomes unreliable rather than failing
/// outright: some pulses clear the threshold and some do not, and which is
/// which changes as the corner comes down. Without it every pulse of a given
/// length crosses or none of them does, and the slider has notches in it
/// rather than a range.
const GRAIN: f32 = 0.08;

/// The tape's grain at a given moment: the same every time, since a load has
/// to be repeatable, but with no pattern for a pulse rate to beat against.
fn grain(at: u64) -> f32 {
    let mut x = at.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    ((x >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
}

impl Default for Quality {
    fn default() -> Self {
        Self {
            wobble: false,
            wow: 0.0,
            flutter: 0.0,
            alignment: false,
            alignment_offset: 0.0,
            alignment_wobble: 0.0,
            noise: false,
            // A hiss well under the reader's threshold, so switching it on
            // shows something on the scope without stopping a load. Turning it
            // up past THRESHOLD is what makes the machine hear it.
            noise_level: 0.2,
        }
    }
}

impl Quality {
    /// The filter's corner, in Hz, at absolute T-state `at`.
    pub fn cutoff(&self, at: u64) -> f32 {
        let base = CUTOFF_SQUARE * (CUTOFF_WORST / CUTOFF_SQUARE).powf(self.alignment_offset);
        if self.alignment_wobble == 0.0 {
            return base;
        }
        let seconds = at as f32 / crate::machine::CPU_HZ as f32;
        // Slower again than the motor's wander: a head creeps.
        let wander = (seconds * std::f32::consts::TAU * 0.027).sin();
        base * (1.0 + self.alignment_wobble * wander)
    }

    /// What the motor does to a pulse of `len` T-states.
    ///
    /// Wow first — a couple of turns of the reel, which is a period of
    /// seconds rather than milliseconds — with a little flutter over it. Both
    /// are slow enough to hear as a waver rather than as a buzz, which is what
    /// a tape does.
    fn wavered(&self, len: u32, at: u64) -> u32 {
        if !self.wobble || (self.wow <= 0.0 && self.flutter <= 0.0) {
            return len;
        }
        let seconds = at as f32 / crate::machine::CPU_HZ as f32;
        // Wow is the reel turning out of true: a period of seconds, with a
        // slower drift under it.
        let turn = (seconds * std::f32::consts::TAU * 0.037).sin();
        let drift = (seconds * std::f32::consts::TAU * 0.012).sin() * 0.6;
        let wow = self.wow * (turn + drift) / 1.6;
        // Flutter is the capstan and the tape's own stiffness, which is quick
        // enough to hear as a warble rather than as a waver.
        let quick = (seconds * std::f32::consts::TAU * 1.4).sin();
        let quicker = (seconds * std::f32::consts::TAU * 6.3).sin() * 0.4;
        let flutter = self.flutter * (quick + quicker) / 1.4;
        (len as f32 * (1.0 + wow + flutter)).max(1.0) as u32
    }

    /// The hiss at a given moment, between -1 and 1 before it is scaled.
    ///
    /// Tape hiss is there whenever the head is on the tape — through the
    /// silence between blocks as much as under the signal — and gone the
    /// moment the head lifts. It is the same every time, as everything else
    /// here is, so a load can be repeated.
    pub fn hiss(&self, at: u64) -> f32 {
        if !self.noise {
            return 0.0;
        }
        // Two grains at different rates, so it is not a single tone.
        let quick = grain(at);
        let slower = grain(at / 37);
        self.noise_level * (quick * 0.75 + slower * 0.25)
    }

    /// The filter's time constant at `at`, in T-states.
    fn tau(&self, at: u64) -> f32 {
        crate::machine::CPU_HZ as f32 / (std::f32::consts::TAU * self.cutoff(at).max(20.0))
    }

    /// Put one pulse through the head and hand back what the reader makes of
    /// it: when it decides the line has changed, if it decides that within the
    /// pulse at all, and where the filter is left afterwards.
    ///
    /// This is the filter itself rather than a rule about it. A one-pole
    /// corner charges towards the level the tape is holding —
    /// `y = u + (y₀ - u)e^(-t/τ)` — and the reader flips when that gets past
    /// its threshold. So a signal only a little rolled off flips late; one
    /// rolled off further flips later still; one whose swing no longer reaches
    /// the threshold does not flip at all, and leaves the next pulse starting
    /// from wherever this one left it. Nothing steps off a cliff: the edges
    /// creep, then some of them go missing, then most of them do.
    ///
    /// `reader` is which way the reader currently has the line, since that is
    /// what decides which threshold it is watching for.
    fn through_the_head(
        &self,
        len: u32,
        at: u64,
        from: f32,
        to: f32,
        reader: bool,
    ) -> (Option<u32>, f32) {
        if !self.alignment {
            return (Some(0), to);
        }
        let tau = self.tau(at);
        let span = len as f32;
        // What this pulse is worth, which is not quite what the last one was,
        // and the hiss it arrives on top of.
        let to = to * (1.0 + GRAIN * grain(at)) + self.hiss(at);
        let settled = to + (from - to) * (-span / tau).exp();
        // The reader is watching for the threshold on the other side of where
        // it has the line now; a pulse heading the way it already thinks the
        // line is going cannot change its mind.
        let want = if reader { -THRESHOLD } else { THRESHOLD };
        let crossing =
            if (to - want).abs() > f32::EPSILON && (from - want).signum() != (to - want).signum() {
                let t = tau * ((to - from) / (to - want)).ln();
                (t >= 0.0 && t < span).then_some(t as u32)
            } else {
                None
            };
        (crossing, settled)
    }
}

#[derive(Clone)]
pub struct Tape {
    pub name: String,
    pub blocks: Vec<Block>,
    pub playing: bool,
    /// Index of the block being played.
    pub block: usize,
    phase: Phase,
    pub level: bool,
    /// Absolute T-state at which the current pulse ends.
    next_edge: u64,
    loop_stack: Vec<(usize, u16)>,
    call_stack: Vec<usize>,
    /// Set when a $20 pause-of-zero or $2A block stops the tape.
    pub stopped_by_block: bool,
    /// Where the tape has been played up to, so progress can be worked out
    /// during the silence at the end of a block as well as during the sound.
    /// The T-state the deck has been advanced to, which is the moment the
    /// level it is reporting belongs to.
    pub clock: u64,
    /// The pause a block ends with: when it starts and when it ends.
    pause_span: Option<(u64, u64)>,
    /// How well the deck is behaving.
    pub quality: Quality,
    /// The tape's own level, which is not what the reader sees once the head
    /// is out of square.
    raw_level: bool,
    /// Where the filter has charged to, between -1 and 1.
    filter_y: f32,
    /// When the current pulse ends, whatever the reader made of it.
    raw_next: u64,
    /// The level change the reader is about to make, and when.
    pending: Option<(u64, bool)>,
    /// Whether what is going past the head at the moment is a silence of the
    /// deck's own making rather than something read off the tape.
    silent: bool,
    /// Whether the head is on the tape. Pause leaves it there — the hiss
    /// carries on — and Stop lifts it, which is silence.
    pub head_down: bool,
    /// When the tape last stopped moving. Everything the scope holds from
    /// before then is a block that has already gone past, not something the
    /// head can hear now.
    pub quiet_from: u64,
    /// The signal itself, as it reaches the reader: samples between -1 and 1,
    /// so the scope can draw the shape the head made of it rather than the
    /// squares the reader made of that. Oldest are dropped.
    pub trace: VecDeque<(u64, f32)>,
    /// Total pulses emitted, for the UI.
    pub pulses: u64,
    /// Recent level changes as (absolute T-state, new level), for the
    /// oscilloscope. Oldest entries are dropped.
    pub edges: VecDeque<(u64, bool)>,
    /// Edges produced since the sound mixer last collected them, so each one
    /// can be mixed in at the T-state it actually happened rather than the
    /// tape's level being sampled whenever the CPU happens to look.
    pending_edges: Vec<(u64, bool)>,
}

/// How many edges the oscilloscope can look back through.
pub const EDGE_HISTORY: usize = 16384;

/// And how many samples of the signal's own shape, for drawing what the head
/// made of it rather than what the reader made of that.
const TRACE_SAMPLES: usize = 4096;

impl Tape {
    pub fn load(path: &Path) -> Result<Tape, String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        Tape::from_bytes(&name, &data)
    }

    /// A tape from bytes rather than from a file, so one that arrived inside
    /// an archive can be read without being written out first. The name is
    /// what the file was called, which decides how it is read and what the
    /// tape is called afterwards.
    pub fn from_bytes(file_name: &str, data: &[u8]) -> Result<Tape, String> {
        let path = Path::new(file_name);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let blocks = match ext.as_str() {
            "tzx" => parse_tzx(data)?,
            "tap" => parse_tap(data)?,
            // ZX81 program dumps. A .p has no name on the front, so one is
            // made up from the file name; a .p81 carries its own.
            "p" | "81" => vec![zx81_block(&zx81_name(&stem), data)],
            "p81" => parse_p81(data)?,
            // Sniff the signature if the extension is unhelpful.
            _ => {
                if data.starts_with(b"ZXTape!\x1a") {
                    parse_tzx(data)?
                } else {
                    parse_tap(data)?
                }
            }
        };
        Ok(Tape::from_blocks(file_name.to_string(), blocks))
    }

    /// Whether the tape is sitting in the silence at the end of a block.
    ///
    /// Nothing is being loaded here: the pulses have stopped and the program
    /// is doing whatever it does between blocks, which is usually drawing the
    /// screen it has just loaded.
    ///
    /// Asked of the span rather than the phase. The pause is played as one
    /// long silent pulse and the phase moves on the moment it is handed over,
    /// so `Phase::BlockPause` is true for no time at all while the silence
    /// itself lasts a second.
    pub fn in_block_pause(&self) -> bool {
        self.pause_span
            .is_some_and(|(from, to)| self.clock >= from && self.clock < to)
    }

    /// Whether the tape stops at the end of the pause it is in.
    ///
    /// The silence between two blocks of a multi-load is the loader getting
    /// ready for the next one and nobody is watching it; the silence at the
    /// end of the tape, or before a block that stops it, is where the program
    /// takes over and where somebody has asked to be. Only the second is worth
    /// coming back to normal speed for.
    pub fn pause_ends_the_tape(&self) -> bool {
        if !self.in_block_pause() {
            return false;
        }
        match self.blocks.get(self.block + 1) {
            None => true,
            Some(Block::Pause(0)) | Some(Block::StopIf48k) => true,
            Some(_) => false,
        }
    }

    pub fn from_blocks(name: String, blocks: Vec<Block>) -> Tape {
        Tape {
            name,
            blocks,
            playing: false,
            block: 0,
            phase: Phase::Enter,
            level: false,
            next_edge: 0,
            loop_stack: Vec::new(),
            call_stack: Vec::new(),
            stopped_by_block: false,
            clock: 0,
            pause_span: None,
            quality: Quality::default(),
            raw_level: false,
            filter_y: 0.0,
            raw_next: 0,
            pending: None,
            silent: false,
            head_down: false,
            quiet_from: 0,
            trace: VecDeque::with_capacity(TRACE_SAMPLES),
            pulses: 0,
            edges: VecDeque::with_capacity(EDGE_HISTORY),
            pending_edges: Vec::new(),
        }
    }

    /// Start (or resume) playing at absolute T-state `now`.
    pub fn play(&mut self, now: u64) {
        if self.block >= self.blocks.len() {
            self.rewind();
        }
        self.playing = true;
        self.head_down = true;
        self.stopped_by_block = false;
        // No edge is recorded here: the first generated pulse starts at `now`
        // and records its own leading edge.
        self.next_edge = now;
        self.raw_next = now;
        self.pending = None;
        self.raw_level = false;
        self.filter_y = 0.0;
        self.trace.clear();
    }

    /// Take the head off the tape: no signal and no hiss either.
    pub fn stop(&mut self) {
        self.playing = false;
        self.head_down = false;
        self.quiet_from = self.clock;
        self.level = false;
        self.trace.clear();
    }

    /// Hold the tape still with the head on it, which is what a pause button
    /// does: nothing is read, and the hiss carries on.
    pub fn pause(&mut self) {
        self.playing = false;
        self.quiet_from = self.clock;
    }

    /// What the reader makes of a tape that is not moving.
    ///
    /// With the head off the tape there is nothing at all — the line sits at
    /// the middle and the scope draws it flat. With the head down and the tape
    /// hissing, the line is the hiss, which crosses the reader's threshold
    /// only when it is turned up far enough to be heard as more than a hiss.
    fn idle_hiss(&mut self, now: u64) -> bool {
        if !self.head_down || !self.quality.noise {
            self.level = false;
            self.filter_y = 0.0;
            return false;
        }
        self.hiss_reaches_the_reader(now, 0.0)
    }

    /// What the reader makes of the hiss on top of the level the line is
    /// holding.
    ///
    /// Noise is on the tape rather than in the signal, so it is there in the
    /// silence between blocks and on a tape held still as much as it is under
    /// a block. Turned up past the threshold it is edges, and a loader waiting
    /// through a gap hears them.
    fn hiss_reaches_the_reader(&mut self, now: u64, under: f32) -> bool {
        let signal = under + self.quality.hiss(now);
        self.filter_y = signal;
        let was = self.level;
        self.level = if signal > THRESHOLD {
            true
        } else if signal < -THRESHOLD {
            false
        } else {
            was
        };
        if self.level != was {
            self.push_edge(now);
        }
        self.level
    }

    /// Keep the shape of the signal across one pulse, for the scope.
    ///
    /// A pulse the head has not touched is a step and needs no more than its
    /// corner; one it has rolled off is a curve, so it is sampled along its
    /// length. What the scope then draws is the signal as it arrives rather
    /// than the squares the reader makes of it.
    fn record_trace(&mut self, start: u64, len: u32, from: f32, to: f32, settled: f32) {
        while self.trace.len() + 6 > TRACE_SAMPLES {
            self.trace.pop_front();
        }
        let end = start + len as u64;
        if !self.quality.alignment || (settled - to).abs() < 0.01 {
            // A step and then a hold. Both ends are needed: a single sample a
            // pulse is a corner with nothing joining it to the next, and a
            // line drawn through those is a triangle wave rather than a
            // square one.
            self.trace.push_back((start, to));
            self.trace.push_back((end, to));
            return;
        }
        let tau = self.quality.tau(start);
        for step in 0..5 {
            let along = len as f32 * step as f32 / 5.0;
            let y = to + (from - to) * (-along / tau).exp();
            self.trace.push_back((start + along as u64, y));
        }
        self.trace.push_back((end, settled));
    }

    /// How loud the hiss is to the loudspeaker, rather than to the reader.
    ///
    /// The reader only hears the hiss when it crosses its threshold, so a
    /// quiet one is inaudible where the tape is silent — which is not what a
    /// tape sounds like. The mixer is given the hiss itself instead, so it is
    /// there through the gaps between blocks and under the signal, as it is on
    /// a real deck with the volume up.
    pub fn audible_hiss(&self) -> f32 {
        if !self.head_down || !self.quality.noise {
            return 0.0;
        }
        // Against the 0.08 the EAR line is worth in the mix: a hiss you can
        // hear under a loading tone without it being the loudest thing there.
        self.quality.noise_level * 0.06
    }

    /// The signal on the line across a window of time, at whatever resolution
    /// the scope asks for.
    ///
    /// The deck keeps the signal's corners — a step, or the curve the head
    /// rolls it into — and nothing more, because that is all the shape there
    /// is. The hiss is put on here instead of being stored: it is white noise
    /// and it has a value at every instant, so sampling it at the resolution
    /// of the screen is both cheaper and truer than keeping a recording of it
    /// that a scope would then have to draw between.
    pub fn scope_samples(&self, from: u64, to: u64, count: usize) -> Vec<(u64, f32)> {
        let count = count.max(2);
        let span = to.saturating_sub(from).max(1);
        // Nothing from before the tape stopped: that block has gone past the
        // head and is not what is on the line now.
        let floor = if self.playing { 0 } else { self.quiet_from };
        let kept: Vec<(u64, f32)> = self
            .trace
            .iter()
            .copied()
            .filter(|(t, _)| *t >= floor)
            .collect();

        let mut at = 0usize;
        (0..count)
            .map(|step| {
                let t = from + span * step as u64 / (count - 1) as u64;
                while at + 1 < kept.len() && kept[at + 1].0 <= t {
                    at += 1;
                }
                let level = match (kept.get(at), kept.get(at + 1)) {
                    // Between two corners: the line is on its way from one to
                    // the other.
                    (Some(&(t0, y0)), Some(&(t1, y1))) if t >= t0 && t1 > t0 => {
                        let along = (t - t0) as f32 / (t1 - t0) as f32;
                        y0 + (y1 - y0) * along.clamp(0.0, 1.0)
                    }
                    // Past the last corner, or before the first: a tape that
                    // is running is holding the level it last reached, and one
                    // that is not is sitting at the middle of the screen.
                    (Some(&(_, y)), _) if self.playing => y,
                    _ => 0.0,
                };
                let hiss = if self.head_down {
                    self.quality.hiss(t)
                } else {
                    0.0
                };
                (t, level + hiss)
            })
            .collect()
    }

    /// Record a level change for the oscilloscope and the mixer.
    fn push_edge(&mut self, at: u64) {
        if self.edges.len() >= EDGE_HISTORY {
            self.edges.pop_front();
        }
        self.edges.push_back((at, self.level));
        // Fast-forwarding can outrun the mixer; drop the backlog rather than
        // letting it grow without bound.
        if self.pending_edges.len() >= EDGE_HISTORY {
            self.pending_edges.clear();
        }
        self.pending_edges.push((at, self.level));
    }

    /// Hand over the edges produced since the last call.
    pub fn take_pending_edges(&mut self, out: &mut Vec<(u64, bool)>) {
        out.append(&mut self.pending_edges);
    }

    /// Move to the previous or next block, keeping playback going.
    pub fn skip(&mut self, delta: i32, now: u64) {
        let target =
            (self.block as i32 + delta).clamp(0, self.blocks.len().saturating_sub(1) as i32);
        self.seek(target as usize);
        if self.playing {
            self.next_edge = now;
        }
    }

    /// Playing time of the whole tape, in T-states.
    pub fn duration_t(&self) -> u64 {
        self.blocks.iter().map(|b| b.duration_t()).sum()
    }

    /// How far through the whole tape playback has got, 0.0 to 1.0, by time
    /// rather than by block. A tape is mostly one or two long blocks with a
    /// scattering of short ones, so counting blocks would have the reels lurch
    /// from a fifth to two fifths as a nineteen-byte header goes past.
    pub fn progress(&self) -> f32 {
        let total = self.duration_t();
        if total == 0 {
            return 0.0;
        }
        let before: u64 = self
            .blocks
            .iter()
            .take(self.block)
            .map(|b| b.duration_t())
            .sum();
        let within = match self.blocks.get(self.block) {
            Some(block) => self.block_progress().unwrap_or(0.0) as f64 * block.duration_t() as f64,
            None => 0.0,
        };
        ((before as f64 + within) / total as f64).clamp(0.0, 1.0) as f32
    }

    /// How far through the current block playback has got, 0.0 to 1.0.
    ///
    /// Worked out from the pulse generator's position rather than from the
    /// clock, so it stays right after a seek or a pause.
    pub fn block_progress(&self) -> Option<f32> {
        let block = self.blocks.get(self.block)?;
        let seg = block.segment_times();
        let total = seg.total();
        if total == 0 {
            // A group marker or a text block takes no time, so there is never
            // any of it left to play. Saying so keeps the progress on the
            // block's row rather than having it disappear as the tape passes
            // through one.
            return Some(1.0);
        }

        let done: u64 = match self.phase {
            Phase::Enter => 0,
            Phase::Pilot { left } => {
                // `left` counts down the pilot pulses still to come.
                let pulses = match block {
                    Block::Standard { data, .. } => {
                        let flag = data.first().copied().unwrap_or(0xff);
                        if flag < 0x80 {
                            HEADER_PILOT_PULSES
                        } else {
                            DATA_PILOT_PULSES
                        }
                    }
                    Block::Turbo { pilot_pulses, .. } => *pilot_pulses,
                    _ => 0,
                } as u64;
                let done_pulses = pulses.saturating_sub(left as u64);
                seg.pilot.checked_mul(done_pulses).unwrap_or(0) / pulses.max(1)
            }
            Phase::Sync1 => seg.pilot,
            Phase::Sync2 => seg.pilot + seg.sync / 2,
            Phase::Data { byte, .. } | Phase::Direct { byte, .. } => {
                let len = match block {
                    Block::Standard { data, .. }
                    | Block::Turbo { data, .. }
                    | Block::PureData { data, .. }
                    | Block::Direct { data, .. } => data.len(),
                    _ => 0,
                };
                let along = if len == 0 {
                    0
                } else {
                    seg.data * byte.min(len) as u64 / len as u64
                };
                seg.pilot + seg.sync + along
            }
            Phase::Tone { left } => {
                let count = match block {
                    Block::PureTone { count, .. } => *count as u64,
                    _ => 0,
                };
                let done = count.saturating_sub(left as u64);
                seg.pilot.checked_mul(done).unwrap_or(0) / count.max(1)
            }
            Phase::PulseList { idx } => match block {
                Block::Pulses(p) => p.iter().take(idx).map(|l| *l as u64).sum(),
                _ => 0,
            },
            Phase::Zx81 { byte, .. } => {
                let len = match block {
                    Block::Zx81 { data, .. } => data.len(),
                    _ => 0,
                };
                if len == 0 {
                    0
                } else {
                    seg.data * byte.min(len) as u64 / len as u64
                }
            }
            Phase::PauseEdge { .. } | Phase::BlockPause { .. } => seg.pilot + seg.sync + seg.data,
            Phase::Next | Phase::Finished => match self.pause_span {
                // Still in the silence this block ends with.
                Some((from, to)) if self.clock < to && to > from => {
                    let sound = seg.pilot + seg.sync + seg.data;
                    let through = (self.clock.saturating_sub(from)) as f64 / (to - from) as f64;
                    sound + (seg.pause as f64 * through) as u64
                }
                _ => total,
            },
        };
        Some((done as f32 / total as f32).clamp(0.0, 1.0))
    }

    /// Index of the next block in `dir` that actually makes a sound.
    pub fn next_data_block(&self, dir: i32) -> usize {
        let mut i = self.block as i32;
        loop {
            i += dir;
            if i <= 0 {
                return 0;
            }
            if i >= self.blocks.len() as i32 {
                return self.blocks.len().saturating_sub(1);
            }
            if self.blocks[i as usize].is_data() {
                return i as usize;
            }
        }
    }

    pub fn rewind(&mut self) {
        self.pause_span = None;
        self.pending_edges.clear();
        self.block = 0;
        self.phase = Phase::Enter;
        self.level = false;
        self.loop_stack.clear();
        self.call_stack.clear();
        self.stopped_by_block = false;
        self.pulses = 0;
    }

    /// Position the deck at the silence that ends a block, as though it had
    /// just been played.
    ///
    /// What a block hands the machine is not only its bytes: the pause behind
    /// them is what gives the program time to get going before the next block
    /// starts. Handing the bytes over and jumping straight to the next block's
    /// pilot takes that time away, and a loader that needs it — Cobra's has
    /// under two seconds of pilot to catch — never gets started.
    /// Whether the current block's data has gone past: the tape is in its
    /// closing edge or its silence, or about to start the next block.
    pub fn data_behind(&self) -> bool {
        matches!(
            self.phase,
            Phase::PauseEdge { .. } | Phase::BlockPause { .. } | Phase::Next
        )
    }

    pub fn seek_to_pause_after(&mut self, block: usize) {
        self.pause_span = None;
        self.level = false;
        if block >= self.blocks.len() {
            self.block = self.blocks.len();
            self.phase = Phase::Finished;
            return;
        }
        self.block = block;
        let ms = match &self.blocks[block] {
            Block::Standard { pause_ms, .. } => *pause_ms,
            Block::Turbo { pause_ms, .. } => *pause_ms,
            Block::PureData { pause_ms, .. } => *pause_ms,
            _ => 0,
        };
        self.phase = if ms == 0 {
            Phase::Next
        } else {
            Phase::PauseEdge { ms }
        };
    }

    /// What follows the last bit of a block: the closing edge, and the silence
    /// if the block asked for one.
    ///
    /// A pulse ends where the next one begins, so with another block behind it
    /// the transition that closes the last bit comes for free — which is why a
    /// block with no pause needed nothing here. It is not free when nothing
    /// follows: a block written with no pause and a "stop the tape" behind it
    /// ends with the line simply stopping, and a loader waiting for the edge
    /// that ends its last byte waits for ever. Starglider 2's loading screen
    /// is such a block, and that was the freeze.
    fn after_data(&self, pause_ms: u16) -> Phase {
        if pause_ms > 0 {
            return Phase::PauseEdge { ms: pause_ms };
        }
        // A block that stops the tape behind it: the deck is about to stand
        // still, so nothing else will provide the edge. The end of the tape is
        // not the same case — there is no loader left waiting by then, and
        // giving it an edge changes what the last block's loader sees.
        let quiet = matches!(
            self.blocks.get(self.block + 1),
            Some(Block::Pause(0)) | Some(Block::StopIf48k)
        );
        if quiet {
            Phase::PauseEdge { ms: 0 }
        } else {
            Phase::Next
        }
    }

    /// Jump straight to a block, e.g. from the tape window.
    /// Put a stop-the-tape block in front of a block.
    ///
    /// A tape image is a list of blocks and the deck plays through it; a stop
    /// block is how a tape says "the loader is waiting for something", and
    /// putting one in is how somebody working on a game gets the deck to stop
    /// where the tape's author did not. The block being played keeps being the
    /// block being played, whichever side of it the new one goes.
    pub fn insert_stop_before(&mut self, index: usize) {
        let index = index.min(self.blocks.len());
        self.blocks.insert(index, Block::Pause(0));
        if index <= self.block {
            self.block += 1;
        }
    }

    /// Take a block out of the tape, which is only offered for the stop
    /// blocks put in from the block list.
    pub fn remove_block(&mut self, index: usize) {
        if index >= self.blocks.len() {
            return;
        }
        self.blocks.remove(index);
        if index < self.block {
            self.block -= 1;
        }
    }

    pub fn seek(&mut self, block: usize) {
        self.pause_span = None;
        self.block = block.min(self.blocks.len());
        self.phase = Phase::Enter;
        self.level = false;
        self.raw_level = false;
        self.filter_y = 0.0;
        self.pending = None;
        self.trace.clear();
    }

    pub fn finished(&self) -> bool {
        matches!(self.phase, Phase::Finished) || self.block >= self.blocks.len()
    }

    /// When the level next changes, in absolute T-states.
    ///
    /// A loader waiting for an edge is going to sit in its sampling loop until
    /// this moment, and there is nothing to be learned by watching it do so.
    /// See [`crate::flashload`].
    pub fn next_edge(&self) -> u64 {
        self.next_edge
    }

    /// EAR level at absolute T-state `now`, advancing the pulse generator to
    /// get there. Cheap: it only does work when pulses have actually elapsed.
    pub fn level_at(&mut self, now: u64) -> bool {
        self.clock = now;
        if !self.playing {
            // A tape held still under the head is not silence: it hisses, and
            // a hiss loud enough is edges as far as the machine is concerned.
            return self.idle_hiss(now);
        }
        // Guard against a huge jump (e.g. after a long pause) taking forever.
        let mut guard = 0u32;
        while now >= self.next_edge && self.playing {
            // A change the reader is due to make part way through the pulse
            // it is in the middle of.
            if let Some((at, level)) = self.pending {
                if now >= at {
                    self.pending = None;
                    self.next_edge = self.raw_next;
                    if level != self.level {
                        self.level = level;
                        self.push_edge(at);
                    }
                    continue;
                }
            }
            match self.next_pulse() {
                Some(p) => {
                    let start = self.raw_next.max(self.next_edge);
                    let len = self.quality.wavered(p.len.max(1), start);
                    let forced = p.level;
                    let raw = forced.unwrap_or(!self.raw_level);
                    self.raw_level = raw;
                    // What the line is worth: a pulse is the signal one way
                    // or the other, and a silence is nothing at all. Playing a
                    // silence as the level held low put the gap between blocks
                    // on the floor of the scope, and the hiss with it.
                    let to = if p.silent {
                        0.0
                    } else if raw {
                        1.0
                    } else {
                        -1.0
                    };
                    let (crossing, settled) = if forced.is_some() {
                        // The deck's own doing — the silence behind a block —
                        // rather than something read off the tape, so the head
                        // has nothing to say about it and the line settles.
                        (Some(0), to)
                    } else {
                        self.quality
                            .through_the_head(len, start, self.filter_y, to, self.level)
                    };
                    self.record_trace(start, len, self.filter_y, to, settled);
                    self.filter_y = settled;
                    self.raw_next = start.saturating_add(len as u64);
                    self.pending = crossing.map(|at| (start.saturating_add(at as u64), raw));
                    self.next_edge = match self.pending {
                        Some((at, _)) => at.min(self.raw_next),
                        None => self.raw_next,
                    };
                    self.pulses += 1;
                    self.silent = p.silent;
                }
                None => {
                    self.playing = false;
                    self.level = false;
                    self.push_edge(self.next_edge);
                    break;
                }
            }
            guard += 1;
            if guard > 2_000_000 {
                break;
            }
        }
        // Through a gap the tape is still going past the head, so the hiss is
        // still arriving. Under a block it is folded into the pulses instead,
        // where it either carries them over the threshold or does not.
        if self.silent && self.quality.noise && self.pending.is_none() {
            // Nothing under it: a silence is no signal, so the hiss is all
            // there is on the line.
            return self.hiss_reaches_the_reader(now, 0.0);
        }
        self.level
    }

    /// Produce the next pulse, moving between blocks as needed.
    fn next_pulse(&mut self) -> Option<Pulse> {
        loop {
            match self.phase {
                Phase::Finished => return None,
                Phase::Next => {
                    self.block += 1;
                    self.phase = Phase::Enter;
                    self.pause_span = None;
                }
                Phase::Enter => {
                    if self.block >= self.blocks.len() {
                        self.phase = Phase::Finished;
                        return None;
                    }
                    if let Some(p) = self.enter_block() {
                        return Some(p);
                    }
                }
                Phase::Pilot { left } => {
                    let (pilot, next) = match &self.blocks[self.block] {
                        Block::Standard { .. } => (PILOT_PULSE, Phase::Sync1),
                        Block::Turbo { pilot, .. } => (*pilot, Phase::Sync1),
                        _ => (PILOT_PULSE, Phase::Sync1),
                    };
                    if left == 0 {
                        self.phase = next;
                    } else {
                        self.phase = Phase::Pilot { left: left - 1 };
                        return Some(Pulse {
                            len: pilot as u32,
                            level: None,
                            silent: false,
                        });
                    }
                }
                Phase::Sync1 => {
                    let sync = match &self.blocks[self.block] {
                        Block::Turbo { sync1, .. } => *sync1,
                        _ => SYNC1_PULSE,
                    };
                    self.phase = Phase::Sync2;
                    return Some(Pulse {
                        len: sync as u32,
                        level: None,
                        silent: false,
                    });
                }
                Phase::Sync2 => {
                    let sync = match &self.blocks[self.block] {
                        Block::Turbo { sync2, .. } => *sync2,
                        _ => SYNC2_PULSE,
                    };
                    self.phase = Phase::Data {
                        byte: 0,
                        bit: 0,
                        second: false,
                    };
                    return Some(Pulse {
                        len: sync as u32,
                        level: None,
                        silent: false,
                    });
                }
                Phase::Data { byte, bit, second } => {
                    let (data, zero, one, used_bits, pause_ms) = match &self.blocks[self.block] {
                        Block::Standard { data, pause_ms } => {
                            (data, ZERO_PULSE, ONE_PULSE, 8, *pause_ms)
                        }
                        Block::Turbo {
                            data,
                            zero,
                            one,
                            used_bits,
                            pause_ms,
                            ..
                        } => (data, *zero, *one, *used_bits, *pause_ms),
                        Block::PureData {
                            data,
                            zero,
                            one,
                            used_bits,
                            pause_ms,
                        } => (data, *zero, *one, *used_bits, *pause_ms),
                        _ => {
                            self.phase = Phase::Next;
                            continue;
                        }
                    };

                    // The final byte may carry fewer than eight meaningful bits.
                    let last = byte + 1 == data.len();
                    let bits_here = if last && (1..8).contains(&used_bits) {
                        used_bits
                    } else {
                        8
                    };
                    if byte >= data.len() || bit >= bits_here {
                        if byte >= data.len() {
                            self.phase = self.after_data(pause_ms);
                            continue;
                        }
                        self.phase = Phase::Data {
                            byte: byte + 1,
                            bit: 0,
                            second: false,
                        };
                        continue;
                    }

                    let set = data[byte] & (0x80 >> bit) != 0;
                    let len = if set { one } else { zero } as u32;
                    self.phase = if second {
                        Phase::Data {
                            byte,
                            bit: bit + 1,
                            second: false,
                        }
                    } else {
                        Phase::Data {
                            byte,
                            bit,
                            second: true,
                        }
                    };
                    return Some(Pulse {
                        len,
                        level: None,
                        silent: false,
                    });
                }
                Phase::Tone { left } => {
                    let len = match &self.blocks[self.block] {
                        Block::PureTone { len, .. } => *len,
                        _ => PILOT_PULSE,
                    };
                    if left == 0 {
                        self.phase = Phase::Next;
                    } else {
                        self.phase = Phase::Tone { left: left - 1 };
                        return Some(Pulse {
                            len: len as u32,
                            level: None,
                            silent: false,
                        });
                    }
                }
                Phase::PulseList { idx } => {
                    let pulse = match &self.blocks[self.block] {
                        Block::Pulses(v) => v.get(idx).copied(),
                        _ => None,
                    };
                    match pulse {
                        Some(len) => {
                            self.phase = Phase::PulseList { idx: idx + 1 };
                            return Some(Pulse {
                                len: len as u32,
                                level: None,
                                silent: false,
                            });
                        }
                        None => self.phase = Phase::Next,
                    }
                }
                Phase::Zx81 {
                    byte,
                    bit,
                    pulse,
                    second,
                } => {
                    let (data, pause_ms) = match &self.blocks[self.block] {
                        Block::Zx81 { data, pause_ms, .. } => (data, *pause_ms),
                        _ => {
                            self.phase = Phase::Next;
                            continue;
                        }
                    };
                    if byte >= data.len() {
                        self.phase = self.after_data(pause_ms);
                        continue;
                    }
                    // Bits go out most significant first, each as a burst of
                    // four pulses for a 0 or nine for a 1.
                    let set = data[byte] & (0x80 >> bit) != 0;
                    let burst = if set {
                        ZX81_ONE_PULSES
                    } else {
                        ZX81_ZERO_PULSES
                    };
                    if pulse >= burst {
                        let (byte, bit) = if bit == 7 {
                            (byte + 1, 0)
                        } else {
                            (byte, bit + 1)
                        };
                        self.phase = Phase::Zx81 {
                            byte,
                            bit,
                            pulse: 0,
                            second: false,
                        };
                        continue;
                    }

                    // Each pulse is a high half then a low half; the low half
                    // of the last pulse in a burst carries the gap that tells
                    // the loader the bit has ended. Two half-pulses per pulse
                    // keeps the level back at rest when the bit finishes.
                    let last_half = second && pulse + 1 == burst;
                    let len =
                        ZX81_HALF_PULSE as u32 + if last_half { ZX81_BIT_GAP as u32 } else { 0 };
                    self.phase = Phase::Zx81 {
                        byte,
                        bit,
                        pulse: if second { pulse + 1 } else { pulse },
                        second: !second,
                    };
                    return Some(Pulse {
                        len,
                        level: None,
                        silent: false,
                    });
                }
                Phase::Direct { byte, bit } => {
                    let (data, t, used_bits, pause_ms) = match &self.blocks[self.block] {
                        Block::Direct {
                            data,
                            t_per_sample,
                            used_bits,
                            pause_ms,
                        } => (data, *t_per_sample, *used_bits, *pause_ms),
                        _ => {
                            self.phase = Phase::Next;
                            continue;
                        }
                    };
                    let last = byte + 1 == data.len();
                    let bits_here = if last && (1..8).contains(&used_bits) {
                        used_bits
                    } else {
                        8
                    };
                    if byte >= data.len() {
                        self.phase = self.after_data(pause_ms);
                        continue;
                    }
                    if bit >= bits_here {
                        self.phase = Phase::Direct {
                            byte: byte + 1,
                            bit: 0,
                        };
                        continue;
                    }
                    // Direct recording sets the level per sample rather than
                    // toggling it.
                    let level = data[byte] & (0x80 >> bit) != 0;
                    self.phase = Phase::Direct { byte, bit: bit + 1 };
                    return Some(Pulse {
                        len: t as u32,
                        level: Some(level),
                        silent: false,
                    });
                }
                // The edge that finishes the last pulse of the block. Every
                // pulse ends with the line changing, and the last one is no
                // different — but the silence behind it is at the low level,
                // so a block whose last pulse was already low used to end with
                // no change at all. A loader waiting for that edge waits for
                // ever: Cobra's reads all eight bits of its last byte and then
                // hangs on the closing edge of the last one, and its
                // protection takes the silence for a snapped tape.
                //
                // The reference calls it a millisecond of the current level
                // before the pause proper, which is the same thing said the
                // other way round.
                Phase::PauseEdge { ms } => {
                    self.phase = Phase::BlockPause { ms };
                    return Some(Pulse {
                        len: T_PER_MS,
                        level: None,
                        silent: false,
                    });
                }
                Phase::BlockPause { ms } => {
                    self.phase = Phase::Next;
                    let len = (ms as u32).saturating_sub(1) * T_PER_MS;
                    if len > 0 {
                        // The pause is played as one long silent pulse. Noting
                        // when it runs is what lets the block's progress keep
                        // moving through it rather than sticking at the end of
                        // the sound.
                        self.pause_span = Some((self.next_edge, self.next_edge + len as u64));
                        return Some(Pulse {
                            len,
                            level: Some(false),
                            silent: true,
                        });
                    }
                }
            }
        }
    }

    /// Set up the pulse generator for the block at `self.block`, running any
    /// control blocks (jumps, loops, pauses) immediately.
    fn enter_block(&mut self) -> Option<Pulse> {
        let block = self.blocks[self.block].clone();
        match block {
            Block::Standard { ref data, .. } => {
                let flag = data.first().copied().unwrap_or(0xff);
                let pulses = if flag < 0x80 {
                    HEADER_PILOT_PULSES
                } else {
                    DATA_PILOT_PULSES
                };
                self.phase = Phase::Pilot {
                    left: pulses as u32,
                };
                None
            }
            Block::Turbo { pilot_pulses, .. } => {
                self.phase = Phase::Pilot {
                    left: pilot_pulses as u32,
                };
                None
            }
            Block::PureTone { count, .. } => {
                self.phase = Phase::Tone { left: count as u32 };
                None
            }
            Block::Pulses(_) => {
                self.phase = Phase::PulseList { idx: 0 };
                None
            }
            Block::PureData { .. } => {
                self.phase = Phase::Data {
                    byte: 0,
                    bit: 0,
                    second: false,
                };
                None
            }
            Block::Direct { .. } => {
                self.phase = Phase::Direct { byte: 0, bit: 0 };
                None
            }
            Block::Zx81 { .. } => {
                self.phase = Phase::Zx81 {
                    byte: 0,
                    bit: 0,
                    pulse: 0,
                    second: false,
                };
                None
            }
            Block::Pause(ms) => {
                if ms == 0 {
                    // "Stop the tape" — the loader is expected to be waiting.
                    self.stopped_by_block = true;
                    self.playing = false;
                    self.phase = Phase::Next;
                    None
                } else {
                    self.phase = Phase::Next;
                    Some(Pulse {
                        len: ms as u32 * T_PER_MS,
                        level: Some(false),
                        silent: true,
                    })
                }
            }
            Block::StopIf48k => {
                self.stopped_by_block = true;
                self.playing = false;
                self.phase = Phase::Next;
                None
            }
            Block::SetLevel(l) => {
                self.level = l;
                self.phase = Phase::Next;
                None
            }
            Block::Jump(offset) => {
                let target = self.block as i32 + offset as i32;
                self.block = target.clamp(0, self.blocks.len() as i32) as usize;
                self.phase = Phase::Enter;
                None
            }
            Block::LoopStart(count) => {
                if count > 0 {
                    self.loop_stack.push((self.block, count));
                }
                self.phase = Phase::Next;
                None
            }
            Block::LoopEnd => {
                if let Some((start, count)) = self.loop_stack.pop() {
                    if count > 1 {
                        self.loop_stack.push((start, count - 1));
                        self.block = start;
                        self.phase = Phase::Next;
                        return None;
                    }
                }
                self.phase = Phase::Next;
                None
            }
            Block::CallSequence(ref offsets) => {
                if let Some(&first) = offsets.first() {
                    self.call_stack.push(self.block);
                    let target = self.block as i32 + first as i32;
                    self.block = target.clamp(0, self.blocks.len() as i32) as usize;
                    self.phase = Phase::Enter;
                } else {
                    self.phase = Phase::Next;
                }
                None
            }
            Block::Return => {
                if let Some(back) = self.call_stack.pop() {
                    self.block = back;
                }
                self.phase = Phase::Next;
                None
            }
            Block::GroupStart(_) | Block::GroupEnd | Block::Info(_) => {
                self.phase = Phase::Next;
                None
            }
        }
    }
}
