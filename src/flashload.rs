//! Loading a tape by answering the ROM instead of playing it.
//!
//! A tape normally loads at 1,500 baud whatever the emulator does about it:
//! the deck makes pulses, the ROM's loader counts them, and four minutes of
//! tape takes four minutes of machine time however fast the machine is run.
//! Running the CPU flat out shortens the wait; it cannot remove it.
//!
//! This removes it. When the machine calls the ROM's LD-BYTES with a standard
//! block under the head, the block is handed over whole — copied into memory,
//! the registers left as the routine would have left them — and the return is
//! taken there and then. A tape loads in the time it takes to memcpy it.
//!
//! It only works where the ROM is doing the loading. A game with a loader of
//! its own is reading the port itself and counting its own pulses, and nothing
//! here can help it: those still load at whatever speed the machine is being
//! run at, which is what Max Speed is for.

use crate::machine::Spectrum;
use crate::tape::Block;

/// Where the 48K ROM's LD-BYTES starts.
pub const LD_BYTES: u16 = 0x0556;

/// The first bytes of it: `INC D / EX AF,AF' / DEC D / DI / LD A,$0F /
/// OUT ($FE),A`.
///
/// Checked rather than assumed, because $0556 is only the loader while the
/// right ROM is paged in — a 128K is running its own ROM until a game pages
/// the other one back — and because a program is free to put anything it
/// likes at that address in RAM.
const LD_BYTES_SIGNATURE: [u8; 8] = [0x14, 0x08, 0x15, 0xF3, 0x3E, 0x0F, 0xD3, 0xFE];

/// A byte of a sampling loop that may be anything: the port's high byte, a
/// jump displacement, a filler.
const ANY: u16 = 0x100;

/// The edge-sampling loops the commercial loaders are built round, and the
/// name each is known by here.
///
/// They are all the same idea — count turns of a loop in B until the EAR bit
/// changes, and give up if B comes round — and they differ in how the bit is
/// got at and in what happens when it does not change. The list was read off
/// the tapes themselves, by running each game and taking the bytes at the
/// address the machine spent its time at while the tape ran; each is written
/// out below in the form it was found in.
///
/// The immediate loaded into A before the `IN` is wildcarded. It is the port's
/// high byte and games differ on it — $7F for most, $FF for Astro Marine
/// Corps, $00 for City Slicker — and none of it changes what the loop is. The
/// jump back at the end is wildcarded for the same reason it always was: its
/// displacement depends on where the loop starts, which is how the first
/// search for Speedlock's found nothing at all.
const CORES: &[(&str, &[u16])] = &[
    // The ROM's own, with the BREAK check taken out. Speedlock (Head over
    // Heels, Daley Thompson's Decathlon).
    //
    //   INC B / RET Z / LD A,$7F / IN A,($FE) / RRA / XOR C / AND $20 / JR Z
    (
        "the ROM's sampler",
        &[
            0x04, 0xC8, 0x3E, ANY, 0xDB, 0xFE, 0x1F, 0xA9, 0xE6, 0x20, 0x28,
        ],
    ),
    // The same with the check left in — `RET NC` after the RRA, so BREAK
    // aborts the load. Dinamic (Astro Marine Corps, Freddy Hardest) and the
    // Search loader (Blood Brothers).
    (
        "a sampler that answers BREAK",
        &[
            0x04, 0xC8, 0x3E, ANY, 0xDB, 0xFE, 0x1F, 0xD0, 0xA9, 0xE6, 0x20, 0x28,
        ],
    ),
    // The same again with a byte of filler where that check would be: `AND A`
    // in Microsphere's (Skool Daze, Contact Sam Cruise), `NOP` in Bleepload's
    // (Bubble Bobble, Starglider).
    (
        "the ROM's sampler with a byte of filler",
        &[
            0x04, 0xC8, 0x3E, ANY, 0xDB, 0xFE, 0x1F, ANY, 0xA9, 0xE6, 0x20, 0x28,
        ],
    ),
    // No RRA: the EAR bit is left where it is and masked with $40 instead of
    // being rotated down to $20 first. The Search loader's variant (Lotus
    // Esprit Turbo Challenge, Space Crusade).
    (
        "a sampler masking the EAR bit where it lies",
        &[0x04, 0xC8, 0x3E, ANY, 0xDB, 0xFE, 0xA9, 0xE6, 0x40, 0x28],
    ),
    // The same, with `RET C` in it: the carry is the loader's own flag for
    // having been told to stop. City Slicker.
    (
        "a sampler masking bit 6 and answering the carry",
        &[0x04, 0xC8, 0x3E, ANY, 0xDB, 0xFE, 0xA9, 0xE6, 0x40, 0xD8],
    ),
    // Alkatraz's (Cobra, 720 Degrees), which gives up by falling through to a
    // `RET` rather than by returning from inside the loop, and looks at the
    // bit before deciding whether B has come round:
    //
    //   INC B / JR NZ,+3 / RET / (two bytes jumped over) / IN A,($FE) / RRA /
    //   RET Z / XOR C / AND $20 / JR Z
    (
        "Alkatraz's sampler",
        &[
            0x04, 0x20, 0x03, 0xC9, ANY, ANY, 0xDB, 0xFE, 0x1F, 0xC8, 0xA9, 0xE6, ANY, 0x28,
        ],
    ),
    // Digital Integration's (ATF, Tomahawk): B counts *down*, the port's high
    // byte is whatever was last on it rather than being loaded each turn, and
    // the loop closes with an absolute jump.
    //
    //   DEC B / RET Z / IN A,($FE) / XOR C / AND $40 / JP Z
    (
        "Digital Integration's sampler",
        &[0x05, 0xC8, 0xDB, 0xFE, 0xA9, 0xE6, 0x40, 0xCA],
    ),
];

/// What one turn of the ROM's loop costs when nothing is in its way: `INC B`
/// 4, `RET Z` 5, `LD A,$7F` 7, `IN A,($FE)` 11, `RRA` 4, `XOR C` 4, `AND $20`
/// 7, and `JR Z` 12 when it is taken.
///
/// Only when nothing is in its way. The port it reads is $7FFE, whose high
/// byte is in the contended range, so the ULA stalls the read by however much
/// it feels like: measured over a load of Head over Heels, 54 T-states most of
/// the time but 56, 58 and 60 often enough to matter. That is why the wait
/// cannot simply be divided out and skipped — see the note in
/// `docs/tape-loading.md`.
#[allow(dead_code)]
const SAMPLER_T: u64 = 54;

/// Which loader's sampling loop the machine is sitting in, if it is in one.
///
/// Which is to say: is a loader of the game's own reading the tape? The ROM's
/// blocks can be handed over whole, and these cannot — the bytes on the tape
/// are not the bytes that reach memory, since Speedlock decrypts each one as
/// it goes with a key it rewrites into its own code — but knowing that a
/// loader is at work is what says the machine should be let run flat out, and
/// saying which one is at work is worth more than saying that one is.
///
/// The loops are checked longest first, since the shorter ones are prefixes of
/// nothing but would match the wrong thing if a longer one were skipped.
pub fn sampler(spec: &Spectrum) -> Option<&'static str> {
    use crate::z80::Bus;
    sampler_at(&|a| spec.bus.peek(a), spec.cpu.pc)
}

/// Which loader's sampling loop starts at this address, if any — the same
/// test as `sampler`, anywhere in memory, so a loader can be recognised long
/// after it has finished.
pub fn sampler_at<F: Fn(u16) -> u8>(peek: &F, at: u16) -> Option<&'static str> {
    CORES
        .iter()
        .find(|(_, pattern)| {
            pattern.iter().enumerate().all(|(offset, byte)| {
                *byte == ANY || peek(at.wrapping_add(offset as u16)) as u16 == *byte
            })
        })
        .map(|(name, _)| *name)
}

/// Is the machine sitting in a loader's sampling loop at all?
pub fn at_sampler(spec: &Spectrum) -> bool {
    sampler(spec).is_some()
}

/// What came of trying to hand a block over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loaded {
    /// A block was handed over and the routine returned.
    Block { bytes: usize, wanted: usize },
    /// The block under the head is not the one the program asked for — a
    /// header where it wants data, or the other way about. The ROM answers
    /// that with "no", and so does this.
    WrongKind,
    /// Nothing could be done: no tape, no standard block under the head, or
    /// the ROM that owns the address is not the one paged in.
    NotOurs,
}

/// Answer the ROM's loader, if that is what the machine is calling.
///
/// Returns what happened, having left the machine as the routine would have:
/// the return address popped, the registers set, and the deck moved on to the
/// next block.
pub fn load_block(spec: &mut Spectrum) -> Loaded {
    if spec.cpu.pc != LD_BYTES || !rom_loader_is_here(spec) {
        return Loaded::NotOurs;
    }
    let Some(tape) = spec.bus.tape.as_ref() else {
        return Loaded::NotOurs;
    };
    // Only from a deck that is running. A stopped tape is a stopped tape,
    // however quickly the emulator is willing to read one: `LOAD ""` on a
    // machine with the tape paused waits for somebody to press Play, and
    // handing it blocks anyway ran the whole tape through the moment it was
    // asked for.
    if !tape.playing {
        return Loaded::NotOurs;
    }

    // What the caller asked for: the flag it expects in A, the address in IX,
    // the length in DE, and carry telling load from verify. The routine keeps
    // them in the alternate set, but it has not run yet, so they are here.
    let wanted_flag = spec.cpu.a;
    let verifying = spec.cpu.f & 0x01 == 0;
    let mut at = spec.cpu.ix;
    let wanted = ((spec.cpu.d as u16) << 8) | spec.cpu.e as u16;

    // The block the program asked for, which is the next one along with the
    // right flag byte: a program looking for its data steps over the headers
    // in between, and so does this. The search stops at anything that is not
    // a plain block of bytes — a turbo block or a stretch of pulses belongs
    // to a loader of the game's own, and the ROM would never get past it.
    let mut index = tape.block;
    let found = loop {
        match block_bytes(tape.blocks.get(index)) {
            Some(data) if data.len() >= 2 && data[0] == wanted_flag => break Some(data.to_vec()),
            Some(_) => index += 1,
            None => break None,
        }
    };
    let Some(data) = found else {
        return Loaded::NotOurs;
    };

    // The bytes the ROM would have read: the flag, then as many as were asked
    // for, then one more as the parity byte. A block longer than the program
    // wanted is ordinary — the ROM stops listening and the rest of the block
    // goes past unread — so what decides success is whether there were enough
    // of them and whether the parity agrees.
    let body = &data[1..];
    let count = (wanted as usize).min(body.len());
    let mut parity = wanted_flag;
    for byte in &body[..count] {
        parity ^= byte;
        if !verifying {
            spec.bus.poke(at, *byte);
        }
        at = at.wrapping_add(1);
    }
    let enough = count == wanted as usize;
    if let Some(checksum) = body.get(count) {
        parity ^= checksum;
    }
    // A block too short for what was asked is the "R Tape loading error" every
    // mistyped POKE ends in.
    let ok = enough && body.len() > count && parity == 0;
    seek_past(spec, index);
    finish(spec, ok, at, wanted - count as u16);
    Loaded::Block {
        bytes: count,
        wanted: wanted as usize,
    }
}

/// Is the ROM that owns $0556 the one paged in?
fn rom_loader_is_here(spec: &Spectrum) -> bool {
    use crate::z80::Bus;
    LD_BYTES_SIGNATURE
        .iter()
        .enumerate()
        .all(|(offset, byte)| spec.bus.peek(LD_BYTES + offset as u16) == *byte)
}

/// The bytes of a block the ROM could have read: a flag, the data, and a
/// checksum. A turbo block holds its bytes the same way — only its pulses are
/// quicker — but the ROM is not the one reading those, so they are left alone.
fn block_bytes(block: Option<&Block>) -> Option<&[u8]> {
    match block? {
        Block::Standard { data, .. } => Some(data),
        _ => None,
    }
}

/// Move the deck to just past the block handed over.
fn seek_past(spec: &mut Spectrum, block: usize) {
    let now = spec.bus.total_t();
    if let Some(tape) = spec.bus.tape.as_mut() {
        // To the silence behind the block, not past it: that silence is what
        // the program does its work in before the next block starts.
        tape.seek_to_pause_after(block);
        // Playing from the end rewinds — which, having just handed over the
        // last block of the tape, started the whole thing loading again.
        if tape.playing && !tape.finished() {
            tape.play(now);
        } else {
            tape.stop();
        }
    }
}

/// Leave the machine where LD-BYTES would have left it, and take the return.
fn finish(spec: &mut Spectrum, ok: bool, at: u16, left: u16) {
    spec.cpu.ix = at;
    spec.cpu.d = (left >> 8) as u8;
    spec.cpu.e = left as u8;
    // Carry says whether it worked, which is the only flag the callers read.
    // The loader ends with the parity byte in H, zero when it agreed.
    spec.cpu.f = if ok {
        spec.cpu.f | 0x01
    } else {
        spec.cpu.f & !0x01
    };
    spec.cpu.h = 0;
    // Interrupts are disabled on the way in and put back on the way out.
    spec.cpu.iff1 = true;
    spec.cpu.iff2 = true;

    let low = spec.bus.mem(spec.cpu.sp) as u16;
    let high = spec.bus.mem(spec.cpu.sp.wrapping_add(1)) as u16;
    spec.cpu.sp = spec.cpu.sp.wrapping_add(2);
    spec.cpu.pc = (high << 8) | low;
}
