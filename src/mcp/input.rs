//! Typing at the machine.
//!
//! The keyboard is the same eight rows of five on every Spectrum, and the ROM
//! scans it once a frame: what makes a keypress work is holding it long enough
//! to be scanned twice, not sending an event.

use crate::machine::{Model, FRAME_T};
use crate::mcp::json::Json;
use crate::mcp::tools::{count, flag, text, Session};

/// Hold keys down, run, and let go.
///
/// The ROM scans the keyboard once a frame and wants a key on two scans
/// running before it believes in it, so a key held for one frame types
/// nothing. Ten frames is a fifth of a second, which is a person pressing
/// a key rather than a machine pretending to.
pub fn press_keys(session: &mut Session, args: &Json) -> Result<String, String> {
    let names: Vec<String> = match args.get("keys") {
        Some(Json::Arr(items)) => items
            .iter()
            .map(|i| {
                i.as_str()
                    .map(|s| s.to_string())
                    .ok_or_else(|| format!("{i} is not a key name"))
            })
            .collect::<Result<_, _>>()?,
        Some(Json::Str(one)) => vec![one.clone()],
        _ => return Err("press_keys needs keys: a name, or a list of them".into()),
    };
    let hold = count(args, "frames", 10)?.clamp(1, 600);
    let after = count(args, "then_frames", 10)?.min(600);

    let mut pressed = Vec::new();
    let note = session.let_go_of_recording();
    for name in &names {
        pressed.push(key_named(name, session.spec.bus.model)?);
    }
    for (row, bit) in &pressed {
        session.spec.bus.keys[*row] &= !(1 << bit);
    }
    for _ in 0..hold {
        session.spec.run(FRAME_T);
    }
    for (row, bit) in &pressed {
        session.spec.bus.keys[*row] |= 1 << bit;
    }
    for _ in 0..after {
        session.spec.run(FRAME_T);
    }
    Ok(format!(
        "{note}Held {} for {hold} frames, then ran {after} more. {}",
        names.join(" + "),
        session.registers()
    ))
}

/// Type a line the way somebody at the keyboard would, keywords and all.
///
/// A Spectrum's keywords are not spelled out: `LOAD` is one press of the L
/// key, and `CAT` is extended mode with a shift on the 9. So the text is read
/// the way the machine would have it typed — every legend on every key is
/// known, so `LOAD *"m";1;"prog"` and `FORMAT "m";1;"cart"` type themselves,
/// and anything inside quotes is typed letter by letter, since a filename
/// called "info" is not the keyword IN followed by "fo".
pub fn type_text(session: &mut Session, args: &Json) -> Result<String, String> {
    let line = text(args, "text")?;
    let hold = count(args, "frames", 6)?.clamp(1, 60);
    let note = session.let_go_of_recording();

    for step in plan(&line)? {
        // A keyword only exists in the mode the machine is in. K mode is the
        // start of a line, where a letter key gives a whole word; after that
        // the machine is in L mode and the same key gives the letter.
        let in_k = session.spec.bus.peek_raw(0x5C3B) & 0x08 == 0;
        match (&step.needs, in_k) {
            (Mode::Keyword, false) => {
                return Err(format!(
                    "{:?} is a keyword, and the machine is past the start of the line — in                      L mode a letter key types its letter. Keywords go first: LOAD, SAVE,                      PRINT, LET. registers or read_memory $5C3B bit 3 says which mode it                      is in.",
                    step.what
                ))
            }
            (Mode::Letter, true) => {
                return Err(format!(
                    "{:?} at the start of a line types a keyword rather than a letter: the                      machine is in K mode. A line starts with a keyword — LET a$=... rather                      than a$=...",
                    step.what
                ))
            }
            _ => {}
        }
        for keys in &step.presses {
            for (row, bit) in keys {
                session.spec.bus.keys[*row] &= !(1 << bit);
            }
            for _ in 0..hold {
                session.spec.run(FRAME_T);
            }
            for (row, bit) in keys {
                session.spec.bus.keys[*row] |= 1 << bit;
            }
            for _ in 0..hold {
                session.spec.run(FRAME_T);
            }
        }
    }

    if flag(args, "enter", true) {
        let (row, bit) = key_named("ENTER", session.spec.bus.model)?;
        session.spec.bus.keys[row] &= !(1 << bit);
        for _ in 0..hold {
            session.spec.run(FRAME_T);
        }
        session.spec.bus.keys[row] |= 1 << bit;
        for _ in 0..hold {
            session.spec.run(FRAME_T);
        }
    }
    Ok(format!("{note}Typed {line:?}. {}", session.registers()))
}

/// What mode a piece of text needs the machine to be in.
#[derive(PartialEq, Eq)]
enum Mode {
    /// A keyword: only at the start of a line.
    Keyword,
    /// A letter: only once the line has started.
    Letter,
    /// A digit, a space, a symbol or an extended-mode word, which are the same
    /// either way.
    Either,
}

/// One thing to type: what it is, which mode it needs, and the keys for it.
struct Step {
    what: String,
    needs: Mode,
    presses: Vec<Vec<(usize, u8)>>,
}

const CAPS: (usize, u8) = (0, 0);
const SYM: (usize, u8) = (7, 1);

/// Read a line the way the machine would have it typed.
fn plan(line: &str) -> Result<Vec<Step>, String> {
    let upper = line.to_ascii_uppercase();
    let chars: Vec<char> = upper.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    let mut in_string = false;

    while at < chars.len() {
        if chars[at] == '"' {
            in_string = !in_string;
        }
        // Keywords are only keywords outside a string: a file called "info"
        // is not IN followed by "fo".
        if !in_string {
            if let Some(step) = keyword_at(&chars, at) {
                at += step.what.chars().count();
                out.push(step);
                continue;
            }
        }
        out.push(character(chars[at])?);
        at += 1;
    }
    Ok(out)
}

/// The longest keyword printed on any key that starts here.
fn keyword_at(chars: &[char], at: usize) -> Option<Step> {
    let rest: String = chars[at..].iter().collect();
    let mut best: Option<Step> = None;
    for key in crate::keyboard::SPECTRUM.iter() {
        for (legend, mode, extended, shifted) in [
            (key.word, Mode::Keyword, false, false),
            (key.over, Mode::Either, true, false),
            (key.under, Mode::Either, true, true),
        ] {
            if legend.is_empty() || !rest.starts_with(legend) {
                continue;
            }
            // A keyword ends where a letter stops: PRINTER is not PRINT.
            let after = chars.get(at + legend.chars().count());
            if after.is_some_and(|c| c.is_ascii_alphanumeric())
                && legend
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
            {
                continue;
            }
            if best
                .as_ref()
                .is_some_and(|b| b.what.chars().count() >= legend.chars().count())
            {
                continue;
            }
            let mut presses = Vec::new();
            if extended {
                presses.push(vec![CAPS, SYM]);
            }
            presses.push(if shifted {
                vec![SYM, key.press[0]]
            } else {
                vec![key.press[0]]
            });
            best = Some(Step {
                what: legend.to_string(),
                needs: mode,
                presses,
            });
        }
    }
    best
}

/// One character, on its own key or behind SYMBOL SHIFT.
fn character(c: char) -> Result<Step, String> {
    let name = if c == ' ' { "SPACE" } else { "" };
    for key in crate::keyboard::SPECTRUM.iter() {
        let is_main = (!name.is_empty() && key.main == name)
            || (name.is_empty() && key.main.chars().count() == 1 && key.main.starts_with(c));
        if is_main {
            return Ok(Step {
                what: c.to_string(),
                needs: if c.is_ascii_alphabetic() {
                    Mode::Letter
                } else {
                    Mode::Either
                },
                presses: vec![vec![key.press[0]]],
            });
        }
    }
    for key in crate::keyboard::SPECTRUM.iter() {
        if !key.sym.is_empty() && key.sym.chars().count() == 1 && key.sym.starts_with(c) {
            return Ok(Step {
                what: c.to_string(),
                needs: Mode::Either,
                presses: vec![vec![SYM, key.press[0]]],
            });
        }
    }
    Err(format!(
        "{c:?} is not on the keyboard. The symbols are the ones printed in red on the keys —          press_keys takes SYMBOL SHIFT and a key together for anything this cannot find."
    ))
}

/// Where a key is in the matrix, by the name written on it.
///
/// The keyboard is the same eight rows of five on every Spectrum and on the
/// ZX81; what differs is the words printed on the keys, which is why this asks
/// the layout rather than holding a table of its own.
pub fn key_named(name: &str, _model: Model) -> Result<(usize, u8), String> {
    let wanted = name.trim().to_ascii_uppercase();
    let wanted = match wanted.as_str() {
        // What people call them, against what is printed on them.
        "CAPS" | "SHIFT" | "CAPSSHIFT" | "CAPS_SHIFT" => "CAPS SHIFT".to_string(),
        "SYMBOL" | "SYM" | "SYMBOLSHIFT" | "SYMBOL_SHIFT" => "SYMBOL SHIFT".to_string(),
        "RETURN" | "NEWLINE" | "CR" => "ENTER".to_string(),
        other => other.to_string(),
    };
    crate::keyboard::SPECTRUM
        .iter()
        .find(|key| key.main.eq_ignore_ascii_case(&wanted))
        .map(|key| key.press[0])
        .ok_or_else(|| {
            format!(
                "no key called {name:?}. The forty are 0-9, A-Z, ENTER, SPACE, \
                 CAPS SHIFT and SYMBOL SHIFT."
            )
        })
}

/// Move whichever mouse is fitted and set its buttons, then run a few frames
/// so the program reading it has had a chance to.
pub fn mouse(session: &mut Session, args: &Json) -> Result<String, String> {
    use crate::hardware::Peripheral;
    let hardware = &session.spec.bus.hardware;
    let (kempston, amx) = (
        hardware.fitted(Peripheral::KempstonMouse),
        hardware.fitted(Peripheral::AmxMouse),
    );
    if !kempston && !amx {
        return Err(
            "no mouse is fitted: fit one first (fit with what: kempston_mouse or amx_mouse)".into(),
        );
    }
    let signed = |key: &str| -> Result<i32, String> {
        match args.get(key) {
            None => Ok(0),
            Some(v) => v
                .as_i64()
                .map(|n| n.clamp(-255, 255) as i32)
                .ok_or_else(|| format!("{key} should be a whole number of pixels, not {v}")),
        }
    };
    let (dx, dy) = (signed("dx")?, signed("dy")?);
    let (left, middle, right) = (
        flag(args, "left", false),
        flag(args, "middle", false),
        flag(args, "right", false),
    );
    let frames = count(args, "frames", 5)?.min(600);
    let held = match (left, middle, right) {
        (false, false, false) => "none held".to_string(),
        _ => {
            [(left, "left"), (middle, "middle"), (right, "right")]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, name)| *name)
                .collect::<Vec<_>>()
                .join(" and ")
                + " held"
        }
    };

    if amx {
        if let Some(mouse) = session.spec.bus.amx.as_mut() {
            mouse.queue(dx, dy);
            mouse.set_buttons(left, middle, right);
        }
        for _ in 0..frames {
            session.spec.run(FRAME_T);
        }
        let mouse = session.spec.bus.amx.clone().unwrap_or_default();
        return Ok(format!(
            "AMX mouse: queued ({dx}, {dy}), buttons {held}; ran {frames} frames. Steps are \
             delivered as PIO interrupts once the program has turned them on; ({}, {}) are \
             still waiting. Buttons ${:02X} at $DF — active low, left bit 7, middle 6, right 5.",
            mouse.pending_x, mouse.pending_y, mouse.buttons
        ));
    }

    let mouse = &mut session.spec.bus.mouse;
    mouse.move_by(dx, dy);
    mouse.set_buttons(left, right);
    for _ in 0..frames {
        session.spec.run(FRAME_T);
    }
    let mouse = session.spec.bus.mouse;
    Ok(format!(
        "Kempston mouse: moved by ({dx}, {dy}), buttons {held}; ran {frames} frames. The \
         counters read X {} and Y {} (Y counts up the screen), buttons ${:02X} at $FADF — \
         active low, left is bit 1 and right bit 0.",
        mouse.x, mouse.y, mouse.buttons
    ))
}
