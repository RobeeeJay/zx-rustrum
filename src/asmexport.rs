//! The program in memory, written out as assembly source that assembles back
//! into the same bytes.
//!
//! What is written is the RAM the machine can see, $4000 to $FFFF, and never
//! the ROM: references into the ROM are kept as addresses, with the ROM's own
//! names as EQUs where they are known. Code is what has been seen to run —
//! the tracker marks every opcode fetch — and everything else is data, so
//! nothing is guessed at: a table read as instructions would still assemble,
//! but it would be a lie about the program.
//!
//! The one promise is that the file assembles back to the bytes it came from.
//! A Z80 instruction can have more than one encoding — NEG, RETN and IM have
//! duplicates, LD HL,(nn) has a long form, a DD or FD prefix before an
//! instruction that does not use IX or IY does nothing — and an assembler
//! picks the usual one, which is not necessarily the one in memory. So only
//! documented instructions with the one encoding are written as mnemonics;
//! the rest, and the undocumented ones, are DEFB with the instruction in a
//! comment, which any assembler turns back into the same bytes.

use std::collections::{BTreeMap, HashSet};

use crate::disasm;
use crate::machine::{Model, Spectrum};
use crate::notes::Notes;

/// Where the file starts by default: the ROM is below it.
pub const FROM: u16 = 0x4000;

/// How long a comment can be and still sit beside its instruction.
pub const BESIDE: usize = 52;

/// Lines no longer than `width`, keeping the line breaks that were written.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.len() + 1 + word.len() > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

/// What goes into the file.
pub struct Source<'a> {
    /// The stretch written out; anything named outside it is an EQU.
    pub from: u16,
    pub to: u16,
    /// What was measured about an address, for the lines above its label.
    pub above: &'a dyn Fn(u16) -> Vec<String>,
    /// What was measured about an instruction this long, to go beside it.
    pub marks: &'a dyn Fn(u16, u8) -> String,
    /// The 64K as the machine sees it.
    pub memory: &'a [u8],
    /// Where an opcode has been fetched, address by address.
    pub ran: &'a [bool],
    pub notes: &'a Notes,
    /// A name for a ROM address, from the notes or a symbol file.
    pub rom_name: &'a dyn Fn(u16) -> Option<String>,
    /// Lines for the comment at the top: the program, the machine, the ROMs.
    pub header: Vec<String>,
}

/// The source, and what it came to.
pub struct Exported {
    pub text: String,
    /// Instructions written as mnemonics.
    pub instructions: usize,
    /// Bytes written as DEFB, instructions that had to be included.
    pub data_bytes: usize,
}

/// Words an assembler keeps for itself, which a label cannot be.
const RESERVED: &[&str] = &[
    "A",
    "B",
    "C",
    "D",
    "E",
    "H",
    "L",
    "I",
    "R",
    "F",
    "AF",
    "BC",
    "DE",
    "HL",
    "SP",
    "IX",
    "IY",
    "IXH",
    "IXL",
    "IYH",
    "IYL",
    "HX",
    "LX",
    "HY",
    "LY",
    "XH",
    "XL",
    "YH",
    "YL",
    "NZ",
    "Z",
    "NC",
    "PO",
    "PE",
    "P",
    "M",
    "ADC",
    "ADD",
    "AND",
    "BIT",
    "CALL",
    "CCF",
    "CP",
    "CPD",
    "CPDR",
    "CPI",
    "CPIR",
    "CPL",
    "DAA",
    "DEC",
    "DI",
    "DJNZ",
    "EI",
    "EX",
    "EXX",
    "HALT",
    "IM",
    "IN",
    "INC",
    "IND",
    "INDR",
    "INI",
    "INIR",
    "JP",
    "JR",
    "LD",
    "LDD",
    "LDDR",
    "LDI",
    "LDIR",
    "NEG",
    "NOP",
    "OR",
    "OTDR",
    "OTIR",
    "OUT",
    "OUTD",
    "OUTI",
    "POP",
    "PUSH",
    "RES",
    "RET",
    "RETI",
    "RETN",
    "RL",
    "RLA",
    "RLC",
    "RLCA",
    "RLD",
    "RR",
    "RRA",
    "RRC",
    "RRCA",
    "RRD",
    "RST",
    "SBC",
    "SCF",
    "SET",
    "SLA",
    "SLL",
    "SLI",
    "SRA",
    "SRL",
    "SUB",
    "XOR",
    "ORG",
    "EQU",
    "DEFB",
    "DEFW",
    "DEFS",
    "DEFM",
    "DEFL",
    "DB",
    "DW",
    "DS",
    "DM",
    "DZ",
    "END",
    "IF",
    "ELSE",
    "ENDIF",
    "MACRO",
    "ENDM",
    "INCLUDE",
    "INCBIN",
    "DEVICE",
    "OUTPUT",
    "ALIGN",
    "BLOCK",
    "BYTE",
    "WORD",
    "DUP",
    "EDUP",
    "REPT",
    "ENDR",
    "STRUCT",
    "ENDS",
    "MODULE",
    "ENDMODULE",
    "ASSERT",
    "DISPLAY",
    "FIELD",
    "SAVEBIN",
    "OPT",
    "LUA",
    "ENDLUA",
];

/// Whether an instruction can be written as its mnemonic and trusted to come
/// back as the same bytes.
fn plain(bytes: &[u8], text: &str) -> bool {
    if text.starts_with("DB ") {
        return false;
    }
    match bytes[0] {
        // SLL is undocumented and spelt differently everywhere.
        0xCB => !(0x30..=0x37).contains(&bytes[1]),
        0xED => matches!(
            bytes[1],
            0x40 | 0x48 | 0x50 | 0x58 | 0x60 | 0x68 | 0x78
                | 0x41 | 0x49 | 0x51 | 0x59 | 0x61 | 0x69 | 0x79
                | 0x42 | 0x4A | 0x52 | 0x5A | 0x62 | 0x6A | 0x72 | 0x7A
                | 0x43 | 0x53 | 0x73 | 0x4B | 0x5B | 0x7B
                | 0x44 | 0x45 | 0x4D | 0x46 | 0x56 | 0x5E
                | 0x47 | 0x4F | 0x57 | 0x5F | 0x67 | 0x6F
                | 0xA0..=0xA3 | 0xA8..=0xAB | 0xB0..=0xB3 | 0xB8..=0xBB
        ),
        0xDD | 0xFD => {
            let index = text.contains("IX") || text.contains("IY");
            let halves = ["IXH", "IXL", "IYH", "IYL"]
                .iter()
                .any(|h| text.contains(h));
            if !index || halves {
                return false;
            }
            match bytes.get(1) {
                // DD CB d op: only the ones on (IX+d) alone, not the
                // undocumented copies into a register, and not SLL.
                Some(0xCB) => bytes
                    .get(3)
                    .is_some_and(|op| op & 7 == 6 && !(0x30..=0x37).contains(op)),
                _ => true,
            }
        }
        _ => true,
    }
}

/// A label as an assembler will take it: letters, digits and underscores, not
/// starting with a digit, and not one of the assembler's own words.
fn tidy(label: &str) -> String {
    let mut out: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out = "label".into();
    }
    if out.starts_with(|c: char| c.is_ascii_digit())
        || RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&out))
    {
        out = format!("L_{out}");
    }
    out
}

/// The names in use, kept unique however an assembler cases them.
struct Names {
    taken: HashSet<String>,
}

impl Names {
    fn claim(&mut self, wanted: &str, addr: u16) -> String {
        let mut name = tidy(wanted);
        if self.taken.contains(&name.to_lowercase()) {
            name = format!("{name}_{addr:04X}");
        }
        self.taken.insert(name.to_lowercase());
        name
    }
}

enum Body {
    Code(String),
    /// Bytes, and the instruction they are if they are one.
    Data(Vec<u8>, Option<String>),
    /// What somebody has said a stretch is: the directive, how many bytes it
    /// covers, and what goes beside it.
    Typed(String, u16, String),
}

struct Line {
    at: u16,
    body: Body,
    /// Comments on bytes inside the line rather than at its start.
    inner: Vec<(u16, String)>,
}

/// Write the source.
pub fn write(src: &Source) -> Exported {
    let peek = |a: u16| src.memory[a as usize];
    let mut names = Names {
        taken: HashSet::new(),
    };
    // Every label the notes have in RAM, by the name the file will use.
    let mut ram_names: BTreeMap<u16, (String, String)> = BTreeMap::new();
    for (addr, label, _) in src.notes.labelled() {
        if addr >= src.from && addr <= src.to {
            let name = names.claim(label, addr);
            ram_names.insert(addr, (name, label.to_string()));
        }
    }
    let labelled = |a: u16| ram_names.contains_key(&a);
    let commented = |a: u16| !src.notes.comment(a).trim().is_empty();

    // Lay the memory out as lines.
    let mut lines: Vec<Line> = Vec::new();
    let end = src.to as u32;
    let mut at: u32 = src.from as u32;
    while at <= end {
        let a = at as u16;
        if let Some(typed) = src.notes.layout().data.get(&a) {
            // The picture goes above a bitmap, so its rows have nothing beside them.
            let shown = crate::layout::picture(typed, &peek).is_empty();
            for row in crate::layout::rows(typed, &peek) {
                let beside = if shown { row.beside } else { String::new() };
                lines.push(Line {
                    at: row.at,
                    body: Body::Typed(row.directive, row.len, beside),
                    inner: Vec::new(),
                });
                at += row.len as u32;
            }
            continue;
        }
        if src.ran[a as usize] {
            let insn = disasm::disasm(&peek, a);
            let len = insn.len.max(1) as u32;
            let fits = at + len <= end + 1;
            // Another instruction starting inside this one is code that runs
            // two ways; bytes keep both honest. The byte after a DD, ED, CB or
            // FD prefix does not count: the Z80 fetches it as an opcode too,
            // so the tracker has it as run, and it is part of this one.
            let prefixed = matches!(insn.bytes[0], 0xCB | 0xED | 0xDD | 0xFD);
            let overlapped =
                fits && (1..len).any(|k| src.ran[(at + k) as usize] && !(prefixed && k == 1));
            if fits && !overlapped {
                let inner: Vec<(u16, String)> = (1..len)
                    .map(|k| (at + k) as u16)
                    .filter(|b| commented(*b))
                    .map(|b| (b, src.notes.comment(b).trim().to_string()))
                    .collect();
                let body = if plain(&insn.bytes, &insn.text) {
                    Body::Code(insn.text)
                } else {
                    Body::Data(insn.bytes.clone(), Some(insn.text))
                };
                lines.push(Line { at: a, body, inner });
                at += len;
                continue;
            }
        }
        // Data, sixteen bytes a line at most, and a new line wherever a label
        // or a comment starts or code begins.
        let mut bytes = vec![peek(a)];
        let mut next = at + 1;
        while next <= end && bytes.len() < 16 {
            let b = next as u16;
            if src.ran[b as usize]
                || labelled(b)
                || commented(b)
                || src.notes.layout().data.contains_key(&b)
            {
                break;
            }
            bytes.push(peek(b));
            next += 1;
        }
        at = next;
        lines.push(Line {
            at: a,
            body: Body::Data(bytes, None),
            inner: Vec::new(),
        });
    }

    // Labels that do not start a line — on an operand, most often, which is
    // what self-modifying code labels — are EQUs instead.
    let starts: HashSet<u16> = lines.iter().map(|l| l.at).collect();
    let equ_inside: Vec<(u16, String)> = ram_names
        .iter()
        .filter(|(addr, _)| !starts.contains(addr))
        .map(|(addr, (name, _))| (*addr, name.clone()))
        .collect();

    // Addresses in instructions become names where there are names for them.
    let mut rom_names: BTreeMap<u16, String> = BTreeMap::new();
    let mut named = |text: &str| -> String {
        crate::layout::replace_addresses(text, &mut |value| {
            if value >= src.from && value <= src.to {
                ram_names.get(&value).map(|(n, _)| n.clone())
            } else if let Some(n) = rom_names.get(&value) {
                Some(n.clone())
            } else {
                (src.rom_name)(value).map(|wanted| {
                    let n = names.claim(&wanted, value);
                    rom_names.insert(value, n.clone());
                    n
                })
            }
        })
    };
    let bodies: Vec<String> = lines
        .iter()
        .map(|l| match &l.body {
            Body::Code(text) => named(text),
            Body::Typed(directive, _, _) => named(directive),
            Body::Data(bytes, _) => format!(
                "DEFB {}",
                bytes
                    .iter()
                    .map(|b| format!("${b:02X}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        })
        .collect();

    let mut out = String::new();
    for line in &src.header {
        out.push_str(&format!("; {line}\n").replace("; \n", ";\n"));
    }
    out.push_str(&format!(
        ";\n\
         ; This file is ${:04X}-${:04X}. Anything it names outside that, the ROM\n\
         ; included, is an EQU with the address, under the name the notes or the\n\
         ; ROM's symbols give it.\n\
         ; Code is what was seen to run since this program was started, and\n\
         ; everything else is DEFB. Undocumented instructions, and documented ones\n\
         ; with more than one encoding, are DEFB with the instruction beside them,\n\
         ; so any assembler gives back the same bytes.\n\
         ; \"seen:\" and \"used by:\" lines and [square brackets] were measured, not\n\
         ; written by anybody.\n\
         ; To build it: sjasmplus --raw=program.bin this.asm, or pasmo this.asm\n\
         ; program.bin. Either gives the {} bytes from ${:04X}.\n\n",
        src.from,
        src.to,
        src.to as u32 - src.from as u32 + 1,
        src.from
    ));
    for (addr, name) in &rom_names {
        let place = if *addr < 0x4000 {
            "in the ROM"
        } else {
            "outside this file"
        };
        out.push_str(&format!("{name} EQU ${addr:04X}   ; {place}\n"));
    }
    for (addr, name) in &equ_inside {
        out.push_str(&format!(
            "{name} EQU ${addr:04X}   ; inside an instruction or a run of bytes below\n"
        ));
    }
    out.push_str(&format!("\n        ORG ${:04X}\n", src.from));

    let mut instructions = 0;
    let mut data_bytes = 0;
    for (line, body) in lines.iter().zip(bodies) {
        let labelled = ram_names.get(&line.at);
        if let Some((name, original)) = labelled {
            // Said when the name had to change, so the notes can be matched up.
            if original == name {
                out.push_str(&format!("\n{name}:\n"));
            } else {
                out.push_str(&format!("\n{name}:   ; labelled \"{original}\"\n"));
            }
        }
        // A label's description goes above it, as does anything too long to
        // read at the end of a line of code.
        let comment = src.notes.comment(line.at).trim().to_string();
        let guess = if src.notes.comment_is_auto(line.at) {
            "guess: "
        } else {
            ""
        };
        let lead = if labelled.is_some() { "" } else { "        " };
        let mut beside: Vec<String> = Vec::new();
        if labelled.is_some() || comment.contains('\n') || comment.len() > BESIDE {
            for (i, part) in wrap(&format!("{guess}{comment}"), 76).iter().enumerate() {
                if !part.is_empty() || i > 0 {
                    out.push_str(format!("{lead}; {part}").trim_end());
                    out.push('\n');
                }
            }
        } else if !comment.is_empty() {
            beside.push(format!("{guess}{comment}"));
        }
        if labelled.is_some() {
            for fact in (src.above)(line.at) {
                out.push_str(&format!("; {fact}\n"));
            }
        }
        if let Some(typed) = src.notes.layout().data.get(&line.at) {
            for row in crate::layout::picture(typed, &peek) {
                out.push_str(format!("{lead}; {row}").trim_end());
                out.push('\n');
            }
        }
        let length = match &line.body {
            Body::Code(_) => disasm::disasm(&peek, line.at).len,
            Body::Data(bytes, Some(_)) => bytes.len() as u8,
            _ => 0,
        };
        if length > 0 {
            let marks = (src.marks)(line.at, length);
            if !marks.is_empty() {
                beside.push(marks);
            }
        }
        match &line.body {
            Body::Code(text) => {
                instructions += 1;
                if let Some((_, said)) = src.notes.layout().name_displacement(line.at, text) {
                    beside.push(said);
                }
            }
            Body::Typed(_, length, said) => {
                data_bytes += *length as usize;
                if !said.is_empty() {
                    beside.push(said.clone());
                }
            }
            Body::Data(bytes, text) => {
                data_bytes += bytes.len();
                if let Some(text) = text {
                    beside.push(text.clone());
                }
            }
        }
        for (at, c) in &line.inner {
            beside.push(format!("${at:04X}: {}", c.replace('\n', " / ")));
        }
        if beside.is_empty() {
            out.push_str(&format!("        {body}\n"));
        } else {
            out.push_str(&format!("        {body:<28}; {}\n", beside.join("; ")));
        }
    }
    Exported {
        text: out,
        instructions,
        data_bytes,
    }
}

/// ROM dumps this knows by their CRC32, as dsp-emulator lists them.
const KNOWN_ROMS: &[(u32, &str)] = &[
    (0xDDEE531F, "the Sinclair 48K ROM"),
    (0xE76799D2, "Sinclair's 128K ROM 0, the editor"),
    (0xB96A36BE, "Sinclair's 128K ROM 1, 48 BASIC"),
    (0x5D2E8C66, "the +2 ROM 0"),
    (0x98B1320B, "the +2 ROM 1"),
    (0x30C9F490, "the +3 ROM 0"),
    (0xA7916B3F, "the +3 ROM 1"),
    (0xC9A0B748, "the +3 ROM 2"),
    (0xB88FD6E3, "the +3 ROM 3"),
];

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}

/// A line saying which ROM, and how to tell it is that one.
pub fn describe_rom(what: &str, bytes: &[u8]) -> String {
    let sum = crc32(bytes);
    let known = KNOWN_ROMS
        .iter()
        .find(|(c, _)| *c == sum)
        .map_or("not a dump this knows", |(_, name)| name);
    format!("  {what}: {} bytes, CRC32 {sum:08X} — {known}", bytes.len())
}

/// Everything the file says about what the program was built against.
fn header(spec: &Spectrum, program: &str) -> Vec<String> {
    let bus = &spec.bus;
    let mut lines = vec![
        program.to_string(),
        format!(
            "Exported by ZX-Rustrum {} from a ZX Spectrum {}.",
            env!("CARGO_PKG_VERSION"),
            bus.model.name()
        ),
        String::new(),
        "Built against, and not included here:".to_string(),
    ];
    let rom = &bus.rom;
    let pages = rom.len() / 0x4000;
    for page in 0..pages {
        let what = if pages == 1 {
            "ROM".to_string()
        } else {
            format!("ROM {page}")
        };
        lines.push(describe_rom(
            &what,
            &rom[page * 0x4000..(page + 1) * 0x4000],
        ));
    }
    if let Some(if1) = &bus.if1 {
        if let Some(rom) = &if1.rom {
            lines.push(describe_rom("Interface 1 ROM", rom));
        }
    }
    for mf in &bus.multifaces {
        if let Some(rom) = &mf.rom {
            lines.push(describe_rom(&format!("{:?} ROM", mf.model), rom));
        }
    }
    if let Some(rom) = bus.uspeech.as_ref().and_then(|u| u.rom.as_ref()) {
        lines.push(describe_rom("Currah µSpeech ROM", rom));
    }
    match bus.model {
        Model::Spectrum48 => {}
        Model::Spectrum128 => lines.push(format!(
            "Paged in: ROM {} at $0000, RAM bank {} at $C000.",
            (bus.page_reg >> 4) & 1,
            bus.page_reg & 7
        )),
        Model::Plus2A | Model::Plus3 => {
            if bus.page_reg_1ffd & 1 != 0 {
                lines.push(
                    "Special paging is on: $0000-$3FFF is RAM, and is left out all the same."
                        .to_string(),
                );
            } else {
                let page = ((bus.page_reg_1ffd >> 1) & 2) | ((bus.page_reg >> 4) & 1);
                lines.push(format!(
                    "Paged in: ROM {page} at $0000, RAM bank {} at $C000.",
                    bus.page_reg & 7
                ));
            }
        }
    }
    lines
}

/// The machine's RAM as source, with the notes' labels and comments.
pub fn from_machine(
    spec: &Spectrum,
    notes: &Notes,
    symbols: Option<&crate::autodoc::Symbols>,
    program: &str,
) -> Exported {
    from_machine_with(spec, notes, symbols, program, Extras::default())
}

/// What an export can be given beyond the machine and its notes.
pub struct Extras<'a> {
    /// What else is known to have run, by physical location as the tracker
    /// keeps it.
    pub ran_before: &'a [bool],
    pub from: u16,
    pub to: u16,
    pub above: &'a dyn Fn(u16) -> Vec<String>,
    pub marks: &'a dyn Fn(u16, u8) -> String,
}

impl Default for Extras<'_> {
    fn default() -> Self {
        Extras {
            ran_before: &[],
            from: FROM,
            to: 0xFFFF,
            above: &|_| Vec::new(),
            marks: &|_, _| String::new(),
        }
    }
}

/// As `from_machine`, with more to go on.
pub fn from_machine_with(
    spec: &Spectrum,
    notes: &Notes,
    symbols: Option<&crate::autodoc::Symbols>,
    program: &str,
    extras: Extras,
) -> Exported {
    let ran_before = extras.ran_before;
    let memory: Vec<u8> = (0..=0xFFFFu16).map(|a| spec.bus.peek_raw(a)).collect();
    let ran: Vec<bool> = (0..=0xFFFFu16)
        .map(|a| {
            let phys = spec.bus.phys_index(a);
            spec.bus.tracker.executed[phys] || ran_before.get(phys).copied().unwrap_or(false)
        })
        .collect();
    let rom_name = |a: u16| -> Option<String> {
        let own = notes.label(a);
        if !own.is_empty() {
            return Some(own.to_string());
        }
        symbols
            .and_then(|s| s.get(a))
            .map(|(name, _)| name.to_string())
    };
    write(&Source {
        from: extras.from,
        to: extras.to,
        above: extras.above,
        marks: extras.marks,
        memory: &memory,
        ran: &ran,
        notes,
        rom_name: &rom_name,
        header: header(spec, program),
    })
}
