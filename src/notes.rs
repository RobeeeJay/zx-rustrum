//! Labels and comments the user writes against addresses while disassembling.
//!
//! They are kept in a plain text file beside whatever is being disassembled —
//! the tape if one is in the deck, otherwise the ROM — so a listing built up
//! over an evening is still there next time, and can be read, edited or diffed
//! without the emulator. One line per address:
//!
//! ```text
//! # ZX-Rustrum notes
//! 8000 start      ; wait for the frame to finish
//! 8003            ; the border is set here
//! 800A @clear_screen_800A ; @Fills the display file with one value
//! ```
//!
//! An `@` marks something AutoDoc worked out rather than something the user
//! wrote. The distinction is what lets a later, better guess replace an
//! earlier one while never touching a line somebody typed themselves.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::blocks::Block;
use crate::layout::Layout;

/// What is written against one address. Either half may be empty; an entry
/// with both empty is dropped rather than written out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Note {
    pub label: String,
    pub comment: String,
    /// Whether each half is AutoDoc's guess rather than the user's own words.
    /// Tracked separately: somebody may name a routine themselves and leave
    /// the guessed comment under it, or the other way about.
    pub label_auto: bool,
    pub comment_auto: bool,
}

impl Note {
    fn is_empty(&self) -> bool {
        self.label.is_empty() && self.comment.is_empty()
    }
}

/// The notes for one file, and where they are saved.
#[derive(Clone, Debug, Default)]
pub struct Notes {
    entries: BTreeMap<u16, Note>,
    /// Where the routines and the data are, when anything has worked it out.
    /// Kept here rather than in a file of their own: it is one more thing
    /// known about the same program, and two files would come apart.
    blocks: Vec<Block>,
    /// What the data is, where somebody has said: tables, text, bitmaps and
    /// records. For the same reason, it is in this file too.
    layout: Layout,
    /// Where these are written. None when nothing is loaded to write beside,
    /// in which case the notes are still usable, just not kept.
    path: Option<PathBuf>,
    /// Whether there is anything to write that has not been written.
    dirty: bool,
}

impl Notes {
    /// The extension the sidecar file carries.
    pub const EXTENSION: &'static str = "zxrs.txt";

    /// Where the notes for a file live: beside it, under its own name.
    /// `games/manic.tap` keeps its notes in `games/manic.zxrs.txt`.
    pub fn sidecar(source: &Path) -> PathBuf {
        source.with_extension(Self::EXTENSION)
    }

    /// Read the notes for a file. A file that is not there yet is not an
    /// error: it is simply a listing nobody has annotated.
    pub fn for_file(source: &Path) -> Notes {
        let path = Self::sidecar(source);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        Notes {
            entries: parse(&text),
            blocks: parse_blocks(&text),
            layout: parse_layout(&text),
            path: Some(path),
            dirty: false,
        }
    }

    /// Notes with nowhere to go: used until something is loaded.
    pub fn unattached() -> Notes {
        Notes::default()
    }

    pub fn file(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn label(&self, addr: u16) -> &str {
        self.entries.get(&addr).map_or("", |n| n.label.as_str())
    }

    pub fn comment(&self, addr: u16) -> &str {
        self.entries.get(&addr).map_or("", |n| n.comment.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether each half of a note is a guess.
    pub fn label_is_auto(&self, addr: u16) -> bool {
        self.entries.get(&addr).is_some_and(|n| n.label_auto)
    }

    pub fn comment_is_auto(&self, addr: u16) -> bool {
        self.entries.get(&addr).is_some_and(|n| n.comment_auto)
    }

    /// Every address with a label, for the list of places to go.
    pub fn labelled(&self) -> impl Iterator<Item = (u16, &str, bool)> {
        self.entries
            .iter()
            .filter(|(_, note)| !note.label.is_empty())
            .map(|(addr, note)| (*addr, note.label.as_str(), note.label_auto))
    }

    /// What the user typed. Typing over a guess makes it theirs.
    pub fn set_label(&mut self, addr: u16, label: &str) {
        self.edit(addr, |note| {
            note.label = label.trim().to_string();
            note.label_auto = false;
        });
    }

    pub fn set_comment(&mut self, addr: u16, comment: &str) {
        self.edit(addr, |note| {
            note.comment = comment.trim().to_string();
            note.comment_auto = false;
        });
    }

    /// A comment as it is being typed. Only the start is trimmed: a space typed
    /// at the end is the gap before the next word, and trimming it on every
    /// keystroke — as `set_comment` does — took it away before the next word
    /// arrived, so a comment could not be typed with spaces in it at all. It is
    /// tidied when the field is left and whenever the file is written.
    pub fn type_comment(&mut self, addr: u16, comment: &str) {
        self.edit(addr, |note| {
            note.comment = comment.trim_start().to_string();
            note.comment_auto = false;
        });
    }

    /// What AutoDoc worked out. A guess replaces an earlier guess — a later
    /// run may have better code to look at — but never a line the user wrote,
    /// and never puts an empty guess over an existing one.
    pub fn suggest(&mut self, addr: u16, label: &str, comment: &str) {
        let (label, comment) = (label.trim(), comment.trim());
        let user_label = self
            .entries
            .get(&addr)
            .is_some_and(|n| !n.label_auto && !n.label.is_empty());
        let user_comment = self
            .entries
            .get(&addr)
            .is_some_and(|n| !n.comment_auto && !n.comment.is_empty());

        self.edit(addr, |note| {
            if !label.is_empty() && !user_label {
                note.label = label.to_string();
                note.label_auto = true;
            }
            if !comment.is_empty() && !user_comment {
                note.comment = comment.to_string();
                note.comment_auto = true;
            }
        });
    }

    /// Where the routines and the data are.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Replace what is known about the shape of the program. Nothing is
    /// merged here: the caller decides whether this run adds to what was known
    /// or replaces it, since only the caller knows which it meant.
    pub fn set_blocks(&mut self, blocks: Vec<Block>) {
        if blocks == self.blocks {
            return;
        }
        self.blocks = blocks;
        self.dirty = true;
    }

    /// What the data is.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Say something about what the data is.
    pub fn change_layout(&mut self, change: impl FnOnce(&mut Layout)) {
        let before = self.layout.clone();
        change(&mut self.layout);
        if self.layout != before {
            self.dirty = true;
        }
    }

    /// How many addresses have anything written against them.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Throw the lot away. The file goes with them when the notes are next
    /// written, which is what makes this worth a confirmation.
    pub fn clear(&mut self) {
        if self.entries.is_empty() && self.blocks.is_empty() && self.layout.is_empty() {
            return;
        }
        self.entries.clear();
        self.blocks.clear();
        self.layout = Layout::default();
        self.dirty = true;
    }

    fn edit(&mut self, addr: u16, change: impl FnOnce(&mut Note)) {
        let note = self.entries.entry(addr).or_default();
        let before = note.clone();
        change(note);
        if *note == before {
            return;
        }
        if note.is_empty() {
            self.entries.remove(&addr);
        }
        self.dirty = true;
    }

    /// Whether there are changes waiting to be written.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Write the notes out, if they have changed and there is somewhere to
    /// write them. Returns what went wrong, if anything: a listing is not
    /// worth stopping the emulator over, but the user should be told.
    pub fn save_if_dirty(&mut self) -> Result<bool, String> {
        if !self.dirty {
            return Ok(false);
        }
        let Some(path) = self.path.clone() else {
            return Ok(false);
        };
        // An empty set of notes leaves no litter behind: the file is removed
        // rather than left as a header with nothing under it.
        let result = if self.entries.is_empty() && self.blocks.is_empty() && self.layout.is_empty()
        {
            match std::fs::remove_file(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        } else {
            std::fs::write(&path, self.to_text())
        };
        match result {
            Ok(()) => {
                self.dirty = false;
                Ok(true)
            }
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// The file's contents: a header saying what it is, then the notes in
    /// address order so the file reads like the listing it describes.
    pub fn to_text(&self) -> String {
        let mut out = String::from(
            "# ZX-Rustrum notes. One line per address:\n\
             #   ADDR [label] [; comment]\n\
             # a comment's line breaks are written \\n, and a backslash \\\\\n\
             # and where the routines and the data are:\n\
             #   block CODE|DATA FROM-TO\n\
             # and what the data is:\n\
             #   data ADDR words COUNT | text LENGTH | bitmap WIDTH ROWS HEADER\n\
             #   struct NAME SIZE OFFSET=FIELD ...\n\
             #   array ADDR STRUCT COUNT\n\
             #   bind IX|IY STRUCT FROM-TO\n",
        );
        for line in self.layout.to_lines() {
            out.push_str(&line);
            out.push('\n');
        }
        for block in &self.blocks {
            out.push_str(&crate::blocks::to_line(block));
            out.push('\n');
        }
        for (addr, note) in &self.entries {
            out.push_str(&format!("{addr:04X}"));
            if !note.label.is_empty() {
                let mark = if note.label_auto { "@" } else { "" };
                out.push_str(&format!(" {mark}{}", note.label));
            }
            // Trimmed on the way out, since what is being typed is kept as it
            // was typed until the field is left.
            if !note.comment.trim().is_empty() {
                let mark = if note.comment_auto { "@" } else { "" };
                out.push_str(&format!(" ; {mark}{}", escape(note.comment.trim())));
            }
            out.push('\n');
        }
        out
    }
}

/// A comment as the file keeps it: on one line, its line breaks written as
/// `\n` and a backslash as `\\`, so it reads back whole. Written out as it
/// stood, the second line of a comment became a line of its own, which the
/// reader skipped — or, if it began with something like `BEEF`, read as a
/// note for an address nobody wrote one against.
fn escape(comment: &str) -> String {
    comment
        .replace('\\', "\\\\")
        .replace('\r', "")
        .replace('\n', "\\n")
}

/// And back: `\n` a line break, `\\` a backslash, and any other backslash
/// left as it is, since the file may have been written by hand.
fn unescape(comment: &str) -> String {
    let mut out = String::with_capacity(comment.len());
    let mut chars = comment.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some('n')) => {
                out.push('\n');
                chars.next();
            }
            ('\\', Some('\\')) => {
                out.push('\\');
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// What the data is, from a notes file.
pub fn parse_layout(text: &str) -> Layout {
    let mut layout = Layout::default();
    for line in text.lines() {
        layout.read_line(line.trim());
    }
    layout
}

/// The block lines from a notes file, in address order.
pub fn parse_blocks(text: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = text.lines().filter_map(crate::blocks::from_line).collect();
    blocks.sort();
    blocks
}

/// Read the file back. Anything that cannot be understood is skipped rather
/// than throwing the rest away: these files are meant to be edited by hand.
pub fn parse(text: &str) -> BTreeMap<u16, Note> {
    let mut entries = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (addr, rest) = match line.split_once(char::is_whitespace) {
            Some((addr, rest)) => (addr, rest.trim()),
            None => (line, ""),
        };
        let Ok(addr) = u16::from_str_radix(addr.trim_start_matches('$'), 16) else {
            continue;
        };
        let (label, comment) = match rest.split_once(';') {
            Some((label, comment)) => (label.trim(), comment.trim()),
            None => (rest, ""),
        };
        let note = Note {
            label: label.trim_start_matches('@').to_string(),
            comment: unescape(comment.trim_start_matches('@')),
            label_auto: label.starts_with('@'),
            comment_auto: comment.starts_with('@'),
        };
        if !note.is_empty() {
            entries.insert(addr, note);
        }
    }
    entries
}
