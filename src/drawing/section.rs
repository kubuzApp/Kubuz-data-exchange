//! Sections through the folded part, built from the flat pattern and its
//! bends alone (ISO 128-44). A cutting line perpendicular to a family of
//! parallel bend lines crosses the blank; along it the sheet is a run of
//! flat pieces (the flanges' flat lengths between the bend zones) and
//! bends (the zone's angle, direction and inside radius). Walking that
//! run with the outer skin on top gives the section: tangent arcs of the
//! inside and outside radius between straight flanges, drawn blackened as
//! ISO 128-50 has thin sections. Every flange is dimensioned along itself
//! between the virtual sharp corners on the convex side (where the bend's
//! outer skins would meet) and the free ends, bends that are not 90° get
//! their angle of deviation drawn at that corner, and the section's
//! overall width and height are added when no flange already says them.
//! A flange much longer than the section is tall is interrupted (ISO
//! 128-34) with its true value on the dimension.
//!
//! The same walk, run per zone of the blank, folds the whole part in 3D
//! for the extents in the title block.

use super::dims::{self, overlap, reading, H};
use super::features::{outer_loop, Features};
use super::text;
use crate::dxf::blank::{BendLine, Blank, Seg, P2};
use crate::pdf::{font, Content};
use crate::view::BendDirection;

/// Dimension line distance from the flange, and the angle arc's radius.
const D: f64 = dims::D;
const ANGLE_R: f64 = 10.0;
/// Paper gap of an interrupted flange and the zigzag's half height.
const BREAK_GAP: f64 = 4.0;
const SAME: f64 = super::features::SAME;

#[derive(Clone, Debug)]
pub enum Piece {
    Flat {
        len: f64,
    },
    Bend {
        nr: usize,
        angle: f64,
        up: bool,
        r_in: f64,
    },
}

/// One cut: where it runs on the blank and what the sheet does along it.
#[derive(Clone, Debug)]
pub struct Profile {
    pub letter: char,
    /// Unit cut direction on the blank (the section's x axis).
    pub c: P2,
    /// The bend lines' direction, perpendicular to `c`.
    pub e: P2,
    /// Entry and exit of the cut on the outline (model).
    pub cut: [P2; 2],
    pub pieces: Vec<Piece>,
    /// 1-based bend numbers in the order crossed.
    pub bends: Vec<usize>,
    pub t: f64,
}

#[derive(Clone, Copy, Debug)]
pub enum Elem {
    Line {
        a: P2,
        b: P2,
    },
    /// From `start` degrees about `c`, `sweep` degrees (negative: clockwise).
    Arc {
        c: P2,
        r: f64,
        start: f64,
        sweep: f64,
    },
}

/// One flange in the section: its outer skin from `a` to `b` along `d`,
/// the material on the `m` side.
#[derive(Clone, Debug)]
pub struct FlangeGeo {
    pub piece: usize,
    pub a: P2,
    pub b: P2,
    pub d: P2,
    pub m: P2,
}

/// One bend in the section: the virtual sharp corner on the convex side,
/// the directions before and after, the signed turn.
#[derive(Clone, Debug)]
pub struct CornerGeo {
    pub piece: usize,
    pub v: P2,
    pub d_before: P2,
    pub d_after: P2,
    pub turn: f64,
    pub up: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Geometry {
    pub outer: Vec<Elem>,
    pub inner: Vec<Elem>,
    /// The sheet's two free ends, across the thickness.
    pub ends: Vec<[P2; 2]>,
    pub flanges: Vec<FlangeGeo>,
    pub corners: Vec<CornerGeo>,
    /// Model box of everything.
    pub bbox: [f64; 4],
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
fn rot(v: P2, deg: f64) -> P2 {
    let (s, c) = deg.to_radians().sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
}
fn ang(v: P2) -> f64 {
    v[1].atan2(v[0]).to_degrees()
}
fn unit(v: P2) -> P2 {
    let l = (v[0] * v[0] + v[1] * v[1]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l]
}

/// Where the line `p + t·u` crosses `seg`, as values of t.
fn hits(seg: &Seg, p: P2, u: P2) -> Vec<f64> {
    let n = perp(u);
    match *seg {
        Seg::Line { a, b } => {
            let da = dot([a[0] - p[0], a[1] - p[1]], n);
            let db = dot([b[0] - p[0], b[1] - p[1]], n);
            if (da < 0.0) == (db < 0.0) && da.abs() > 1e-9 && db.abs() > 1e-9 {
                return vec![];
            }
            if (da - db).abs() < 1e-12 {
                return vec![];
            }
            let k = da / (da - db);
            let q = [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k];
            vec![dot([q[0] - p[0], q[1] - p[1]], u)]
        }
        Seg::Arc { .. } | Seg::Circle { .. } => {
            let (c, r, start, end, full) = match *seg {
                Seg::Arc { c, r, start, end } => (c, r, start, end, false),
                Seg::Circle { c, r } => (c, r, 0.0, 360.0, true),
                Seg::Line { .. } => unreachable!(),
            };
            let dc = dot([c[0] - p[0], c[1] - p[1]], n);
            if dc.abs() > r {
                return vec![];
            }
            let h = (r * r - dc * dc).max(0.0).sqrt();
            let tc = dot([c[0] - p[0], c[1] - p[1]], u);
            let mut v = Vec::new();
            for t in [tc - h, tc + h] {
                let q = add(p, u, t);
                let a = ang([q[0] - c[0], q[1] - c[1]]);
                let sweep = (end - start).rem_euclid(360.0);
                let sweep = if sweep < 1e-9 { 360.0 } else { sweep };
                if full || (a - start).rem_euclid(360.0) <= sweep + 1e-6 {
                    v.push(t);
                }
            }
            v
        }
    }
}

/// Unit direction of a bend line, turned to point up (or right).
fn bend_dir(b: &BendLine) -> P2 {
    let u = unit([b.b[0] - b.a[0], b.b[1] - b.a[1]]);
    if u[1] < -1e-9 || (u[1].abs() <= 1e-9 && u[0] < 0.0) {
        [-u[0], -u[1]]
    } else {
        u
    }
}

/// Does the paper segment `a`–`b` cross the box?
use super::dims::crosses_box;

/// Choose the cuts: one per direction of bend lines, through as many of
/// them as one line can cross, away from holes and from the flat
/// pattern's texts; more cuts for the bends the first one missed.
pub fn profiles(blank: &Blank, f: &Features, lo: &dims::Layout, s: f64, t: f64) -> Vec<Profile> {
    let mut out = Vec::new();
    let bends = &blank.bends;
    let mut done = vec![false; bends.len()];
    let outer = outer_loop(&f.loops);
    let centre = [blank.max[0] / 2.0, blank.max[1] / 2.0];
    let mut letter = b'A';
    loop {
        // The direction with the most bends still to cut.
        let mut best: Option<(P2, usize)> = None;
        for (i, b) in bends.iter().enumerate() {
            if done[i] {
                continue;
            }
            let e = bend_dir(b);
            let count = bends
                .iter()
                .enumerate()
                .filter(|(j, bj)| !done[*j] && dot(bend_dir(bj), e) > 1.0 - 1e-6)
                .count();
            if best.is_none_or(|(_, c)| count > c) {
                best = Some((e, count));
            }
        }
        let Some((e, _)) = best else { break };
        let c = reading(perp(e));
        let members: Vec<usize> = (0..bends.len())
            .filter(|&j| !done[j] && dot(bend_dir(&bends[j]), e) > 1.0 - 1e-6)
            .collect();
        // Intervals along e where each member is crossed.
        let mut cuts: Vec<f64> = Vec::new();
        for &j in &members {
            cuts.push(dot(bends[j].a, e));
            cuts.push(dot(bends[j].b, e));
        }
        cuts.sort_by(|a, b| a.total_cmp(b));
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        // Offsets to try: within each interval, from its middle outward.
        let mut candidates: Vec<(usize, f64, f64)> = Vec::new();
        for w in cuts.windows(2) {
            let (lo_, hi_) = (w[0], w[1]);
            if hi_ - lo_ < 1.0 {
                continue;
            }
            let mid = (lo_ + hi_) / 2.0;
            let crossed = members
                .iter()
                .filter(|&&j| {
                    let (p, q) = (dot(bends[j].a, e), dot(bends[j].b, e));
                    mid > p.min(q) + 1e-6 && mid < p.max(q) - 1e-6
                })
                .count();
            if crossed == 0 {
                continue;
            }
            for k in [0.0, 0.15, -0.15, 0.3, -0.3, 0.42, -0.42] {
                let o = mid + k * (hi_ - lo_);
                candidates.push((crossed, (o - dot(centre, e)).abs(), o));
            }
        }
        if candidates.is_empty() {
            for &j in &members {
                done[j] = true;
            }
            continue;
        }
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.total_cmp(&b.1)));
        let mut chosen: Option<(f64, Vec<usize>)> = None;
        let mut fallback: Option<(f64, Vec<usize>)> = None;
        for &(_, _, o) in &candidates {
            let p0 = add([0.0, 0.0], e, o);
            // Entry and exit on the outer loop.
            let mut ts: Vec<f64> = f.loops[outer]
                .iter()
                .flat_map(|sg| hits(sg, p0, c))
                .collect();
            if ts.len() < 2 {
                continue;
            }
            ts.sort_by(|a, b| a.total_cmp(b));
            let (t0, t1) = (ts[0], ts[ts.len() - 1]);
            // Which bends it crosses: only members, else the cut is unclear.
            let mut crossed: Vec<(f64, usize)> = Vec::new();
            let mut foreign = false;
            for (j, b) in bends.iter().enumerate() {
                let (pa, pb) = (dot(b.a, e), dot(b.b, e));
                if o <= pa.min(pb) + 1e-6 || o >= pa.max(pb) - 1e-6 {
                    continue;
                }
                if !members.contains(&j) {
                    foreign = true;
                    break;
                }
                // The crossing point along c.
                let k = (o - pa) / (pb - pa);
                let q = [
                    b.a[0] + (b.b[0] - b.a[0]) * k,
                    b.a[1] + (b.b[1] - b.a[1]) * k,
                ];
                crossed.push((dot(q, c), j));
            }
            if foreign || crossed.is_empty() {
                continue;
            }
            crossed.sort_by(|a, b| a.0.total_cmp(&b.0));
            let ids: Vec<usize> = crossed.iter().map(|c| c.1).collect();
            if fallback.is_none() {
                fallback = Some((o, ids.clone()));
            }
            // Holes and cutouts on the way, and texts on the paper.
            let hole = f
                .loops
                .iter()
                .enumerate()
                .filter(|(li, _)| *li != outer)
                .any(|(_, lp)| lp.iter().any(|sg| !hits(sg, p0, c).is_empty()));
            if hole {
                continue;
            }
            let a = [p0[0] * s, p0[1] * s];
            let pa = add(a, c, t0 * s - 40.0);
            let pb = add(a, c, t1 * s + 40.0);
            if lo.boxes.iter().any(|b| crosses_box(pa, pb, *b)) {
                continue;
            }
            chosen = Some((o, ids));
            break;
        }
        let Some((o, ids)) = chosen.or(fallback) else {
            for &j in &members {
                done[j] = true;
            }
            continue;
        };
        let p0 = add([0.0, 0.0], e, o);
        let mut ts: Vec<f64> = f.loops[outer]
            .iter()
            .flat_map(|sg| hits(sg, p0, c))
            .collect();
        ts.sort_by(|a, b| a.total_cmp(b));
        let (t0, t1) = (ts[0], ts[ts.len() - 1]);
        let mut pieces = Vec::new();
        let mut at = t0;
        let mut nrs = Vec::new();
        for &j in &ids {
            let b = &bends[j];
            let (pa, pb) = (dot(b.a, e), dot(b.b, e));
            let k = (o - pa) / (pb - pa);
            let q = [
                b.a[0] + (b.b[0] - b.a[0]) * k,
                b.a[1] + (b.b[1] - b.a[1]) * k,
            ];
            let tc = dot(q, c);
            let half = b.allowance / 2.0;
            pieces.push(Piece::Flat {
                len: (tc - half - at).max(0.0),
            });
            pieces.push(Piece::Bend {
                nr: j + 1,
                angle: b.angle_deg,
                up: b.direction == BendDirection::Up,
                r_in: b.inside_radius,
            });
            at = tc + half;
            nrs.push(j + 1);
            done[j] = true;
        }
        pieces.push(Piece::Flat {
            len: (t1 - at).max(0.0),
        });
        out.push(Profile {
            letter: letter as char,
            c,
            e,
            cut: [add(p0, c, t0), add(p0, c, t1)],
            pieces,
            bends: nrs,
            t,
        });
        letter += 1;
        if letter > b'Z' {
            break;
        }
    }
    out
}

/// Walk the pieces with the outer skin on top, starting at the origin
/// heading right; `shown` replaces the flat lengths (an interrupted
/// flange) when given.
pub fn walk(pieces: &[Piece], t: f64, shown: Option<&[f64]>) -> Geometry {
    let mut g = Geometry::default();
    let mut p = [0.0, 0.0];
    let mut d = [1.0, 0.0];
    // Material side, clockwise from d.
    let mut m = [0.0, -1.0];
    let sense = -1.0;
    let mut bbox = [0.0, -t, 0.0, 0.0];
    let grow = |q: P2, bbox: &mut [f64; 4]| {
        bbox[0] = bbox[0].min(q[0]);
        bbox[1] = bbox[1].min(q[1]);
        bbox[2] = bbox[2].max(q[0]);
        bbox[3] = bbox[3].max(q[1]);
    };
    g.ends.push([p, add(p, m, t)]);
    // Per flat piece, the outer skin line; the virtual corners come from
    // consecutive pieces' convex skin lines.
    let mut last_flat: Option<(usize, P2, P2, P2)> = None;
    for (i, piece) in pieces.iter().enumerate() {
        match *piece {
            Piece::Flat { len } => {
                let len = shown.map_or(len, |v| v[i]);
                let q = add(p, d, len);
                g.outer.push(Elem::Line { a: p, b: q });
                g.inner.push(Elem::Line {
                    a: add(p, m, t),
                    b: add(q, m, t),
                });
                grow(p, &mut bbox);
                grow(q, &mut bbox);
                grow(add(p, m, t), &mut bbox);
                grow(add(q, m, t), &mut bbox);
                g.flanges.push(FlangeGeo {
                    piece: i,
                    a: p,
                    b: q,
                    d,
                    m,
                });
                last_flat = Some((i, p, d, m));
                p = q;
            }
            Piece::Bend {
                angle, up, r_in, ..
            } => {
                let turn = if up { -sense * angle } else { sense * angle };
                let (r_outer_skin, r_inner_skin, centre) = if up {
                    (r_in, r_in + t, add(p, m, -r_in))
                } else {
                    (r_in + t, r_in, add(p, m, r_in + t))
                };
                let start = ang([p[0] - centre[0], p[1] - centre[1]]);
                g.outer.push(Elem::Arc {
                    c: centre,
                    r: r_outer_skin,
                    start,
                    sweep: turn,
                });
                g.inner.push(Elem::Arc {
                    c: centre,
                    r: r_inner_skin,
                    start,
                    sweep: turn,
                });
                // The arc's extreme points.
                for k in 0..=8 {
                    let a = start + turn * k as f64 / 8.0;
                    for r in [r_outer_skin, r_inner_skin] {
                        grow(
                            add(centre, [a.to_radians().cos(), a.to_radians().sin()], r),
                            &mut bbox,
                        );
                    }
                }
                let d_before = d;
                d = rot(d, turn);
                m = rot(m, turn);
                p = if up {
                    add(centre, m, r_in)
                } else {
                    add(centre, m, -(r_in + t))
                };
                // The virtual corner: the convex skins' lines meet there.
                if let Some((_, pf, df, mf)) = last_flat {
                    let off = if up { t } else { 0.0 };
                    let la = add(pf, mf, off);
                    let lb = add(p, m, off);
                    // Intersect la + x·df with lb + y·d.
                    let den = df[0] * d[1] - df[1] * d[0];
                    let v = if den.abs() < 1e-9 {
                        lb
                    } else {
                        let x = ((lb[0] - la[0]) * d[1] - (lb[1] - la[1]) * d[0]) / den;
                        add(la, df, x)
                    };
                    g.corners.push(CornerGeo {
                        piece: i,
                        v,
                        d_before,
                        d_after: d,
                        turn,
                        up,
                    });
                }
            }
        }
    }
    g.ends.push([p, add(p, m, t)]);
    // Turn the whole section so the longest flange lies along +x with its
    // outer skin on top, as the flat pattern shows it.
    let longest = g
        .flanges
        .iter()
        .max_by(|a, b| {
            dims::dist(a.a, a.b)
                .total_cmp(&dims::dist(b.a, b.b))
                .then(b.piece.cmp(&a.piece))
        })
        .map(|f| f.d);
    if let Some(d0) = longest {
        let phi = -ang(d0);
        if phi.abs() > 1e-9 {
            let r = |q: P2| rot(q, phi);
            for e in g.outer.iter_mut().chain(g.inner.iter_mut()) {
                match e {
                    Elem::Line { a, b } => {
                        *a = r(*a);
                        *b = r(*b);
                    }
                    Elem::Arc { c, start, .. } => {
                        *c = r(*c);
                        *start += phi;
                    }
                }
            }
            for e in &mut g.ends {
                e[0] = r(e[0]);
                e[1] = r(e[1]);
            }
            for f in &mut g.flanges {
                f.a = r(f.a);
                f.b = r(f.b);
                f.d = r(f.d);
                f.m = r(f.m);
            }
            for c in &mut g.corners {
                c.v = r(c.v);
                c.d_before = r(c.d_before);
                c.d_after = r(c.d_after);
            }
            bbox = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            let mut pts: Vec<P2> = Vec::new();
            for e in g.outer.iter().chain(g.inner.iter()) {
                sample_elem(e, 1.0, &mut pts);
            }
            for q in pts {
                grow(q, &mut bbox);
            }
        }
    }
    g.bbox = bbox;
    g
}

/// A dimension of the section, in model coordinates of the shown
/// geometry, with its true value.
#[derive(Clone, Debug)]
pub struct SecDim {
    /// `flange`, `width` or `height`.
    pub kind: &'static str,
    pub piece: Option<usize>,
    pub p: P2,
    pub q: P2,
    pub v: P2,
    pub off: f64,
    pub value: f64,
    pub text: String,
    pub bx: [f64; 4],
}

#[derive(Clone, Debug)]
pub struct SecAngle {
    pub nr: usize,
    pub v: P2,
    /// Paper radius of the arc.
    pub radius: f64,
    pub start: f64,
    pub sweep: f64,
    pub value: f64,
    pub text: String,
    pub at: P2,
    pub bx: [f64; 4],
}

#[derive(Clone, Debug)]
pub struct SectionPlan {
    pub s: f64,
    pub t: f64,
    pub geo: Geometry,
    /// Shown flat length per piece (flat pieces only), and which pieces
    /// are interrupted with their true length.
    pub shown: Vec<f64>,
    pub breaks: Vec<(usize, f64)>,
    pub dims: Vec<SecDim>,
    pub angles: Vec<SecAngle>,
    /// Paper box of everything, relative to the walk's origin.
    pub bbox: [f64; 4],
    pub boxes: Vec<[f64; 4]>,
    /// True extents of the section (model).
    pub width: f64,
    pub height: f64,
}

fn sample_elem(e: &Elem, s: f64, out: &mut Vec<P2>) {
    match *e {
        Elem::Line { a, b } => dims::sample_line([a[0] * s, a[1] * s], [b[0] * s, b[1] * s], out),
        Elem::Arc { c, r, start, sweep } => {
            let n = ((r * s * sweep.abs().to_radians()).ceil() as usize).max(3);
            for i in 0..=n {
                let a = (start + sweep * i as f64 / n as f64).to_radians();
                out.push([(c[0] + r * a.cos()) * s, (c[1] + r * a.sin()) * s]);
            }
        }
    }
}

/// Lay one section out at scale `s`.
pub fn plan(p: &Profile, s: f64) -> SectionPlan {
    let true_geo = walk(&p.pieces, p.t, None);
    let width = true_geo.bbox[2] - true_geo.bbox[0];
    let height = true_geo.bbox[3] - true_geo.bbox[1];
    // Interrupt a flat piece much longer than the section is tall.
    let mut shown: Vec<f64> = p
        .pieces
        .iter()
        .map(|pc| match pc {
            Piece::Flat { len } => *len,
            _ => 0.0,
        })
        .collect();
    let mut breaks = Vec::new();
    let keep = (25.0 / s).max(2.5 * height);
    for (i, pc) in p.pieces.iter().enumerate() {
        if let Piece::Flat { len } = pc {
            if *len - keep > 15.0 / s {
                shown[i] = keep;
                breaks.push((i, *len));
            }
        }
    }
    let geo = walk(&p.pieces, p.t, Some(&shown));
    let mut samples: Vec<P2> = Vec::new();
    for e in geo.outer.iter().chain(geo.inner.iter()) {
        sample_elem(e, s, &mut samples);
    }
    let mut boxes: Vec<[f64; 4]> = Vec::new();
    let mut dims_v: Vec<SecDim> = Vec::new();
    let mut values: Vec<f64> = Vec::new();

    // The angle between the flanges at bends that are not 90°: an arc
    // from one flange to the other at the virtual corner, no larger than
    // the flanges beside it, the value where it meets no other text.
    let mut angles = Vec::new();
    for c in &geo.corners {
        let value = 180.0 - c.turn.abs();
        if (c.turn.abs() - 90.0).abs() < 0.05 {
            continue;
        }
        let nr = match p.pieces[c.piece] {
            Piece::Bend { nr, .. } => nr,
            _ => 0,
        };
        let shortest = geo
            .flanges
            .iter()
            .filter(|f| f.piece + 1 == c.piece || f.piece == c.piece + 1)
            .map(|f| dims::dist(f.a, f.b) * s)
            .fold(f64::INFINITY, f64::min);
        let radius = (0.7 * shortest).clamp(5.0, ANGLE_R);
        // From the flange before (back along it) to the flange after.
        let start = ang(c.d_before) + 180.0;
        let sweep = c.turn - 180.0 * c.turn.signum();
        let mid = (start + sweep / 2.0).to_radians();
        let txt = text::deg(value);
        let tw = font::width(&txt, H);
        let mut chosen = None;
        // The arc itself, sampled closely, keeps the value off it.
        let arc: Vec<P2> = (0..=36)
            .map(|k| {
                let a = (start + sweep * k as f64 / 36.0).to_radians();
                [c.v[0] * s + radius * a.cos(), c.v[1] * s + radius * a.sin()]
            })
            .collect();
        for extra in [4.0, 7.0, 10.0, 14.0, 20.0] {
            let r = (radius + extra) / s;
            let centre_t = [(c.v[0] + r * mid.cos()) * s, (c.v[1] + r * mid.sin()) * s];
            let at = [centre_t[0] - tw / 2.0, centre_t[1] - H / 2.0];
            let bx = [at[0] - 0.5, at[1] - 0.5, at[0] + tw + 0.5, at[1] + H + 0.5];
            let far = (radius + 30.0) / s;
            let hit = boxes.iter().any(|b| overlap(*b, bx))
                || samples.iter().any(|q| inside(bx, *q))
                || arc.iter().any(|q| inside(bx, *q))
                || [[-c.d_before[0], -c.d_before[1]], c.d_after]
                    .iter()
                    .any(|d| {
                        crosses_box(
                            [c.v[0] * s, c.v[1] * s],
                            [(c.v[0] + d[0] * far) * s, (c.v[1] + d[1] * far) * s],
                            bx,
                        )
                    });
            if !hit || extra >= 20.0 {
                chosen = Some((at, bx));
                break;
            }
        }
        let (at, bx) = chosen.unwrap();
        boxes.push(bx);
        // The arc and the arrowheads are obstacles for the flange dimensions.
        for k in 0..=12 {
            let a = (start + c.turn * k as f64 / 12.0).to_radians();
            samples.push([
                c.v[0] * s + (radius + 1.0) * a.cos(),
                c.v[1] * s + (radius + 1.0) * a.sin(),
            ]);
            samples.push([
                c.v[0] * s + (radius - 1.0) * a.cos(),
                c.v[1] * s + (radius - 1.0) * a.sin(),
            ]);
        }
        angles.push(SecAngle {
            nr,
            v: c.v,
            radius,
            start,
            sweep,
            value,
            text: txt,
            at,
            bx,
        });
    }

    // Flange lengths between virtual corners and free ends.
    fn corner_at(piece: usize, g: &Geometry) -> Option<&CornerGeo> {
        g.corners.iter().find(|c| c.piece == piece)
    }
    for (fi, fl) in geo.flanges.iter().enumerate() {
        let tf = &true_geo.flanges[fi];
        let (before, after) = (
            if fl.piece > 0 {
                corner_at(fl.piece - 1, &geo)
            } else {
                None
            },
            corner_at(fl.piece + 1, &geo),
        );
        let (tb, ta) = (
            if fl.piece > 0 {
                corner_at(fl.piece - 1, &true_geo)
            } else {
                None
            },
            corner_at(fl.piece + 1, &true_geo),
        );
        let start = before.map_or(fl.a, |c| c.v);
        let end = after.map_or(fl.b, |c| c.v);
        let ts = tb.map_or(tf.a, |c| c.v);
        let te = ta.map_or(tf.b, |c| c.v);
        let value = (dot(te, tf.d) - dot(ts, tf.d)).abs();
        if value < SAME {
            continue;
        }
        // Side: the convex side of the neighbouring bends, else outside.
        let mut score = 0i32;
        for c in [before, after].into_iter().flatten() {
            score += if c.up { 1 } else { -1 };
        }
        let outer_side = [-fl.m[0], -fl.m[1]];
        let first = if score > 0 { fl.m } else { outer_side };
        let second = if score > 0 { outer_side } else { fl.m };
        let txt = text::mm(value);
        let tw = font::width(&txt, H);
        let mut placed = None;
        'side: for v in [first, second] {
            for off in [D, D + 8.0, D + 16.0, D + 24.0, D + 32.0] {
                let (bx, lines) = linear_box(start, end, v, off, s, tw);
                let band = band_box(start, end, v, off, s, tw);
                let hit = samples
                    .iter()
                    .any(|q| inside(band, *q) && !on_segment(*q, start, end, v, s))
                    || boxes.iter().any(|b| overlap(*b, bx))
                    || lines
                        .iter()
                        .any(|l| boxes.iter().any(|b| crosses_box(l[0], l[1], *b)));
                if !hit {
                    placed = Some((v, off, bx));
                    break 'side;
                }
            }
        }
        let (v, off, bx) = placed.unwrap_or_else(|| {
            let (bx, _) = linear_box(start, end, first, D, s, tw);
            (first, D, bx)
        });
        boxes.push(bx);
        values.push(value);
        dims_v.push(SecDim {
            kind: "flange",
            piece: Some(fl.piece),
            p: start,
            q: end,
            v,
            off,
            value,
            text: txt,
            bx,
        });
    }

    // Overall width and height when no flange says them: outside all
    // that was placed.
    let mut ext = [
        geo.bbox[0] * s,
        geo.bbox[1] * s,
        geo.bbox[2] * s,
        geo.bbox[3] * s,
    ];
    let grow_all = |ext: &mut [f64; 4], boxes: &[[f64; 4]], dims_v: &[SecDim]| {
        for b in boxes {
            ext[0] = ext[0].min(b[0]);
            ext[1] = ext[1].min(b[1]);
            ext[2] = ext[2].max(b[2]);
            ext[3] = ext[3].max(b[3]);
        }
        for d in dims_v {
            let (_, lines) = linear_box(d.p, d.q, d.v, d.off, s, 0.0);
            for l in &lines {
                for q in l {
                    ext[0] = ext[0].min(q[0]);
                    ext[1] = ext[1].min(q[1]);
                    ext[2] = ext[2].max(q[0]);
                    ext[3] = ext[3].max(q[1]);
                }
            }
        }
    };
    grow_all(&mut ext, &boxes, &dims_v);
    let shown_w = geo.bbox[2] - geo.bbox[0];
    let shown_h = geo.bbox[3] - geo.bbox[1];
    for (kind, value, shown_v) in [("width", width, shown_w), ("height", height, shown_h)] {
        if values.iter().any(|v| (v - value).abs() < SAME) || shown_v < SAME {
            continue;
        }
        let (p_, q_, v) = if kind == "width" {
            let lo = geo.bbox[0];
            let hi = geo.bbox[2];
            let y = geo.bbox[1];
            ([lo, y], [hi, y], [0.0, -1.0])
        } else {
            let lo = geo.bbox[1];
            let hi = geo.bbox[3];
            let x = geo.bbox[2];
            ([x, lo], [x, hi], [1.0, 0.0])
        };
        // Offset so the line sits D beyond everything placed on that side.
        let far_now = if kind == "width" { -ext[1] } else { ext[2] };
        let off = far_now - dot([p_[0] * s, p_[1] * s], v) + D;
        let txt = text::mm(value);
        let tw = font::width(&txt, H);
        let (bx, _) = linear_box(p_, q_, v, off, s, tw);
        boxes.push(bx);
        values.push(value);
        dims_v.push(SecDim {
            kind,
            piece: None,
            p: p_,
            q: q_,
            v,
            off,
            value,
            text: txt,
            bx,
        });
        grow_all(&mut ext, &boxes, &dims_v);
    }
    // Angle arcs reach ANGLE_R past their corner.
    for a in &angles {
        ext[0] = ext[0].min(a.v[0] * s - a.radius - 3.0);
        ext[1] = ext[1].min(a.v[1] * s - a.radius - 3.0);
        ext[2] = ext[2].max(a.v[0] * s + a.radius + 3.0);
        ext[3] = ext[3].max(a.v[1] * s + a.radius + 3.0);
    }
    SectionPlan {
        s,
        t: p.t,
        geo,
        shown,
        breaks,
        dims: dims_v,
        angles,
        bbox: [ext[0] - 1.0, ext[1] - 1.0, ext[2] + 1.0, ext[3] + 1.0],
        boxes,
        width,
        height,
    }
}

fn inside(b: [f64; 4], p: P2) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}

/// Is paper point `q` on the dimensioned flange itself (the band between
/// the extension points, which the extension lines leave)?
fn on_segment(q: P2, p: P2, r: P2, v: P2, s: f64) -> bool {
    let n = unit([r[0] - p[0], r[1] - p[1]]);
    let t = dot(q, n);
    let (t0, t1) = (dot(p, n) * s, dot(r, n) * s);
    let h = dot(q, v) - dot(p, v).max(dot(r, v)) * s;
    t >= t0.min(t1) - 0.3 && t <= t0.max(t1) + 0.3 && h < 1.0
}

/// The band a dimension occupies: from the features out to past the text.
fn band_box(p: P2, q: P2, v: P2, off: f64, s: f64, tw: f64) -> [f64; 4] {
    let pp = [p[0] * s, p[1] * s];
    let qq = [q[0] * s, q[1] * s];
    let far = dot(pp, v).max(dot(qq, v));
    let n = unit([q[0] - p[0], q[1] - p[1]]);
    let len = (dot(qq, n) - dot(pp, n)).abs();
    let extra = if tw + 2.0 > len { tw + 4.0 } else { 0.0 };
    let base = add([0.0, 0.0], v, far);
    let pts = [
        add(add(pp, v, far - dot(pp, v) + 1.0), n, -extra),
        add(add(qq, v, far - dot(qq, v) + 1.0), n, extra),
        add(
            add(base, v, off + H + 2.0),
            n,
            dot(pp, n).min(dot(qq, n)) - extra,
        ),
        add(
            add(base, v, off + H + 2.0),
            n,
            dot(pp, n).max(dot(qq, n)) + extra,
        ),
    ];
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for c in pts {
        b[0] = b[0].min(c[0]);
        b[1] = b[1].min(c[1]);
        b[2] = b[2].max(c[0]);
        b[3] = b[3].max(c[1]);
    }
    b
}

/// The text box and the lines of a linear dimension (paper).
fn linear_box(p: P2, q: P2, v: P2, off: f64, s: f64, tw: f64) -> ([f64; 4], Vec<[P2; 2]>) {
    let pp = [p[0] * s, p[1] * s];
    let qq = [q[0] * s, q[1] * s];
    let far = dot(pp, v).max(dot(qq, v)) + off;
    let a = add(pp, v, far - dot(pp, v));
    let b = add(qq, v, far - dot(qq, v));
    let n = unit([b[0] - a[0], b[1] - a[1]]);
    let l = dims::dist(a, b);
    let ur = reading(n);
    let up = perp(ur);
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let at = if tw + 2.0 <= l {
        add(add(mid, ur, -tw / 2.0), up, 0.8)
    } else {
        let end = if dot(n, ur) > 0.0 { b } else { a };
        add(add(end, ur, dims::ARROW_L + 1.5), up, 0.8)
    };
    let corners = [
        at,
        add(at, ur, tw),
        add(add(at, ur, tw), up, H),
        add(at, up, H),
    ];
    let mut bx = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for c in corners {
        bx[0] = bx[0].min(c[0]);
        bx[1] = bx[1].min(c[1]);
        bx[2] = bx[2].max(c[0]);
        bx[3] = bx[3].max(c[1]);
    }
    let bx = [bx[0] - 0.4, bx[1] - 0.4, bx[2] + 0.4, bx[3] + 0.4];
    let lines = vec![
        [add(pp, v, dims::GAP), add(a, v, dims::EXT)],
        [add(qq, v, dims::GAP), add(b, v, dims::EXT)],
        [a, b],
    ];
    (bx, lines)
}

/// Draw the section with the walk's origin at paper `o`.
pub fn draw(p: &SectionPlan, c: &mut Content, o: P2) {
    let s = p.s;
    let pm = |q: P2| [o[0] + q[0] * s, o[1] + q[1] * s];
    c.save();
    // Outer skin forward, inner skin back.
    let mut breaks_at: Vec<(usize, f64)> = Vec::new();
    for (i, _) in &p.breaks {
        breaks_at.push((*i, p.shown[*i]));
    }
    // The sheet as it is: both skins and the ends, one closed path per
    // unbroken run.
    c.width(if p.t * s >= 1.0 { 0.35 } else { 0.25 });
    let mut path_outer: Vec<Vec<Elem>> = vec![vec![]];
    let mut path_inner: Vec<Vec<Elem>> = vec![vec![]];
    let mut ends: Vec<[P2; 2]> = vec![p.geo.ends[0]];
    let mut zigzags: Vec<([P2; 2], P2)> = Vec::new();
    for (k, e) in p.geo.outer.iter().enumerate() {
        let inner = p.geo.inner[k];
        // Which piece is this element: flat and bend alternate from the
        // first flat.
        let broken = breaks_at.iter().any(|(i, _)| *i == k);
        if let (true, Elem::Line { a, b }, Elem::Line { a: ia, b: ib }) = (broken, *e, inner) {
            let d = unit([b[0] - a[0], b[1] - a[1]]);
            let len = dims::dist(a, b);
            let gap = BREAK_GAP / s;
            let m1 = add(a, d, (len - gap) / 2.0);
            let m2 = add(a, d, (len + gap) / 2.0);
            let im1 = add(ia, d, (len - gap) / 2.0);
            let im2 = add(ia, d, (len + gap) / 2.0);
            path_outer.last_mut().unwrap().push(Elem::Line { a, b: m1 });
            path_inner
                .last_mut()
                .unwrap()
                .push(Elem::Line { a: ia, b: im1 });
            ends.push([m1, im1]);
            zigzags.push(([m1, im1], d));
            path_outer.push(vec![Elem::Line { a: m2, b }]);
            path_inner.push(vec![Elem::Line { a: im2, b: ib }]);
            ends.push([m2, im2]);
            zigzags.push(([m2, im2], d));
        } else {
            path_outer.last_mut().unwrap().push(*e);
            path_inner.last_mut().unwrap().push(inner);
        }
    }
    ends.push(p.geo.ends[1]);
    for (run, (po, pi)) in path_outer.iter().zip(path_inner.iter()).enumerate() {
        let start = ends[2 * run][0];
        c.move_to(pm(start));
        for e in po {
            trace(c, e, &pm, s, false);
        }
        let end_in = ends[2 * run + 1][1];
        c.line_to(pm(end_in));
        for e in pi.iter().rev() {
            trace(c, e, &pm, s, true);
        }
        c.close();
        c.stroke();
    }
    // Break lines: a zigzag across the sheet and a little beyond.
    c.width(0.25);
    for (across, d) in &zigzags {
        let a = pm(across[0]);
        let b = pm(across[1]);
        let n = unit([b[0] - a[0], b[1] - a[1]]);
        let l = dims::dist(a, b);
        let dd = [d[0], d[1]];
        let pts = [
            add(a, n, -1.5),
            add(a, n, l * 0.35),
            add(add(a, n, l * 0.5), dd, 0.9),
            add(add(a, n, l * 0.5), dd, -0.9),
            add(a, n, l * 0.65),
            add(b, n, 1.5),
        ];
        c.polyline(&pts, false);
    }

    // Dimensions.
    c.width(dims::NARROW);
    for d in &p.dims {
        let pp = pm(d.p);
        let qq = pm(d.q);
        dims::linear(c, pp, qq, d.v, d.off, &d.text);
    }
    for a in &p.angles {
        let v = pm(a.v);
        // The arc runs from one flange to the other.
        let dir = [a.start.to_radians().cos(), a.start.to_radians().sin()];
        let p0 = add(v, dir, a.radius);
        c.move_to(p0);
        c.arc_sweep(v, a.radius, a.start, a.sweep);
        c.stroke();
        let end = a.start + a.sweep;
        let u_end = rot(
            [end.to_radians().cos(), end.to_radians().sin()],
            if a.sweep > 0.0 { 90.0 } else { -90.0 },
        );
        dims::arrow(
            c,
            add(
                v,
                [end.to_radians().cos(), end.to_radians().sin()],
                a.radius,
            ),
            u_end,
        );
        let u0 = rot(dir, if a.sweep > 0.0 { -90.0 } else { 90.0 });
        dims::arrow(c, p0, u0);
        c.text([o[0] + a.at[0], o[1] + a.at[1]], H, 0.0, &a.text);
    }
    c.restore();
}

fn trace(c: &mut Content, e: &Elem, pm: &impl Fn(P2) -> P2, s: f64, reverse: bool) {
    match *e {
        Elem::Line { a, b } => c.line_to(pm(if reverse { a } else { b })),
        Elem::Arc {
            c: cc,
            r,
            start,
            sweep,
        } => {
            if reverse {
                c.arc_sweep(pm(cc), r * s, start + sweep, -sweep);
            } else {
                c.arc_sweep(pm(cc), r * s, start, sweep);
            }
        }
    }
}

/// Mark the cut of `p` on the flat pattern (paper origin `o`, scale `s`):
/// no line across the part, only an arrow in the viewing direction at the
/// entry end, outside the dimensions, with the letter beside it. `ext` is
/// the paper box (relative to the model origin) the dimensions occupy.
/// Returns the paper boxes of the letter and arrow (relative to the model
/// origin).
/// The cut marker: letter height, arrow shaft length (mm on paper).
const CUT_LETTER_H: f64 = 4.0;
const CUT_SHAFT: f64 = 6.0;

pub fn draw_cut(
    p: &Profile,
    c: &mut Content,
    o: P2,
    s: f64,
    ext: [f64; 4],
    avoid: &[[f64; 4]],
    lines: &[[P2; 2]],
) -> Vec<[f64; 4]> {
    let (tip, tail, at, bx) = cut_marker(p, s, ext, avoid, lines);
    let u = unit([p.cut[1][0] - p.cut[0][0], p.cut[1][1] - p.cut[0][1]]);
    let vd = perp(u);
    let letter = p.letter.to_string();
    c.save();
    c.width(dims::WIDE);
    c.line(
        [o[0] + tail[0], o[1] + tail[1]],
        [o[0] + tip[0], o[1] + tip[1]],
    );
    big_arrow(c, [o[0] + tip[0], o[1] + tip[1]], vd);
    c.text([o[0] + at[0], o[1] + at[1]], CUT_LETTER_H, 0.0, &letter);
    c.restore();
    vec![bx]
}

/// Where the cut's marker goes: the arrow's tip and tail, the letter's
/// start and the box of all of it (paper, relative to the model origin).
pub fn cut_marker(
    p: &Profile,
    s: f64,
    ext: [f64; 4],
    avoid: &[[f64; 4]],
    lines: &[[P2; 2]],
) -> (P2, P2, P2, [f64; 4]) {
    let a = [p.cut[0][0] * s, p.cut[0][1] * s];
    let b = [p.cut[1][0] * s, p.cut[1][1] * s];
    let u = unit([b[0] - a[0], b[1] - a[1]]);
    // Viewing direction: the section's x is the cut direction and its y
    // the outer skin's normal, so the viewer stands at -perp(c).
    let vd = perp(u);
    let letter = p.letter.to_string();
    let tw = font::width(&letter, CUT_LETTER_H);
    // The arrow sits at the entry end just outside the dimensions, or
    // further out, or at the exit end, wherever it meets no text.
    let candidates = {
        let mut v = Vec::new();
        for (end, sign) in [(a, -1.0), (b, 1.0)] {
            let mut t = 0.0f64;
            while inside(ext, add(end, u, sign * (t + 1.0))) && t < 600.0 {
                t += 1.0;
            }
            for extra in [3.0, 9.0, 15.0, 21.0, 28.0, 36.0, 45.0, 55.0] {
                v.push(add(end, u, sign * (t + extra)));
            }
        }
        v
    };
    let place = |tip: P2| -> (P2, P2, [f64; 4]) {
        let tail = add(tip, vd, -CUT_SHAFT);
        // The letter upright, centred a little past the tail.
        let centre = add(tail, vd, -(2.5 + tw.max(CUT_LETTER_H) / 2.0));
        let at = [centre[0] - tw / 2.0, centre[1] - CUT_LETTER_H / 2.0];
        let lo = [
            tip[0].min(tail[0]).min(at[0]) - 1.0,
            tip[1].min(tail[1]).min(at[1]) - 1.0,
        ];
        let hi = [
            tip[0].max(tail[0]).max(at[0] + tw) + 1.0,
            tip[1].max(tail[1]).max(at[1] + CUT_LETTER_H) + 1.0,
        ];
        (tail, at, [lo[0], lo[1], hi[0], hi[1]])
    };
    let mut chosen = place(candidates[0]);
    let mut tip = candidates[0];
    for cand in &candidates {
        let pl = place(*cand);
        if !avoid.iter().any(|bx| overlap(*bx, pl.2))
            && !lines.iter().any(|l| crosses_box(l[0], l[1], pl.2))
        {
            chosen = pl;
            tip = *cand;
            break;
        }
    }
    let (tail, at, bx) = chosen;
    (tip, tail, at, bx)
}

fn big_arrow(c: &mut Content, tip: P2, u: P2) {
    let n = perp(u);
    let base = add(tip, u, -3.5);
    c.move_to(tip);
    c.line_to(add(base, n, 0.8));
    c.line_to(add(base, n, -0.8));
    c.close();
    c.fill();
}

/// The section's title: `DOORSNEDE A-A`, with the scale when it differs
/// from the sheet's.
pub fn title(p: &Profile, lang: text::Lang, s: f64, main: f64) -> String {
    let mut t = format!("{} {}-{}", lang.t("section"), p.letter, p.letter);
    if (s - main).abs() > 1e-9 {
        t.push_str(&format!(" ({})", text::scale(s)));
    }
    t
}

// ---------------------------------------------------------------------
// Folding the whole blank for the extents.

pub type P3 = [f64; 3];

#[derive(Clone, Copy, Debug)]
pub struct Affine {
    m: [[f64; 3]; 3],
    t: P3,
}

impl Affine {
    const ID: Affine = Affine {
        m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: [0.0; 3],
    };
    pub fn apply(&self, p: P3) -> P3 {
        let mut q = self.t;
        for (qi, row) in q.iter_mut().zip(&self.m) {
            *qi += row.iter().zip(&p).map(|(m, x)| m * x).sum::<f64>();
        }
        q
    }
    fn then(&self, outer: &Affine) -> Affine {
        // outer ∘ self
        let mut m = [[0.0; 3]; 3];
        for (row, orow) in m.iter_mut().zip(&outer.m) {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| orow[k] * self.m[k][j]).sum();
            }
        }
        Affine {
            m,
            t: outer.apply(self.t),
        }
    }
}

fn point_in_polygon(p: P2, poly: &[P2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

fn area(poly: &[P2]) -> f64 {
    let n = poly.len();
    let mut a = 0.0;
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    a.abs() / 2.0
}

/// One bend of the folded blank: the parent flange's transform and the
/// walk's arc in the parent's (n, w) plane, so a point of the bend zone
/// (along the bend line `e`, across it `n`) lands on the cylinder.
#[derive(Clone, Copy, Debug)]
pub struct BendFold {
    pub parent: Affine,
    pub e: P2,
    pub n: P2,
    /// Arc centre in the (n, w) plane, the outer skin's radius and the
    /// inner skin's, the signed turn, and which way the skin lies from
    /// the centre.
    pub centre: P2,
    pub r_skin: f64,
    pub r_other: f64,
    pub turn: f64,
    pub sign: f64,
    /// Where the bend zone starts along `n` and how wide it is.
    pub n_start: f64,
    pub allowance: f64,
}

impl BendFold {
    /// A point of the bend zone (blank coordinates) on the folded skin
    /// (`skin` 0) or on the inner skin (`skin` 1), in the base frame.
    pub fn apply(&self, p: P2, skin: usize) -> P3 {
        let along = dot(p, self.e);
        let f = ((dot(p, self.n) - self.n_start) / self.allowance).clamp(0.0, 1.0);
        let mk = rot([0.0, -1.0], self.turn * f);
        let r = if skin == 0 { self.r_skin } else { self.r_other };
        let pt = add(self.centre, mk, self.sign * r);
        self.parent.apply([
            self.e[0] * along + self.n[0] * pt[0],
            self.e[1] * along + self.n[1] * pt[0],
            pt[1],
        ])
    }
}

/// The blank folded: a transform per flange zone (the largest flange is
/// the base, every other flange hangs off it through its bend, as
/// [`walk`] folds a section) and the cylinder per bend.
pub struct Fold {
    pub transform: Vec<Option<Affine>>,
    pub bends: Vec<Option<BendFold>>,
    /// Zone indices that are flanges (not bend zones).
    pub flanges: Vec<usize>,
}

/// The folded part's extents along the base flange's axes (L, W, H).
/// `None` when the zones do not chain (a bend without a flange on each
/// side).
pub fn folded_extents(blank: &Blank, t: f64) -> Option<[f64; 3]> {
    if blank.bends.is_empty() {
        // A flat plate: its blank and the thickness.
        return (t > 0.0).then_some([blank.max[0], blank.max[1], t]);
    }
    let fd = fold(blank, t)?;
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut take = |p: P3| {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    };
    for (z, tr) in fd.transform.iter().enumerate() {
        let Some(tr) = tr else {
            continue;
        };
        for p in &blank.zones[z].points {
            take(tr.apply([p[0], p[1], 0.0]));
            take(tr.apply([p[0], p[1], -t]));
        }
    }
    // Bend zones: both skins' arcs, at both ends of the line.
    for (bi, b) in blank.bends.iter().enumerate() {
        let Some(bf) = fd.bends[bi] else {
            continue;
        };
        for k in 0..=8 {
            let f = k as f64 / 8.0;
            for skin in [0, 1] {
                for end in [b.a, b.b] {
                    let p = add(end, bf.n, bf.n_start + bf.allowance * f - dot(end, bf.n));
                    take(bf.apply(p, skin));
                }
            }
        }
    }
    Some([hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]])
}

/// Fold the blank; `None` when a flange is unreachable from the base.
pub fn fold(blank: &Blank, t: f64) -> Option<Fold> {
    if blank.zones.is_empty() {
        return None;
    }
    let is_bend_zone = |id: &str| blank.bends.iter().any(|b| b.zone_id == id);
    let flanges: Vec<usize> = (0..blank.zones.len())
        .filter(|&z| !is_bend_zone(&blank.zones[z].id))
        .collect();
    let base = *flanges
        .iter()
        .max_by(|a, b| area(&blank.zones[**a].points).total_cmp(&area(&blank.zones[**b].points)))?;
    // Each bend's two neighbouring flanges: the zones holding a point just
    // past each edge of its zone.
    // A relief slot may sit on the line, so several points along it are
    // tried.
    let mut neighbours: Vec<Option<(usize, usize)>> = Vec::new();
    for b in &blank.bends {
        let n = perp(bend_dir(b));
        let mut found = None;
        for k in [0.5, 0.3, 0.7, 0.15, 0.85, 0.05, 0.95] {
            let at = [
                b.a[0] + (b.b[0] - b.a[0]) * k,
                b.a[1] + (b.b[1] - b.a[1]) * k,
            ];
            let find = |sign: f64| {
                let q = add(at, n, sign * (b.allowance / 2.0 + 0.3));
                flanges
                    .iter()
                    .copied()
                    .find(|&z| point_in_polygon(q, &blank.zones[z].points))
            };
            if let (Some(p), Some(q)) = (find(1.0), find(-1.0)) {
                found = Some((p, q));
                break;
            }
        }
        neighbours.push(found);
    }
    // Breadth-first from the base. For a bend from parent to child, in
    // the parent's flat frame (e along the bend line, n toward the child,
    // w up out of the outer skin), the (n, w) plane holds the walk of a
    // section: the outer skin at w = 0, the material below it.
    let mut transform: Vec<Option<Affine>> = vec![None; blank.zones.len()];
    transform[base] = Some(Affine::ID);
    let mut queue = vec![base];
    // Per bend: the parent's transform and the walk's arc (centre, radii,
    // turn) in the parent's (n, w) plane, for sampling the zone.
    let mut arcs: Vec<Option<BendFold>> = vec![None; blank.bends.len()];
    while let Some(z) = queue.pop() {
        let tz = transform[z].unwrap();
        for (bi, b) in blank.bends.iter().enumerate() {
            let Some((p, q)) = neighbours[bi] else {
                continue;
            };
            let child = if p == z {
                q
            } else if q == z {
                p
            } else {
                continue;
            };
            if transform[child].is_some() {
                continue;
            }
            let e = bend_dir(b);
            let mut n = perp(e);
            let mid = [(b.a[0] + b.b[0]) / 2.0, (b.a[1] + b.b[1]) / 2.0];
            let c_child = centroid(&blank.zones[child].points);
            if dot([c_child[0] - mid[0], c_child[1] - mid[1]], n) < 0.0 {
                n = [-n[0], -n[1]];
            }
            let mid_n = dot(mid, n);
            let half = b.allowance / 2.0;
            let up = b.direction == BendDirection::Up;
            // The walk in the (n, w) plane, absolute along n.
            let n0 = [mid_n - half, 0.0];
            let d = [1.0, 0.0];
            let m = [0.0, -1.0];
            let turn = if up { b.angle_deg } else { -b.angle_deg };
            let (centre, r_skin) = if up {
                (add(n0, m, -b.inside_radius), b.inside_radius)
            } else {
                (add(n0, m, b.inside_radius + t), b.inside_radius + t)
            };
            let d2 = rot(d, turn);
            let m2 = rot(m, turn);
            let end = if up {
                add(centre, m2, r_skin)
            } else {
                add(centre, m2, -r_skin)
            };
            // Child point (x, y, w): along e unchanged; s' = n·(x, y) -
            // (mid_n + half) beyond the far edge, along d'; depth w along
            // -m'. In 3D: e3·along + n3·P_n + w3·P_w.
            let e3 = [e[0], e[1], 0.0];
            let v3 = |pn: f64, pw: f64| [n[0] * pn, n[1] * pn, pw];
            let dp = v3(d2[0], d2[1]);
            let mp = v3(m2[0], m2[1]);
            let origin = v3(
                end[0] - (mid_n + half) * d2[0],
                end[1] - (mid_n + half) * d2[1],
            );
            let mut mm = [[0.0; 3]; 3];
            for i in 0..3 {
                mm[i][0] = e3[i] * e[0] + dp[i] * n[0];
                mm[i][1] = e3[i] * e[1] + dp[i] * n[1];
                mm[i][2] = -mp[i];
            }
            let fold = Affine { m: mm, t: origin };
            transform[child] = Some(fold.then(&tz));
            let r_other = if up {
                b.inside_radius + t
            } else {
                b.inside_radius
            };
            arcs[bi] = Some(BendFold {
                parent: tz,
                e,
                n,
                centre,
                r_skin,
                r_other,
                turn,
                sign: if up { 1.0 } else { -1.0 },
                n_start: mid_n - half,
                allowance: b.allowance,
            });
            queue.push(child);
        }
    }
    if flanges.iter().any(|&z| transform[z].is_none()) {
        return None;
    }
    Some(Fold {
        transform,
        bends: arcs,
        flanges,
    })
}

fn centroid(poly: &[P2]) -> P2 {
    let n = poly.len().max(1) as f64;
    let mut c = [0.0, 0.0];
    for p in poly {
        c[0] += p[0] / n;
        c[1] += p[1] / n;
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces_u() -> Vec<Piece> {
        vec![
            Piece::Flat { len: 20.0 },
            Piece::Bend {
                nr: 1,
                angle: 90.0,
                up: false,
                r_in: 1.5,
            },
            Piece::Flat { len: 100.0 },
            Piece::Bend {
                nr: 2,
                angle: 90.0,
                up: false,
                r_in: 1.5,
            },
            Piece::Flat { len: 20.0 },
        ]
    }

    #[test]
    fn a_u_channel_folds_down_with_outside_lengths() {
        let g = walk(&pieces_u(), 1.5, None);
        assert_eq!(g.flanges.len(), 3);
        assert_eq!(g.corners.len(), 2);
        // Both flanges hang down from the base, which lies along +x: the
        // section is 3 (outer radius) + 20 tall and the base's outside
        // length is 100 + 2 × 3.
        let v0 = g.corners[0].v;
        let v1 = g.corners[1].v;
        assert!((v1[0] - v0[0] - 106.0).abs() < 1e-6, "{v0:?} {v1:?}");
        assert!((g.bbox[3] - g.bbox[1] - 23.0).abs() < 1e-6, "{:?}", g.bbox);
        assert!((g.bbox[2] - g.bbox[0] - 106.0).abs() < 1e-6);
        let base = g.flanges.iter().find(|f| f.piece == 2).unwrap();
        assert!(base.d[1].abs() < 1e-9 && base.a[1] > g.bbox[1] + 20.0);
    }

    #[test]
    fn an_up_bend_rises_and_keeps_the_inside_radius_on_top() {
        let pieces = vec![
            Piece::Flat { len: 50.0 },
            Piece::Bend {
                nr: 1,
                angle: 60.0,
                up: true,
                r_in: 1.5,
            },
            Piece::Flat { len: 30.0 },
        ];
        let g = walk(&pieces, 1.5, None);
        assert!(g.bbox[3] > 20.0);
        assert!((g.corners[0].turn - 60.0).abs() < 1e-9);
        let p = Profile {
            letter: 'A',
            c: [1.0, 0.0],
            e: [0.0, 1.0],
            cut: [[0.0, 0.0], [80.0, 0.0]],
            pieces,
            bends: vec![1],
            t: 1.5,
        };
        let plan = plan(&p, 1.0);
        assert_eq!(plan.angles.len(), 1);
        // A 60° bend leaves 120° between the flanges.
        assert_eq!(plan.angles[0].text, "120°");
        assert!(plan.dims.iter().any(|d| d.kind == "flange"));
        assert!(plan.dims.iter().any(|d| d.kind == "height"));
        let mut c = Content::new();
        draw(&plan, &mut c, [50.0, 50.0]);
        // "120°" as glyph ids.
        assert!(
            c.as_str().contains("<0012001300110069> Tj"),
            "{}",
            c.as_str()
        );
        assert_eq!(title(&p, text::Lang::Nl, 1.0, 1.0), "DOORSNEDE A-A");
        assert_eq!(title(&p, text::Lang::Nl, 2.0, 1.0), "DOORSNEDE A-A (2:1)");
    }

    #[test]
    fn long_flanges_are_interrupted() {
        let p = Profile {
            letter: 'A',
            c: [1.0, 0.0],
            e: [0.0, 1.0],
            cut: [[0.0, 0.0], [140.0, 0.0]],
            pieces: vec![
                Piece::Flat { len: 20.0 },
                Piece::Bend {
                    nr: 1,
                    angle: 90.0,
                    up: false,
                    r_in: 1.5,
                },
                Piece::Flat { len: 1000.0 },
                Piece::Bend {
                    nr: 2,
                    angle: 90.0,
                    up: false,
                    r_in: 1.5,
                },
                Piece::Flat { len: 20.0 },
            ],
            bends: vec![1, 2],
            t: 1.5,
        };
        let plan = plan(&p, 0.5);
        assert_eq!(plan.breaks.len(), 1);
        assert_eq!(plan.breaks[0].0, 2);
        let base = plan.dims.iter().find(|d| d.piece == Some(2)).unwrap();
        assert_eq!(base.text, "1006");
        assert!(plan.bbox[2] - plan.bbox[0] < 200.0);
    }
}
