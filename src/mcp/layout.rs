//! Saying what a program's data is. See `crate::layout`.

use crate::layout::{parse_fields, Binding, Kind, Struct};
use crate::mcp::json::Json;
use crate::mcp::tools::{addr, count, flag, text, Session};

pub fn set_data(session: &mut Session, args: &Json) -> Result<String, String> {
    let at = addr(args, "address")?;
    let kind = text(args, "kind")?;
    match kind.as_str() {
        "bytes" => session.notes.change_layout(|l| {
            l.data.remove(&at);
        }),
        "words" => {
            let words = count(args, "length", 1)? as u16;
            session
                .notes
                .change_layout(|l| l.set_data(at, Kind::Words, words.saturating_mul(2)));
        }
        "text" => {
            let length = count(args, "length", 1)? as u16;
            session
                .notes
                .change_layout(|l| l.set_data(at, Kind::Text, length));
        }
        "bitmap" => {
            let width = count(args, "width", 1)?;
            let rows = count(args, "rows", 8)?;
            let header = count(args, "header", 0)?;
            if width == 0 || width > 32 || rows == 0 || header > 255 {
                return Err(format!(
                    "a bitmap needs a width of 1 to 32 bytes and at least one row, \
                     not width {width}, rows {rows}, header {header}"
                ));
            }
            let bottom_up = flag(args, "bottom_up", false);
            session.notes.change_layout(|l| {
                l.set_bitmap(at, width as u8, rows as u16, header as u8, bottom_up)
            });
        }
        other => {
            return Err(format!(
                "kind {other:?}: it should be words, text, bitmap or bytes"
            ))
        }
    }
    Ok(format!("${at:04X} is {kind} now."))
}

pub fn define_struct(session: &mut Session, args: &Json) -> Result<String, String> {
    let name = text(args, "name")?;
    let size = count(args, "size", 0)?;
    if size == 0 || size > 255 {
        return Err(format!("a size of {size}: it should be 1 to 255"));
    }
    let fields = parse_fields(&text(args, "fields")?);
    let said = format!("{name}: {size} bytes, {} fields named.", fields.len());
    session.notes.change_layout(|l| {
        l.structs.insert(
            name,
            Struct {
                size: size as u16,
                fields,
            },
        );
    });
    Ok(said)
}

fn known_struct(session: &Session, name: &str) -> Result<(), String> {
    if session.notes.layout().structs.contains_key(name) {
        return Ok(());
    }
    Err(format!("no struct called {name:?}; define_struct first"))
}

pub fn struct_array(session: &mut Session, args: &Json) -> Result<String, String> {
    let at = addr(args, "address")?;
    let of = text(args, "struct")?;
    let how_many = count(args, "count", 0)? as u16;
    if how_many == 0 {
        session.notes.change_layout(|l| {
            l.arrays.remove(&at);
        });
        return Ok(format!("${at:04X} is no longer an array."));
    }
    known_struct(session, &of)?;
    let said = format!("${at:04X} is {how_many} of {of}.");
    session.notes.change_layout(|l| {
        l.arrays.insert(
            at,
            crate::layout::Array {
                at,
                of,
                count: how_many,
            },
        );
    });
    Ok(said)
}

pub fn bind_struct(session: &mut Session, args: &Json) -> Result<String, String> {
    let register = text(args, "register")?.to_ascii_uppercase();
    if register != "IX" && register != "IY" {
        return Err(format!("{register}: only IX and IY index with a field"));
    }
    let of = text(args, "struct")?;
    let (from, to) = (addr(args, "from")?, addr(args, "to")?);
    if to < from {
        return Err("to should not be below from".into());
    }
    if !of.is_empty() {
        known_struct(session, &of)?;
    }
    let said = if of.is_empty() {
        format!("Nothing bound to {register} over ${from:04X}-${to:04X}.")
    } else {
        format!("{register} is a {of} over ${from:04X}-${to:04X}.")
    };
    session.notes.change_layout(|l| {
        l.bindings
            .retain(|b| !(b.register == register && b.from == from && b.to == to));
        if !of.is_empty() {
            l.bindings.push(Binding {
                register,
                of,
                from,
                to,
            });
        }
    });
    Ok(said)
}
