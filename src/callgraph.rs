//! Arranging a call graph so it can be looked at.
//!
//! The observer knows who called whom and how often. That is a graph, and a
//! graph drawn as it comes — nodes wherever they happen to fall — is a ball of
//! wool. This puts it in layers: a routine sits one layer to the right of
//! whatever called it, so the picture reads left to right the way the calls
//! went, and within a layer the routines are ordered to keep the lines between
//! them as untangled as the arithmetic can manage.
//!
//! It is the standard way of drawing this — layer, then order, then place —
//! done in as little code as the job needs and no crate.

use std::collections::{BTreeMap, BTreeSet};

/// A routine in the drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub entry: u16,
    /// How deep the calls go to reach it: 0 for something nothing calls.
    pub layer: usize,
    /// Where it sits within its layer, counted from the top.
    pub row: usize,
    /// How much work it does, as a share of the busiest routine's, 0 to 1.
    pub weight: f32,
}

/// One call, and how often it was made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub from: u16,
    pub to: u16,
    pub calls: u32,
    /// How often, as a share of the busiest edge's, 0 to 1.
    pub weight: f32,
}

/// The graph, laid out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    /// How many routines sit in each layer, so a drawing can size itself.
    pub widest: usize,
    pub layers: usize,
}

impl Layout {
    pub fn node(&self, entry: u16) -> Option<&Node> {
        self.nodes.iter().find(|n| n.entry == entry)
    }
}

/// How far a graph is followed. A game's whole call graph is hundreds of
/// routines; what is worth drawing is the busy part of it.
pub const MAX_NODES: usize = 40;

/// Lay out the calls between routines.
///
/// `edges` is who called whom and how often; `work` is what each routine cost,
/// which decides both which routines are worth drawing and how heavily they
/// are drawn. Only the busiest [`MAX_NODES`] routines are kept: past that a
/// picture is not a picture.
pub fn layout(edges: &BTreeMap<(u16, u16), u32>, work: &BTreeMap<u16, u64>) -> Layout {
    layout_keeping(edges, work, MAX_NODES)
}

/// As `layout`, keeping the busiest `most` routines rather than [`MAX_NODES`]: a
/// drawing to keep can afford more than a window can.
pub fn layout_keeping(
    edges: &BTreeMap<(u16, u16), u32>,
    work: &BTreeMap<u16, u64>,
    most: usize,
) -> Layout {
    // The routines worth drawing, busiest first, and then back into address
    // order so the picture does not reshuffle itself every time it is drawn.
    let mut kept: Vec<u16> = {
        let mut all: Vec<(u16, u64)> = work.iter().map(|(at, w)| (*at, *w)).collect();
        for (from, to) in edges.keys() {
            for at in [from, to] {
                if !all.iter().any(|(a, _)| a == at) {
                    all.push((*at, 0));
                }
            }
        }
        all.sort_by_key(|(at, w)| (std::cmp::Reverse(*w), *at));
        all.truncate(most);
        all.into_iter().map(|(at, _)| at).collect()
    };
    kept.sort_unstable();
    let keep: BTreeSet<u16> = kept.iter().copied().collect();

    let links: Vec<(u16, u16, u32)> = edges
        .iter()
        .filter(|((from, to), _)| keep.contains(from) && keep.contains(to) && from != to)
        .map(|((from, to), calls)| (*from, *to, *calls))
        .collect();

    let layer = layers(&kept, &links);
    let order = order_within(&kept, &links, &layer);

    let most_work = work.values().copied().max().unwrap_or(1).max(1);
    let most_calls = links.iter().map(|(_, _, c)| *c).max().unwrap_or(1).max(1);
    let mut nodes: Vec<Node> = kept
        .iter()
        .map(|at| Node {
            entry: *at,
            layer: layer[at],
            row: order[at],
            weight: (*work.get(at).unwrap_or(&0) as f32 / most_work as f32).clamp(0.0, 1.0),
        })
        .collect();
    nodes.sort_by_key(|n| (n.layer, n.row, n.entry));

    Layout {
        widest: (0..=nodes.iter().map(|n| n.layer).max().unwrap_or(0))
            .map(|l| nodes.iter().filter(|n| n.layer == l).count())
            .max()
            .unwrap_or(0),
        layers: nodes.iter().map(|n| n.layer + 1).max().unwrap_or(0),
        nodes,
        links: links
            .iter()
            .map(|(from, to, calls)| Link {
                from: *from,
                to: *to,
                calls: *calls,
                weight: (*calls as f32 / most_calls as f32).clamp(0.0, 1.0),
            })
            .collect(),
    }
}

/// Which layer each routine belongs in: one further along than whatever calls
/// it, and at the left if nothing does.
///
/// A call graph has cycles in it — a routine calling something that calls it
/// back, or itself through two others — and a cycle has no such thing as "one
/// further along". Those edges are the ones that would push a routine past a
/// layer it has already been given, and they are left out of the reckoning:
/// the drawing shows them as lines going back the way they came, which is what
/// a recursive call is.
fn layers(kept: &[u16], links: &[(u16, u16, u32)]) -> BTreeMap<u16, usize> {
    let back = back_edges(kept, links);
    let mut layer: BTreeMap<u16, usize> = kept.iter().map(|at| (*at, 0)).collect();
    // Longest path, relaxed until nothing moves. With the back edges out
    // what is left has no cycle, so a routine's layer settles within as many
    // passes as there are routines. Relaxing the cycles as well pushed every
    // routine on one to the last layer there was.
    for _ in 0..kept.len() {
        let mut moved = false;
        for (from, to, _) in links {
            if back.contains(&(*from, *to)) {
                continue;
            }
            let want = layer[from] + 1;
            if want > layer[to] && want < kept.len() {
                layer.insert(*to, want);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    layer
}

/// The calls that close a cycle: following calls from the routines nothing
/// calls, in address order, an edge to a routine still being followed is one
/// that goes back. Starting from those routines is what makes the main loop's
/// call the forward one and the call back the one left out.
fn back_edges(kept: &[u16], links: &[(u16, u16, u32)]) -> BTreeSet<(u16, u16)> {
    let mut callees: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
    for (from, to, _) in links {
        callees.entry(*from).or_default().push(*to);
    }
    let called: BTreeSet<u16> = links.iter().map(|(_, to, _)| *to).collect();
    let starts = kept
        .iter()
        .filter(|at| !called.contains(at))
        .chain(kept.iter().filter(|at| called.contains(at)));

    let mut back = BTreeSet::new();
    let mut done: BTreeSet<u16> = BTreeSet::new();
    let mut on_path: BTreeSet<u16> = BTreeSet::new();
    for start in starts {
        if done.contains(start) {
            continue;
        }
        // Depth first without recursion: each entry is a routine and how many
        // of its callees have been looked at.
        let mut stack: Vec<(u16, usize)> = vec![(*start, 0)];
        on_path.insert(*start);
        while let Some(&mut (at, ref mut next)) = stack.last_mut() {
            let to = callees.get(&at).and_then(|c| c.get(*next)).copied();
            *next += 1;
            match to {
                Some(to) if on_path.contains(&to) => {
                    back.insert((at, to));
                }
                Some(to) if !done.contains(&to) => {
                    on_path.insert(to);
                    stack.push((to, 0));
                }
                Some(_) => {}
                None => {
                    on_path.remove(&at);
                    done.insert(at);
                    stack.pop();
                }
            }
        }
    }
    back
}

/// Where each routine sits within its layer.
///
/// The barycentre heuristic: a routine is put where the average of the things
/// it is joined to sits, sweeping right and then left a few times. It does not
/// promise the fewest crossings — nothing cheap does — but it turns a ball of
/// wool into something that can be followed.
fn order_within(
    kept: &[u16],
    links: &[(u16, u16, u32)],
    layer: &BTreeMap<u16, usize>,
) -> BTreeMap<u16, usize> {
    let layers = layer.values().copied().max().unwrap_or(0) + 1;
    let mut rows: Vec<Vec<u16>> = vec![Vec::new(); layers];
    for at in kept {
        rows[layer[at]].push(*at);
    }

    let mut place: BTreeMap<u16, usize> = BTreeMap::new();
    for row in &rows {
        for (i, at) in row.iter().enumerate() {
            place.insert(*at, i);
        }
    }

    for sweep in 0..6 {
        let forwards = sweep % 2 == 0;
        let order: Vec<usize> = if forwards {
            (1..layers).collect()
        } else {
            (0..layers.saturating_sub(1)).rev().collect()
        };
        for l in order {
            let mut with_bary: Vec<(f32, u16)> = rows[l]
                .iter()
                .map(|at| {
                    let neighbours: Vec<usize> = links
                        .iter()
                        .filter_map(|(from, to, _)| {
                            if forwards && to == at && layer[from] < l {
                                Some(place[from])
                            } else if !forwards && from == at && layer[to] > l {
                                Some(place[to])
                            } else {
                                None
                            }
                        })
                        .collect();
                    let bary = if neighbours.is_empty() {
                        place[at] as f32
                    } else {
                        neighbours.iter().sum::<usize>() as f32 / neighbours.len() as f32
                    };
                    (bary, *at)
                })
                .collect();
            with_bary.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
            rows[l] = with_bary.iter().map(|(_, at)| *at).collect();
            for (i, at) in rows[l].iter().enumerate() {
                place.insert(*at, i);
            }
        }
    }
    place
}

/// How many pairs of links cross, which is what the ordering is trying to
/// reduce. Only used to check that it does.
pub fn crossings(layout: &Layout) -> usize {
    let row_of = |at: u16| layout.node(at).map(|n| (n.layer, n.row));
    let mut crossed = 0;
    for (i, a) in layout.links.iter().enumerate() {
        for b in layout.links.iter().skip(i + 1) {
            let (Some(a1), Some(a2), Some(b1), Some(b2)) =
                (row_of(a.from), row_of(a.to), row_of(b.from), row_of(b.to))
            else {
                continue;
            };
            // Only links spanning the same pair of layers can cross.
            if a1.0 != b1.0 || a2.0 != b2.0 || a1.0 >= a2.0 {
                continue;
            }
            if (a1.1 < b1.1 && a2.1 > b2.1) || (a1.1 > b1.1 && a2.1 < b2.1) {
                crossed += 1;
            }
        }
    }
    crossed
}
