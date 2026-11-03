//! Pictures to keep beside a disassembly: the call graph as an SVG, and every
//! bitmap the notes describe as a PNG of its own.

use std::collections::BTreeMap;
use std::path::Path;

use crate::layout::Kind;
use crate::mcp::json::Json;
use crate::mcp::tools::{count, text, Session};

const NODE_W: f32 = 190.0;
const NODE_H: f32 = 30.0;
const LAYER_GAP: f32 = 80.0;
const ROW_GAP: f32 = 14.0;
const MARGIN: f32 = 20.0;
const TOP: f32 = 60.0;

/// Text as SVG will take it.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The first sentence of a description, for a tooltip.
fn first_sentence(comment: &str) -> &str {
    let comment = comment.lines().next().unwrap_or("");
    match comment.find(". ") {
        Some(end) => &comment[..=end],
        None => comment,
    }
}

pub fn export_call_graph(session: &mut Session, args: &Json) -> Result<String, String> {
    let path = text(args, "path")?;
    let most = count(args, "limit", 60)?.clamp(2, 200) as usize;
    let from = match args.get("from") {
        Some(_) => crate::mcp::tools::addr(args, "from")?,
        None => 0,
    };
    let to = match args.get("to") {
        Some(_) => crate::mcp::tools::addr(args, "to")?,
        None => 0xFFFF,
    };
    let inside = |a: u16| a >= from && a <= to;
    let observer = &session.spec.bus.observer;
    if observer.edges.is_empty() {
        return Err("no calls have been watched: call watch_routines, then run the machine".into());
    }
    let edges: BTreeMap<(u16, u16), u32> = observer
        .edges
        .iter()
        .filter(|((caller, callee), _)| inside(*caller) && inside(*callee))
        .map(|(pair, edge)| (*pair, edge.calls))
        .collect();
    if edges.is_empty() {
        return Err(format!(
            "no calls were watched between ${from:04X} and ${to:04X}"
        ));
    }
    // Work done by instructions run, the routine's own and its callees': what
    // a reader wants to see first is where the time goes.
    let work: BTreeMap<u16, u64> = observer
        .routines
        .iter()
        .filter(|(at, _)| inside(**at))
        .map(|(at, seen)| (*at, seen.instructions))
        .collect();
    let graph = crate::callgraph::layout_keeping(&edges, &work, most);

    let at = |layer: usize, row: usize, top: f32| {
        (
            MARGIN + layer as f32 * (NODE_W + LAYER_GAP),
            top + row as f32 * (NODE_H + ROW_GAP),
        )
    };
    let width = MARGIN * 2.0 + graph.layers as f32 * (NODE_W + LAYER_GAP) - LAYER_GAP;
    let height = TOP + graph.widest as f32 * (NODE_H + ROW_GAP) + MARGIN + 140.0;
    let program = session
        .loaded
        .clone()
        .unwrap_or_else(|| "a machine with nothing loaded".into());
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
         viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"Menlo, Consolas, monospace\">\n\
         <rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n\
         <text x=\"{MARGIN}\" y=\"24\" font-size=\"15\" font-weight=\"bold\">{}</text>\n",
        escape(&program),
    );
    // What the picture is and is not, in lines short enough for a narrow graph.
    let said = format!(
        "Who called whom over {} frames watched: the busiest {} routines by instructions run, \
         left to right. A thicker line is a call made more often; a dashed one goes back to \
         something already reached, which is a loop. Hover over a routine for its description. \
         Only calls are drawn: code reached by a jump, such as handlers jumped to through a \
         table, is not, and a routine that moves the stack pointer can make calls look as if \
         they came from it.",
        observer.frames,
        graph.nodes.len()
    );
    let per_line = ((width - MARGIN * 2.0) / 6.6).max(40.0) as usize;
    let lines = crate::asmexport::wrap(&said, per_line);
    for (i, line) in lines.iter().enumerate() {
        svg.push_str(&format!(
            "<text x=\"{MARGIN}\" y=\"{:.0}\" font-size=\"11\" fill=\"#555\">{}</text>\n",
            42.0 + 14.0 * i as f32,
            escape(line)
        ));
    }
    let top = TOP + 14.0 * lines.len().saturating_sub(1) as f32;
    for link in &graph.links {
        let (Some(from), Some(to)) = (graph.node(link.from), graph.node(link.to)) else {
            continue;
        };
        let (fx, fy) = at(from.layer, from.row, top);
        let (tx, ty) = at(to.layer, to.row, top);
        let stroke = 1.0 + 4.0 * link.weight;
        let (x1, y1) = (fx + NODE_W, fy + NODE_H / 2.0);
        let (x2, y2) = (tx, ty + NODE_H / 2.0);
        let back = to.layer <= from.layer;
        let d = if back {
            // Round underneath, so a loop does not cross the boxes between.
            let below = fy.max(ty) + NODE_H + ROW_GAP;
            format!(
                "M{x1:.1},{y1:.1} C{:.1},{below:.1} {:.1},{below:.1} {x2:.1},{y2:.1}",
                x1 + 40.0,
                x2 - 40.0
            )
        } else {
            let mid = (x1 + x2) / 2.0;
            format!("M{x1:.1},{y1:.1} C{mid:.1},{y1:.1} {mid:.1},{y2:.1} {x2:.1},{y2:.1}")
        };
        svg.push_str(&format!(
            "<path class=\"call\" d=\"{d}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{stroke:.1}\"{} \
             opacity=\"0.7\"><title>{} calls</title></path>\n",
            if back { "#b5452b" } else { "#3a6ea5" },
            if back { " stroke-dasharray=\"5,4\"" } else { "" },
            link.calls
        ));
    }
    for node in &graph.nodes {
        let (x, y) = at(node.layer, node.row, top);
        // The notes' name, or the ROM's from a symbol file, as the listing has it.
        let name = crate::mcp::listing::name_of(session, node.entry)
            .unwrap_or_else(|| format!("${:04X}", node.entry));
        let calls = observer.routines.get(&node.entry).map_or(0, |r| r.calls);
        let shade = 245.0 - 110.0 * node.weight;
        let tip = first_sentence(session.notes.comment(node.entry));
        svg.push_str(&format!(
            "<g class=\"routine\"><title>{}</title>\
             <rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{NODE_W}\" height=\"{NODE_H}\" rx=\"4\" \
             fill=\"rgb({s},{s},255)\" stroke=\"#33335a\"/>\
             <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"11\">{}</text>\
             <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"9\" fill=\"#444\">${:04X} · {calls} calls</text></g>\n",
            escape(&format!("{name}: {tip}")),
            x + 6.0,
            y + 13.0,
            escape(&name),
            x + 6.0,
            y + 25.0,
            node.entry,
            s = shade as u8,
        ));
    }
    svg.push_str("</svg>\n");
    std::fs::write(&path, &svg).map_err(|e| format!("{path}: {e}"))?;
    Ok(format!(
        "Wrote {path}: {} routines and {} calls between them, in {} layers. Routines called \
         from nothing watched sit at the left.",
        graph.nodes.len(),
        graph.links.len(),
        graph.layers
    ))
}

/// A bitmap as RGBA pixels, `scale` screen pixels to each of its own: ink black,
/// paper white, and outside a masked sprite's mask transparent.
pub fn bitmap_pixels(
    peek: &dyn Fn(u16) -> u8,
    at: u16,
    kind: Kind,
    scale: usize,
) -> Option<(Vec<u8>, usize, usize)> {
    let Kind::Bitmap {
        width,
        rows,
        header,
        bottom_up,
        masked,
    } = kind
    else {
        return None;
    };
    let (w, h) = (width as usize * 8, rows as usize);
    let stride = width as u16 * if masked { 2 } else { 1 };
    let mut rgba = vec![0u8; w * h * scale * scale * 4];
    for row in 0..h {
        let start = at.wrapping_add(header as u16 + row as u16 * stride);
        let drawn = if bottom_up { h - 1 - row } else { row };
        for byte in 0..width as u16 {
            let (mask, ink) = if masked {
                let pair = start.wrapping_add(byte * 2);
                (peek(pair), peek(pair.wrapping_add(1)))
            } else {
                (0xFF, peek(start.wrapping_add(byte)))
            };
            for bit in 0..8 {
                let on = ink & (0x80 >> bit) != 0;
                let covered = mask & (0x80 >> bit) != 0;
                let colour = match (on, covered || !masked) {
                    (true, _) => [0, 0, 0, 255],
                    (false, true) => [255, 255, 255, 255],
                    (false, false) => [0, 0, 0, 0],
                };
                let x = byte as usize * 8 + bit;
                for dy in 0..scale {
                    for dx in 0..scale {
                        let p = ((drawn * scale + dy) * w * scale + x * scale + dx) * 4;
                        rgba[p..p + 4].copy_from_slice(&colour);
                    }
                }
            }
        }
    }
    Some((rgba, w * scale, h * scale))
}

pub fn export_sprites(session: &mut Session, args: &Json) -> Result<String, String> {
    let dir = text(args, "dir")?;
    let scale = count(args, "scale", 4)?.clamp(1, 16) as usize;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
    let bus = &session.spec.bus;
    let peek = |a: u16| bus.peek_raw(a);
    let mut written = 0;
    for typed in session.notes.layout().data.values() {
        let Some((rgba, w, h)) = bitmap_pixels(&peek, typed.at, typed.kind, scale) else {
            continue;
        };
        let label = session.notes.label(typed.at);
        let name: String = if label.is_empty() {
            format!("bitmap_{:04X}", typed.at)
        } else {
            label
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        };
        let png = crate::mcp::picture::encode(&rgba, w, h)?;
        let file = Path::new(&dir).join(format!("{name}.png"));
        std::fs::write(&file, png).map_err(|e| format!("{}: {e}", file.display()))?;
        written += 1;
    }
    if written == 0 {
        return Err("no bitmaps have been described: set_data with kind bitmap first".into());
    }
    Ok(format!(
        "Wrote {written} PNGs to {dir}, {scale} pixels to each of the sprite's own: ink black, \
         paper white, and outside a masked sprite's mask transparent. Each is named by its label \
         and drawn from memory as it is now."
    ))
}
