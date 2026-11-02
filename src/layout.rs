//! What the bytes of a program are, where somebody has said: tables of words,
//! text, bitmaps, and the records a register walks over.
//!
//! All of this is a claim made by whoever is reading the program, kept in the
//! notes file and never worked out here, so it says nothing about what the
//! machine does. Its one duty is to show every byte it was told about: a row
//! here always holds exactly the bytes at its address, which is what lets an
//! export of it assemble back into the same program.

use std::collections::BTreeMap;

/// What a stretch of memory holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Sixteen-bit values, usually addresses.
    Words,
    Text,
    /// `width` bytes a row for `rows` rows, after `header` bytes that are not
    /// picture. Some programs draw up the screen, so the first row in memory is
    /// the bottom of the picture.
    Bitmap {
        width: u8,
        rows: u16,
        header: u8,
        bottom_up: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Typed {
    pub at: u16,
    pub kind: Kind,
    /// How many bytes it covers.
    pub length: u16,
}

/// A record: fields by offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Struct {
    pub size: u16,
    pub fields: BTreeMap<u8, String>,
}

/// Records side by side in memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Array {
    pub at: u16,
    pub of: String,
    pub count: u16,
}

/// Over this range of code, the register points at one of these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub register: String,
    pub of: String,
    pub from: u16,
    pub to: u16,
}

/// One line of a typed stretch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub at: u16,
    pub len: u16,
    /// What an assembler would take: DEFB, DEFW or DEFM.
    pub directive: String,
    /// What goes beside it, such as the row of the picture.
    pub beside: String,
}

/// Everything said about the shape of a program's data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub data: BTreeMap<u16, Typed>,
    pub structs: BTreeMap<String, Struct>,
    pub arrays: BTreeMap<u16, Array>,
    pub bindings: Vec<Binding>,
}

impl Layout {
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
            && self.structs.is_empty()
            && self.arrays.is_empty()
            && self.bindings.is_empty()
    }

    /// The name of a byte inside a record: `rocket.x` where the record has a
    /// label, or `object[6].x` by its place in the table where it does not.
    pub fn field_name(&self, addr: u16, label: &dyn Fn(u16) -> String) -> Option<String> {
        let array = self.arrays.values().find(|a| {
            let size = self.structs.get(&a.of).map_or(0, |s| s.size) as u32;
            addr >= a.at && (addr as u32) < a.at as u32 + size * a.count as u32
        })?;
        let shape = self.structs.get(&array.of)?;
        let offset = addr - array.at;
        let slot = offset / shape.size;
        let within = (offset % shape.size) as u8;
        let own = label(array.at + slot * shape.size);
        let record = if own.is_empty() {
            format!("{}[{slot}]", array.of)
        } else {
            own
        };
        Some(match shape.fields.get(&within) {
            Some(field) => format!("{record}.{field}"),
            None => format!("{record}+{within}"),
        })
    }

    /// What a register's displacement means at this address. The narrowest
    /// range wins, so one handler can say more than the whole table does.
    pub fn bound_field(&self, at: u16, register: &str, displacement: u8) -> Option<&str> {
        let binding = self
            .bindings
            .iter()
            .filter(|b| b.register == register && at >= b.from && at <= b.to)
            .min_by_key(|b| b.to - b.from)?;
        self.structs
            .get(&binding.of)?
            .fields
            .get(&displacement)
            .map(String::as_str)
    }

    /// An instruction's `(IX+$05)` with the field named, if there is a name.
    pub fn name_displacement(&self, at: u16, text: &str) -> Option<(String, String)> {
        for register in ["IX", "IY"] {
            let opening = format!("({register}+$");
            let Some(start) = text.find(&opening) else {
                continue;
            };
            let digits = start + opening.len();
            let value = u8::from_str_radix(text.get(digits..digits + 2)?, 16).ok()?;
            let field = self.bound_field(at, register, value)?;
            let named = format!(
                "{}({register}+{field}){}",
                &text[..start],
                &text[digits + 3..]
            );
            return Some((named, format!("{register}+${value:02X} is {field}")));
        }
        None
    }

    /// The lines of the file these are kept in.
    pub fn to_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, shape) in &self.structs {
            let fields: Vec<String> = shape
                .fields
                .iter()
                .map(|(offset, field)| format!("{offset}={field}"))
                .collect();
            out.push(format!("struct {name} {} {}", shape.size, fields.join(" ")));
        }
        for array in self.arrays.values() {
            out.push(format!(
                "array {:04X} {} {}",
                array.at, array.of, array.count
            ));
        }
        for b in &self.bindings {
            out.push(format!(
                "bind {} {} {:04X}-{:04X}",
                b.register, b.of, b.from, b.to
            ));
        }
        for typed in self.data.values() {
            out.push(match typed.kind {
                Kind::Words => format!("data {:04X} words {}", typed.at, typed.length / 2),
                Kind::Text => format!("data {:04X} text {}", typed.at, typed.length),
                Kind::Bitmap {
                    width,
                    rows,
                    header,
                    bottom_up,
                } => format!(
                    "data {:04X} bitmap {width} {rows} {header}{}",
                    typed.at,
                    if bottom_up { " bottom_up" } else { "" }
                ),
            });
        }
        out
    }

    /// Takes one line of the notes file, if it is one of these. Anything it
    /// cannot read is left alone: these files are edited by hand.
    pub fn read_line(&mut self, line: &str) {
        let words: Vec<&str> = line.split_whitespace().collect();
        let hex = |s: &str| u16::from_str_radix(s, 16).ok();
        let number = |s: &str| s.parse::<u16>().ok();
        match words.as_slice() {
            ["struct", name, size, fields @ ..] => {
                if let Some(size) = number(size).filter(|s| *s > 0) {
                    self.structs.insert(
                        name.to_string(),
                        Struct {
                            size,
                            fields: parse_fields(&fields.join(" ")),
                        },
                    );
                }
            }
            ["array", at, of, count] => {
                if let (Some(at), Some(count)) = (hex(at), number(count)) {
                    self.arrays.insert(
                        at,
                        Array {
                            at,
                            of: of.to_string(),
                            count,
                        },
                    );
                }
            }
            ["bind", register, of, range] => {
                if let Some((from, to)) = range.split_once('-') {
                    if let (Some(from), Some(to)) = (hex(from), hex(to)) {
                        self.bindings.push(Binding {
                            register: register.to_string(),
                            of: of.to_string(),
                            from,
                            to,
                        });
                    }
                }
            }
            ["data", at, "words", count] => {
                if let (Some(at), Some(count)) = (hex(at), number(count)) {
                    self.set_data(at, Kind::Words, count.saturating_mul(2));
                }
            }
            ["data", at, "text", length] => {
                if let (Some(at), Some(length)) = (hex(at), number(length)) {
                    self.set_data(at, Kind::Text, length);
                }
            }
            ["data", at, "bitmap", width, rows, header, rest @ ..] => {
                if let (Some(at), Some(width), Some(rows), Some(header)) =
                    (hex(at), number(width), number(rows), number(header))
                {
                    self.set_bitmap(at, width as u8, rows, header as u8, rest == ["bottom_up"]);
                }
            }
            _ => {}
        }
    }

    pub fn set_data(&mut self, at: u16, kind: Kind, length: u16) {
        self.data.insert(at, Typed { at, kind, length });
    }

    pub fn set_bitmap(&mut self, at: u16, width: u8, rows: u16, header: u8, bottom_up: bool) {
        let length = header as u16 + width as u16 * rows;
        self.set_data(
            at,
            Kind::Bitmap {
                width,
                rows,
                header,
                bottom_up,
            },
            length,
        );
    }
}

/// `0=type 1=x` into fields. A piece without an `=` is skipped.
pub fn parse_fields(text: &str) -> BTreeMap<u8, String> {
    text.split_whitespace()
        .filter_map(|piece| {
            let (offset, name) = piece.split_once('=')?;
            Some((offset.parse().ok()?, name.to_string()))
        })
        .collect()
}

fn defb(bytes: &[u8]) -> String {
    let list: Vec<String> = bytes.iter().map(|b| format!("${b:02X}")).collect();
    format!("DEFB {}", list.join(","))
}

/// One row of a picture, a character a pixel, with the left-hand pixel the
/// byte's top bit, as the display file has it.
pub fn pixels(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|b| (0..8).rev().map(move |bit| (b >> bit) & 1 == 1))
        .map(|on| if on { '#' } else { '.' })
        .collect()
}

/// A bitmap as a picture, a line a row, the right way up: two characters a pixel,
/// because a character is about twice as tall as it is wide and one a pixel squashes
/// a sprite to a sliver. Empty for anything that isn't a bitmap.
pub fn picture(typed: &Typed, peek: &dyn Fn(u16) -> u8) -> Vec<String> {
    let Kind::Bitmap {
        width,
        rows,
        header,
        bottom_up,
    } = typed.kind
    else {
        return Vec::new();
    };
    let width = width.max(1) as u16;
    let mut lines: Vec<String> = (0..rows)
        .map(|row| {
            let start = typed.at.wrapping_add(header as u16 + row * width);
            let bytes: Vec<u8> = (0..width).map(|i| peek(start.wrapping_add(i))).collect();
            let line: String = pixels(&bytes)
                .chars()
                .map(|c| if c == '#' { "##" } else { "  " })
                .collect();
            line.trim_end().to_string()
        })
        .collect();
    if bottom_up {
        lines.reverse();
    }
    lines
}

/// A character an assembler takes inside a quoted string without any fuss.
fn plain_character(b: u8) -> bool {
    // A dollar too, since addresses in a line are turned into names.
    (0x20..0x7F).contains(&b) && !matches!(b, b'"' | b'\\' | b'\'' | b'`' | b'$')
}

/// A stretch of text: what can be quoted is, and what cannot is bytes, so a
/// string ending in a character with its top bit set says what that character
/// is rather than losing it.
fn text_rows(at: u16, bytes: &[u8]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        if plain_character(bytes[i]) {
            while i < bytes.len() && plain_character(bytes[i]) && i - start < 48 {
                i += 1;
            }
            let quoted: String = bytes[start..i].iter().map(|b| *b as char).collect();
            rows.push(Row {
                at: at + start as u16,
                len: (i - start) as u16,
                directive: format!("DEFM \"{quoted}\""),
                beside: String::new(),
            });
        } else {
            i += 1;
            let low = bytes[start] & 0x7F;
            let beside = if bytes[start] >= 0x80 && (0x20..0x7F).contains(&low) {
                format!("'{}' with the top bit set", low as char)
            } else {
                String::new()
            };
            rows.push(Row {
                at: at + start as u16,
                len: 1,
                directive: defb(&bytes[start..=start]),
                beside,
            });
        }
    }
    rows
}

/// The lines a typed stretch is written as.
pub fn rows(typed: &Typed, peek: &dyn Fn(u16) -> u8) -> Vec<Row> {
    let bytes: Vec<u8> = (0..typed.length)
        .map(|i| peek(typed.at.wrapping_add(i)))
        .collect();
    match typed.kind {
        Kind::Text => text_rows(typed.at, &bytes),
        Kind::Words => bytes
            .chunks(2)
            .enumerate()
            .map(|(i, pair)| Row {
                at: typed.at + (i as u16) * 2,
                len: pair.len() as u16,
                directive: match pair {
                    [low, high] => {
                        format!("DEFW ${:04X}", u16::from_le_bytes([*low, *high]))
                    }
                    other => defb(other),
                },
                beside: String::new(),
            })
            .collect(),
        Kind::Bitmap {
            width,
            header,
            bottom_up,
            ..
        } => {
            let header = (header as usize).min(bytes.len());
            let width = width.max(1) as usize;
            let mut out = Vec::new();
            if header > 0 {
                out.push(Row {
                    at: typed.at,
                    len: header as u16,
                    directive: defb(&bytes[..header]),
                    beside: String::new(),
                });
            }
            let picture: Vec<&[u8]> = bytes[header..].chunks(width).collect();
            for (i, row) in picture.iter().enumerate() {
                // Upside-down rows can't be read as a picture, so the same rows
                // go beside them again from the other end, which is what the
                // screen will show. The first column stays the bytes on its line.
                let beside = if bottom_up {
                    let drawn = picture[picture.len() - 1 - i];
                    format!("{} | {}", pixels(row), pixels(drawn))
                } else {
                    pixels(row)
                };
                out.push(Row {
                    at: typed.at + (header + i * width) as u16,
                    len: row.len() as u16,
                    directive: defb(row),
                    beside,
                });
            }
            out
        }
    }
}

/// Every four-digit address in a line of assembly, each replaced by what
/// `name` says it is called, if anything.
pub fn replace_addresses(text: &str, name: &mut dyn FnMut(u16) -> Option<String>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let four = i + 5 <= chars.len()
            && chars[i] == '$'
            && chars[i + 1..i + 5].iter().all(|c| c.is_ascii_hexdigit())
            && !chars.get(i + 5).is_some_and(|c| c.is_ascii_hexdigit());
        if four {
            let hex: String = chars[i + 1..i + 5].iter().collect();
            let value = u16::from_str_radix(&hex, 16).expect("four hex digits");
            if let Some(found) = name(value) {
                out.push_str(&found);
                i += 5;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}
