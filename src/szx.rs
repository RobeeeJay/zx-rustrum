//! `.szx`: the snapshot format that can say what machine it came off.
//!
//! A `.sna` is a memory dump with the registers on the front and no room for
//! anything else; a `.z80` grew extensions until it ran out of them. SZX is a
//! container instead — a small header saying which machine, and then a chain
//! of blocks, each with a four-character name and a length. An emulator reads
//! the blocks it knows and steps over the rest, which is why the format has
//! been able to grow microdrives, Multifaces, disk drives and the rest without
//! anything having to be rewritten.
//!
//! What is read and written here is the machine itself: the registers, the
//! paging, the border, every page of RAM, the AY, and which joystick interface
//! is plugged in. Blocks for the peripherals are read past rather than thrown
//! away — a file from another emulator loads, and what this cannot represent
//! is named in what `load` gives back rather than being lost quietly.
//!
//! The layout is libspectrum's `szx.c`, which is the reference every emulator
//! is written against.

use crate::machine::{Model, Spectrum};

/// What every one of them starts with.
const MAGIC: &[u8; 4] = b"ZXST";

/// The version this writes. Readers take anything they understand.
const MAJOR: u8 = 1;
const MINOR: u8 = 5;

/// A RAM page block whose data is zlib-compressed says so in bit 0.
const COMPRESSED: u16 = 1;

pub fn is_szx(data: &[u8]) -> bool {
    data.len() >= 8 && &data[0..4] == MAGIC
}

/// Which machine a file is of, without loading it.
pub fn probe_model(data: &[u8]) -> Result<Model, String> {
    if !is_szx(data) {
        return Err("not a .szx: it does not start with ZXST".into());
    }
    model_of(data[6])
}

/// The machine ids, as libspectrum numbers them.
fn model_of(id: u8) -> Result<Model, String> {
    match id {
        // A 16K is a 48K with less RAM in it, and nothing here has less RAM.
        0 | 1 | 15 => Ok(Model::Spectrum48),
        2 | 3 | 16 => Ok(Model::Spectrum128),
        4 => Ok(Model::Plus2A),
        5 | 6 => Ok(Model::Plus3),
        7 => Err("a Pentagon, which this emulator is not".into()),
        8 | 9 | 12 => Err("a Timex, which this emulator is not".into()),
        10 => Err("a Scorpion, which this emulator is not".into()),
        11 => Err("a Spectrum SE, which this emulator is not".into()),
        13 | 14 => Err("a Pentagon 512 or 1024, which this emulator is not".into()),
        other => Err(format!(
            "machine {other}, which this emulator does not know"
        )),
    }
}

fn id_of(model: Model) -> u8 {
    match model {
        Model::Spectrum48 => 1,
        Model::Spectrum128 => 2,
        Model::Plus2A => 4,
        Model::Plus3 => 5,
    }
}

/// One block: four characters and a length, then that many bytes.
struct Block<'a> {
    id: [u8; 4],
    data: &'a [u8],
}

fn blocks(data: &[u8]) -> Result<Vec<Block<'_>>, String> {
    let mut out = Vec::new();
    let mut at = 8;
    while at + 8 <= data.len() {
        let id = [data[at], data[at + 1], data[at + 2], data[at + 3]];
        let len =
            u32::from_le_bytes([data[at + 4], data[at + 5], data[at + 6], data[at + 7]]) as usize;
        let from = at + 8;
        let to = from
            .checked_add(len)
            .ok_or("a block longer than the file")?;
        if to > data.len() {
            return Err(format!(
                "the {} block says it is {len} bytes and the file has {} left",
                String::from_utf8_lossy(&id).trim_end_matches('\0'),
                data.len() - from
            ));
        }
        out.push(Block {
            id,
            data: &data[from..to],
        });
        at = to;
    }
    Ok(out)
}

fn u16at(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

/// Load one. What comes back names the blocks that were stepped over, since
/// what is not read is what the machine will be missing.
pub fn load(spec: &mut Spectrum, data: &[u8]) -> Result<String, String> {
    if !is_szx(data) {
        return Err("not a .szx: it does not start with ZXST".into());
    }
    let model = model_of(data[6])?;
    if spec.bus.model != model {
        return Err(format!(
            "this is a {} snapshot and the machine is a {}: switch machine first",
            model.name(),
            spec.bus.model.name()
        ));
    }

    let mut skipped: Vec<String> = Vec::new();
    let mut pages = 0;
    for block in blocks(data)? {
        match &block.id {
            b"Z80R" => read_z80r(spec, block.data)?,
            b"SPCR" => read_spcr(spec, block.data)?,
            b"RAMP" => {
                read_ramp(spec, block.data)?;
                pages += 1;
            }
            b"AY\0\0" => read_ay(spec, block.data),
            b"JOY\0" => read_joy(spec, block.data),
            // The creator's name, and the keyboard's state, are worth nothing
            // here: neither changes the machine.
            b"CRTR" | b"KEYB" => {}
            other => {
                let name = String::from_utf8_lossy(other)
                    .trim_end_matches('\0')
                    .to_string();
                if !skipped.contains(&name) {
                    skipped.push(name);
                }
            }
        }
    }
    if pages == 0 {
        return Err("no RAM in it: every .szx has at least one RAMP block".into());
    }
    // The picture is what the ULA painted, and nothing has been painted since
    // this machine was somebody else's.
    spec.bus.show_screen_now();

    Ok(if skipped.is_empty() {
        format!("{} pages of RAM", pages)
    } else {
        format!(
            "{pages} pages of RAM. Stepped over: {} — this emulator cannot put {} back",
            skipped.join(", "),
            if skipped.len() == 1 { "it" } else { "them" }
        )
    })
}

fn read_z80r(spec: &mut Spectrum, d: &[u8]) -> Result<(), String> {
    if d.len() < 37 {
        return Err(format!("the Z80R block is {} bytes, not 37", d.len()));
    }
    let c = &mut spec.cpu;
    c.f = d[0];
    c.a = d[1];
    c.c = d[2];
    c.b = d[3];
    c.e = d[4];
    c.d = d[5];
    c.l = d[6];
    c.h = d[7];
    c.f_ = d[8];
    c.a_ = d[9];
    c.c_ = d[10];
    c.b_ = d[11];
    c.e_ = d[12];
    c.d_ = d[13];
    c.l_ = d[14];
    c.h_ = d[15];
    c.ix = u16at(d, 16);
    c.iy = u16at(d, 18);
    c.sp = u16at(d, 20);
    c.pc = u16at(d, 22);
    c.i = d[24];
    // R's top bit is kept apart, the way the CPU keeps it.
    c.r = d[25] & 0x7F;
    c.r7 = d[25] & 0x80;
    c.iff1 = d[26] != 0;
    c.iff2 = d[27] != 0;
    c.im = d[28];
    let tstates = u32::from_le_bytes([d[29], d[30], d[31], d[32]]);
    spec.bus.tstates = tstates % spec.bus.model.frame_t();
    // Bit 1 of the flags is the machine having been halted.
    c.halted = d[34] & 0x02 != 0;
    c.wz = u16at(d, 35);
    Ok(())
}

fn read_spcr(spec: &mut Spectrum, d: &[u8]) -> Result<(), String> {
    if d.len() < 4 {
        return Err(format!("the SPCR block is {} bytes, not 8", d.len()));
    }
    spec.bus.border = d[0] & 7;
    if spec.bus.model.has_paging() {
        spec.bus.page_reg = d[1];
        // Bit 5 is the lock, and a register that says locked with paging
        // that is not lets a 48K program page its own stack away.
        spec.bus.paging_locked = d[1] & 0x20 != 0;
        if spec.bus.model.has_plus3_paging() {
            spec.bus.page_reg_1ffd = d[2];
        }
        spec.bus.apply_paging();
    }
    spec.bus.last_fe = d[3];
    Ok(())
}

fn read_ramp(spec: &mut Spectrum, d: &[u8]) -> Result<(), String> {
    if d.len() < 3 {
        return Err("a RAMP block with no page in it".into());
    }
    let flags = u16at(d, 0);
    let page = d[2] as usize;
    let body = &d[3..];
    let bytes = if flags & COMPRESSED != 0 {
        inflate(body)?
    } else {
        body.to_vec()
    };
    if bytes.len() != 0x4000 {
        return Err(format!(
            "page {page} came out {} bytes, and a page is 16K",
            bytes.len()
        ));
    }
    if page >= 8 {
        // Pages above the machine's own are somebody else's hardware.
        return Ok(());
    }
    spec.bus.ram[page * 0x4000..(page + 1) * 0x4000].copy_from_slice(&bytes);
    Ok(())
}

fn read_ay(spec: &mut Spectrum, d: &[u8]) {
    if d.len() < 18 {
        return;
    }
    spec.bus.audio.ay.selected = d[1] & 0x0F;
    for (i, value) in d[2..18].iter().enumerate() {
        spec.bus.audio.ay.regs[i] = *value;
    }
}

fn read_joy(spec: &mut Spectrum, d: &[u8]) {
    if d.len() < 6 {
        return;
    }
    // Only the first stick: this emulator has one.
    spec.bus.set_joystick(joystick_of(d[4]));
}

/// The joystick ids SZX uses, which are not the order anything else keeps.
fn joystick_of(id: u8) -> crate::joystick::Kind {
    use crate::joystick::Kind;
    match id {
        0 => Kind::Kempston,
        1 => Kind::Cursor,
        2 => Kind::Sinclair1,
        3 => Kind::Sinclair2,
        6 => Kind::Fuller,
        _ => Kind::None,
    }
}

fn joystick_id(kind: crate::joystick::Kind) -> u8 {
    use crate::joystick::Kind;
    match kind {
        Kind::Kempston => 0,
        Kind::Cursor => 1,
        Kind::Sinclair1 => 2,
        Kind::Sinclair2 => 3,
        Kind::Fuller => 6,
        // The format has no DK'Tronics, so each socket goes as what it
        // behaves like: port No. 2 is Kempston's IN 31, and port No. 1 is
        // Interface 2's first — which loses its up and down being the other
        // way round, since there is nowhere in the format to say so.
        Kind::DkTronicsKempston => 0,
        Kind::DkTronicsKeys => 2,
        // Taught keys have nowhere to go in the format, so the programmable
        // one goes as nothing plugged in.
        Kind::DkTronicsProgrammable => 5,
        // 5 is "none" in the format's own numbering.
        Kind::None => 5,
    }
}

/// Write the machine out.
pub fn save(spec: &Spectrum) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(MAJOR);
    out.push(MINOR);
    out.push(id_of(spec.bus.model));
    out.push(0); // no flags: this does not write "late timings" and the rest

    let mut crtr = Vec::new();
    let mut name = [0u8; 32];
    for (at, byte) in b"ZX-Rustrum".iter().enumerate() {
        name[at] = *byte;
    }
    crtr.extend_from_slice(&name);
    crtr.extend_from_slice(&1u16.to_le_bytes());
    crtr.extend_from_slice(&0u16.to_le_bytes());
    chunk(&mut out, b"CRTR", &crtr);

    let c = &spec.cpu;
    let mut z80r = Vec::new();
    z80r.extend_from_slice(&[c.f, c.a, c.c, c.b, c.e, c.d, c.l, c.h]);
    z80r.extend_from_slice(&[c.f_, c.a_, c.c_, c.b_, c.e_, c.d_, c.l_, c.h_]);
    for word in [c.ix, c.iy, c.sp, c.pc] {
        z80r.extend_from_slice(&word.to_le_bytes());
    }
    z80r.push(c.i);
    z80r.push(c.r | c.r7);
    z80r.push(u8::from(c.iff1));
    z80r.push(u8::from(c.iff2));
    z80r.push(c.im);
    z80r.extend_from_slice(&spec.bus.tstates.to_le_bytes());
    // How long an interrupt could still be taken for, which the format keeps
    // and this machine works out for itself.
    z80r.push(0);
    z80r.push(if c.halted { 0x02 } else { 0 });
    z80r.extend_from_slice(&c.wz.to_le_bytes());
    chunk(&mut out, b"Z80R", &z80r);

    let mut spcr = Vec::new();
    spcr.push(spec.bus.border & 7);
    spcr.push(if spec.bus.model.has_paging() {
        spec.bus.page_reg
    } else {
        0
    });
    spcr.push(if spec.bus.model.has_plus3_paging() {
        spec.bus.page_reg_1ffd
    } else {
        0
    });
    spcr.push(spec.bus.last_fe);
    spcr.extend_from_slice(&[0; 4]);
    chunk(&mut out, b"SPCR", &spcr);

    if spec.bus.model.has_ay() {
        let mut ay = Vec::new();
        ay.push(0);
        ay.push(spec.bus.audio.ay.selected);
        ay.extend_from_slice(&spec.bus.audio.ay.regs);
        chunk(&mut out, b"AY\0\0", &ay);
    }

    let mut joy = Vec::new();
    joy.extend_from_slice(&0u32.to_le_bytes());
    joy.push(joystick_id(spec.bus.joystick.kind));
    joy.push(joystick_id(crate::joystick::Kind::None));
    chunk(&mut out, b"JOY\0", &joy);

    // Every page a 48K has is 5, 2 and 0; a 128K has all eight, and writing
    // the lot is what makes the snapshot a machine rather than a picture of
    // one.
    let pages: &[usize] = if spec.bus.model.has_paging() {
        &[0, 1, 2, 3, 4, 5, 6, 7]
    } else {
        &[0, 2, 5]
    };
    for page in pages {
        let from = page * 0x4000;
        let mut ramp = Vec::new();
        let packed = deflate(&spec.bus.ram[from..from + 0x4000]);
        ramp.extend_from_slice(&COMPRESSED.to_le_bytes());
        ramp.push(*page as u8);
        ramp.extend_from_slice(&packed);
        chunk(&mut out, b"RAMP", &ramp);
    }
    out
}

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
}

/// zlib, which is what the format compresses its pages with — the same
/// `flate2` the zip reader uses, so nothing is added to the lock file for it.
fn deflate(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).expect("writing to a Vec");
    encoder.finish().expect("finishing a Vec")
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| format!("a compressed page would not unpack: {e}"))?;
    Ok(out)
}
