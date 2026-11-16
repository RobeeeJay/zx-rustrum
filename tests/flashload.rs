//! Handing tape blocks straight to the ROM's loader.

use zx_rustrum::flashload::{LD_BYTES, LD_BYTES_BODY};
use zx_rustrum::machine::{Spectrum, FRAME_T};
use zx_rustrum::tape::{Block, Tape};

/// A standard block: the flag byte, the data, and the checksum that makes the
/// lot exclusive-or to zero.
fn block(flag: u8, data: &[u8]) -> Block {
    let mut bytes = vec![flag];
    bytes.extend_from_slice(data);
    let checksum = bytes.iter().fold(0u8, |acc, b| acc ^ b);
    bytes.push(checksum);
    Block::Standard {
        pause_ms: 1000,
        data: bytes,
    }
}

/// A machine with just enough ROM to be recognised as the one that owns
/// LD-BYTES: the eight instructions the check reads, up to the push of
/// SA/LD-RET, and RETs after them in case anything falls through.
fn machine(blocks: Vec<Block>) -> Spectrum {
    let mut rom = vec![0xC9u8; 0x4000];
    rom[LD_BYTES as usize..LD_BYTES as usize + 12].copy_from_slice(&[
        0x14, 0x08, 0x15, 0xF3, 0x3E, 0x0F, 0xD3, 0xFE, 0x21, 0x3F, 0x05, 0xE5,
    ]);
    let mut spec = Spectrum::new();
    spec.load_rom(&rom);
    spec.bus.tape = Some(Tape::from_blocks("test".into(), blocks));
    spec.bus.tape_flash = true;
    spec.bus.tape.as_mut().unwrap().play(0);
    spec
}

/// Call the loader as the ROM's own callers do: the flag it wants in A, the
/// length in DE, the address in IX, carry set for loading rather than
/// verifying, and a return address on the stack.
fn call_loader(spec: &mut Spectrum, flag: u8, at: u16, length: u16) {
    spec.cpu.sp = 0x7FFE;
    spec.bus.poke(0x7FFE, 0x34);
    spec.bus.poke(0x7FFF, 0x12); // returns to $1234
    spec.cpu.a = flag;
    spec.cpu.f |= 0x01;
    spec.cpu.ix = at;
    spec.cpu.d = (length >> 8) as u8;
    spec.cpu.e = length as u8;
    spec.cpu.pc = LD_BYTES;
    spec.step_instruction();
}

/// A tape loads at 1,500 baud however fast the machine is run: the pulses take
/// as long as they take. The only way to be quicker is not to play them —
/// hand the block to the routine that was going to read it, and return.
#[test]
fn a_block_is_handed_over_whole() {
    let mut spec = machine(vec![block(0xFF, &[1, 2, 3, 4, 5, 6, 7, 8])]);
    call_loader(&mut spec, 0xFF, 0x8000, 8);

    for (offset, want) in [1u8, 2, 3, 4, 5, 6, 7, 8].iter().enumerate() {
        assert_eq!(
            spec.bus.mem(0x8000 + offset as u16),
            *want,
            "byte {offset} of the block should be in memory"
        );
    }
    assert_eq!(spec.cpu.f & 0x01, 1, "carry set says it loaded");
    assert_eq!(spec.cpu.ix, 0x8008, "IX ends past the bytes loaded");
    assert_eq!(
        ((spec.cpu.d as u16) << 8) | spec.cpu.e as u16,
        0,
        "DE ends at nothing left to load"
    );
    assert_eq!(
        spec.cpu.pc, 0x1234,
        "and the routine returned to its caller"
    );
}

/// A program looking for its data steps over the headers in between, and so
/// does this: the block handed over is the next one with the flag asked for.
#[test]
fn a_block_with_the_wrong_flag_is_stepped_over() {
    let mut spec = machine(vec![
        block(0x00, &[9; 17]), // a header
        block(0xFF, &[7; 4]),  // the data that goes with it
        block(0x00, &[9; 17]), // and another header behind that
    ]);
    call_loader(&mut spec, 0xFF, 0x8000, 4);

    assert_eq!(spec.bus.mem(0x8000), 7, "the data block should have loaded");
    assert_eq!(spec.cpu.f & 0x01, 1, "and reported success");
    assert_eq!(
        spec.bus.tape.as_ref().unwrap().block,
        1,
        "the deck should be on the block it handed over, playing the pause \
         behind it, with the header before it stepped past"
    );
}

/// A block shorter than the program asked for is the tape loading error every
/// mistyped POKE ends in, and the routine has to report it as one.
#[test]
fn a_block_that_is_the_wrong_length_fails_as_the_rom_would() {
    let mut spec = machine(vec![block(0xFF, &[1, 2, 3])]);
    call_loader(&mut spec, 0xFF, 0x8000, 40);

    assert_eq!(
        spec.cpu.f & 0x01,
        0,
        "carry clear says the load failed, which is what a short block is"
    );
    assert_eq!(spec.cpu.pc, 0x1234, "and it still returns to its caller");
}

/// A block longer than the program asked for is ordinary: the ROM stops
/// listening after the length it wanted and takes the next byte as the parity,
/// and the rest of the block goes past unread.
///
/// Daley Thompson's Decathlon's headers carry a spare byte after the checksum
/// — twenty bytes where nineteen is the usual — and calling that a failure
/// made BASIC retry the load for ever while the tape ran on into the game's
/// own loader, which then had nothing to sync to.
#[test]
fn a_block_longer_than_asked_for_is_not_a_failure() {
    let mut bytes = match block(0xFF, &[3; 17]) {
        Block::Standard { data, .. } => data,
        _ => unreachable!(),
    };
    bytes.push(0x80); // the spare byte on the end
    let mut spec = machine(vec![Block::Standard {
        pause_ms: 1000,
        data: bytes,
    }]);
    call_loader(&mut spec, 0xFF, 0x8000, 17);

    assert_eq!(spec.cpu.f & 0x01, 1, "it should have loaded");
    assert_eq!(spec.bus.mem(0x8000), 3, "and put the bytes where they go");
    assert_eq!(
        spec.bus.mem(0x8011),
        0,
        "and stopped after the seventeen it was asked for"
    );
    assert_eq!(spec.cpu.ix, 0x8011, "IX ends after those seventeen");
}

/// Only where the ROM that owns the address is the one paged in. A program is
/// free to put anything it likes at $0556, and a 128K is running a different
/// ROM there until a game pages the other one back.
#[test]
fn the_loader_is_only_answered_when_that_rom_is_there() {
    let mut spec = machine(vec![block(0xFF, &[1, 2, 3, 4])]);
    // A ROM with something else at the address.
    let rom = vec![0x00u8; 0x4000];
    spec.load_rom(&rom);
    call_loader(&mut spec, 0xFF, 0x8000, 4);

    assert_ne!(
        spec.bus.mem(0x8000),
        1,
        "nothing should have been loaded: that is not the loader"
    );
    assert_ne!(spec.cpu.pc, 0x1234, "and nothing returned early");
}

/// With the switch off the tape plays, whatever is under the head.
#[test]
fn nothing_is_handed_over_unless_it_is_switched_on() {
    let mut spec = machine(vec![block(0xFF, &[1, 2, 3, 4])]);
    spec.bus.tape_flash = false;
    call_loader(&mut spec, 0xFF, 0x8000, 4);

    assert_ne!(spec.bus.mem(0x8000), 1, "the block should not have loaded");
    assert_ne!(
        spec.cpu.pc, 0x1234,
        "the machine should be running the routine, not past it"
    );
}

/// The silence behind a block belongs to the block.
///
/// A block hands the machine its bytes and then a pause, and that pause is
/// what the program does its work in — starting the game's own loader, say —
/// before the next block begins. Handing the bytes over and jumping straight
/// to the next block's pilot takes that time away. Cobra's loader has under
/// two seconds of pilot to catch and never caught it: with the pause left in
/// it finds all 256 of the pilot pulses it wants, exactly as it does when the
/// tape is played.
#[test]
fn the_pause_behind_a_block_is_left_on_the_tape() {
    let mut spec = machine(vec![block(0xFF, &[1, 2, 3, 4]), block(0xFF, &[5, 6, 7, 8])]);
    call_loader(&mut spec, 0xFF, 0x8000, 4);

    let tape = spec.bus.tape.as_ref().expect("a tape");
    assert_eq!(
        tape.block, 0,
        "the deck should still be on the block it handed over, playing its pause"
    );
    assert!(
        tape.playing,
        "and still running: the pause is part of the tape"
    );

    // And the next block does not start until that pause has played. A
    // thousand milliseconds is about three and a half million T-states.
    let started = spec.bus.total_t();
    while spec.bus.tape.as_ref().unwrap().block == 0 {
        spec.bus.tstates += 1000;
        let now = spec.bus.total_t();
        spec.bus.tape.as_mut().unwrap().level_at(now);
        assert!(
            now - started < 10_000_000,
            "the pause should end eventually"
        );
    }
    let waited = spec.bus.total_t() - started;
    assert!(
        waited > 3_000_000,
        "the pause behind the block should have played: only {waited} T-states passed"
    );
}

/// A stopped deck is a stopped deck.
///
/// `LOAD ""` with the tape paused waits for somebody to press Play, and no
/// amount of hurry changes that. Handing blocks over anyway ran the whole tape
/// through the instant it was asked for, and left the machine stuck part way
/// into a tape nobody had started.
#[test]
fn a_stopped_tape_is_not_handed_over() {
    let mut spec = machine(vec![block(0xFF, &[1, 2, 3, 4])]);
    spec.bus.tape.as_mut().unwrap().stop();
    call_loader(&mut spec, 0xFF, 0x8000, 4);

    assert_ne!(
        spec.bus.mem(0x8000),
        1,
        "nothing should have been loaded from a tape that is not playing"
    );
    assert_ne!(
        spec.cpu.pc, 0x1234,
        "the machine should be in the ROM's loader, waiting for a pilot"
    );
    assert_eq!(
        spec.bus.tape.as_ref().unwrap().block,
        0,
        "and the deck should not have moved"
    );
}

/// The whole way round, in the order a person does it: reset, tape in but not
/// running, `LOAD ""`, and then Play.
#[test]
fn nothing_happens_until_play_is_pressed() {
    let (Ok(rom), Ok(tape)) = (
        std::fs::read("roms/48.rom"),
        Tape::load(std::path::Path::new("tapes/borderbreak.tap")),
    ) else {
        eprintln!("need roms/48.rom and tapes/borderbreak.tap; skipping");
        return;
    };
    let mut spec = Spectrum::new();
    spec.load_rom(&rom);
    spec.reset();
    spec.bus.tape_flash = true;
    for _ in 0..120 {
        spec.run(FRAME_T);
    }
    spec.bus.tape = Some(tape);
    spec.bus.tape.as_mut().unwrap().stop();

    // LOAD ""
    for keys in [
        &[(6usize, 3u8)][..],
        &[(7, 1), (5, 0)][..],
        &[(7, 1), (5, 0)][..],
        &[(6, 0)][..],
    ] {
        for (row, bit) in keys {
            spec.bus.keys[*row] &= !(1 << bit);
        }
        for _ in 0..4 {
            spec.run(FRAME_T);
        }
        for (row, bit) in keys {
            spec.bus.keys[*row] |= 1 << bit;
        }
        for _ in 0..4 {
            spec.run(FRAME_T);
        }
    }
    // A couple of seconds of waiting, which is what a stopped tape gives you.
    for _ in 0..100 {
        spec.run(FRAME_T);
    }
    assert_eq!(
        spec.bus.tape.as_ref().unwrap().block,
        0,
        "the deck ran on its own with nobody pressing Play"
    );
    assert!(!spec.bus.tape_playing(), "and it should still be stopped");

    // Now press Play.
    let now = spec.bus.total_t();
    spec.bus.tape.as_mut().unwrap().play(now);
    let mut frames = 0;
    while frames < 4000 {
        spec.run(FRAME_T);
        frames += 1;
        if !spec.bus.tape_playing() {
            break;
        }
    }
    for _ in 0..300 {
        spec.run(FRAME_T);
    }
    let drawn = (0x4000..0x5800u16)
        .filter(|a| spec.bus.mem(*a) != 0)
        .count();
    assert!(
        drawn > 500,
        "and then it should load: only {drawn} bytes on screen after {frames} frames"
    );
}

/// End to end, against a real tape: the same machine state at the end, in two
/// frames instead of two and a half thousand.
#[test]
fn a_real_tape_loads_in_a_frame_or_two() {
    let (Ok(rom), Ok(tape)) = (
        std::fs::read("roms/48.rom"),
        Tape::load(std::path::Path::new("tapes/borderbreak.tap")),
    ) else {
        eprintln!("need roms/48.rom and tapes/borderbreak.tap; skipping");
        return;
    };

    let run = |flash: bool, tape: Tape| -> (u32, u16, usize) {
        let mut spec = Spectrum::new();
        spec.load_rom(&rom);
        spec.reset();
        spec.bus.tape_flash = flash;
        for _ in 0..120 {
            spec.run(FRAME_T);
        }
        // The tape goes in and the deck is started before anything is typed,
        // as it would be: the answer is given when the ROM's loader is called,
        // and if the machine is already inside it — waiting for a pilot with
        // the deck stopped — the block it is waiting for has to play.
        spec.bus.tape = Some(tape);
        let now = spec.bus.total_t();
        spec.bus.tape.as_mut().unwrap().play(now);
        // LOAD ""
        for keys in [
            &[(6usize, 3u8)][..],
            &[(7, 1), (5, 0)][..],
            &[(7, 1), (5, 0)][..],
            &[(6, 0)][..],
        ] {
            for (row, bit) in keys {
                spec.bus.keys[*row] &= !(1 << bit);
            }
            for _ in 0..4 {
                spec.run(FRAME_T);
            }
            for (row, bit) in keys {
                spec.bus.keys[*row] |= 1 << bit;
            }
            for _ in 0..4 {
                spec.run(FRAME_T);
            }
        }
        let mut frames = 0;
        while frames < 60_000 {
            spec.run(FRAME_T);
            frames += 1;
            if !spec.bus.tape_playing() {
                break;
            }
        }
        // Let the loaded program get going.
        for _ in 0..300 {
            spec.run(FRAME_T);
        }
        let drawn = (0x4000..0x5800u16)
            .filter(|a| spec.bus.mem(*a) != 0)
            .count();
        (frames, spec.cpu.pc, drawn)
    };

    let (slow_frames, slow_pc, slow_drawn) = run(false, tape.clone());
    let (fast_frames, fast_pc, fast_drawn) = run(true, tape);

    // Not instant: the pauses between the blocks are real tape and are still
    // played, because a loader that needs that gap to get going has to have
    // it. What goes is the four minutes of pulses.
    assert!(
        fast_frames < 100,
        "the tape should be over in a second or so, not {fast_frames} frames"
    );
    assert!(
        slow_frames > 1000,
        "and played it should take the best part of a minute, not {slow_frames} frames"
    );
    assert_eq!(
        (fast_pc, fast_drawn),
        (slow_pc, slow_drawn),
        "the machine should end up in the same place either way"
    );
}

/// The EAR input hears the machine's own loudspeaker when no tape is playing.
///
/// It is not a dead line: on an issue 3 board bit 6 follows bit 4 of the last
/// write to $FE, and on an issue 2 it follows the MIC bit as well. A loader
/// that writes to the port and reads straight back is asking about exactly
/// that, and one that hears nothing at all decides the tape has been taken
/// away.
#[test]
fn the_ear_bit_hears_the_last_write_when_no_tape_is_playing() {
    use zx_rustrum::z80::Bus;

    let mut spec = Spectrum::new();
    let ear = |spec: &mut Spectrum| spec.bus.io_read(0x7FFE) & 0x40 != 0;

    spec.bus.issue2 = false;
    spec.bus.io_write(0x00FE, 0x10); // speaker on
    assert!(ear(&mut spec), "an issue 3 board hears the speaker");
    spec.bus.io_write(0x00FE, 0x08); // MIC only
    assert!(!ear(&mut spec), "and not the MIC bit");

    spec.bus.issue2 = true;
    spec.bus.io_write(0x00FE, 0x08);
    assert!(ear(&mut spec), "an issue 2 board hears the MIC bit too");
    spec.bus.io_write(0x00FE, 0x00);
    assert!(!ear(&mut spec), "and silence is silence either way");
}

/// A tape under the head is what the line carries, whatever was last written.
#[test]
fn a_playing_tape_is_what_the_ear_bit_carries() {
    use zx_rustrum::z80::Bus;

    let mut spec = machine(vec![block(0xFF, &[0x55; 400])]);
    spec.bus.issue2 = true;
    // A write that would read straight back as a set bit if the loudspeaker
    // were what the line carried.
    spec.bus.io_write(0x00FE, 0x18);

    // Over a stretch of the pilot tone the line has to follow the deck, which
    // means it has to go low somewhere: the loudspeaker would hold it high.
    let mut low = 0;
    for _ in 0..400 {
        spec.bus.tstates += 500;
        if spec.bus.io_read(0x7FFE) & 0x40 == 0 {
            low += 1;
        }
    }
    assert!(
        low > 0,
        "with a tape playing the line carries the tape, not the loudspeaker"
    );
}

// A loader of headerless blocks asks for the next one the moment the last is
// in, before the silence after it has run out. The deck is parked in that
// silence, and the search used to start at the block it is parked in, so the
// same block was handed over twice and the load failed for being too short.
// Headers and data alternate their flags, which is why LOAD "" never saw it.
#[test]
fn headerless_blocks_asked_for_back_to_back_are_each_handed_over_once() {
    let mut spec = machine(vec![
        block(0xFF, &[1, 2, 3, 4]),
        block(0xFF, &[5, 6, 7, 8, 9, 10]),
    ]);
    call_loader(&mut spec, 0xFF, 0x8000, 4);
    assert_eq!(spec.cpu.f & 0x01, 0x01, "the first block loaded");
    call_loader(&mut spec, 0xFF, 0x9000, 6);
    let got: Vec<u8> = (0x9000..0x9006).map(|a| spec.bus.peek_raw(a)).collect();
    assert_eq!(
        (got.as_slice(), spec.cpu.f & 0x01),
        (&[5u8, 6, 7, 8, 9, 10][..], 0x01),
        "the second call is handed the second block, and loads"
    );
}

/// A loader that must keep interrupts off while its last block covers its own
/// interrupt table does LD-BYTES's first steps itself, pushes its own return
/// in place of SA/LD-RET (which ends in EI), and comes in at $0562. Knight
/// Lore's rebuilt tape does that, and Fastload answered only $0556, so its
/// biggest block was left to load from the pulses: two and a half minutes of
/// machine time, and never at all with the CPU turbo on. From $0562 the flag
/// and the load/verify carry are in the alternate set, where the caller's
/// EX AF,AF' put them, and the return leaves interrupts as they were.
#[test]
fn a_loader_that_comes_in_past_the_push_of_sa_ld_ret_is_answered_too() {
    let mut spec = machine(vec![block(0xFF, &[9, 8, 7, 6])]);
    spec.cpu.sp = 0x7FFE;
    spec.bus.poke(0x7FFE, 0x34);
    spec.bus.poke(0x7FFF, 0x12); // its own return, $1234
                                 // LD A,$FF / SCF / INC D / EX AF,AF' / DEC D / DI, done by the caller.
    spec.cpu.a_ = 0xFF;
    spec.cpu.f_ = 0x01;
    spec.cpu.a = 0x0F;
    spec.cpu.f = 0x00;
    spec.cpu.iff1 = false;
    spec.cpu.iff2 = false;
    spec.cpu.ix = 0x8000;
    spec.cpu.d = 0;
    spec.cpu.e = 4;
    spec.cpu.pc = LD_BYTES_BODY;
    spec.step_instruction();
    let loaded: Vec<u8> = (0..4).map(|o| spec.bus.mem(0x8000 + o)).collect();
    assert_eq!(loaded, [9, 8, 7, 6], "the block, handed over from $0562");
    assert_eq!(spec.cpu.f & 0x01, 1, "carry set says it loaded");
    assert_eq!(spec.cpu.pc, 0x1234, "returned to the caller's own address");
    assert!(
        !spec.cpu.iff1 && !spec.cpu.iff2,
        "interrupts left off, as the ROM would leave them from here"
    );
}
