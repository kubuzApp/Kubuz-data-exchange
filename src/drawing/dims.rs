//! Where the dimensions of a flat pattern go, and how any linear
//! dimension is drawn.
//!
//! The flat pattern is dimensioned the way a press-brake shop reads it:
//! chain dimensions (ISO 129-1) in rows along each axis. Below and left of
//! the blank run the bend chains, from one edge over every bend line to
//! the other edge (the back-gauge settings), then the chain of the
//! outline's corners, then the overall size. Above and right run the
//! holes, from the edge over every hole centre to the far edge. Bend lines
//! that are oblique get their own chain along their own direction. Each
//! value is written once, above its dimension line; where it does not fit
//! between the extension lines it is raised, or set beyond the end of the
//! chain. Internal points of a chain carry a dot, the ends an arrowhead
//! (3 × 0,75 mm). Holes and radii get one callout per size (`4× ⌀9`,
//! `2× R5`) with a leader to one member; a bend line carries its note
//! (`NEER 90° R1,5`) and a numbered tag at its far end.
//!
//! Everything here is in paper millimetres relative to the model origin's
//! place on the sheet; the scale `s` maps model to paper.

use super::features::{arc_mid, Features};
use super::text::{self, Lang};
use crate::dxf::blank::{BendLine, Seg, P2};
use crate::pdf::{font, Content};
use crate::view::BendDirection;

/// Lettering height of dimension values.
pub const H: f64 = 3.0;
/// Distance of the first dimension line from the part.
pub const D: f64 = 10.0;
/// Distance between parallel dimension lines.
pub const SPACING: f64 = 6.0;
/// Extension lines run this far past the dimension line.
pub const EXT: f64 = 1.0;
/// And start this far from the feature.
pub const GAP: f64 = 0.5;
/// Arrowhead length and full height.
pub const ARROW_L: f64 = 3.0;
pub const ARROW_H: f64 = 0.75;
/// Radius of the dot at an internal point of a chain.
const DOT_R: f64 = 0.5;
/// Text sits this far above its dimension line.
const TEXT_UP: f64 = 1.5;
/// A raised value sits this much higher than the one below.
const RAISE: f64 = H + 1.0;
const CALLOUT_PITCH: f64 = 7.0;
const CALLOUT_GAP: f64 = 12.0;
const TAG_R: f64 = 2.5;
const TAG_PITCH: f64 = 5.6;
const OVERSHOOT: f64 = 2.0;
/// How far a local callout's leader runs out from the feature's edge.
const LEADER: f64 = 5.0;
pub const WIDE: f64 = 0.5;
pub const NARROW: f64 = 0.25;
const SAME: f64 = super::features::SAME;

/// One point of a chain: its position along the chain (model mm), where
/// its extension line starts (model), and what put it there.
#[derive(Clone, Debug)]
pub struct Station {
    pub t: f64,
    pub from: P2,
    pub sources: Vec<String>,
    /// The extension line starts right at `from` (a bend line's end).
    pub no_gap: bool,
}

/// One value of a chain, placed.
#[derive(Clone, Debug)]
pub struct ChainText {
    pub value: f64,
    pub text: String,
    /// Paper baseline start and rotation.
    pub at: P2,
    pub angle: f64,
    pub bx: [f64; 4],
    /// A reference line from the value to its gap, when it sits above
    /// the neighbouring gap.
    pub leader: Option<[P2; 2]>,
}

/// A row of chain dimensions along direction `n`, whose extension lines
/// run along `e`; the dimension line sits on the `side` of the blank.
#[derive(Clone, Debug)]
pub struct Chain {
    /// `bend`, `outline`, `hole` or `overall`.
    pub kind: &'static str,
    pub n: P2,
    pub e: P2,
    pub side: f64,
    /// Paper distance of the dimension line along `side · e`.
    pub line: f64,
    pub stations: Vec<Station>,
    pub texts: Vec<ChainText>,
    /// Paper extent of the dimension line along `n`, with what was added
    /// for arrows or values outside the chain.
    pub span: [f64; 2],
    /// How far out from the line (along `side · e`) its values reach.
    pub reach: f64,
}

#[derive(Clone, Debug)]
pub struct Callout {
    /// Paper point the leader starts from, at the text's edge.
    pub from: P2,
    pub text: String,
    pub kind: &'static str,
    pub value: f64,
    pub count: usize,
    /// Model point the leader's arrow lands on, and the centre it points at.
    pub anchor: P2,
    pub centre: P2,
    /// Paper start of the text (baseline).
    pub at: P2,
}

#[derive(Clone, Debug)]
pub struct Tag {
    pub nr: usize,
    /// Paper ends of the drawn (extended) bend line.
    pub line: [P2; 2],
    /// Paper centre of the circle.
    pub at: P2,
    /// Unit direction out along the line, in which the tag moves when it
    /// sits on something.
    pub u: P2,
    /// The note along the line, when it fits: text, paper start, angle.
    pub note: Option<(String, P2, f64)>,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub s: f64,
    pub chains: Vec<Chain>,
    pub callouts: Vec<Callout>,
    pub tags: Vec<Tag>,
    /// Paper mm beyond the scaled blank: left, bottom, right, top.
    pub margin: [f64; 4],
    /// Every text box placed (paper, relative to the model origin).
    pub boxes: Vec<[f64; 4]>,
    /// Every line drawn (paper, relative to the model origin).
    pub lines: Vec<[P2; 2]>,
}

/// Callouts of one kind and size: (kind, value, (anchor, centre) members).
type Group = (&'static str, f64, Vec<(P2, P2)>);

pub fn dist(a: P2, b: P2) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn dot(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn perp(v: P2) -> P2 {
    [-v[1], v[0]]
}

fn add(a: P2, b: P2, k: f64) -> P2 {
    [a[0] + b[0] * k, a[1] + b[1] * k]
}

fn inside(b: [f64; 4], p: P2) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}

pub fn overlap(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Points along a model segment, in paper mm, about one per millimetre.
pub(crate) fn sample(seg: &Seg, s: f64, out: &mut Vec<P2>) {
    match *seg {
        Seg::Line { a, b } => sample_line([a[0] * s, a[1] * s], [b[0] * s, b[1] * s], out),
        Seg::Arc { c, r, start, end } => {
            let sweep = (end - start).rem_euclid(360.0);
            let sweep = if sweep < 1e-9 { 360.0 } else { sweep };
            let n = ((r * s * sweep.to_radians()).ceil() as usize).max(2);
            for i in 0..=n {
                let a = (start + sweep * i as f64 / n as f64).to_radians();
                out.push([(c[0] + r * a.cos()) * s, (c[1] + r * a.sin()) * s]);
            }
        }
        Seg::Circle { c, r } => {
            let n = ((2.0 * std::f64::consts::PI * r * s).ceil() as usize).max(8);
            for i in 0..n {
                let a = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
                out.push([(c[0] + r * a.cos()) * s, (c[1] + r * a.sin()) * s]);
            }
        }
    }
}

pub(crate) fn sample_line(a: P2, b: P2, out: &mut Vec<P2>) {
    let n = (dist(a, b).ceil() as usize).max(1);
    for i in 0..=n {
        let t = i as f64 / n as f64;
        out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
    }
}

/// The reading direction of text along `n`: `n` itself when it points
/// right or straight up, else its opposite (ISO 129-1: values read from
/// below or from the right).
pub fn reading(n: P2) -> P2 {
    if n[0] < -1e-9 || (n[0].abs() <= 1e-9 && n[1] < 0.0) {
        [-n[0], -n[1]]
    } else {
        n
    }
}

/// The box of text `w` wide and `H` tall starting at `at` along `u`
/// (axis-aligned hull).
fn text_box(at: P2, u: P2, w: f64, h: f64) -> [f64; 4] {
    let v = perp(u);
    let corners = [at, add(at, u, w), add(add(at, u, w), v, h), add(at, v, h)];
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for c in corners {
        b[0] = b[0].min(c[0]);
        b[1] = b[1].min(c[1]);
        b[2] = b[2].max(c[0]);
        b[3] = b[3].max(c[1]);
    }
    b
}

/// The bend line of station sources `bend:<nr>`, when there is one.
fn bend_of(sources: &[String]) -> Option<usize> {
    sources
        .iter()
        .find_map(|s| s.strip_prefix("bend:").and_then(|n| n.parse().ok()))
}

/// Unit direction of a bend line, turned to point up (or right).
fn bend_dir(b: &BendLine) -> P2 {
    let d = [b.b[0] - b.a[0], b.b[1] - b.a[1]];
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-12);
    let u = [d[0] / l, d[1] / l];
    if u[1] < -1e-9 || (u[1].abs() <= 1e-9 && u[0] < 0.0) {
        [-u[0], -u[1]]
    } else {
        u
    }
}

/// Whether a line of the outline perpendicular to `n` runs through `p`:
/// an edge a chain along `n` locates. The line is extended past its ends
/// for the virtual corner a relief hides.
fn edge_through(f: &Features, p: P2, n: P2) -> bool {
    f.loops.iter().flatten().any(|seg| {
        let Seg::Line { a, b } = seg else {
            return false;
        };
        let l = dist(*a, *b);
        if l < 1e-9 {
            return false;
        }
        let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
        if dot(u, n).abs() > 0.02 {
            return false;
        }
        let t = dot([p[0] - a[0], p[1] - a[1]], u);
        if !(-50.0..=l + 50.0).contains(&t) {
            return false;
        }
        dist(p, add(*a, u, t)) <= SAME
    })
}

/// Whether an outline ordinate belongs on the chain along `n`: one of
/// its corners ends an edge perpendicular to `n`, or lies on a curve.
fn locatable(f: &Features, sources: &[String], n: P2) -> bool {
    sources.iter().any(|src| {
        let mut it = src.split(':');
        if it.next() != Some("corner") {
            return false;
        }
        let Some(i) = it.next().and_then(|x| x.parse::<usize>().ok()) else {
            return false;
        };
        let Some(c) = f.corners.get(i) else {
            return false;
        };
        c.on_curve || edge_through(f, c.p, n)
    })
}

/// Whether the segment crosses the rectangle of text `w` wide and `H`
/// tall written from `at` along `u`.
fn crosses_text(l: [P2; 2], at: P2, u: P2, w: f64) -> bool {
    let v = perp(u);
    let local = |p: P2| {
        [
            dot([p[0] - at[0], p[1] - at[1]], u),
            dot([p[0] - at[0], p[1] - at[1]], v),
        ]
    };
    crosses_box(local(l[0]), local(l[1]), [0.0, 0.0, w, H])
}

/// Whether the segment `a`–`b` crosses the box (Liang–Barsky).
pub fn crosses_box(a: P2, b: P2, bx: [f64; 4]) -> bool {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let d = [b[0] - a[0], b[1] - a[1]];
    for (p, q) in [
        (-d[0], a[0] - bx[0]),
        (d[0], bx[2] - a[0]),
        (-d[1], a[1] - bx[1]),
        (d[1], bx[3] - a[1]),
    ] {
        if p.abs() < 1e-12 {
            if q < 0.0 {
                return false;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
        if t0 > t1 {
            return false;
        }
    }
    true
}

fn axis_aligned(u: P2) -> bool {
    u[0].abs() < 1e-6 || u[1].abs() < 1e-6
}

/// The outline's extreme along `n`, and a point there nearest the side
/// `side · e`.
fn extreme(samples: &[P2], n: P2, e: P2, side: f64, max: bool) -> (f64, P2) {
    let mut best = if max {
        f64::NEG_INFINITY
    } else {
        f64::INFINITY
    };
    let mut at = [0.0, 0.0];
    let mut hb = f64::NEG_INFINITY;
    for p in samples {
        let t = dot(*p, n);
        let better = if max {
            t > best + 1e-6
        } else {
            t < best - 1e-6
        };
        let same = (t - best).abs() <= 1e-6;
        let h = side * dot(*p, e);
        if better || (same && h > hb) {
            best = t;
            at = *p;
            hb = h;
        }
    }
    (best, at)
}

pub fn layout(f: &Features, blank_bends: &[BendLine], s: f64, lang: Lang) -> Layout {
    // The outline sampled on paper: the extremes come from it and nothing
    // may be written on it.
    let mut outline: Vec<P2> = Vec::new();
    for seg in f.loops.iter().flatten() {
        sample(seg, s, &mut outline);
    }
    let mut obstacles: Vec<P2> = outline.clone();
    for b in &f.bends {
        sample(&Seg::Line { a: b.a, b: b.b }, s, &mut obstacles);
    }
    let mut boxes: Vec<[f64; 4]> = Vec::new();
    let mut chains: Vec<Chain> = Vec::new();

    // Which bends are oblique: their ends are no station of the x and y
    // chains; they get a chain of their own.
    let oblique: Vec<bool> = blank_bends
        .iter()
        .map(|b| !axis_aligned(bend_dir(b)))
        .collect();

    // Axis chains. Below (x) and left (y): bends, outline, overall.
    // Above and right: holes.
    type Rows = Vec<(&'static str, Vec<Station>)>;
    let mut per_axis: Vec<(P2, P2, f64, f64, Rows, Rows)> = Vec::new();
    for axis in 0..2usize {
        let n: P2 = if axis == 0 { [1.0, 0.0] } else { [0.0, 1.0] };
        let e: P2 = if axis == 0 { [0.0, 1.0] } else { [1.0, 0.0] };
        let ords = if axis == 0 { &f.xs } else { &f.ys };
        let lo = 0.0;
        let hi = f.max[axis];
        let mut bend_st: Vec<Station> = Vec::new();
        let mut hole_st: Vec<Station> = Vec::new();
        let mut out_st: Vec<Station> = Vec::new();
        for o in ords {
            if o.value <= lo + SAME || o.value >= hi - SAME {
                continue;
            }
            let sources: Vec<String> = o
                .sources
                .iter()
                .filter(|src| match bend_of(std::slice::from_ref(src)) {
                    Some(nr) => !oblique.get(nr - 1).copied().unwrap_or(false),
                    None => true,
                })
                .cloned()
                .collect();
            if sources.is_empty() {
                continue;
            }
            let is_bend = sources.iter().any(|s| s.starts_with("bend:"));
            let is_hole = sources.iter().any(|s| s.starts_with("circle:"));
            // Extension line start: the point nearest the dimension line.
            let pick = |side: f64| -> P2 {
                *o.points
                    .iter()
                    .max_by(|a, b| (side * dot(**a, e)).total_cmp(&(side * dot(**b, e))))
                    .unwrap()
            };
            if is_bend {
                bend_st.push(Station {
                    t: o.value,
                    from: pick(-1.0),
                    sources,
                    no_gap: true,
                });
            } else if is_hole {
                hole_st.push(Station {
                    t: o.value,
                    from: pick(1.0),
                    sources,
                    no_gap: false,
                });
            } else if locatable(f, &sources, n) {
                out_st.push(Station {
                    t: o.value,
                    from: pick(-1.0),
                    sources,
                    no_gap: false,
                });
            }
        }
        let mut below: Vec<(&'static str, Vec<Station>)> = Vec::new();
        if !bend_st.is_empty() {
            below.push(("bend", bend_st));
        }
        if !out_st.is_empty() {
            below.push(("outline", out_st));
        }
        below.push(("overall", Vec::new()));
        let mut above: Vec<(&'static str, Vec<Station>)> = Vec::new();
        if !hole_st.is_empty() {
            above.push(("hole", hole_st));
        }
        per_axis.push((n, e, lo, hi, below, above));
    }

    // Place the rows: the hole chains above and right first, then the
    // rows below and left, then the stops of oblique bends, each keeping
    // its values clear of what was drawn before.
    // The part itself (its outline, sampled, and its bend lines) is a
    // line no value may sit on.
    let mut lines: Vec<[P2; 2]> = Vec::new();
    for lp in &f.loops {
        let pts = super::features::loop_points(lp, s);
        for w in pts.windows(2) {
            lines.push([w[0], w[1]]);
        }
        if let (Some(a), Some(b)) = (pts.last(), pts.first()) {
            lines.push([*a, *b]);
        }
    }
    for b in &f.bends {
        lines.push([
            add([b.a[0] * s, b.a[1] * s], unit2(b.a, b.b), -OVERSHOOT),
            add([b.b[0] * s, b.b[1] * s], unit2(b.a, b.b), OVERSHOOT),
        ]);
    }
    let part_lines = lines.len();
    // The lines a value may not sit on: the part, dimension lines and
    // leaders. Extension lines are interrupted where a value crosses them.
    let mut hard: Vec<[P2; 2]> = lines.clone();
    let place_rows = |side: f64,
                      which: usize,
                      chains: &mut Vec<Chain>,
                      boxes: &mut Vec<[f64; 4]>,
                      lines: &mut Vec<[P2; 2]>,
                      hard: &mut Vec<[P2; 2]>| {
        for (n, e, lo, hi, below, above) in &per_axis {
            let rows = if which == 0 { below } else { above };
            let mut line = extreme(&outline, *e, *e, side, side > 0.0).0 * side + D;
            for (kind, st) in rows {
                let mut st = st.clone();
                let (a, b) = end_stations(&outline, *n, *e, side, s, *lo, *hi);
                st.insert(0, a);
                st.push(b);
                let ch = place_chain(kind, *n, *e, side, line, st, s, boxes, hard);
                line += SPACING + chain_rise(&ch);
                let own = chain_lines(&ch, s);
                hard.push(own[0]);
                lines.extend(own);
                chains.push(ch);
            }
        }
    };
    place_rows(1.0, 1, &mut chains, &mut boxes, &mut lines, &mut hard);
    place_rows(-1.0, 0, &mut chains, &mut boxes, &mut lines, &mut hard);
    // Oblique bends: each gets the press-brake stop of its flange, a
    // dimension across the flange from the bend line to the parallel
    // edge of the blank, its value outside the blank past that edge,
    // somewhere along the line where it meets the least. The sections
    // give the distances between bends.
    for (j, b) in blank_bends.iter().enumerate() {
        if !oblique[j] {
            continue;
        }
        let e = bend_dir(b);
        let n = reading([e[1], -e[0]]);
        let t = dot([(b.a[0] + b.b[0]) / 2.0, (b.a[1] + b.b[1]) / 2.0], n);
        let (t0, _) = extreme(&outline, n, e, 1.0, false);
        let (t1, _) = extreme(&outline, n, e, 1.0, true);
        let te = if t - t0 / s <= t1 / s - t {
            t0 / s
        } else {
            t1 / s
        };
        let mut best: Option<(usize, Chain, Vec<[f64; 4]>)> = None;
        'spots: for frac in [0.5, 0.35, 0.65, 0.2, 0.8] {
            let pb = [
                b.a[0] + (b.b[0] - b.a[0]) * frac,
                b.a[1] + (b.b[1] - b.a[1]) * frac,
            ];
            let pe = add(pb, n, te - t);
            let st = vec![
                Station {
                    t: te,
                    from: pe,
                    sources: vec!["extent".into()],
                    no_gap: true,
                },
                Station {
                    t,
                    from: pb,
                    sources: vec![format!("bend:{}", j + 1)],
                    no_gap: true,
                },
            ];
            let line = dot(pb, e) * s;
            let mut trial = boxes.clone();
            let ch = place_chain("bend", n, e, 1.0, line, st, s, &mut trial, &hard);
            let own = chain_lines(&ch, s);
            let bad = ch
                .texts
                .iter()
                .filter(|t| {
                    boxes.iter().any(|b| overlap(*b, t.bx))
                        || hard
                            .iter()
                            .any(|l| crosses_text(*l, t.at, reading(n), font::width(&t.text, H)))
                })
                .count()
                + own
                    .iter()
                    .map(|l| {
                        boxes
                            .iter()
                            .filter(|b| crosses_box(l[0], l[1], **b))
                            .count()
                    })
                    .sum::<usize>();
            if best.as_ref().is_none_or(|(b, _, _)| bad < *b) {
                best = Some((bad, ch, trial));
                if bad == 0 {
                    break 'spots;
                }
            }
        }
        let (_, ch, trial) = best.unwrap();
        boxes = trial;
        let own = chain_lines(&ch, s);
        hard.push(own[0]);
        lines.extend(own);
        chains.push(ch);
    }

    // Every line is known now: place the values again so that none sits
    // on the dimension line of a row placed after its own.
    let mut all_lines: Vec<[P2; 2]> = lines[..part_lines].to_vec();
    hard.truncate(part_lines);
    for ch in &chains {
        let own = chain_lines(ch, s);
        hard.push(own[0]);
        all_lines.extend(own);
    }
    boxes.clear();
    for ch in &mut chains {
        place_texts(ch, s, &mut boxes, &hard);
    }
    lines = all_lines;

    // Everything drawn so far is an obstacle for callouts and tags.
    for ch in &chains {
        for pts in chain_lines(ch, s) {
            sample_line(pts[0], pts[1], &mut obstacles);
        }
    }

    // Callouts: one per size, next to its feature when a corner of the
    // sheet around it is free, else in a column up the right side.
    let w = f.max[0] * s;
    let h = f.max[1] * s;
    let mut callouts = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    for c in &f.circles {
        let d = (c.r * 2.0 * 10.0).round() / 10.0;
        let anchor = [c.c[0] + c.r, c.c[1]];
        push_group(&mut groups, "diameter", d, (anchor, c.c));
    }
    for a in &f.arcs {
        let r = (a.r * 10.0).round() / 10.0;
        push_group(&mut groups, "radius", r, (arc_mid(a), a.c));
    }
    let right_edge = chains
        .iter()
        .filter(|c| c.side > 0.0 && c.e[0] > 0.5)
        .map(|c| c.line + H + 2.0)
        .fold(w, f64::max);
    let mut column = 0usize;
    for (kind, value, members) in &groups {
        let n = members.len();
        let sym = if *kind == "diameter" { "⌀" } else { "R" };
        let t = if n > 1 {
            format!("{n}× {sym}{}", text::mm(*value))
        } else {
            format!("{sym}{}", text::mm(*value))
        };
        let tw = font::width(&t, H);
        // The member nearest the top right, where a callout reads best.
        let (anchor, centre) = *members
            .iter()
            .max_by(|a, b| (a.1[0] + 0.25 * a.1[1]).total_cmp(&(b.1[0] + 0.25 * b.1[1])))
            .unwrap();
        let r = dist(anchor, centre) * s;
        let pc = [centre[0] * s, centre[1] * s];
        // Where the leader may leave the feature: a hole from four
        // diagonals, an arc only through the arc itself.
        let dirs: Vec<P2> = if *kind == "diameter" {
            let k = std::f64::consts::FRAC_1_SQRT_2;
            vec![[k, k], [-k, k], [k, -k], [-k, -k]]
        } else {
            let arc = f
                .arcs
                .iter()
                .find(|a| dist(a.c, centre) < 1e-9 && (a.r - r / s).abs() < 1e-6)
                .cloned();
            match arc {
                Some(a) => {
                    let sweep = (a.end - a.start).rem_euclid(360.0);
                    let sweep = if sweep < 1e-9 { 360.0 } else { sweep };
                    [0.5, 0.25, 0.75]
                        .iter()
                        .map(|t| {
                            let ang = (a.start + sweep * t).to_radians();
                            [ang.cos(), ang.sin()]
                        })
                        .collect()
                }
                None => vec![],
            }
        };
        let mut placed: Option<(P2, P2)> = None;
        'leader: for leader in [LEADER, LEADER + 7.0, LEADER + 15.0, LEADER + 25.0] {
            for u in &dirs {
                let end = [pc[0] + u[0] * (r + leader), pc[1] + u[1] * (r + leader)];
                let at = if u[0] >= 0.0 {
                    [end[0] + 1.0, end[1] - H / 2.0]
                } else {
                    [end[0] - 1.0 - tw, end[1] - H / 2.0]
                };
                let bx = [at[0] - 0.8, at[1] - 0.8, at[0] + tw + 0.8, at[1] + H + 0.8];
                if obstacles.iter().any(|p| inside(bx, *p)) || boxes.iter().any(|b| overlap(bx, *b))
                {
                    continue;
                }
                let from = if u[0] >= 0.0 {
                    [at[0] - 0.8, at[1] + H / 2.0]
                } else {
                    [at[0] + tw + 0.8, at[1] + H / 2.0]
                };
                placed = Some((at, from));
                break 'leader;
            }
        }
        let (at, from) = placed.unwrap_or_else(|| {
            let at = [
                right_edge + CALLOUT_GAP,
                h - H - CALLOUT_PITCH * column as f64,
            ];
            column += 1;
            (at, [at[0] - 0.8, at[1] + H / 2.0])
        });
        boxes.push([at[0] - 0.8, at[1] - 0.8, at[0] + tw + 0.8, at[1] + H + 0.8]);
        callouts.push(Callout {
            from,
            text: t,
            kind,
            value: *value,
            count: n,
            anchor,
            centre,
            at,
        });
    }

    // Bend tags beyond the far end of each line, spread apart where lines
    // sit close together, and the note along the line.
    let mut right: Vec<usize> = Vec::new();
    let mut top: Vec<usize> = Vec::new();
    let mut tags = Vec::new();
    for (bi, b) in f.bends.iter().enumerate() {
        let (a, e) = ([b.a[0] * s, b.a[1] * s], [b.b[0] * s, b.b[1] * s]);
        let d = [e[0] - a[0], e[1] - a[1]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
        let u = [d[0] / l, d[1] / l];
        let horizontal = u[1].abs() < 1e-6;
        // The far end: right for a horizontal line, top otherwise.
        let far_first = if horizontal { a[0] > e[0] } else { a[1] > e[1] };
        let (near_end, far_end, u) = if far_first {
            (e, a, [-u[0], -u[1]])
        } else {
            (a, e, u)
        };
        let line = [add(near_end, u, -OVERSHOOT), add(far_end, u, OVERSHOOT)];
        let at = add(far_end, u, OVERSHOOT + 1.5 + TAG_R);
        if horizontal {
            right.push(tags.len());
        } else {
            top.push(tags.len());
        }
        // The note: along the line, centred, on whichever side is free.
        let mut note = None;
        if let Some(bl) = blank_bends.get(bi) {
            let dir = match bl.direction {
                BendDirection::Up => lang.t("up"),
                BendDirection::Down => lang.t("down"),
            };
            let t = format!(
                "{dir} {} R{}",
                text::deg(bl.angle_deg),
                text::mm(bl.inside_radius)
            );
            let tw = font::width(&t, H);
            if tw + 4.0 <= l {
                let ur = reading(u);
                let v = perp(ur);
                let mid = [(a[0] + e[0]) / 2.0, (a[1] + e[1]) / 2.0];
                for sign in [1.0, -1.0] {
                    let off = if sign > 0.0 { TEXT_UP } else { -(TEXT_UP + H) };
                    let at_t = add(add(mid, ur, -tw / 2.0), v, off);
                    let bx = text_box(at_t, ur, tw, H);
                    let bx = [bx[0] - 0.5, bx[1] - 0.5, bx[2] + 0.5, bx[3] + 0.5];
                    let hit = obstacles
                        .iter()
                        .any(|p| inside(bx, *p) && dist_to_line(*p, a, e) > 0.3)
                        || boxes.iter().any(|b| overlap(bx, *b));
                    if !hit {
                        boxes.push(bx);
                        note = Some((t.clone(), at_t, ur[1].atan2(ur[0]).to_degrees()));
                        break;
                    }
                }
            }
        }
        tags.push(Tag {
            nr: b.nr,
            line,
            at,
            u,
            note,
        });
    }
    spread(&mut tags, &right, 1);
    spread(&mut tags, &top, 0);
    // Where a tag lands on another, on a text or on the drawing, move it
    // further out along its line; where its leader would run through a
    // text, put it at the other end of the line instead.
    let mut flipped = vec![false; tags.len()];
    for _ in 0..40 {
        let mut moved = false;
        for i in 0..tags.len() {
            let at = tags[i].at;
            let u = tags[i].u;
            let leader = [tags[i].line[1], add(at, u, -TAG_R)];
            if !flipped[i] && boxes.iter().any(|b| crosses_box(leader[0], leader[1], *b)) {
                flipped[i] = true;
                tags[i].u = [-u[0], -u[1]];
                tags[i].line.swap(0, 1);
                tags[i].at = add(tags[i].line[1], tags[i].u, 1.5 + TAG_R);
                moved = true;
                continue;
            }
            let hit = (0..tags.len()).any(|j| j != i && dist(at, tags[j].at) < 2.0 * TAG_R + 0.5)
                || boxes.iter().any(|b| {
                    overlap(
                        *b,
                        [at[0] - TAG_R, at[1] - TAG_R, at[0] + TAG_R, at[1] + TAG_R],
                    )
                })
                || obstacles.iter().any(|p| dist(at, *p) < TAG_R + 0.4);
            if hit {
                tags[i].at = add(at, u, 2.0);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    for t in &tags {
        boxes.push([
            t.at[0] - TAG_R,
            t.at[1] - TAG_R,
            t.at[0] + TAG_R,
            t.at[1] + TAG_R,
        ]);
        lines.push(t.line);
        lines.push([t.line[1], add(t.at, t.u, -TAG_R)]);
    }
    for ch in &chains {
        for t in &ch.texts {
            if let Some(l) = t.leader {
                lines.push(l);
            }
        }
    }

    // The margin: everything drawn, against the blank's box.
    let mut ext = [0.0, 0.0, w, h];
    let grow = |ext: &mut [f64; 4], p: P2, pad: f64| {
        ext[0] = ext[0].min(p[0] - pad);
        ext[1] = ext[1].min(p[1] - pad);
        ext[2] = ext[2].max(p[0] + pad);
        ext[3] = ext[3].max(p[1] + pad);
    };
    for ch in &chains {
        for pts in chain_lines(ch, s) {
            grow(&mut ext, pts[0], 0.5);
            grow(&mut ext, pts[1], 0.5);
        }
        for t in &ch.texts {
            grow(&mut ext, [t.bx[0], t.bx[1]], 0.5);
            grow(&mut ext, [t.bx[2], t.bx[3]], 0.5);
        }
    }
    for b in &boxes {
        grow(&mut ext, [b[0], b[1]], 0.5);
        grow(&mut ext, [b[2], b[3]], 0.5);
    }
    for t in &tags {
        grow(&mut ext, t.at, TAG_R + 0.5);
        grow(&mut ext, t.line[0], 0.5);
        grow(&mut ext, t.line[1], 0.5);
    }
    Layout {
        s,
        chains,
        callouts,
        tags,
        margin: [-ext[0], -ext[1], ext[2] - w, ext[3] - h],
        boxes,
        lines,
    }
}

/// The unit direction from `a` to `b`.
fn unit2(a: P2, b: P2) -> P2 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
    [d[0] / l, d[1] / l]
}

fn dist_to_line(p: P2, a: P2, b: P2) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-12);
    ((p[0] - a[0]) * d[1] - (p[1] - a[1]) * d[0]).abs() / l
}

/// How much further out than a value on its line a chain reaches.
fn chain_rise(ch: &Chain) -> f64 {
    (ch.reach - (TEXT_UP + H)).max(0.0)
}

/// The two stations at the blank's extent along `n`, their extension
/// lines starting at the outline point nearest the `side`.
fn end_stations(
    outline: &[P2],
    n: P2,
    e: P2,
    side: f64,
    s: f64,
    lo: f64,
    hi: f64,
) -> (Station, Station) {
    let (_, p0) = extreme(outline, n, e, side, false);
    let (_, p1) = extreme(outline, n, e, side, true);
    (
        Station {
            t: lo,
            from: [p0[0] / s, p0[1] / s],
            sources: vec!["extent".into()],
            no_gap: false,
        },
        Station {
            t: hi,
            from: [p1[0] / s, p1[1] / s],
            sources: vec!["extent".into()],
            no_gap: false,
        },
    )
}

/// One chain with its stations sorted and merged; its values placed by
/// [`place_texts`].
#[allow(clippy::too_many_arguments)]
fn place_chain(
    kind: &'static str,
    n: P2,
    e: P2,
    side: f64,
    line: f64,
    mut stations: Vec<Station>,
    s: f64,
    boxes: &mut Vec<[f64; 4]>,
    lines: &[[P2; 2]],
) -> Chain {
    stations.sort_by(|a, b| a.t.total_cmp(&b.t));
    stations.dedup_by(|a, b| {
        if (a.t - b.t).abs() <= SAME {
            let extra: Vec<String> = std::mem::take(&mut a.sources);
            for src in extra {
                if !b.sources.contains(&src) {
                    b.sources.push(src);
                }
            }
            true
        } else {
            false
        }
    });
    let mut ch = Chain {
        kind,
        n,
        e,
        side,
        line,
        stations,
        texts: Vec::new(),
        span: [0.0, 0.0],
        reach: 0.0,
    };
    place_texts(&mut ch, s, boxes, lines);
    ch
}

/// Place one chain's values: between the extension lines when they fit,
/// slid along the gap where something drawn before is in the way;
/// beyond the end for the first and last; a value too wide for its gap
/// above the neighbouring gap with a leader to its own, or away from the
/// blank under the line; never on a line in `lines` or a box in `boxes`
/// when it can be helped.
/// Pool-adjacent-violators: the non-decreasing sequence nearest to `y`
/// (weighted least squares).
fn isotonic(y: &[f64], w: &[f64]) -> Vec<f64> {
    let mut blocks: Vec<(f64, f64, usize)> = Vec::new();
    for (yi, wi) in y.iter().zip(w) {
        blocks.push((yi * wi, *wi, 1));
        while blocks.len() >= 2 {
            let n = blocks.len();
            let (a, b) = (blocks[n - 2], blocks[n - 1]);
            if a.0 / a.1 <= b.0 / b.1 + 1e-12 {
                break;
            }
            blocks[n - 2] = (a.0 + b.0, a.1 + b.1, a.2 + b.2);
            blocks.pop();
        }
    }
    let mut out = Vec::with_capacity(y.len());
    for b in blocks {
        for _ in 0..b.2 {
            out.push(b.0 / b.1);
        }
    }
    out
}

/// Where a chain's values go. Every value wants the middle of its gap,
/// above the line; values that do not fit are squeezed in along the line
/// in their order, pushing their neighbours aside (and the first and last
/// out past the ends), never over a line or another text. A value that
/// still finds no room there goes above a neighbouring gap with a leader,
/// or away from the blank under the line.
fn place_texts(ch: &mut Chain, s: f64, boxes: &mut Vec<[f64; 4]>, lines: &[[P2; 2]]) {
    let (n, e, side, line) = (ch.n, ch.e, ch.side, ch.line);
    let stations = &ch.stations;
    let ur = reading(n);
    let up = perp(ur);
    let out = [side * e[0], side * e[1]];
    // Whether a value above its line (ISO 129-1) is away from the blank,
    // where no extension line runs.
    let up_out = dot(up, out) > 0.0;
    let base = add([0.0, 0.0], e, side * line);
    let on_line = |t: f64| add(base, n, t * s);
    let m = stations.len();
    let mut span = [stations[0].t * s, stations[m - 1].t * s];
    let mut reach: f64 = 0.0;
    // Clear of every box (by hull) and of every hard line (by the text's
    // own rectangle, so a slanted value is not blamed for its hull).
    let clear = |at: P2, bx: [f64; 4], tw: f64, boxes: &[[f64; 4]]| {
        !boxes.iter().any(|b| overlap(*b, bx))
            && !lines.iter().any(|l| crosses_text(*l, at, ur, tw))
    };
    let slides = |play: f64| -> Vec<f64> {
        let mut v = vec![0.0];
        let mut k = 4.0;
        while k <= play {
            v.push(k);
            v.push(-k);
            k += 4.0;
        }
        v
    };
    // Local coordinates: u along the reading direction, v up from the
    // dimension line.
    let local = |p: P2| {
        [
            dot([p[0] - base[0], p[1] - base[1]], ur),
            dot([p[0] - base[0], p[1] - base[1]], up),
        ]
    };

    // The values, in reading order.
    let count = m - 1;
    let mut order: Vec<usize> = (0..count).collect();
    if dot(n, ur) < 0.0 {
        order.reverse();
    }
    let txts: Vec<String> = (0..count)
        .map(|i| text::mm(stations[i + 1].t - stations[i].t))
        .collect();
    let tws: Vec<f64> = txts.iter().map(|t| font::width(t, H)).collect();
    let mids: Vec<P2> = (0..count)
        .map(|i| on_line((stations[i].t + stations[i + 1].t) / 2.0))
        .collect();

    // What the text band (above the line) may not cover: lines through
    // it (extension lines excepted: they are interrupted at a value) and
    // boxes on it, as intervals along u.
    // (Descenders reach below the baseline.)
    let band = [TEXT_UP - 0.8, TEXT_UP + H + 0.3];
    let mut forbidden: Vec<[f64; 2]> = Vec::new();
    for l in lines {
        let (a, b) = (local(l[0]), local(l[1]));
        // Clip the segment to the band's v range.
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let dv = b[1] - a[1];
        if dv.abs() < 1e-12 {
            if a[1] < band[0] || a[1] > band[1] {
                continue;
            }
        } else {
            let ta = (band[0] - a[1]) / dv;
            let tb = (band[1] - a[1]) / dv;
            t0 = t0.max(ta.min(tb));
            t1 = t1.min(ta.max(tb));
            if t0 > t1 {
                continue;
            }
        }
        let u0 = a[0] + (b[0] - a[0]) * t0;
        let u1 = a[0] + (b[0] - a[0]) * t1;
        forbidden.push([u0.min(u1) - 0.4, u0.max(u1) + 0.4]);
    }
    for b in boxes.iter() {
        let cs = [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]].map(local);
        let (mut u0, mut u1, mut v0, mut v1) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        );
        for c in cs {
            u0 = u0.min(c[0]);
            u1 = u1.max(c[0]);
            v0 = v0.min(c[1]);
            v1 = v1.max(c[1]);
        }
        if v1 >= band[0] && v0 <= band[1] {
            forbidden.push([u0 - 0.3, u1 + 0.3]);
        }
    }

    // Squeeze: preferred centres in reading order, minimum separations,
    // isotonic regression on the slack; a value over a forbidden
    // interval is pinned beside it and the rest solved again.
    let pref: Vec<f64> = order.iter().map(|&i| local(mids[i])[0]).collect();
    let widths: Vec<f64> = order.iter().map(|&i| tws[i] + 0.8).collect();
    let mut target = pref.clone();
    let mut weight = vec![1.0; count];
    let mut tried: Vec<u8> = vec![0; count];
    let mut centres = vec![0.0; count];
    for _ in 0..3 * count + 3 {
        let mut d = vec![0.0; count];
        for k in 1..count {
            d[k] = d[k - 1] + (widths[k - 1] + widths[k]) / 2.0;
        }
        let y: Vec<f64> = (0..count).map(|k| target[k] - d[k]).collect();
        let z = isotonic(&y, &weight);
        for k in 0..count {
            centres[k] = z[k] + d[k];
        }
        let mut moved = false;
        for k in 0..count {
            let half = widths[k] / 2.0;
            let hit = forbidden
                .iter()
                .filter(|f| f[0] < centres[k] + half && centres[k] - half < f[1])
                .fold(None, |acc: Option<[f64; 2]>, f| {
                    Some(acc.map_or(*f, |a| [a[0].min(f[0]), a[1].max(f[1])]))
                });
            let Some(f) = hit else {
                continue;
            };
            if tried[k] >= 2 {
                continue;
            }
            let left = f[0] - half - 0.1;
            let right = f[1] + half + 0.1;
            let nearer_left = (centres[k] - left).abs() <= (right - centres[k]).abs();
            let pick = match (tried[k], nearer_left) {
                (0, true) | (1, false) => left,
                _ => right,
            };
            tried[k] += 1;
            target[k] = pick;
            weight[k] = 50.0;
            moved = true;
            break;
        }
        if !moved {
            break;
        }
    }

    let mut texts: Vec<Option<ChainText>> = vec![None; count];
    let mut unresolved = Vec::new();
    for (k, &i) in order.iter().enumerate() {
        let tw = tws[i];
        let half = widths[k] / 2.0;
        let blocked = forbidden
            .iter()
            .any(|f| f[0] < centres[k] + half && centres[k] - half < f[1]);
        let at = add(
            add(mids[i], ur, centres[k] - pref[k] - tw / 2.0),
            up,
            TEXT_UP,
        );
        let bx = text_box(at, ur, tw, H);
        if !blocked && clear(at, bx, tw, boxes) {
            boxes.push(bx);
            let (u0, u1) = (centres[k] - tw / 2.0 - 1.0, centres[k] + tw / 2.0 + 1.0);
            // The dimension line runs under a value pushed past an end.
            let ends = [
                local(on_line(stations[0].t))[0],
                local(on_line(stations[m - 1].t))[0],
            ];
            let (lo_u, hi_u) = (ends[0].min(ends[1]), ends[0].max(ends[1]));
            let sign = if dot(n, ur) > 0.0 { 1.0 } else { -1.0 };
            if u0 < lo_u {
                let st = if sign > 0.0 {
                    &mut span[0]
                } else {
                    &mut span[1]
                };
                let v = stations[0].t.min(stations[m - 1].t) * s - (lo_u - u0);
                if sign > 0.0 {
                    *st = st.min(v);
                } else {
                    *st = st.max(stations[m - 1].t.max(stations[0].t) * s + (lo_u - u0));
                }
            }
            if u1 > hi_u {
                if sign > 0.0 {
                    span[1] = span[1].max(stations[m - 1].t * s + (u1 - hi_u));
                } else {
                    span[0] = span[0].min(stations[0].t * s - (u1 - hi_u));
                }
            }
            texts[i] = Some(ChainText {
                value: stations[i + 1].t - stations[i].t,
                text: txts[i].clone(),
                at,
                angle: ur[1].atan2(ur[0]).to_degrees(),
                bx,
                leader: None,
            });
        } else {
            unresolved.push(i);
        }
    }

    for i in unresolved {
        let tw = tws[i];
        let mid = mids[i];
        let mut choice: Option<(P2, [f64; 4], Option<[P2; 2]>)> = None;
        // Above a neighbouring gap, the wider first, with a leader.
        let mut neighbours: Vec<usize> = Vec::new();
        if i > 0 {
            neighbours.push(i - 1);
        }
        if i + 2 < m {
            neighbours.push(i + 1);
        }
        neighbours.sort_by(|a, b| {
            let la = stations[a + 1].t - stations[*a].t;
            let lb = stations[b + 1].t - stations[*b].t;
            lb.total_cmp(&la)
        });
        'nb: for j in neighbours {
            let glen = (stations[j + 1].t - stations[j].t) * s;
            if glen < tw + 2.0 {
                continue;
            }
            let gm = on_line((stations[j].t + stations[j + 1].t) / 2.0);
            let towards = if dot(gm, ur) > dot(mid, ur) {
                -1.0
            } else {
                1.0
            };
            for sl in slides((glen - tw - 2.0) / 2.0) {
                let at = add(add(gm, ur, sl - tw / 2.0), up, TEXT_UP + RAISE);
                let bx = text_box(at, ur, tw, H);
                let tip = add(
                    add(at, ur, if towards > 0.0 { tw + 0.6 } else { -0.6 }),
                    up,
                    H / 2.0,
                );
                let leader = [mid, tip];
                if clear(at, bx, tw, boxes)
                    && !boxes.iter().any(|b| crosses_box(leader[0], leader[1], *b))
                {
                    choice = Some((at, bx, Some(leader)));
                    break 'nb;
                }
            }
        }
        if choice.is_none() {
            // Away from the blank, under the line.
            'out: for level in 1..=3 {
                let off = if up_out {
                    TEXT_UP + RAISE * level as f64
                } else {
                    -(TEXT_UP + 0.5 + H + RAISE * (level - 1) as f64)
                };
                for sl in slides(6.0) {
                    let at = add(add(mid, ur, sl - tw / 2.0), up, off);
                    let bx = text_box(at, ur, tw, H);
                    if clear(at, bx, tw, boxes) {
                        choice = Some((at, bx, None));
                        break 'out;
                    }
                }
            }
        }
        let (at, bx, leader) = choice.unwrap_or_else(|| {
            let at = add(add(mid, ur, -tw / 2.0), up, TEXT_UP + RAISE);
            (at, text_box(at, ur, tw, H), None)
        });
        boxes.push(bx);
        texts[i] = Some(ChainText {
            value: stations[i + 1].t - stations[i].t,
            text: txts[i].clone(),
            at,
            angle: ur[1].atan2(ur[0]).to_degrees(),
            bx,
            leader,
        });
    }
    let texts: Vec<ChainText> = texts.into_iter().map(|t| t.unwrap()).collect();
    for t in &texts {
        let bx = t.bx;
        for c in [
            [bx[0], bx[1]],
            [bx[2], bx[1]],
            [bx[0], bx[3]],
            [bx[2], bx[3]],
        ] {
            reach = reach.max(dot(c, out) - dot(base, out));
        }
    }
    // Arrows outside a short end segment need room beyond the end.
    if m >= 2 {
        if (stations[1].t - stations[0].t) * s < ARROW_L + 1.0 {
            span[0] = span[0].min(stations[0].t * s - ARROW_L - 2.0);
        }
        if (stations[m - 1].t - stations[m - 2].t) * s < ARROW_L + 1.0 {
            span[1] = span[1].max(stations[m - 1].t * s + ARROW_L + 2.0);
        }
    }
    ch.texts = texts;
    ch.span = span;
    ch.reach = reach;
}

/// The parts of the segment `l` outside the boxes (grown by `margin`).
fn cut_out(l: [P2; 2], boxes: &[[f64; 4]], margin: f64) -> Vec<[P2; 2]> {
    let d = [l[1][0] - l[0][0], l[1][1] - l[0][1]];
    let mut holes: Vec<[f64; 2]> = Vec::new();
    for b in boxes {
        let bx = [b[0] - margin, b[1] - margin, b[2] + margin, b[3] + margin];
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let mut ok = true;
        for k in 0..2 {
            if d[k].abs() < 1e-12 {
                if l[0][k] < bx[k] || l[0][k] > bx[k + 2] {
                    ok = false;
                }
            } else {
                let ta = (bx[k] - l[0][k]) / d[k];
                let tb = (bx[k + 2] - l[0][k]) / d[k];
                t0 = t0.max(ta.min(tb));
                t1 = t1.min(ta.max(tb));
            }
        }
        if ok && t0 < t1 {
            holes.push([t0, t1]);
        }
    }
    holes.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut out = Vec::new();
    let mut t = 0.0;
    let at = |t: f64| [l[0][0] + d[0] * t, l[0][1] + d[1] * t];
    for h in holes {
        if h[0] > t + 1e-9 {
            out.push([at(t), at(h[0])]);
        }
        t = t.max(h[1]);
    }
    if t < 1.0 - 1e-9 {
        out.push([at(t), at(1.0)]);
    }
    out
}

/// The lines a chain draws (paper): its dimension line and the extension
/// lines.
fn chain_lines(ch: &Chain, s: f64) -> Vec<[P2; 2]> {
    let base = add([0.0, 0.0], ch.e, ch.side * ch.line);
    let mut v = vec![[add(base, ch.n, ch.span[0]), add(base, ch.n, ch.span[1])]];
    for st in &ch.stations {
        let from = [st.from[0] * s, st.from[1] * s];
        let gap = if st.no_gap { 0.0 } else { GAP };
        let start = add(from, ch.e, ch.side * gap);
        let end = add(add(base, ch.n, st.t * s), ch.e, ch.side * EXT);
        v.push([start, end]);
    }
    v
}

fn push_group(groups: &mut Vec<Group>, kind: &'static str, v: f64, m: (P2, P2)) {
    for g in groups.iter_mut() {
        if g.0 == kind && (g.1 - v).abs() < 1e-9 {
            g.2.push(m);
            return;
        }
    }
    groups.push((kind, v, vec![m]));
}

/// Push the tags in `which` apart along axis `axis` (0 = x, 1 = y) so
/// their circles never overlap.
fn spread(tags: &mut [Tag], which: &[usize], axis: usize) {
    let mut order: Vec<usize> = which.to_vec();
    order.sort_by(|a, b| tags[*a].at[axis].total_cmp(&tags[*b].at[axis]));
    for w in 1..order.len() {
        let prev = tags[order[w - 1]].at[axis];
        let i = order[w];
        if tags[i].at[axis] < prev + TAG_PITCH {
            tags[i].at[axis] = prev + TAG_PITCH;
        }
    }
}

/// A closed filled arrowhead with its tip at `tip`, pointing along `u`.
pub fn arrow(c: &mut Content, tip: P2, u: P2) {
    let n = perp(u);
    let base = add(tip, u, -ARROW_L);
    c.move_to(tip);
    c.line_to(add(base, n, ARROW_H / 2.0));
    c.line_to(add(base, n, -ARROW_H / 2.0));
    c.close();
    c.fill();
}

/// The terminators of a dimension line from `a` to `b`: arrowheads inside
/// when the line is long enough, else outside pointing in. Returns how far
/// the line must be extended beyond each end.
pub fn terminators(c: &mut Content, a: P2, b: P2) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
    let u = [d[0] / l, d[1] / l];
    if l >= 2.0 * ARROW_L + 1.0 {
        arrow(c, a, [-u[0], -u[1]]);
        arrow(c, b, u);
        0.0
    } else {
        arrow(c, a, u);
        arrow(c, b, [-u[0], -u[1]]);
        c.line(add(a, u, -ARROW_L - 2.0), a);
        c.line(b, add(b, u, ARROW_L + 2.0));
        ARROW_L + 2.0
    }
}

/// A linear dimension between the model points `p` and `q`, its line
/// offset `off` paper mm from them along `v` (unit, pointing away from the
/// part), with extension lines, terminators and the value written above
/// the line in reading direction. Returns the text box.
pub fn linear(c: &mut Content, p: P2, q: P2, v: P2, off: f64, text: &str) -> [f64; 4] {
    let d = [q[0] - p[0], q[1] - p[1]];
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
    let n = [d[0] / l, d[1] / l];
    // The dimension line through the point `off` away from the farther of
    // p and q along v.
    let far = dot(p, v).max(dot(q, v)) + off;
    let a = add(p, v, far - dot(p, v));
    let b = add(q, v, far - dot(q, v));
    c.line(add(p, v, GAP), add(a, v, EXT));
    c.line(add(q, v, GAP), add(b, v, EXT));
    c.line(a, b);
    terminators(c, a, b);
    let ur = reading(n);
    let up = perp(ur);
    let tw = font::width(text, H);
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let at = if tw + 2.0 <= l {
        add(add(mid, ur, -tw / 2.0), up, TEXT_UP)
    } else {
        // Beyond the end that reads last.
        let end = if dot(n, ur) > 0.0 { b } else { a };
        add(add(end, ur, ARROW_L + 1.5), up, TEXT_UP)
    };
    c.text(at, H, ur[1].atan2(ur[0]).to_degrees(), text);
    text_box(at, ur, tw, H)
}

/// Draw the dimensions with the model origin at paper `o`.
pub fn draw(lo: &Layout, f: &Features, c: &mut Content, o: P2) {
    let s = lo.s;
    let pp = |p: P2| [o[0] + p[0], o[1] + p[1]];
    let pm = |p: P2| [o[0] + p[0] * s, o[1] + p[1] * s];
    c.save();
    c.width(NARROW);
    c.solid();

    // Centre marks on holes.
    for ci in &f.circles {
        let cc = pm(ci.c);
        let e = (ci.r * s + 1.5).max(2.0);
        c.line([cc[0] - e, cc[1]], [cc[0] + e, cc[1]]);
        c.line([cc[0], cc[1] - e], [cc[0], cc[1] + e]);
    }

    for ch in &lo.chains {
        let lines = chain_lines(ch, s);
        c.line(pp(lines[0][0]), pp(lines[0][1]));
        // Extension lines are interrupted where a value sits on them.
        for l in &lines[1..] {
            for piece in cut_out(*l, &lo.boxes, 1.0) {
                c.line(pp(piece[0]), pp(piece[1]));
            }
        }
        let base = add([0.0, 0.0], ch.e, ch.side * ch.line);
        let m = ch.stations.len();
        let pts: Vec<P2> = ch
            .stations
            .iter()
            .map(|st| pp(add(base, ch.n, st.t * s)))
            .collect();
        // Ends: arrows inside when the end segment has room, else outside.
        for (i, j, sign) in [(0usize, 1usize, -1.0), (m - 1, m - 2, 1.0)] {
            let u = [ch.n[0] * sign, ch.n[1] * sign];
            let len = (ch.stations[i].t - ch.stations[j].t).abs() * s;
            let room = if m == 2 {
                2.0 * ARROW_L + 1.0
            } else {
                ARROW_L + 1.0
            };
            if len >= room {
                arrow(c, pts[i], u);
            } else {
                arrow(c, pts[i], [-u[0], -u[1]]);
            }
        }
        for p in &pts[1..m - 1] {
            c.circle_fill(*p, DOT_R);
        }
        for t in &ch.texts {
            c.text(pp(t.at), H, t.angle, &t.text);
            if let Some(l) = t.leader {
                c.line(pp(l[0]), pp(l[1]));
            }
        }
    }

    // Callouts with leaders ending in an arrow on the feature.
    for co in &lo.callouts {
        let at = pp(co.at);
        let start = pp(co.from);
        let centre = pm(co.centre);
        let d = [centre[0] - start[0], centre[1] - start[1]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
        let u = [d[0] / l, d[1] / l];
        let r = dist(co.anchor, co.centre) * s;
        let tip = [centre[0] - u[0] * r, centre[1] - u[1] * r];
        c.line(start, tip);
        arrow(c, tip, u);
        c.text(at, H, 0.0, &co.text);
    }

    // Bend lines, extended past the outline, their notes and tags.
    for t in &lo.tags {
        c.line(pp(t.line[0]), pp(t.line[1]));
        if let Some((note, at, angle)) = &t.note {
            c.text(pp(*at), H, *angle, note);
        }
        let at = pp(t.at);
        let far = pp(t.line[1]);
        let d = [at[0] - far[0], at[1] - far[1]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if l > TAG_R + 0.6 {
            let u = [d[0] / l, d[1] / l];
            c.line(far, add(at, u, -TAG_R));
        }
        c.circle(at, TAG_R);
        c.text_centred(at[0], at[1] - H / 2.0 + 0.2, H - 0.4, &t.nr.to_string());
    }
    c.restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dxf::blank::{Blank, Seg};

    #[test]
    fn chains_run_from_edge_to_edge() {
        // A 50 × 30 plate whose top-right corner has a 2 × 10 notch, with
        // a hole: x chains below (outline, overall) and above (hole).
        let blank = Blank {
            zones: vec![],
            loops: vec![
                vec![
                    Seg::Line {
                        a: [0.0, 0.0],
                        b: [50.0, 0.0],
                    },
                    Seg::Line {
                        a: [50.0, 0.0],
                        b: [50.0, 20.0],
                    },
                    Seg::Line {
                        a: [50.0, 20.0],
                        b: [48.0, 20.0],
                    },
                    Seg::Line {
                        a: [48.0, 20.0],
                        b: [48.0, 30.0],
                    },
                    Seg::Line {
                        a: [48.0, 30.0],
                        b: [0.0, 30.0],
                    },
                    Seg::Line {
                        a: [0.0, 30.0],
                        b: [0.0, 0.0],
                    },
                ],
                vec![Seg::Circle {
                    c: [12.0, 15.0],
                    r: 3.0,
                }],
            ],
            bends: vec![],
            min: [0.0, 0.0],
            max: [50.0, 30.0],
        };
        let f = super::super::features::features(&blank);
        let lo = layout(&f, &[], 1.0, Lang::Nl);
        let kinds: Vec<(&str, f64, f64)> =
            lo.chains.iter().map(|c| (c.kind, c.n[0], c.side)).collect();
        assert!(kinds.contains(&("outline", 1.0, -1.0)), "{kinds:?}");
        assert!(kinds.contains(&("hole", 1.0, 1.0)), "{kinds:?}");
        assert!(kinds.contains(&("overall", 0.0, -1.0)), "{kinds:?}");
        let hole = lo
            .chains
            .iter()
            .find(|c| c.kind == "hole" && c.n[0] == 1.0)
            .unwrap();
        let values: Vec<f64> = hole.texts.iter().map(|t| t.value).collect();
        assert_eq!(values, vec![12.0, 38.0]);
        let outline = lo
            .chains
            .iter()
            .find(|c| c.kind == "outline" && c.n[0] == 1.0)
            .unwrap();
        let values: Vec<f64> = outline.texts.iter().map(|t| t.value).collect();
        assert_eq!(values, vec![48.0, 2.0]);
        // "2" does not fit between 48 and 50 at 1:1: it stands beyond the end.
        assert!(outline.span[1] > 50.0);
        assert!(lo.margin[1] > D && lo.margin[3] > D);
        let mut c = Content::new();
        draw(&lo, &f, &mut c, [50.0, 100.0]);
        assert!(c.as_str().contains(" Tj ET"));
    }
}
