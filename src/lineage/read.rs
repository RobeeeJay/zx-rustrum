//! Reading a disassembly into runs of instructions: the `.asm` that
//! `export_asm` writes into `disassembly/`.
//!
//! It is brought to the emulator's own spelling — `LD A,($8000)` whether the
//! file said `LD A,(score)` or `LD A,(32768)` — and from that to a shape with
//! the numbers and names taken out, so code read from a file is compared
//! fairly with code read from memory.

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// One instruction, wherever it was read from.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub at: u16,
    pub len: u16,
    /// The instruction in the emulator's own spelling.
    pub text: String,
}

/// What a disassembly holds that a fingerprint uses.
#[derive(Clone, Debug, Default)]
pub struct Disassembly {
    pub runs: Vec<Vec<Line>>,
    pub labels: BTreeMap<u16, String>,
    /// The file the game was loaded from, where the disassembly says.
    pub source: Option<String>,
}

const REGISTERS: &[&str] = &[
    "A", "B", "C", "D", "E", "H", "L", "I", "R", "F", "AF", "AF'", "BC", "DE", "HL", "SP", "IX",
    "IY", "IXH", "IXL", "IYH", "IYL", "NZ", "Z", "NC", "PO", "PE", "P", "M",
];

/// Some assemblers spell the index registers' halves otherwise.
fn register(word: &str) -> Option<&'static str> {
    let upper = word.to_ascii_uppercase();
    let upper = match upper.as_str() {
        "XH" | "HX" => "IXH",
        "XL" | "LX" => "IXL",
        "YH" | "HY" => "IYH",
        "YL" | "LY" => "IYL",
        other => other,
    };
    REGISTERS.iter().find(|r| **r == upper).copied()
}

/// A number as an assembler might write it: `$1F`, `#1F`, `0x1F`, `1Fh`,
/// `%11111`, `31`, or a character in quotes.
fn number(text: &str) -> Option<u32> {
    let t = text.trim();
    let quoted = |q: char| t.len() == 3 && t.starts_with(q) && t.ends_with(q);
    if quoted('"') || quoted('\'') {
        return t.chars().nth(1).map(|c| c as u32);
    }
    let (digits, radix) = if let Some(rest) = t.strip_prefix('$').or_else(|| t.strip_prefix('#')) {
        (rest, 16)
    } else if let Some(rest) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (rest, 16)
    } else if let Some(rest) = t.strip_prefix('%') {
        (rest, 2)
    } else if let Some(rest) = t
        .strip_suffix('h')
        .or_else(|| t.strip_suffix('H'))
        .filter(|r| r.starts_with(|c: char| c.is_ascii_digit()))
    {
        (rest, 16)
    } else {
        (t, 10)
    };
    if digits.is_empty() {
        return None;
    }
    u32::from_str_radix(digits, radix).ok()
}

fn hex(value: u32) -> String {
    if value <= 0xFF {
        format!("${value:02X}")
    } else {
        format!("${:04X}", value & 0xFFFF)
    }
}

/// An operand that is a value: a number becomes `$xx`, and anything else — a
/// label, an expression — is left as it is written, since only its shape is
/// ever compared.
fn value(text: &str) -> String {
    number(text)
        .map(hex)
        .unwrap_or_else(|| text.trim().to_string())
}

/// Split at commas outside quotes.
fn operands(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for c in text.chars() {
        match (c, quote) {
            ('"' | '\'', None) if !current.trim().eq_ignore_ascii_case("AF") => {
                quote = Some(c);
                current.push(c);
            }
            (q, Some(open)) if q == open => {
                quote = None;
                current.push(c);
            }
            (',', None) => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() || !out.is_empty() {
        out.push(current);
    }
    out.into_iter().map(|o| o.trim().to_string()).collect()
}

/// Cut a comment off a line, minding quotes.
pub fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    for (i, c) in line.char_indices() {
        match (c, quote) {
            (';', None) => return &line[..i],
            ('"', None) => quote = Some('"'),
            (q, Some(open)) if q == open => quote = None,
            _ => {}
        }
    }
    line
}

/// The instruction in the emulator's own spelling.
pub fn canonical(text: &str) -> String {
    let text = text.trim();
    let (mnemonic, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let mnemonic = mnemonic.to_ascii_uppercase();
    let decimal_first = matches!(mnemonic.as_str(), "IM" | "BIT" | "SET" | "RES");
    let parts: Vec<String> = operands(rest)
        .iter()
        .enumerate()
        .map(|(i, o)| {
            if i == 0 && decimal_first {
                return number(o)
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| o.clone());
            }
            if let Some(r) = register(o) {
                return r.to_string();
            }
            if let Some(inner) = o.strip_prefix('(').and_then(|o| o.strip_suffix(')')) {
                let inner = inner.trim();
                if let Some(r) = register(inner) {
                    return format!("({r})");
                }
                let upper = inner.to_ascii_uppercase();
                for index in ["IX", "IY"] {
                    if let Some(offset) = upper.strip_prefix(index) {
                        let offset = offset.trim();
                        let (sign, amount) = match offset.chars().next() {
                            Some('-') => ("-", &inner[inner.len() - offset.len() + 1..]),
                            Some('+') => ("+", &inner[inner.len() - offset.len() + 1..]),
                            _ => ("+", "0"),
                        };
                        return format!("({index}{sign}{})", value(amount));
                    }
                }
                return format!("({})", value(inner));
            }
            value(o)
        })
        .collect();
    if parts.is_empty() {
        mnemonic
    } else {
        format!("{mnemonic} {}", parts.join(","))
    }
}

/// The instruction with its numbers and names taken out: `LD H,$B7` and
/// `LD H,page` are both `LD H,n`. A bit number, an interrupt mode and a
/// restart stay, since they choose the opcode rather than being its operand.
pub fn shape(canonical: &str) -> String {
    let (mnemonic, rest) = canonical.split_once(' ').unwrap_or((canonical, ""));
    let keep_first = matches!(mnemonic, "IM" | "BIT" | "SET" | "RES" | "RST");
    let parts: Vec<String> = operands(rest)
        .iter()
        .enumerate()
        .map(|(i, o)| {
            if (i == 0 && keep_first) || REGISTERS.contains(&o.as_str()) {
                return o.clone();
            }
            if let Some(inner) = o.strip_prefix('(').and_then(|o| o.strip_suffix(')')) {
                if REGISTERS.contains(&inner) {
                    return o.clone();
                }
                for index in ["IX", "IY"] {
                    if inner.starts_with(index) {
                        return format!("({index}+n)");
                    }
                }
                return "(n)".into();
            }
            "n".into()
        })
        .collect();
    if parts.is_empty() {
        mnemonic.to_string()
    } else {
        format!("{mnemonic} {}", parts.join(","))
    }
}

/// How long an instruction of each shape is, read off the emulator's own
/// decoder for every opcode there is. Where two encodings share a shape the
/// shorter is the one an assembler picks.
fn lengths() -> &'static BTreeMap<String, u16> {
    static TABLE: OnceLock<BTreeMap<String, u16>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table: BTreeMap<String, u16> = BTreeMap::new();
        let mut add = |bytes: &[u8]| {
            let mut padded = bytes.to_vec();
            padded.resize(4, 0);
            let peek = |a: u16| padded.get(a as usize).copied().unwrap_or(0);
            let insn = crate::disasm::disasm(&peek, 0);
            let len = insn.len.max(1) as u16;
            let shape = shape(&canonical(&insn.text));
            let entry = table.entry(shape).or_insert(len);
            *entry = (*entry).min(len);
        };
        for op in 0..=255u8 {
            add(&[op]);
            add(&[0xCB, op]);
            add(&[0xED, op]);
            for prefix in [0xDD, 0xFD] {
                add(&[prefix, op]);
                add(&[prefix, 0xCB, 0, op]);
            }
        }
        table
    })
}

pub fn length_of(canonical: &str) -> Option<u16> {
    lengths().get(&shape(canonical)).copied()
}

/// Whether control cannot fall through this instruction to the next. A `JR`
/// is left out: it mostly hops over a few bytes inside the routine it is in.
pub fn ends_run(text: &str) -> bool {
    matches!(text, "RET" | "RETI" | "RETN") || (text.starts_with("JP ") && !text.contains(','))
}

/// How many bytes a data directive puts down, or `None` if the line is not
/// one.
fn data_size(mnemonic: &str, rest: &str) -> Option<Result<u32, String>> {
    let items = operands(rest);
    let count = |item: &String| -> u32 {
        let quoted = item.len() >= 2 && (item.starts_with('"') || item.starts_with('\''));
        if quoted {
            item.len() as u32 - 2
        } else {
            1
        }
    };
    match mnemonic {
        "DEFB" | "DB" | "DEFM" | "DM" => Some(Ok(items.iter().map(count).sum())),
        "DEFW" | "DW" => Some(Ok(items.len() as u32 * 2)),
        "DEFS" | "DS" => Some(
            items
                .first()
                .and_then(|n| number(n))
                .ok_or_else(|| format!("{mnemonic} with no size: {rest}")),
        ),
        _ => None,
    }
}

/// Runs of instructions, cut wherever control cannot fall through, data
/// comes between, or the addresses jump.
#[derive(Default)]
struct Runs {
    runs: Vec<Vec<Line>>,
    run: Vec<Line>,
}

impl Runs {
    fn push(&mut self, line: Line) {
        if let Some(last) = self.run.last() {
            if last.at.wrapping_add(last.len) != line.at {
                self.end();
            }
        }
        let ends = ends_run(&line.text);
        self.run.push(line);
        if ends {
            self.end();
        }
    }

    fn end(&mut self) {
        if !self.run.is_empty() {
            self.runs.push(std::mem::take(&mut self.run));
        }
    }

    fn finish(mut self) -> Vec<Vec<Line>> {
        self.end();
        self.runs
    }
}

/// The `.asm` that `export_asm` writes, or another in the same plain form.
/// Nothing in it says where an instruction is, so the address is counted up
/// from the ORG, and an instruction whose length cannot be told stops the
/// reading rather than putting everything after it in the wrong place.
pub fn read_asm(text: &str) -> Result<Disassembly, String> {
    let mut out = Disassembly::default();
    let mut runs = Runs::default();
    let mut at: u32 = 0;
    let mut pending: Option<String> = None;
    for (number_of, raw) in text.lines().enumerate() {
        if out.source.is_none() {
            for kind in ["; tape ", "; snapshot ", "; recording "] {
                if let Some(path) = raw.strip_prefix(kind) {
                    out.source = Some(path.trim().to_string());
                }
            }
        }
        let line = strip_comment(raw);
        if line.trim().is_empty() {
            continue;
        }
        let mut body = line;
        if !line.starts_with(char::is_whitespace) {
            let (first, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            if rest.trim_start().to_ascii_uppercase().starts_with("EQU") {
                continue;
            }
            match first.strip_suffix(':') {
                Some(name) => {
                    pending = Some(name.to_string());
                    body = rest;
                }
                None => {
                    return Err(format!(
                        "line {}: cannot read {:?}",
                        number_of + 1,
                        raw.trim()
                    ))
                }
            }
            if body.trim().is_empty() {
                continue;
            }
        }
        let body = body.trim();
        let (mnemonic, rest) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
        let mnemonic = mnemonic.to_ascii_uppercase();
        if mnemonic == "ORG" {
            at = number(rest).ok_or_else(|| format!("line {}: ORG {rest}", number_of + 1))?;
            runs.end();
            continue;
        }
        if let Some(name) = pending.take() {
            out.labels.insert(at as u16, name);
        }
        if let Some(size) = data_size(&mnemonic, rest) {
            at += size.map_err(|e| format!("line {}: {e}", number_of + 1))?;
            runs.end();
            continue;
        }
        let text = canonical(body);
        let len = length_of(&text).ok_or_else(|| {
            format!(
                "line {}: no length known for {:?}, so nothing after it can be placed",
                number_of + 1,
                body
            )
        })?;
        runs.push(Line {
            at: at as u16,
            len,
            text,
        });
        at += len as u32;
    }
    out.runs = runs.finish();
    Ok(out)
}
