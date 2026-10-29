//! What a flat pattern is made of, as a drafter sees it: corners (real or
//! the sharp corner a fillet replaced), holes, arcs, bend lines. The blank
//! from [`crate::dxf::blank`] is edge by edge as the kernel cut its faces;
//! here collinear edges become one line and a hole split by a tangent line
//! becomes one circle again, so that a dimension lands on a feature, never
//! on a seam of the model.

use super::pattern::{patterns, Pattern};
use crate::dxf::blank::{arc_point, Blank, Seg, P2};

const TOL: f64 = 1e-4;
/// Two coordinates closer than this are one ordinate (0,05 mm: half of
/// what the drawing can show at a tenth).
pub const SAME: f64 = 0.05;
/// Lines no longer than this that turn no more than 30° at their junction
/// are read as a polyline standing in for a curve.
const SHORT: f64 = 5.0;
const GENTLE_TURN_COS: f64 = 0.866;
/// A line that runs nearly tangent (within 5°) into a free-form run is a
/// piece of that run.
const NEAR_TANGENT_COS: f64 = 0.9962;
/// On the outline, a corner with an edge shorter than this on either side
/// is minor: a relief, a notch or a tab the laser cuts from the DXF. The
/// bound grows with the part (2 % of its size) within these limits; a
/// cutout is never minor, whatever its size.
const MINOR_MIN: f64 = 6.0;
const MINOR_MAX: f64 = 25.0;
/// A round smaller than this that is not a fillet is minor too.
const MINOR_R: f64 = 3.0;
/// An extreme point of a free-form run this close to one of its ends says
/// nothing a drafter would dimension.
const EXTREME_CLEAR: f64 = 3.0;

#[derive(Clone, Debug)]
pub struct Corner {
    pub p: P2,
    /// The intersection of two lines a fillet joins: a corner that is not
    /// on the part but defines it.
    pub virtual_: bool,
    /// Part of a bend relief, a notch or a small round: cut from the DXF,
    /// not dimensioned.
    pub minor: bool,
    /// An end or extreme point of a free-form run.
    pub on_curve: bool,
}

#[derive(Clone, Debug)]
pub struct Circle {
    pub c: P2,
    pub r: f64,
}

#[derive(Clone, Debug)]
pub struct Arc {
    pub c: P2,
    pub r: f64,
    pub start: f64,
    pub end: f64,
    /// Tangent to a line at both ends: only its radius is dimensioned.
    pub fillet: bool,
}

#[derive(Clone, Debug)]
pub struct BendFeat {
    /// 1-based, the order of the kernel's list.
    pub nr: usize,
    pub a: P2,
    pub b: P2,
}

/// A free-form contour: the loop index, the segments it spans (in loop
/// order, wrapping), and the points that fix it.
#[derive(Clone, Debug)]
pub struct Curve {
    pub lp: usize,
    pub first: usize,
    pub len: usize,
    pub points: Vec<P2>,
}

/// One value on an axis and what put it there.
#[derive(Clone, Debug)]
pub struct Ordinate {
    pub value: f64,
    pub sources: Vec<String>,
    /// Where each source sits, so a projection line can start at the one
    /// nearest the dimension line.
    pub points: Vec<P2>,
}

#[derive(Clone, Debug, Default)]
pub struct Features {
    pub loops: Vec<Vec<Seg>>,
    pub corners: Vec<Corner>,
    pub circles: Vec<Circle>,
    pub arcs: Vec<Arc>,
    pub bends: Vec<BendFeat>,
    /// Free-form contours: runs of arcs tangent to each other, or of short
    /// lines turning gently, that approximate a curve. They are dimensioned
    /// by their ends and extreme points only; the DXF carries the shape.
    pub curves: Vec<Curve>,
    /// Fields of equal cut-outs, dimensioned as one: their member loops
    /// add no corners or circles of their own.
    pub patterns: Vec<Pattern>,
    /// The centres of the round ends of slots.
    pub slot_ends: Vec<P2>,
    /// How many corners were left out as minor.
    pub minor: usize,
    pub xs: Vec<Ordinate>,
    pub ys: Vec<Ordinate>,
    pub max: P2,
}

pub fn features(blank: &Blank) -> Features {
    let mut f = Features {
        loops: blank.loops.iter().map(|l| merge(l)).collect(),
        max: blank.max,
        ..Features::default()
    };
    let outer = outer_loop(&f.loops);
    f.patterns = patterns(&f.loops);
    let in_pattern: Vec<usize> = f.patterns.iter().flat_map(|p| p.loops.clone()).collect();
    for (li, lp) in f.loops.iter().enumerate() {
        if in_pattern.contains(&li) {
            continue;
        }
        if let [Seg::Circle { c, r }] = lp.as_slice() {
            f.circles.push(Circle { c: *c, r: *r });
            continue;
        }
        let n = lp.len();
        let len: Vec<f64> = lp.iter().map(seg_len).collect();
        let minor_edge = if li == outer {
            (0.02 * blank.max[0].max(blank.max[1])).clamp(MINOR_MIN, MINOR_MAX)
        } else {
            0.0
        };
        // Which junctions are tangent, and which are faired: part of a
        // free-form contour rather than a fillet between two lines.
        let tangent_at: Vec<bool> = (0..n).map(|i| tangent(&lp[i], &lp[(i + 1) % n])).collect();
        let mut faired_at: Vec<bool> = (0..n).map(|i| faired(&lp[i], &lp[(i + 1) % n])).collect();
        let mut in_curve: Vec<bool> = (0..n)
            .map(|i| faired_at[(i + n - 1) % n] || faired_at[i])
            .collect();
        // A line running nearly tangent into a run belongs to it.
        loop {
            let mut grew = false;
            for i in 0..n {
                let j = (i + 1) % n;
                if faired_at[i] || !(in_curve[i] ^ in_curve[j]) {
                    continue;
                }
                let (run, line) = if in_curve[i] { (i, j) } else { (j, i) };
                if matches!(lp[line], Seg::Line { .. })
                    && matches!(lp[run], Seg::Arc { .. })
                    && turn_cos(&lp[i], &lp[j]) >= NEAR_TANGENT_COS
                {
                    faired_at[i] = true;
                    in_curve[line] = true;
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        // A run no bigger than a relief is tessellation, not a contour:
        // its corners go through the ordinary rules.
        let curve_min = minor_edge.max(MINOR_MIN);
        for (first, len) in runs(&in_curve) {
            let segs: Vec<Seg> = (0..len).map(|k| lp[(first + k) % n]).collect();
            if run_extent(&segs) < curve_min {
                for k in 0..len {
                    in_curve[(first + k) % n] = false;
                }
            }
        }
        // A line with another running back the other way closer than a
        // relief is the side of a slit or a thin tab: its ends are minor.
        let narrow: Vec<bool> = (0..n)
            .map(|i| narrow_line(lp, &len, i, minor_edge))
            .collect();
        for i in 0..n {
            if in_curve[i] {
                continue;
            }
            let prev = &lp[(i + n - 1) % n];
            let next = &lp[(i + 1) % n];
            let (lp_len, ln_len) = (len[(i + n - 1) % n], len[(i + 1) % n]);
            match lp[i] {
                Seg::Arc { c, r, start, end } => {
                    let fillet = tangent_at[(i + n - 1) % n]
                        && tangent_at[i]
                        && matches!(prev, Seg::Line { .. })
                        && matches!(next, Seg::Line { .. });
                    if fillet {
                        let minor = lp_len.min(ln_len) < minor_edge;
                        if let Some(p) = intersection(prev, next) {
                            f.corners.push(Corner {
                                p,
                                virtual_: true,
                                minor,
                                on_curve: false,
                            });
                        } else if li != outer {
                            // Parallel lines in a cut-out: a slot end. Its
                            // centre fixes it, like a hole's.
                            f.slot_ends.push(c);
                        } else {
                            // A rounded tab on the outline.
                            f.corners.push(Corner {
                                p: c,
                                virtual_: true,
                                minor,
                                on_curve: false,
                            });
                        }
                        if minor {
                            f.minor += 1;
                            continue;
                        }
                    } else {
                        if r < MINOR_R {
                            f.minor += 1;
                            continue;
                        }
                        f.corners.push(Corner {
                            p: c,
                            virtual_: true,
                            minor: false,
                            on_curve: false,
                        });
                        let (p0, p1) = ends(&lp[i]);
                        for p in [p0, p1] {
                            f.corners.push(Corner {
                                p,
                                virtual_: false,
                                minor: false,
                                on_curve: false,
                            });
                        }
                    }
                    f.arcs.push(Arc {
                        c,
                        r,
                        start,
                        end,
                        fillet,
                    });
                }
                Seg::Line { a, .. } => {
                    // The junction with the previous segment is a corner
                    // unless it is a tangent fillet.
                    let fillet_before = matches!(prev, Seg::Arc { .. })
                        && tangent_at[(i + n - 1) % n]
                        && is_fillet(lp, (i + n - 1) % n);
                    if !fillet_before {
                        let minor = (lp_len.min(len[i]) < minor_edge
                            || narrow[i]
                            || narrow[(i + n - 1) % n])
                            && !in_curve[(i + n - 1) % n];
                        if minor {
                            f.minor += 1;
                        }
                        f.corners.push(Corner {
                            p: a,
                            virtual_: false,
                            minor,
                            on_curve: false,
                        });
                    }
                }
                Seg::Circle { .. } => {}
            }
        }
        // A run of short edges between two long lines is a relief or a
        // notch: the lines' theoretical corner is dimensioned instead.
        if li == outer && minor_edge > 0.0 {
            let short: Vec<bool> = (0..n)
                .map(|i| len[i] < minor_edge && !in_curve[i])
                .collect();
            if !short.iter().all(|&b| b) {
                for (first, rl) in runs(&short) {
                    let before = &lp[(first + n - 1) % n];
                    let after = &lp[(first + rl) % n];
                    let Some(p) = intersection(before, after) else {
                        continue;
                    };
                    let near = (0..rl).any(|k| {
                        let (a, b) = ends(&lp[(first + k) % n]);
                        dist(a, p) <= 2.0 * minor_edge || dist(b, p) <= 2.0 * minor_edge
                    });
                    if near {
                        f.corners.push(Corner {
                            p,
                            virtual_: true,
                            minor: false,
                            on_curve: false,
                        });
                    }
                }
            }
        }
        // The free-form runs.
        if in_curve.iter().all(|&c| c) {
            let points = extremes(lp.iter(), &[]);
            for p in &points {
                f.corners.push(Corner {
                    p: *p,
                    virtual_: false,
                    minor: false,
                    on_curve: true,
                });
            }
            f.curves.push(Curve {
                lp: li,
                first: 0,
                len: n,
                points,
            });
            continue;
        }
        for (first, len) in runs(&in_curve) {
            let segs: Vec<Seg> = (0..len).map(|k| lp[(first + k) % n]).collect();
            let (a, _) = ends(&segs[0]);
            let (_, b) = ends(&segs[len - 1]);
            let mut points = vec![a, b];
            points.extend(extremes(segs.iter(), &[a, b]));
            for p in &points {
                f.corners.push(Corner {
                    p: *p,
                    virtual_: false,
                    minor: false,
                    on_curve: true,
                });
            }
            f.curves.push(Curve {
                lp: li,
                first,
                len,
                points,
            });
        }
    }
    for (i, b) in blank.bends.iter().enumerate() {
        f.bends.push(BendFeat {
            nr: i + 1,
            a: b.a,
            b: b.b,
        });
    }
    f.collect_ordinates();
    f
}

/// The maximal runs of `true` in a cyclic sequence, as (first, len). A
/// sequence that is all `true` is one run from 0.
fn runs(flags: &[bool]) -> Vec<(usize, usize)> {
    let n = flags.len();
    if flags.iter().all(|&c| c) {
        return if n == 0 { vec![] } else { vec![(0, n)] };
    }
    let mut out = Vec::new();
    for i in 0..n {
        if !flags[i] || flags[(i + n - 1) % n] {
            continue;
        }
        let mut len = 1;
        while flags[(i + len) % n] {
            len += 1;
        }
        out.push((i, len));
    }
    out
}

/// The longer side of the box around a run of segments.
fn run_extent(segs: &[Seg]) -> f64 {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    let mut pts = Vec::new();
    extremes(segs.iter(), &[])
        .into_iter()
        .for_each(|p| pts.push(p));
    for s in segs {
        let (a, b) = ends(s);
        pts.push(a);
        pts.push(b);
    }
    for p in pts {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (hi[0] - lo[0]).max(hi[1] - lo[1])
}

/// The loop with the widest extent: the outline, the rest being cutouts.
pub(crate) fn outer_loop(loops: &[Vec<Seg>]) -> usize {
    let mut best = (0usize, -1.0f64);
    for (i, lp) in loops.iter().enumerate() {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        let mut pts = Vec::new();
        for seg in lp {
            let (a, b) = ends(seg);
            pts.push(a);
            pts.push(b);
            if let Seg::Circle { c, r } = *seg {
                pts.push([c[0] - r, c[1] - r]);
                pts.push([c[0] + r, c[1] + r]);
            }
        }
        for p in pts {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let extent = (hi[0] - lo[0]) + (hi[1] - lo[1]);
        if extent > best.1 {
            best = (i, extent);
        }
    }
    best.0
}

/// Whether line `i` has an antiparallel line of the loop closer than
/// `bound` beside it over at least half its length.
fn narrow_line(lp: &[Seg], len: &[f64], i: usize, bound: f64) -> bool {
    let Seg::Line { a, b } = lp[i] else {
        return false;
    };
    let l = len[i];
    if bound <= 0.0 || l < 1e-9 {
        return false;
    }
    let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
    lp.iter().enumerate().any(|(j, s)| {
        if j == i {
            return false;
        }
        let Seg::Line { a: c, b: e } = *s else {
            return false;
        };
        if len[j] < 1e-9 {
            return false;
        }
        let v = [(e[0] - c[0]) / len[j], (e[1] - c[1]) / len[j]];
        if u[0] * v[0] + u[1] * v[1] > -0.996 {
            return false;
        }
        let off = ((c[0] - a[0]) * u[1] - (c[1] - a[1]) * u[0]).abs();
        if off >= bound {
            return false;
        }
        let t0 = (c[0] - a[0]) * u[0] + (c[1] - a[1]) * u[1];
        let t1 = (e[0] - a[0]) * u[0] + (e[1] - a[1]) * u[1];
        let (lo, hi) = (t0.min(t1).max(0.0), t0.max(t1).min(l));
        hi - lo >= 0.5 * l
    })
}

fn dist(a: P2, b: P2) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

pub(crate) fn seg_len(s: &Seg) -> f64 {
    match *s {
        Seg::Line { a, b } => ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt(),
        Seg::Arc { r, start, end, .. } => {
            let sweep = (end - start).rem_euclid(360.0);
            let sweep = if sweep < 1e-9 { 360.0 } else { sweep };
            r * sweep.to_radians()
        }
        Seg::Circle { r, .. } => 2.0 * std::f64::consts::PI * r,
    }
}

/// The cosine of the turn at the shared end of `a` and `b` (1 = straight
/// on), or -1 when they share no end.
fn turn_cos(a: &Seg, b: &Seg) -> f64 {
    let (a0, a1) = ends(a);
    let (b0, b1) = ends(b);
    let p = if near(a1, b0) || near(a1, b1) {
        a1
    } else if near(a0, b0) || near(a0, b1) {
        a0
    } else {
        return -1.0;
    };
    let (Some(da), Some(db)) = (direction_at(a, p), direction_at(b, p)) else {
        return -1.0;
    };
    (da[0] * db[0] + da[1] * db[1]).abs()
}

/// Whether the junction of `a` and `b` is part of a free-form contour: two
/// arcs meeting tangent, or two short lines turning gently (a polyline
/// standing in for a curve).
fn faired(a: &Seg, b: &Seg) -> bool {
    match (a, b) {
        (Seg::Arc { .. }, Seg::Arc { .. }) => tangent(a, b),
        (Seg::Line { a: p, b: q }, Seg::Line { a: q2, b: r }) => {
            if !near(*q, *q2) {
                return false;
            }
            let d1 = [q[0] - p[0], q[1] - p[1]];
            let d2 = [r[0] - q[0], r[1] - q[1]];
            let l1 = (d1[0] * d1[0] + d1[1] * d1[1]).sqrt();
            let l2 = (d2[0] * d2[0] + d2[1] * d2[1]).sqrt();
            if l1 > SHORT || l2 > SHORT || l1 < TOL || l2 < TOL {
                return false;
            }
            let cos = (d1[0] * d2[0] + d1[1] * d2[1]) / (l1 * l2);
            cos >= GENTLE_TURN_COS
        }
        _ => false,
    }
}

/// The points where a run of segments reaches furthest left, right, down
/// and up, leaving out any near `skip`.
pub(crate) fn extremes<'a>(segs: impl Iterator<Item = &'a Seg>, skip: &[P2]) -> Vec<P2> {
    let mut cands: Vec<P2> = Vec::new();
    for s in segs {
        let (a, b) = ends(s);
        cands.push(a);
        cands.push(b);
        if let Seg::Arc { c, r, start, end } = *s {
            let sweep = (end - start).rem_euclid(360.0);
            for q in [0.0, 90.0, 180.0, 270.0] {
                if (q - start).rem_euclid(360.0) <= sweep {
                    cands.push(arc_point(c, r, q));
                }
            }
        }
    }
    let mut out: Vec<P2> = Vec::new();
    let picks = [
        cands.iter().copied().min_by(|a, b| a[0].total_cmp(&b[0])),
        cands.iter().copied().max_by(|a, b| a[0].total_cmp(&b[0])),
        cands.iter().copied().min_by(|a, b| a[1].total_cmp(&b[1])),
        cands.iter().copied().max_by(|a, b| a[1].total_cmp(&b[1])),
    ];
    for p in picks.into_iter().flatten() {
        let dup = |q: &P2| (q[0] - p[0]).abs() <= SAME && (q[1] - p[1]).abs() <= SAME;
        let close = |q: &P2| ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt() < EXTREME_CLEAR;
        if !skip.iter().any(close) && !out.iter().any(dup) {
            out.push(p);
        }
    }
    out
}

fn is_fillet(lp: &[Seg], i: usize) -> bool {
    let n = lp.len();
    matches!(lp[i], Seg::Arc { .. })
        && matches!(lp[(i + n - 1) % n], Seg::Line { .. })
        && matches!(lp[(i + 1) % n], Seg::Line { .. })
        && tangent(&lp[(i + n - 1) % n], &lp[i])
        && tangent(&lp[i], &lp[(i + 1) % n])
}

impl Features {
    fn collect_ordinates(&mut self) {
        let mut xs: Vec<(f64, String, P2)> = Vec::new();
        let mut ys: Vec<(f64, String, P2)> = Vec::new();
        for (i, c) in self.corners.iter().enumerate() {
            if c.minor {
                continue;
            }
            let tag = if c.virtual_ {
                format!("corner:{i}:virtual")
            } else {
                format!("corner:{i}")
            };
            xs.push((c.p[0], tag.clone(), c.p));
            ys.push((c.p[1], tag, c.p));
        }
        for (i, c) in self.circles.iter().enumerate() {
            xs.push((c.c[0], format!("circle:{i}"), c.c));
            ys.push((c.c[1], format!("circle:{i}"), c.c));
        }
        for (i, c) in self.slot_ends.iter().enumerate() {
            xs.push((c[0], format!("slot:{i}"), *c));
            ys.push((c[1], format!("slot:{i}"), *c));
        }
        // A field is located by the outer edges of its first and last
        // cells (the centres of a field of holes).
        for (k, p) in self.patterns.iter().enumerate() {
            let tag = format!("pattern:{k}");
            let (first, last) = (p.centres[0], p.centres[p.count() - 1]);
            let (lo, hi) = match p.circle {
                Some(_) => (first, last),
                None => (p.lo, p.hi),
            };
            for y in [lo[1], hi[1]] {
                xs.push((lo[0], tag.clone(), [lo[0], y]));
                xs.push((hi[0], tag.clone(), [hi[0], y]));
            }
            for x in [lo[0], hi[0]] {
                ys.push((lo[1], tag.clone(), [x, lo[1]]));
                ys.push((hi[1], tag.clone(), [x, hi[1]]));
            }
        }
        for b in &self.bends {
            let tag = format!("bend:{}", b.nr);
            if (b.a[0] - b.b[0]).abs() < TOL {
                xs.push((b.a[0], tag.clone(), b.a));
                xs.push((b.b[0], tag, b.b));
            } else if (b.a[1] - b.b[1]).abs() < TOL {
                ys.push((b.a[1], tag.clone(), b.a));
                ys.push((b.b[1], tag, b.b));
            } else {
                xs.push((b.a[0], tag.clone(), b.a));
                ys.push((b.a[1], tag.clone(), b.a));
                xs.push((b.b[0], tag.clone(), b.b));
                ys.push((b.b[1], tag, b.b));
            }
        }
        // The blank's extent is always dimensioned, minor corners or not.
        for (axis, v) in [(0usize, &mut xs), (1usize, &mut ys)] {
            for value in [0.0, self.max[axis]] {
                if v.iter().any(|(x, _, _)| (x - value).abs() <= SAME) {
                    continue;
                }
                let p = self
                    .corners
                    .iter()
                    .map(|c| c.p)
                    .find(|p| (p[axis] - value).abs() <= SAME)
                    .unwrap_or(if axis == 0 {
                        [value, 0.0]
                    } else {
                        [0.0, value]
                    });
                v.push((value, "extent".to_string(), p));
            }
        }
        self.xs = bucket(xs);
        self.ys = bucket(ys);
    }
}

/// Distinct values, ascending, each with every source that named it.
fn bucket(mut v: Vec<(f64, String, P2)>) -> Vec<Ordinate> {
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<Ordinate> = Vec::new();
    for (value, src, p) in v {
        if let Some(last) = out.last_mut() {
            if (value - last.value).abs() <= SAME {
                if !last.sources.contains(&src) {
                    last.sources.push(src);
                }
                last.points.push(p);
                continue;
            }
        }
        out.push(Ordinate {
            value,
            sources: vec![src],
            points: vec![p],
        });
    }
    out
}

/// Consecutive collinear lines become one; consecutive arcs on one circle
/// become one; a loop that is one full arc becomes a circle.
pub fn merge(lp: &[Seg]) -> Vec<Seg> {
    let mut out: Vec<Seg> = Vec::new();
    for s in lp {
        if let Some(last) = out.last_mut() {
            if let Some(m) = join(last, s) {
                *last = m;
                continue;
            }
        }
        out.push(*s);
    }
    // The seam between the last and the first.
    while out.len() > 1 {
        let (first, last) = (out[0], *out.last().unwrap());
        match join(&last, &first) {
            Some(m) => {
                out.pop();
                out[0] = m;
            }
            None => break,
        }
    }
    if let [Seg::Arc { c, r, start, end }] = out.as_slice() {
        if ((end - start).rem_euclid(360.0)).abs() < 1e-6 {
            return vec![Seg::Circle { c: *c, r: *r }];
        }
    }
    out
}

fn join(a: &Seg, b: &Seg) -> Option<Seg> {
    match (*a, *b) {
        (Seg::Line { a: p, b: q }, Seg::Line { a: q2, b: r }) => {
            if !near(q, q2) {
                return None;
            }
            let d1 = [q[0] - p[0], q[1] - p[1]];
            let d2 = [r[0] - q[0], r[1] - q[1]];
            let cross = d1[0] * d2[1] - d1[1] * d2[0];
            let dot = d1[0] * d2[0] + d1[1] * d2[1];
            let l1 = (d1[0] * d1[0] + d1[1] * d1[1]).sqrt();
            let l2 = (d2[0] * d2[0] + d2[1] * d2[1]).sqrt();
            if l1 < TOL || l2 < TOL {
                return Some(Seg::Line { a: p, b: r });
            }
            if (cross / (l1 * l2)).abs() < 1e-6 && dot > 0.0 {
                Some(Seg::Line { a: p, b: r })
            } else {
                None
            }
        }
        (
            Seg::Arc { c, r, start, end },
            Seg::Arc {
                c: c2,
                r: r2,
                start: s2,
                end: e2,
            },
        ) => {
            if !near(c, c2) || (r - r2).abs() > TOL {
                return None;
            }
            // Loops run one way round, so the second arc starts where the
            // first ends, or the first starts where the second ends.
            if ((end - s2).rem_euclid(360.0)).abs() < 1e-6
                || ((end - s2).rem_euclid(360.0) - 360.0).abs() < 1e-6
            {
                Some(Seg::Arc {
                    c,
                    r,
                    start,
                    end: e2,
                })
            } else if ((e2 - start).rem_euclid(360.0)).abs() < 1e-6
                || ((e2 - start).rem_euclid(360.0) - 360.0).abs() < 1e-6
            {
                Some(Seg::Arc {
                    c,
                    r,
                    start: s2,
                    end,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn near(a: P2, b: P2) -> bool {
    (a[0] - b[0]).abs() < TOL && (a[1] - b[1]).abs() < TOL
}

/// The two ends of a segment in loop order.
/// The loop's points in its running order, about one per paper
/// millimetre at scale `s` (paper coordinates). Arcs are stored
/// counter-clockwise whichever way the loop runs, so each segment is
/// entered at the end the walk stands on.
pub fn loop_points(lp: &[Seg], s: f64) -> Vec<P2> {
    let mut out: Vec<P2> = Vec::new();
    let Some(first) = lp.first() else {
        return out;
    };
    if let Seg::Circle { .. } = first {
        super::dims::sample(first, s, &mut out);
        return out;
    }
    let near = |p: P2, q: P2| (p[0] - q[0]).abs() + (p[1] - q[1]).abs();
    let (fa, fb) = ends(first);
    let mut cur = match lp.get(1) {
        Some(second) if !matches!(second, Seg::Circle { .. }) => {
            let (sa, sb) = ends(second);
            if near(fb, sa).min(near(fb, sb)) <= near(fa, sa).min(near(fa, sb)) {
                fa
            } else {
                fb
            }
        }
        _ => fa,
    };
    for seg in lp {
        let (a, b) = ends(seg);
        let forward = near(a, cur) <= near(b, cur);
        let mut pts = Vec::new();
        super::dims::sample(seg, s, &mut pts);
        if !forward {
            pts.reverse();
        }
        // Drop the point the previous segment ended on.
        if !out.is_empty() {
            pts.remove(0);
        }
        out.extend(pts);
        cur = if forward { b } else { a };
    }
    if out.len() > 1 && near(out[0], *out.last().unwrap()) < 1e-6 {
        out.pop();
    }
    out
}

pub fn ends(s: &Seg) -> (P2, P2) {
    match *s {
        Seg::Line { a, b } => (a, b),
        Seg::Arc { c, r, start, end } => (arc_point(c, r, start), arc_point(c, r, end)),
        Seg::Circle { c, r } => (arc_point(c, r, 0.0), arc_point(c, r, 0.0)),
    }
}

/// The unit tangent of `s` at its end `p` (sign free: only the angle
/// between two tangents matters).
fn direction_at(s: &Seg, p: P2) -> Option<P2> {
    match *s {
        Seg::Line { a, b } => {
            let d = [b[0] - a[0], b[1] - a[1]];
            let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
            if l < TOL {
                None
            } else {
                Some([d[0] / l, d[1] / l])
            }
        }
        Seg::Arc { c, .. } => {
            let rv = [p[0] - c[0], p[1] - c[1]];
            let l = (rv[0] * rv[0] + rv[1] * rv[1]).sqrt();
            if l < TOL {
                None
            } else {
                Some([-rv[1] / l, rv[0] / l])
            }
        }
        Seg::Circle { .. } => None,
    }
}

/// Whether `a` and `b` meet with a continuous tangent at their shared end.
fn tangent(a: &Seg, b: &Seg) -> bool {
    let (a0, a1) = ends(a);
    let (b0, b1) = ends(b);
    let p = if near(a1, b0) || near(a1, b1) {
        a1
    } else if near(a0, b0) || near(a0, b1) {
        a0
    } else {
        return false;
    };
    let (Some(da), Some(db)) = (direction_at(a, p), direction_at(b, p)) else {
        return false;
    };
    (da[0] * db[1] - da[1] * db[0]).abs() < 1e-3
}

/// Where the lines through `a` and `b` meet.
fn intersection(a: &Seg, b: &Seg) -> Option<P2> {
    let (Seg::Line { a: p1, b: p2 }, Seg::Line { a: p3, b: p4 }) = (*a, *b) else {
        return None;
    };
    let d1 = [p2[0] - p1[0], p2[1] - p1[1]];
    let d2 = [p4[0] - p3[0], p4[1] - p3[1]];
    let den = d1[0] * d2[1] - d1[1] * d2[0];
    let l1 = (d1[0] * d1[0] + d1[1] * d1[1]).sqrt();
    let l2 = (d2[0] * d2[0] + d2[1] * d2[1]).sqrt();
    if (den / (l1 * l2)).abs() < 1e-6 {
        return None;
    }
    let t = ((p3[0] - p1[0]) * d2[1] - (p3[1] - p1[1]) * d2[0]) / den;
    Some([p1[0] + t * d1[0], p1[1] + t * d1[1]])
}

/// The midpoint of an arc, for a leader to land on.
pub fn arc_mid(a: &Arc) -> P2 {
    let sweep = (a.end - a.start).rem_euclid(360.0);
    arc_point(a.c, a.r, a.start + sweep / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_with_fillet() -> Vec<Seg> {
        // A 100 × 50 plate whose top-right corner is a R10 fillet, with the
        // bottom edge split in two collinear pieces.
        vec![
            Seg::Line {
                a: [0.0, 0.0],
                b: [60.0, 0.0],
            },
            Seg::Line {
                a: [60.0, 0.0],
                b: [100.0, 0.0],
            },
            Seg::Line {
                a: [100.0, 0.0],
                b: [100.0, 40.0],
            },
            Seg::Arc {
                c: [90.0, 40.0],
                r: 10.0,
                start: 0.0,
                end: 90.0,
            },
            Seg::Line {
                a: [90.0, 50.0],
                b: [0.0, 50.0],
            },
            Seg::Line {
                a: [0.0, 50.0],
                b: [0.0, 0.0],
            },
        ]
    }

    #[test]
    fn collinear_edges_merge_and_a_fillet_gives_a_virtual_corner() {
        let blank = Blank {
            zones: vec![],
            loops: vec![
                rect_with_fillet(),
                vec![
                    Seg::Arc {
                        c: [30.0, 25.0],
                        r: 4.5,
                        start: 0.0,
                        end: 180.0,
                    },
                    Seg::Arc {
                        c: [30.0, 25.0],
                        r: 4.5,
                        start: 180.0,
                        end: 360.0,
                    },
                ],
            ],
            bends: vec![],
            min: [0.0, 0.0],
            max: [100.0, 50.0],
            ..Default::default()
        };
        let f = features(&blank);
        assert_eq!(f.loops[0].len(), 5);
        assert!(matches!(f.loops[1][0], Seg::Circle { .. }));
        assert_eq!(f.circles.len(), 1);
        assert_eq!(f.arcs.len(), 1);
        assert!(f.arcs[0].fillet);
        let virt: Vec<&Corner> = f.corners.iter().filter(|c| c.virtual_).collect();
        assert_eq!(virt.len(), 1);
        assert!(near(virt[0].p, [100.0, 50.0]), "{:?}", virt[0].p);
        let xs: Vec<f64> = f.xs.iter().map(|o| o.value).collect();
        assert_eq!(xs, vec![0.0, 30.0, 100.0]);
        let ys: Vec<f64> = f.ys.iter().map(|o| o.value).collect();
        assert_eq!(ys, vec![0.0, 25.0, 50.0]);
    }

    #[test]
    fn every_flat_fixture_yields_features_with_the_blank_extent() {
        for name in [
            "four_down_flat",
            "hex_pan_flat",
            "mixed_channel_flat",
            "deck_splash_flat",
            "curved_board_flat",
        ] {
            let json = std::fs::read_to_string(format!("fixtures/{name}.view.json")).unwrap();
            let v: crate::view::View = serde_json::from_str(&json).unwrap();
            for s in &v.solids {
                let b = crate::dxf::blank::flat_blank(s, false, 1.0).unwrap();
                let f = features(&b);
                assert!(!f.xs.is_empty() && !f.ys.is_empty(), "{name}");
                let xmax = f.xs.last().unwrap().value;
                let ymax = f.ys.last().unwrap().value;
                assert!(xmax <= b.max[0] + SAME && ymax <= b.max[1] + SAME, "{name}");
                assert!(f.xs[0].value >= -SAME && f.ys[0].value >= -SAME, "{name}");
            }
        }
    }
}
