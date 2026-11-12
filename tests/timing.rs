//! Per-instruction T-state counts, checked against the published Z80 timings,
//! plus a few ULA contention checks.

use zx_rustrum::machine::Spectrum;
use zx_rustrum::z80::{Bus, Z80};

/// Uncontended flat memory, so a test measures the instruction alone.
struct FlatBus {
    mem: [u8; 65536],
    t: u32,
}

impl FlatBus {
    fn new() -> Self {
        FlatBus {
            mem: [0; 65536],
            t: 0,
        }
    }
}

impl Bus for FlatBus {
    fn fetch_op(&mut self, addr: u16) -> u8 {
        self.t += 4;
        self.mem[addr as usize]
    }
    fn read(&mut self, addr: u16) -> u8 {
        self.t += 3;
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, value: u8) {
        self.t += 3;
        self.mem[addr as usize] = value;
    }
    fn contend(&mut self, _addr: u16, times: u32) {
        self.t += times;
    }
    fn io_read(&mut self, _port: u16) -> u8 {
        self.t += 4;
        0xff
    }
    fn io_write(&mut self, _port: u16, _value: u8) {
        self.t += 4;
    }
    fn peek(&self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }
}

/// Assemble `code` at $0000, run one instruction, return the T-states used.
fn time(code: &[u8], setup: impl Fn(&mut Z80)) -> u32 {
    let mut bus = FlatBus::new();
    bus.mem[..code.len()].copy_from_slice(code);
    let mut cpu = Z80::new();
    cpu.f = 0;
    cpu.sp = 0x8000;
    setup(&mut cpu);
    cpu.pc = 0;
    cpu.step(&mut bus);
    bus.t
}

fn t(code: &[u8]) -> u32 {
    time(code, |_| {})
}

#[test]
fn basic_instruction_timings() {
    assert_eq!(t(&[0x00]), 4, "NOP");
    assert_eq!(t(&[0x78]), 4, "LD A,B");
    assert_eq!(t(&[0x7e]), 7, "LD A,(HL)");
    assert_eq!(t(&[0x77]), 7, "LD (HL),A");
    assert_eq!(t(&[0x36, 0x00]), 10, "LD (HL),n");
    assert_eq!(t(&[0x01, 0x34, 0x12]), 10, "LD BC,nn");
    assert_eq!(t(&[0x34]), 11, "INC (HL)");
    assert_eq!(t(&[0x03]), 6, "INC BC");
    assert_eq!(t(&[0x09]), 11, "ADD HL,BC");
    assert_eq!(t(&[0x22, 0x00, 0x40]), 16, "LD (nn),HL");
    assert_eq!(t(&[0x2a, 0x00, 0x40]), 16, "LD HL,(nn)");
    assert_eq!(t(&[0x32, 0x00, 0x40]), 13, "LD (nn),A");
    assert_eq!(t(&[0x3a, 0x00, 0x40]), 13, "LD A,(nn)");
    assert_eq!(t(&[0xc5]), 11, "PUSH BC");
    assert_eq!(t(&[0xc1]), 10, "POP BC");
    assert_eq!(t(&[0xcd, 0x00, 0x40]), 17, "CALL nn");
    assert_eq!(t(&[0xc9]), 10, "RET");
    assert_eq!(t(&[0xc7]), 11, "RST 0");
    assert_eq!(t(&[0xc3, 0x00, 0x40]), 10, "JP nn");
    assert_eq!(t(&[0x18, 0x00]), 12, "JR d");
    assert_eq!(t(&[0xe3]), 19, "EX (SP),HL");
    assert_eq!(t(&[0xeb]), 4, "EX DE,HL");
    assert_eq!(t(&[0xf9]), 6, "LD SP,HL");
    assert_eq!(t(&[0xdb, 0xfe]), 11, "IN A,(n)");
    assert_eq!(t(&[0xd3, 0xfe]), 11, "OUT (n),A");
    assert_eq!(t(&[0x76]), 4, "HALT");
}

#[test]
fn conditional_timings() {
    // Z clear: RET NZ is taken (11), RET Z is not (5).
    assert_eq!(time(&[0xc0], |c| c.f = 0), 11, "RET NZ taken");
    assert_eq!(time(&[0xc8], |c| c.f = 0), 5, "RET Z not taken");
    assert_eq!(time(&[0xc4, 0, 0x40], |c| c.f = 0), 17, "CALL NZ taken");
    assert_eq!(time(&[0xcc, 0, 0x40], |c| c.f = 0), 10, "CALL Z not taken");
    assert_eq!(time(&[0x20, 0x00], |c| c.f = 0), 12, "JR NZ taken");
    assert_eq!(time(&[0x28, 0x00], |c| c.f = 0), 7, "JR Z not taken");
    assert_eq!(time(&[0x10, 0x00], |c| c.b = 2), 13, "DJNZ taken");
    assert_eq!(time(&[0x10, 0x00], |c| c.b = 1), 8, "DJNZ falling through");
    assert_eq!(time(&[0xc2, 0, 0x40], |c| c.f = 0), 10, "JP NZ");
}

#[test]
fn indexed_timings() {
    assert_eq!(t(&[0xdd, 0x7e, 0x01]), 19, "LD A,(IX+d)");
    assert_eq!(t(&[0xdd, 0x77, 0x01]), 19, "LD (IX+d),A");
    assert_eq!(t(&[0xdd, 0x36, 0x01, 0x00]), 19, "LD (IX+d),n");
    assert_eq!(t(&[0xdd, 0x34, 0x01]), 23, "INC (IX+d)");
    assert_eq!(t(&[0xdd, 0x09]), 15, "ADD IX,BC");
    assert_eq!(t(&[0xdd, 0x23]), 10, "INC IX");
    assert_eq!(t(&[0xdd, 0xe3]), 23, "EX (SP),IX");
    assert_eq!(t(&[0xdd, 0xe5]), 15, "PUSH IX");
    assert_eq!(t(&[0xdd, 0x24]), 8, "INC IXH");
}

#[test]
fn cb_and_ed_timings() {
    assert_eq!(t(&[0xcb, 0x00]), 8, "RLC B");
    assert_eq!(t(&[0xcb, 0x06]), 15, "RLC (HL)");
    assert_eq!(t(&[0xcb, 0x46]), 12, "BIT 0,(HL)");
    assert_eq!(t(&[0xcb, 0xc6]), 15, "SET 0,(HL)");
    assert_eq!(t(&[0xdd, 0xcb, 0x01, 0x46]), 20, "BIT 0,(IX+d)");
    assert_eq!(t(&[0xdd, 0xcb, 0x01, 0xc6]), 23, "SET 0,(IX+d)");
    assert_eq!(t(&[0xed, 0x57]), 9, "LD A,I");
    assert_eq!(t(&[0xed, 0x44]), 8, "NEG");
    assert_eq!(t(&[0xed, 0x56]), 8, "IM 1");
    assert_eq!(t(&[0xed, 0x42]), 15, "SBC HL,BC");
    assert_eq!(t(&[0xed, 0x4b, 0, 0x40]), 20, "LD BC,(nn)");
    assert_eq!(t(&[0xed, 0x67]), 18, "RRD");
    assert_eq!(t(&[0xed, 0x40]), 12, "IN B,(C)");
    assert_eq!(t(&[0xed, 0x41]), 12, "OUT (C),B");
}

#[test]
fn block_instruction_timings() {
    assert_eq!(time(&[0xed, 0xa0], |c| c.set_bc(1)), 16, "LDI");
    assert_eq!(time(&[0xed, 0xb0], |c| c.set_bc(2)), 21, "LDIR repeating");
    assert_eq!(time(&[0xed, 0xb0], |c| c.set_bc(1)), 16, "LDIR last pass");
    assert_eq!(time(&[0xed, 0xa1], |c| c.set_bc(1)), 16, "CPI");
    assert_eq!(
        time(&[0xed, 0xb1], |c| {
            c.set_bc(2);
            c.a = 0xaa; // never matches the zeroed memory
        }),
        21,
        "CPIR repeating"
    );
    assert_eq!(time(&[0xed, 0xa2], |c| c.b = 1), 16, "INI");
    assert_eq!(time(&[0xed, 0xb2], |c| c.b = 2), 21, "INIR repeating");
    assert_eq!(time(&[0xed, 0xa3], |c| c.b = 1), 16, "OUTI");
    assert_eq!(time(&[0xed, 0xb3], |c| c.b = 2), 21, "OTIR repeating");
}

#[test]
fn interrupt_timings() {
    let mut bus = FlatBus::new();
    let mut cpu = Z80::new();
    cpu.iff1 = true;
    cpu.im = 1;
    cpu.sp = 0x8000;
    assert!(cpu.interrupt(&mut bus));
    assert_eq!(bus.t, 13, "IM 1 acknowledge");
    assert_eq!(cpu.pc, 0x0038);

    let mut bus = FlatBus::new();
    let mut cpu = Z80::new();
    cpu.iff1 = true;
    cpu.im = 2;
    cpu.i = 0x80;
    cpu.sp = 0x8000;
    assert!(cpu.interrupt(&mut bus));
    assert_eq!(bus.t, 19, "IM 2 acknowledge");

    let mut bus = FlatBus::new();
    let mut cpu = Z80::new();
    cpu.sp = 0x8000;
    cpu.nmi(&mut bus);
    assert_eq!(bus.t, 11, "NMI");
    assert_eq!(cpu.pc, 0x0066);
}

#[test]
fn ei_blocks_interrupts_for_one_instruction() {
    let mut bus = FlatBus::new();
    bus.mem[0] = 0xfb; // EI
    bus.mem[1] = 0x00; // NOP
    let mut cpu = Z80::new();
    cpu.sp = 0x8000;
    cpu.step(&mut bus);
    assert!(!cpu.interrupt(&mut bus), "interrupt right after EI");
    cpu.step(&mut bus);
    assert!(cpu.interrupt(&mut bus), "interrupt one instruction later");
}

// ---- ULA contention -------------------------------------------------------

const FIRST_PIXEL_T: u32 = 14335; // 48K

fn spectrum_at(t: u32) -> Spectrum {
    let mut s = Spectrum::new();
    s.bus.tstates = t;
    s
}

#[test]
fn uncontended_memory_is_never_delayed() {
    let mut s = spectrum_at(FIRST_PIXEL_T);
    // $8000 is in an uncontended page on a 48K machine.
    s.bus.write(0x8000, 0x00);
    assert_eq!(s.bus.tstates, FIRST_PIXEL_T + 3);
}

#[test]
fn contended_memory_is_delayed_by_the_ula_pattern() {
    // The delay for the first T-state of a pixel-fetch group is 6.
    let mut s = spectrum_at(FIRST_PIXEL_T);
    s.bus.read(0x4000);
    assert_eq!(s.bus.tstates, FIRST_PIXEL_T + 6 + 3);

    // Offsets 6 and 7 in each group of eight are free.
    let mut s = spectrum_at(FIRST_PIXEL_T + 6);
    s.bus.read(0x4000);
    assert_eq!(s.bus.tstates, FIRST_PIXEL_T + 6 + 3);
}

#[test]
fn contention_stops_outside_the_pixel_area() {
    // Before the display starts.
    let mut s = spectrum_at(FIRST_PIXEL_T - 1);
    s.bus.read(0x4000);
    assert_eq!(s.bus.tstates, FIRST_PIXEL_T - 1 + 3);

    // During horizontal blanking (128 T-states of pixels per 224 T-state line).
    let mut s = spectrum_at(FIRST_PIXEL_T + 130);
    s.bus.read(0x4000);
    assert_eq!(s.bus.tstates, FIRST_PIXEL_T + 130 + 3);
}

#[test]
fn a_full_frame_is_69888_tstates() {
    let mut s = Spectrum::new();
    s.bus.rom.fill(0x00); // NOPs everywhere, so PC just walks the address space
    let before = s.bus.frame;
    let mut total = 0u64;
    let mut prev = s.bus.tstates;
    while s.bus.frame == before {
        s.step_instruction();
        // Accumulate across the wrap the machine performs at end of frame.
        total += if s.bus.tstates >= prev {
            (s.bus.tstates - prev) as u64
        } else {
            (s.bus.tstates + zx_rustrum::machine::FRAME_T - prev) as u64
        };
        prev = s.bus.tstates;
    }
    assert_eq!(s.bus.frame, before + 1);
    // The frame ends on an instruction boundary, so the total overshoots by at
    // most one instruction's worth of T-states.
    let frame = zx_rustrum::machine::FRAME_T as u64;
    assert!(
        (frame..frame + 24).contains(&total),
        "frame took {total} T-states"
    );
    assert!(
        s.bus.irq_pending,
        "the ULA raises /INT at the frame boundary"
    );
}

/// The floating bus gives back what the ULA has on it, not $FF.
///
/// Reading an unattached port on a 48K reads whatever the ULA last put on the
/// bus, which is how a game finds the beam with no hardware to ask: Sidewize
/// sits in `LD A,R / IN A,(C) / CP E / JP NZ` on port $40FF until the byte it
/// wants comes back, and does not enable interrupts until it does.
///
/// An IO read is stalled to a free contention slot before it samples, and a
/// free slot is one of the four in eight when the ULA is not fetching. Reading
/// those back as $FF meant every such read gave $FF, and a game waiting for a
/// particular byte waited for ever. The bus is not driven then: what is on it
/// is the last byte the ULA put there.
#[test]
fn the_floating_bus_reads_what_the_ula_is_fetching() {
    use zx_rustrum::machine::Spectrum;
    use zx_rustrum::z80::Bus;

    let mut spec = Spectrum::new();
    for cell in 0..32u16 {
        spec.bus.poke(0x4000 + cell, 0xAA);
        spec.bus.poke(0x5800 + cell, 0x40);
    }

    // Across a display line, every read gives back something the screen holds.
    let first = spec.bus.first_pixel_t();
    for t in first..first + 128 {
        spec.bus.tstates = t;
        let read = spec.bus.io_read(0x40FF);
        assert!(
            read == 0xAA || read == 0x40,
            "at T {t} the bus gave back ${read:02X}, which is neither the \
             pixels nor the colours the ULA is fetching there"
        );
    }

    // And it follows the screen: change what is there and the reads change.
    for cell in 0..32u16 {
        spec.bus.poke(0x5800 + cell, 0x07);
    }
    spec.bus.tstates = first + 64;
    let read = spec.bus.io_read(0x40FF);
    assert!(
        read == 0xAA || read == 0x07,
        "after changing the colours the bus gave back ${read:02X}"
    );

    // Above the picture the ULA is fetching nothing and the bus reads $FF.
    spec.bus.tstates = 0;
    assert_eq!(
        spec.bus.io_read(0x40FF),
        0xFF,
        "in the border above the picture there is nothing on the bus"
    );
}

// What decides whether moving something out of contended memory is worth it
// is how long the CPU waits on it, so the bus can count the waiting by the
// page it was waiting to use. A fetch from $4000 at T-state 14335 waits six
// (the reference's pattern) and one from $8000 not at all; both must be
// counted as such, and nothing counted while nobody asked.
#[test]
fn stalls_are_counted_by_the_page_waited_on() {
    let mut spec = Spectrum::new();
    spec.bus.tstates = 14335;
    spec.bus.fetch_op(0x4000);
    assert!(spec.bus.stalls.is_none(), "counting started unasked");

    let mut spec = Spectrum::new();
    spec.bus.stalls = Some(Box::new([0; 256]));
    spec.bus.tstates = 14335;
    spec.bus.fetch_op(0x4000);
    spec.bus.tstates = 14335;
    spec.bus.fetch_op(0x8000);
    // An internal cycle with a contended address on the bus waits too.
    spec.bus.tstates = 14335;
    spec.bus.contend(0x4100, 1);
    let stalls = spec.bus.stalls.as_ref().unwrap();
    assert_eq!(
        (
            stalls[0x40],
            stalls[0x41],
            stalls[0x80],
            stalls.iter().sum::<u64>()
        ),
        (6, 6, 0, 12),
        "stalls on page $40, $41 and $80, and in all"
    );
}
