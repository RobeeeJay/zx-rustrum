//! Laying a call graph out so it can be followed.

use std::collections::BTreeMap;
use zx_rustrum::callgraph::{crossings, layout, MAX_NODES};

fn graph(edges: &[((u16, u16), u32)], work: &[(u16, u64)]) -> zx_rustrum::callgraph::Layout {
    layout(
        &edges.iter().copied().collect::<BTreeMap<_, _>>(),
        &work.iter().copied().collect::<BTreeMap<_, _>>(),
    )
}

/// A routine sits one layer to the right of whatever calls it, so the picture
/// reads left to right the way the calls went.
#[test]
fn a_routine_sits_to_the_right_of_what_calls_it() {
    // main → draw → plot, and main → sound.
    let g = graph(
        &[
            ((0x8000, 0x8100), 1),
            ((0x8100, 0x8200), 8),
            ((0x8000, 0x8300), 1),
        ],
        &[(0x8000, 100), (0x8100, 60), (0x8200, 40), (0x8300, 10)],
    );
    let layer = |at: u16| g.node(at).expect("in the drawing").layer;
    assert_eq!(layer(0x8000), 0, "nothing calls main, so it is at the left");
    assert_eq!(layer(0x8100), 1, "what main calls is one along");
    assert_eq!(layer(0x8200), 2, "and what that calls is two");
    assert_eq!(
        layer(0x8300),
        1,
        "sound is called by main, so it is one too"
    );
    assert_eq!(g.layers, 3);
    assert_eq!(g.widest, 2, "two routines share the middle layer");
}

/// A cycle has no "one further along", so the edge that would push a routine
/// past a layer it already has is left out of the reckoning and drawn as a
/// line going back the way it came.
#[test]
fn a_loop_in_the_program_does_not_run_the_layers_away() {
    // a → b → c → a, which is a program calling round in a circle.
    let g = graph(
        &[
            ((0x8000, 0x8100), 4),
            ((0x8100, 0x8200), 4),
            ((0x8200, 0x8000), 4),
        ],
        &[(0x8000, 30), (0x8100, 20), (0x8200, 10)],
    );
    assert_eq!(g.layers, 3, "three layers, not thirty");
    assert_eq!(g.links.len(), 3, "and every call is still drawn");
    let back = g
        .links
        .iter()
        .find(|l| (l.from, l.to) == (0x8200, 0x8000))
        .expect("the call that closes the circle");
    let (from, to) = (
        g.node(back.from).unwrap().layer,
        g.node(back.to).unwrap().layer,
    );
    assert!(
        to <= from,
        "it goes back the way it came: layer {from} to {to}"
    );
}

/// The test above cannot fail by running away: with three routines there are
/// only three layers to run to. Trans Am's graph has two routines that seem to
/// call each other, the player's car and the scenery drawer, and among sixty
/// routines that drew every one of them in a layer of its own.
#[test]
fn a_loop_among_many_routines_does_not_run_the_layers_away() {
    // main calls a, a and b call each other, and main calls twenty others.
    let mut edges = vec![
        ((0x8000, 0x8100), 4),
        ((0x8100, 0x8200), 4),
        ((0x8200, 0x8100), 4),
    ];
    let mut work = vec![(0x8000, 300), (0x8100, 200), (0x8200, 100)];
    for i in 0..20u16 {
        edges.push(((0x8000, 0x9000 + i * 4), 1));
        work.push((0x9000 + i * 4, 10));
    }
    let g = graph(&edges, &work);
    let layer = |at: u16| g.node(at).expect("in the drawing").layer;
    assert_eq!(
        (layer(0x8000), layer(0x8100), layer(0x8200)),
        (0, 1, 2),
        "main, then a, then b"
    );
    assert_eq!(
        g.layers, 3,
        "three layers for 23 routines, not {}",
        g.layers
    );
}

/// Ordering the routines within a layer is what keeps the lines followable.
/// The barycentre sweep does not promise the fewest crossings, but it has to
/// beat the order the addresses happen to be in.
#[test]
fn the_ordering_untangles_the_lines() {
    // Two callers whose callees are the other way round by address: laid out
    // in address order this crosses, and put in barycentre order it does not.
    let g = graph(
        &[
            ((0x8000, 0x8300), 1),
            ((0x8000, 0x8400), 1),
            ((0x8100, 0x8300), 1),
            ((0x8100, 0x8400), 1),
            ((0x9000, 0x8400), 9),
        ],
        &[
            (0x8000, 50),
            (0x8100, 50),
            (0x8300, 20),
            (0x8400, 20),
            (0x9000, 50),
        ],
    );
    assert!(
        crossings(&g) <= 2,
        "the lines should be mostly untangled: {} crossings",
        crossings(&g)
    );
    // Every routine has a place, and no two in a layer share a row.
    for layer in 0..g.layers {
        let mut rows: Vec<usize> = g
            .nodes
            .iter()
            .filter(|n| n.layer == layer)
            .map(|n| n.row)
            .collect();
        rows.sort_unstable();
        let before = rows.len();
        rows.dedup();
        assert_eq!(
            rows.len(),
            before,
            "two routines on one row in layer {layer}"
        );
    }
}

/// A game's whole call graph is hundreds of routines. What is drawn is the
/// busy part of it, and the rest is left out rather than making a picture
/// nobody can read.
#[test]
fn only_the_busy_part_is_drawn() {
    let edges: Vec<((u16, u16), u32)> = (0..80u16)
        .map(|i| ((0x8000, 0x8000 + (i + 1) * 4), 1))
        .collect();
    let work: Vec<(u16, u64)> = (0..80u16)
        .map(|i| (0x8000 + (i + 1) * 4, i as u64))
        .collect();
    let g = graph(&edges, &work);
    assert_eq!(g.nodes.len(), MAX_NODES, "the busiest forty and no more");
    assert!(
        g.node(0x8000 + 80 * 4).is_some(),
        "the busiest routine is kept"
    );
    assert!(g.node(0x8000 + 4).is_none(), "and the idlest is not");
    for link in &g.links {
        assert!(
            g.node(link.from).is_some() && g.node(link.to).is_some(),
            "no line to a routine that was left out"
        );
    }
}

/// The same calls give the same picture: a drawing that reshuffles itself
/// every time it is looked at cannot be read.
#[test]
fn the_same_calls_give_the_same_picture() {
    let edges = [
        ((0x8000, 0x8100), 3),
        ((0x8000, 0x8200), 1),
        ((0x8100, 0x8200), 7),
    ];
    let work = [(0x8000, 90), (0x8100, 45), (0x8200, 45)];
    assert_eq!(graph(&edges, &work), graph(&edges, &work));
}

/// How heavily a routine is drawn is how much of the machine's work goes
/// through it, and how thick a line is is how often that call is made.
#[test]
fn the_weights_are_what_was_measured() {
    let g = graph(
        &[((0x8000, 0x8100), 100), ((0x8000, 0x8200), 1)],
        &[(0x8000, 1000), (0x8100, 500), (0x8200, 10)],
    );
    assert_eq!(g.node(0x8000).unwrap().weight, 1.0, "the busiest is full");
    assert!(
        (g.node(0x8100).unwrap().weight - 0.5).abs() < 0.001,
        "half the work is half the bar"
    );
    let heavy = g
        .links
        .iter()
        .find(|l| l.to == 0x8100)
        .expect("the call made a hundred times");
    let light = g.links.iter().find(|l| l.to == 0x8200).unwrap();
    assert_eq!(heavy.weight, 1.0);
    assert!(light.weight < 0.05, "and one call in a hundred is thin");
}
