//! RZX recordings: a snapshot, then every byte the machine read from a port,
//! frame by frame, for as long as somebody played.
//!
//! Playing one back is not emulating input — it is replacing it. The machine
//! runs the number of instructions the recording says, and every IN gives back
//! the byte that was read at that point when the recording was made, so the
//! program takes exactly the path it took then. That is why a recording is
//! worth having for reading a program: it is a run of the real thing, with the
//! keyboard and the loading and the copy protection already dealt with.
//!
//! The format is a header, then blocks: `$10` says who made it, `$30` carries
//! the snapshot to start from, `$80` carries the frames. Both of the last two
//! are usually deflated.

use std::io::Read;

/// One frame of the recording.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// How many instructions to run before the next frame starts.
    pub fetches: u16,
    /// What each IN in that stretch should give back, in order.
    pub inputs: Vec<u8>,
}

/// The machine to start from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// "Z80" or "SNA", as the file says.
    pub extension: String,
    pub data: Vec<u8>,
}

/// A whole recording.
#[derive(Clone, Debug)]
pub struct Recording {
    pub creator: String,
    pub snapshot: Option<Snapshot>,
    /// Snapshots further in, each with the frame it comes before. A recorder
    /// that loads a tape faster than the machine could — SPIN's flash
    /// loading — cannot record the instructions it skipped, so it writes the
    /// machine as it stands afterwards and the frames go on from there.
    pub later: Vec<(usize, Snapshot)>,
    pub frames: Vec<Frame>,
    /// The T-state the first frame starts at.
    pub start_t: u32,
}

impl Recording {
    /// The snapshot the machine is replaced by before `frame` is played, if
    /// the recording carries one there.
    pub fn snapshot_before(&self, frame: usize) -> Option<&Snapshot> {
        self.later
            .iter()
            .find(|(at, _)| *at == frame)
            .map(|(_, snapshot)| snapshot)
    }

    /// How long the recording runs, in frames.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

/// A recording being made: the frames so far, and where the current one
/// started.
///
/// A frame of a recording is a number of *opcode fetches* and the bytes every
/// IN in that stretch gave back. Both are counted as the machine runs, because
/// neither can be worked out afterwards.
#[derive(Clone, Debug, Default)]
pub struct Capture {
    pub frames: Vec<Frame>,
    /// What the INs in the frame being recorded have given back so far.
    pub inputs: Vec<u8>,
    /// The fetch count when this frame started.
    pub mark: u32,
    /// Where in the frame the recording began.
    pub start_t: u32,
}

impl Capture {
    /// Close off the frame that has just ended.
    pub fn end_frame(&mut self, fetches: u32) {
        let ran = fetches.wrapping_sub(self.mark);
        self.mark = fetches;
        self.frames.push(Frame {
            // A frame that somehow ran more instructions than the count can
            // hold is clamped rather than wrapped: a recording that says a
            // frame is three instructions long comes adrift immediately.
            fetches: ran.min(u16::MAX as u32) as u16,
            inputs: std::mem::take(&mut self.inputs),
        });
    }
}

/// Write a recording out as an RZX file.
///
/// Uncompressed, which the format allows and which keeps this to arithmetic:
/// the blocks that may be deflated say so in their flags, and these do not.
pub fn write(recording: &Recording) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"RZX!");
    out.push(0); // major
    out.push(13); // minor
    out.extend_from_slice(&0u32.to_le_bytes()); // flags: not signed

    // Who made it: twenty bytes of name, then a version.
    let mut creator = [b' '; 20];
    for (slot, byte) in creator.iter_mut().zip(recording.creator.bytes()) {
        *slot = byte;
    }
    let mut body = Vec::new();
    body.extend_from_slice(&creator);
    body.extend_from_slice(&0u16.to_le_bytes());
    body.extend_from_slice(&1u16.to_le_bytes());
    block(&mut out, 0x10, &body);

    // The machine to start from.
    if let Some(snapshot) = &recording.snapshot {
        let mut body = Vec::new();
        body.extend_from_slice(&0u32.to_le_bytes()); // flags: here, and not packed
        let mut extension = [0u8; 4];
        for (slot, byte) in extension.iter_mut().zip(snapshot.extension.bytes()) {
            *slot = byte;
        }
        body.extend_from_slice(&extension);
        body.extend_from_slice(&(snapshot.data.len() as u32).to_le_bytes());
        body.extend_from_slice(&snapshot.data);
        block(&mut out, 0x30, &body);
    }

    // And the frames.
    let mut body = Vec::new();
    body.extend_from_slice(&(recording.frames.len() as u32).to_le_bytes());
    body.push(0); // reserved
    body.extend_from_slice(&recording.start_t.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // flags: not packed
    for frame in &recording.frames {
        body.extend_from_slice(&frame.fetches.to_le_bytes());
        body.extend_from_slice(&(frame.inputs.len() as u16).to_le_bytes());
        body.extend_from_slice(&frame.inputs);
    }
    block(&mut out, 0x80, &body);
    out
}

/// One block: its kind, its length including the five bytes of header, and
/// the body.
fn block(out: &mut Vec<u8>, id: u8, body: &[u8]) {
    out.push(id);
    out.extend_from_slice(&(body.len() as u32 + 5).to_le_bytes());
    out.extend_from_slice(body);
}

/// Read a recording from the bytes of a file.
pub fn parse(data: &[u8]) -> Result<Recording, String> {
    if data.len() < 10 || &data[0..4] != b"RZX!" {
        return Err("not an RZX recording".into());
    }
    let mut recording = Recording {
        creator: String::new(),
        snapshot: None,
        later: Vec::new(),
        frames: Vec::new(),
        start_t: 0,
    };

    let mut off = 10;
    while off + 5 <= data.len() {
        let id = data[off];
        let len = u32(data, off + 1) as usize;
        if len < 5 || off + len > data.len() {
            // A block that runs off the end is where the file stops being
            // readable; what has been read so far is still worth having.
            break;
        }
        let body = &data[off + 5..off + len];
        match id {
            0x10 => recording.creator = creator_name(body),
            // Only the first is where the recording starts. Keeping the last
            // instead, as this once did, started Out Run from the machine as
            // it stood at the end and fed it the input recorded at the start.
            0x30 if recording.snapshot.is_none() => recording.snapshot = Some(snapshot(body)?),
            0x30 => recording
                .later
                .push((recording.frames.len(), snapshot(body)?)),
            0x80 => {
                let (frames, start_t) = frames(body)?;
                recording.start_t = start_t;
                recording.frames.extend(frames);
            }
            // $11 and $12 are the security blocks, which say who signed the
            // recording. Nothing here depends on that.
            _ => {}
        }
        off += len;
    }

    if recording.frames.is_empty() {
        return Err("the recording has no frames in it".into());
    }
    Ok(recording)
}

/// Who made it: twenty bytes of name, padded with spaces or nulls.
fn creator_name(body: &[u8]) -> String {
    let name: String = body
        .iter()
        .take(20)
        .map(|b| *b as char)
        .filter(|c| !c.is_control())
        .collect();
    name.trim().to_string()
}

/// The snapshot block: flags, the extension it would have had as a file, its
/// length unpacked, and the snapshot itself.
fn snapshot(body: &[u8]) -> Result<Snapshot, String> {
    if body.len() < 12 {
        return Err("the snapshot block is too short".into());
    }
    let flags = u32(body, 0);
    let extension: String = body[4..8]
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| (*b as char).to_ascii_lowercase())
        .collect();
    let uncompressed = u32(body, 8) as usize;

    // Bit 0 means the snapshot is not here at all, only a name to go and find,
    // which is no use without the file it names.
    if flags & 1 != 0 {
        return Err("the recording points at a snapshot file rather than carrying one".into());
    }
    let data = if flags & 2 != 0 {
        inflate(&body[12..], uncompressed)?
    } else {
        body[12..].to_vec()
    };
    Ok(Snapshot { extension, data })
}

/// The input recording block: a count of frames, where in the frame to start,
/// and then the frames themselves.
fn frames(body: &[u8]) -> Result<(Vec<Frame>, u32), String> {
    if body.len() < 13 {
        return Err("the input block is too short".into());
    }
    let count = u32(body, 0) as usize;
    let start_t = u32(body, 5);
    let flags = u32(body, 9);
    let packed = &body[13..];
    let data = if flags & 2 != 0 {
        // The unpacked length is not recorded, so it is grown as needed.
        inflate(packed, count * 8 + 1024)?
    } else {
        packed.to_vec()
    };

    let mut frames = Vec::with_capacity(count.min(1 << 20));
    let mut off = 0usize;
    let mut previous: Vec<u8> = Vec::new();
    while off + 4 <= data.len() && frames.len() < count {
        let fetches = u16v(&data, off);
        let ins = u16v(&data, off + 2) as usize;
        off += 4;
        // $FFFF means this frame reads exactly what the last one did, which is
        // how a recording of somebody not touching anything stays small.
        let inputs = if ins == 0xFFFF {
            previous.clone()
        } else {
            if off + ins > data.len() {
                break;
            }
            let inputs = data[off..off + ins].to_vec();
            off += ins;
            previous = inputs.clone();
            inputs
        };
        frames.push(Frame { fetches, inputs });
    }
    Ok((frames, start_t))
}

/// Unpack a deflated block. `hint` is only a starting size for the buffer.
fn inflate(data: &[u8], hint: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(hint.min(8 << 20));
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| format!("could not unpack the block: {e}"))?;
    Ok(out)
}

fn u32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn u16v(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

/// Put the machine where a snapshot part way through the recording says it
/// was. It is the same run carried on, not a new one, so what is known about
/// which bytes have run is kept: loading a snapshot otherwise forgets it, and
/// a game whose loader the recorder skipped would come out as never having
/// run its loader's half of the program.
pub fn carry_on_from(
    spec: &mut crate::machine::Spectrum,
    snapshot: &Snapshot,
) -> Result<(), String> {
    let tracker = std::mem::take(&mut spec.bus.tracker);
    let loaded = crate::snapshot::load_bytes(spec, &snapshot.extension, &snapshot.data);
    spec.bus.tracker = tracker;
    // The snapshot is the machine between two frames, with the interrupt
    // already dealt with: the next frame counts from here.
    spec.bus.irq_pending = false;
    loaded.map(|_| ())
}
