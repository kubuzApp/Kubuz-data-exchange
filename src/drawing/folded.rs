//! One sheet per folded sheet-metal part: first-angle principal views
//! chosen from the bends (ISO 128-30), an isometric in the free corner,
//! running dimensions to the outside of the part, hole and radius
//! callouts, bend angles as the deviation from flat, numbered bends and
//! a bend table.
//!
//! The front view looks at the outer skin of the largest flange, its
//! long side horizontal. A side view is added where a bend's axis points
//! at the viewer (so the bend's angle and the flange's length are true),
//! or where a hole faces only that viewer. Each coordinate is dimensioned
//! once across the views; the sheet thickness is written, not drawn as a
//! dimension.

use std::collections::HashMap;

use serde::Serialize;

use super::dims::{self, arrow, sample, sample_line, Layout, H, NARROW, WIDE};
use super::features::{Features, Ordinate, SAME};
use super::flat::{BendReport, DimReport, SheetInfo, SheetReport};
use super::project::{
    add, coincident_circles, dot, norm, project, sample_edge, scale as mul, topology, Basis,
    Projection, Topology,
};
use super::sheet::{self, Paper};
use super::text::{self, Lang};
use super::Meta;
use crate::dxf::blank::{Seg, P2};
use crate::pdf::{font, Content, Doc};
use crate::step::writer::{cross, sub, unit};
use crate::view::{Solid, Surface, P3};

const SCALES: [f64; 9] = [5.0, 2.0, 1.0, 0.5, 0.2, 0.1, 0.05, 0.02, 0.01];
const TABLE_ROW: f64 = 5.0;
const NOTE_PITCH: f64 = 3.6;
const TABLE_COLS: [f64; 4] = [8.0, 14.0, 14.0, 12.0];
/// Space between neighbouring views beyond their dimensions.
const GAP: f64 = 8.0;
/// Radius of an angle dimension's arc on paper.
const ANGLE_R: f64 = 10.0;
const TAG_R: f64 = 2.5;
const MINOR_MIN: f64 = 6.0;
const MINOR_MAX: f64 = 25.0;
const ISO_TITLE_H: f64 = 6.0;

/// A planar zone of the sheet, seen on its outer skin.
#[derive(Clone, Debug)]
pub struct Flange {
    pub zone: String,
    pub face: usize,
    /// Outward unit normal of the outer skin.
    pub n: P3,
    pub origin: P3,
    pub area: f64,
    pub centre: P3,
    /// Direction of the longest straight edge of the outer skin.
    pub long_edge: Option<P3>,
}

/// A bend zone between two flanges.
#[derive(Clone, Debug)]
pub struct Bend {
    pub zone: String,
    pub axis: P3,
    pub origin: P3,
    pub r_outer: f64,
    pub r_inner: f64,
    /// Flange indices, parent (nearer the base) first once the tree is
    /// built.
    pub flanges: [usize; 2],
    pub angle_deg: f64,
    /// Folds toward the outer skin's normal.
    pub up: bool,
    pub nr: usize,
}

pub struct Analysis {
    pub flanges: Vec<Flange>,
    pub bends: Vec<Bend>,
    pub base: usize,
    pub thickness: f64,
    /// Axes of the rolled zones (cylindrical outer skin without flanges).
    pub rolls: Vec<(P3, P3)>,
}

/// Whether the solid is a folded (or rolled) sheet rather than a flat
/// blank: its outer skin is not one plane.
pub fn is_folded(solid: &Solid) -> bool {
    let mut normal: Option<P3> = None;
    for f in &solid.faces {
        if f.role != "outerSkin" {
            continue;
        }
        match solid.surfaces.get(f.surface) {
            Some(Surface::Plane { normal: n, .. }) => {
                let n = unit(*n);
                match normal {
                    None => normal = Some(n),
                    Some(m) => {
                        if dot(m, n) < 0.9999 {
                            return true;
                        }
                    }
                }
            }
            _ => return true,
        }
    }
    false
}

fn face_area_and_centre(solid: &Solid, fi: usize, n: P3) -> (f64, P3, Option<P3>) {
    let f = &solid.faces[fi];
    let e1 = crate::step::writer::any_perp(n);
    let e2 = cross(n, e1);
    let mut area = 0.0;
    let mut cx = 0.0;
    let mut cy = 0.0;
    let mut long: Option<(f64, P3)> = None;
    for (li, lp) in f.loops.iter().enumerate() {
        let mut pts: Vec<P2> = Vec::new();
        for le in lp {
            if le.edge >= solid.edges.len() {
                continue;
            }
            let s = sample_edge(solid, le.edge, 2.0);
            let e = solid.edges[le.edge];
            if li == 0 && matches!(solid.curves.get(e.curve), Some(crate::view::Curve::Line)) {
                let d = sub(solid.vertices[e.end], solid.vertices[e.start]);
                let l = norm(d);
                if l > 1e-9 && long.map(|(ll, _)| l > ll).unwrap_or(true) {
                    long = Some((l, unit(d)));
                }
            }
            let n = s.pts.len();
            let order: Vec<usize> = if le.forward {
                (0..n - 1).collect()
            } else {
                (1..n).rev().collect()
            };
            for i in order {
                pts.push([dot(s.pts[i], e1), dot(s.pts[i], e2)]);
            }
        }
        let m = pts.len();
        let mut a = 0.0;
        let mut sx = 0.0;
        let mut sy = 0.0;
        for i in 0..m {
            let (p, q) = (pts[i], pts[(i + 1) % m]);
            let w = p[0] * q[1] - q[0] * p[1];
            a += w;
            sx += (p[0] + q[0]) * w;
            sy += (p[1] + q[1]) * w;
        }
        let a = a / 2.0;
        if a.abs() > 1e-9 {
            // Holes come with the opposite sign and subtract themselves.
            area += a.abs() * if li == 0 { 1.0 } else { -1.0 };
            let sign = if li == 0 { 1.0 } else { -1.0 };
            cx += sign * sx / 6.0 * a.signum();
            cy += sign * sy / 6.0 * a.signum();
        }
    }
    let area = area.max(1e-9);
    let c2 = [cx / area, cy / area];
    let along = dot(
        *solid.faces[fi]
            .loops
            .first()
            .and_then(|lp| lp.first())
            .and_then(|le| solid.edges.get(le.edge))
            .and_then(|e| solid.vertices.get(e.start))
            .unwrap_or(&[0.0; 3]),
        n,
    );
    let centre = add(add(mul(e1, c2[0]), mul(e2, c2[1])), mul(n, along));
    (area, centre, long.map(|l| l.1))
}

/// Read the zones of a folded sheet.
pub fn analyse(solid: &Solid, topo: &Topology, thickness: f64) -> Result<Analysis, String> {
    let mut flanges: Vec<Flange> = Vec::new();
    let mut by_face: HashMap<usize, usize> = HashMap::new();
    for (fi, f) in solid.faces.iter().enumerate() {
        if f.role != "outerSkin" {
            continue;
        }
        if let Some(Surface::Plane { origin, normal }) = solid.surfaces.get(f.surface) {
            let n = unit(*normal);
            let n = if f.same_sense { n } else { mul(n, -1.0) };
            let (area, centre, long_edge) = face_area_and_centre(solid, fi, n);
            by_face.insert(fi, flanges.len());
            flanges.push(Flange {
                zone: f.zone_id.clone(),
                face: fi,
                n,
                origin: *origin,
                area,
                centre,
                long_edge,
            });
        }
    }
    let mut bends: Vec<Bend> = Vec::new();
    let mut rolls = Vec::new();
    for f in &solid.faces {
        if f.role != "outerSkin" {
            continue;
        }
        let Some(Surface::Cylinder {
            origin,
            axis,
            radius,
        }) = solid.surfaces.get(f.surface)
        else {
            continue;
        };
        let axis = unit(*axis);
        // The flanges this zone runs into, through the edges it shares
        // with planar outer-skin faces.
        // The two flanges it shares the most edge length with.
        let mut shared: Vec<(usize, f64)> = Vec::new();
        for le in f.loops.iter().flatten() {
            for &other in topo
                .faces_of_edge
                .get(le.edge)
                .map(|v| v.as_slice())
                .unwrap_or(&[])
            {
                if let Some(&fl) = by_face.get(&other) {
                    let l = topo.chord.get(le.edge).copied().unwrap_or(0.0);
                    match shared.iter_mut().find(|(k, _)| *k == fl) {
                        Some(e) => e.1 += l,
                        None => shared.push((fl, l)),
                    }
                }
            }
        }
        shared.sort_by(|p, q| q.1.total_cmp(&p.1));
        let touching: Vec<usize> = shared.iter().take(2).map(|(k, _)| *k).collect();
        if bends.iter().any(|b: &Bend| b.zone == f.zone_id) {
            continue;
        }
        // The inner skin of the same zone gives the other radius.
        let r_other = solid
            .faces
            .iter()
            .find(|g| g.role == "innerSkin" && g.zone_id == f.zone_id)
            .and_then(|g| match solid.surfaces.get(g.surface) {
                Some(Surface::Cylinder { radius, .. }) => Some(*radius),
                _ => None,
            })
            .unwrap_or(*radius + thickness);
        if touching.len() != 2 {
            rolls.push((*origin, axis));
            continue;
        }
        let (a, b) = (&flanges[touching[0]], &flanges[touching[1]]);
        let angle_deg = dot(a.n, b.n).clamp(-1.0, 1.0).acos().to_degrees();
        bends.push(Bend {
            zone: f.zone_id.clone(),
            axis,
            origin: *origin,
            r_outer: *radius,
            r_inner: r_other,
            flanges: [touching[0], touching[1]],
            angle_deg,
            up: *radius < r_other,
            nr: 0,
        });
    }
    if flanges.is_empty() && rolls.is_empty() {
        return Err("no outer skin: not a sheet".into());
    }
    // The base: the largest flange. Orient the bends from it.
    let base = (0..flanges.len())
        .max_by(|a, b| flanges[*a].area.total_cmp(&flanges[*b].area))
        .unwrap_or(0);
    let mut seen = vec![false; flanges.len()];
    if !flanges.is_empty() {
        seen[base] = true;
    }
    let mut queue = vec![base];
    let mut order: Vec<usize> = Vec::new();
    while let Some(fl) = queue.first().copied() {
        queue.remove(0);
        for (bi, b) in bends.iter_mut().enumerate() {
            if order.contains(&bi) {
                continue;
            }
            let other = if b.flanges[0] == fl {
                Some(b.flanges[1])
            } else if b.flanges[1] == fl {
                Some(b.flanges[0])
            } else {
                None
            };
            if let Some(o) = other {
                b.flanges = [fl, o];
                order.push(bi);
                if !seen[o] {
                    seen[o] = true;
                    queue.push(o);
                }
            }
        }
    }
    // Bends unreachable from the base keep their order after the rest.
    for bi in 0..bends.len() {
        if !order.contains(&bi) {
            order.push(bi);
        }
    }
    let mut numbered: Vec<Bend> = Vec::with_capacity(bends.len());
    for (k, bi) in order.iter().enumerate() {
        let mut b = bends[*bi].clone();
        b.nr = k + 1;
        numbered.push(b);
    }
    Ok(Analysis {
        flanges,
        bends: numbered,
        base,
        thickness,
        rolls,
    })
}

/// The front view's frame: looking at the base flange's outer skin, the
/// direction of greatest extent to the right.
pub fn front_basis(solid: &Solid, a: &Analysis) -> Basis {
    let d = if let Some(f) = a.flanges.get(a.base) {
        f.n
    } else {
        a.rolls[0].1
    };
    let mut candidates: Vec<(P3, usize)> = Vec::new();
    let push = |v: P3, w: usize, cands: &mut Vec<(P3, usize)>| {
        let v = sub(v, mul(d, dot(v, d)));
        if norm(v) < 1e-6 {
            return;
        }
        let v = unit(v);
        for c in cands.iter_mut() {
            if dot(c.0, v).abs() > 0.9999 {
                c.1 += w;
                return;
            }
        }
        cands.push((v, w));
    };
    for b in &a.bends {
        push(b.axis, 1, &mut candidates);
    }
    if let Some(Some(e)) = a.flanges.get(a.base).map(|f| f.long_edge) {
        push(e, 0, &mut candidates);
    }
    if candidates.is_empty() {
        push(crate::step::writer::any_perp(d), 0, &mut candidates);
    }
    let extent = |v: P3| -> f64 {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in &solid.vertices {
            let x = dot(*p, v);
            lo = lo.min(x);
            hi = hi.max(x);
        }
        hi - lo
    };
    let mut best = candidates[0];
    let mut best_ext = extent(best.0);
    for c in &candidates[1..] {
        let e = extent(c.0);
        if e > best_ext * 1.01 || ((e - best_ext).abs() <= best_ext * 0.01 && c.1 > best.1) {
            best = *c;
            best_ext = e;
        }
    }
    // Sign: along the world axis it leans to.
    let mut r = best.0;
    let k = (0..3)
        .max_by(|i, j| r[*i].abs().total_cmp(&r[*j].abs()))
        .unwrap();
    if r[k] < 0.0 {
        r = mul(r, -1.0);
    }
    Basis::new(d, r)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Centre,
    /// Placed right of the front view: the view from the left.
    Right,
    Left,
    /// Placed below: the view from above.
    Below,
    Above,
    Iso,
}

impl Place {
    fn name(self) -> &'static str {
        match self {
            Place::Centre => "front",
            Place::Right => "left",
            Place::Left => "right",
            Place::Below => "top",
            Place::Above => "bottom",
            Place::Iso => "isometric",
        }
    }
}

/// An angle dimension in a view, in view millimetres.
#[derive(Clone, Debug)]
pub struct AngleDim {
    pub bend: usize,
    pub corner: P2,
    /// Unit direction of the parent flange's line past the corner.
    pub a: P2,
    /// Unit direction of the child flange's line from the corner.
    pub c: P2,
    pub value: f64,
    pub text: String,
    /// Paper radius of the arc and the text's lower-left, relative to the
    /// view origin.
    pub radius: f64,
    pub at: P2,
    pub box_: [f64; 4],
}

#[derive(Clone, Debug)]
pub struct BendTag {
    pub nr: usize,
    /// View point on the convex arc the leader starts at.
    pub on_arc: P2,
    /// Unit direction the tag moves out along (the bend's outward
    /// bisector), or none for the direction away from the view's centre.
    pub dir: Option<P2>,
    /// Paper centre of the tag circle relative to the view origin.
    pub at: P2,
    /// Paper centre of an earlier tag the leader runs from instead of
    /// the arc, when it would cross that tag: the tags then chain.
    pub from: Option<P2>,
}

/// One view on the sheet.
pub struct ViewPlan {
    pub place: Place,
    pub proj: Projection,
    pub feat: Features,
    pub angles: Vec<AngleDim>,
    pub tags: Vec<BendTag>,
    pub layout: Option<Layout>,
    /// Paper margins [left, bottom, right, top] beyond the scaled view.
    pub margin: [f64; 4],
}

impl ViewPlan {
    fn w(&self) -> f64 {
        self.feat.max[0]
    }
    fn h(&self) -> f64 {
        self.feat.max[1]
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewReport {
    pub name: String,
    pub r: P3,
    pub u: P3,
    pub d: P3,
    /// World point of the view's model origin (its lower-left corner).
    pub origin_world: P3,
    pub origin: [f64; 2],
    pub scale_factor: f64,
    pub size: [f64; 2],
    pub angles: Vec<AngleReport>,
    pub tags: Vec<usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AngleReport {
    pub bend: usize,
    pub value: f64,
    pub text: String,
}

fn canon(v: P3) -> (P3, f64) {
    let k = (0..3)
        .max_by(|i, j| v[*i].abs().total_cmp(&v[*j].abs()))
        .unwrap();
    if v[k] < 0.0 {
        (mul(v, -1.0), -1.0)
    } else {
        (v, 1.0)
    }
}

/// Values already dimensioned along each world axis.
#[derive(Default)]
struct Registry {
    axes: Vec<(P3, Vec<f64>)>,
}

impl Registry {
    fn claim(&mut self, axis: P3, world: f64) -> bool {
        let (c, sign) = canon(axis);
        let w = world * sign;
        let entry = match self.axes.iter().position(|(a, _)| dot(*a, c) > 0.9999) {
            Some(i) => &mut self.axes[i].1,
            None => {
                self.axes.push((c, Vec::new()));
                &mut self.axes.last_mut().unwrap().1
            }
        };
        if entry.iter().any(|v| (v - w).abs() < SAME) {
            return false;
        }
        entry.push(w);
        true
    }

    /// Whether a value already dimensioned on this axis lies a sheet
    /// thickness or a bend radius from `world`, farther from `centre`:
    /// the outer line of a pair, which makes `world` derived.
    fn shadowed(&self, axis: P3, world: f64, centre: f64, t: f64, derived: &[f64]) -> bool {
        let (c, sign) = canon(axis);
        let (w, cw) = (world * sign, centre * sign);
        self.axes
            .iter()
            .find(|(a, _)| dot(*a, c) > 0.9999)
            .is_some_and(|(_, vals)| {
                vals.iter().any(|v| {
                    let d = (v - w).abs();
                    d > SAME
                        && (d <= t + SAME || derived.iter().any(|x| (d - x).abs() <= SAME))
                        && (v - cw).abs() > (w - cw).abs()
                })
            })
    }
}

/// Distance from `p` to the segment `a`–`b`.
fn seg_point_dist(a: P2, b: P2, p: P2) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let t = if l2 < 1e-12 {
        0.0
    } else {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0)
    };
    dist(p, [a[0] + d[0] * t, a[1] + d[1] * t])
}

fn dist(a: P2, b: P2) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn minor_bound(solid: &Solid) -> f64 {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &solid.vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let ext = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
    (0.02 * ext).clamp(MINOR_MIN, MINOR_MAX)
}

/// Which vertices may carry a dimension: sharp (no smooth edge), not
/// minor (no adjacent sharp edge shorter than the bound, the edges
/// across the sheet's thickness aside).
fn dimensionable(solid: &Solid, topo: &Topology, t: f64) -> (Vec<bool>, usize) {
    let bound = minor_bound(solid);
    let mut minor = 0;
    let v = (0..solid.vertices.len())
        .map(|vi| {
            let edges = &topo.edges_of_vertex[vi];
            if edges.is_empty() || !topo.on_skin[vi] {
                return false;
            }
            if edges.iter().any(|&e| topo.smooth[e]) || topo.tangent_junction[vi] {
                return false;
            }
            if edges
                .iter()
                .any(|&e| topo.chord[e] < bound && (topo.chord[e] - t).abs() > SAME)
            {
                minor += 1;
                return false;
            }
            if edges.iter().any(|&e| narrow_edge(solid, topo, e, bound)) {
                minor += 1;
                return false;
            }
            true
        })
        .collect();
    (v, minor)
}

/// Whether straight edge `e` has another straight edge of some other
/// face running alongside it closer than `bound` over at least half its
/// length: the side of a relief slit or a sliver between two flanges.
fn narrow_edge(solid: &Solid, topo: &Topology, e: usize, bound: f64) -> bool {
    let line = |k: usize| {
        matches!(
            solid.curves.get(solid.edges[k].curve),
            Some(crate::view::Curve::Line)
        )
    };
    if !line(e) || topo.chord[e] < SAME {
        return false;
    }
    let (a, b) = (
        solid.vertices[solid.edges[e].start],
        solid.vertices[solid.edges[e].end],
    );
    let l = topo.chord[e];
    let u = unit(sub(b, a));
    let faces = &topo.faces_of_edge[e];
    (0..solid.edges.len()).any(|k| {
        if k == e || !line(k) || topo.chord[k] < SAME {
            return false;
        }
        if topo.faces_of_edge[k].iter().any(|f| faces.contains(f)) {
            return false;
        }
        let (c, d) = (
            solid.vertices[solid.edges[k].start],
            solid.vertices[solid.edges[k].end],
        );
        if dot(unit(sub(d, c)), u).abs() < 0.996 {
            return false;
        }
        let w = sub(c, a);
        let along = dot(w, u);
        let off = norm(sub(w, mul(u, along)));
        if off >= bound {
            return false;
        }
        let t1 = dot(sub(d, a), u);
        let (lo, hi) = (along.min(t1).max(0.0), along.max(t1).min(l));
        hi - lo >= 0.5 * l
    })
}

/// Loops of a face-on skin face's holes, in view coordinates.
fn hole_loops(
    solid: &Solid,
    fi: usize,
    proj: &Projection,
    shift: &impl Fn(P2) -> P2,
) -> Vec<Vec<Seg>> {
    let f = &solid.faces[fi];
    let b = proj.basis;
    let mut out = Vec::new();
    for lp in f.loops.iter().skip(1) {
        let mut segs: Vec<Seg> = Vec::new();
        for le in lp {
            let Some(e) = solid.edges.get(le.edge) else {
                continue;
            };
            let (first, last) = if le.forward {
                (e.start, e.end)
            } else {
                (e.end, e.start)
            };
            match solid.curves.get(e.curve) {
                Some(crate::view::Curve::Circle { .. }) => {
                    if let Some(pc) = proj.circles.iter().find(|c| c.edge == le.edge) {
                        if pc.full {
                            segs.push(Seg::Circle {
                                c: shift(pc.c),
                                r: pc.r,
                            });
                        } else {
                            segs.push(Seg::Arc {
                                c: shift(pc.c),
                                r: pc.r,
                                start: pc.start,
                                end: pc.end,
                            });
                        }
                    }
                }
                Some(crate::view::Curve::Bspline { .. }) => {
                    let sm = sample_edge(solid, le.edge, 1.0);
                    let pts: Vec<P2> = sm.pts.iter().map(|p| shift(b.xy(*p))).collect();
                    let pts = if le.forward {
                        pts
                    } else {
                        pts.into_iter().rev().collect()
                    };
                    for w in pts.windows(2) {
                        segs.push(Seg::Line { a: w[0], b: w[1] });
                    }
                }
                _ => segs.push(Seg::Line {
                    a: shift(b.xy(solid.vertices[first])),
                    b: shift(b.xy(solid.vertices[last])),
                }),
            }
        }
        if !segs.is_empty() {
            out.push(segs);
        }
    }
    out
}

/// Build the sheet's views and the geometry each dimensions.
fn plan_views(
    solid: &Solid,
    topo: &Topology,
    a: &Analysis,
    front: Basis,
) -> (Vec<ViewPlan>, usize) {
    let (ok, mut minor) = dimensionable(solid, topo, a.thickness);
    let t = a.thickness;
    // Offsets a bend puts between a flange's face and the next line.
    let derived: Vec<f64> = a
        .bends
        .iter()
        .flat_map(|bd| [bd.r_outer, bd.r_inner])
        .collect();
    let bases = |place: Place| -> Basis {
        let (r, u, d) = (front.r, front.u, front.d);
        match place {
            Place::Centre => front,
            Place::Right => Basis {
                r: d,
                u,
                d: mul(r, -1.0),
            },
            Place::Left => Basis {
                r: mul(d, -1.0),
                u,
                d: r,
            },
            Place::Below => Basis {
                r,
                u: mul(d, -1.0),
                d: u,
            },
            Place::Above => Basis {
                r,
                u: d,
                d: mul(u, -1.0),
            },
            Place::Iso => {
                let dd = unit(add(add(d, r), u));
                Basis::new(dd, unit(sub(r, mul(dd, dot(r, dd)))))
            }
        }
    };
    let candidates = [
        Place::Centre,
        Place::Right,
        Place::Left,
        Place::Below,
        Place::Above,
    ];
    let projs: Vec<Projection> = candidates
        .iter()
        .map(|p| project(solid, topo, bases(*p), t))
        .collect();
    let bends_facing = |b: &Basis| -> usize {
        a.bends
            .iter()
            .filter(|bd| dot(bd.axis, b.d).abs() > 0.9999)
            .count()
    };
    let holes_facing = |p: &Projection| -> usize {
        coincident_circles(&p.circles, SAME)
            .iter()
            .filter(|g| g.iter().any(|&i| p.circles[i].full && p.circles[i].planar))
            .filter(|g| g.iter().any(|&i| p.circles[i].visible > 0.5))
            .count()
    };
    let mut chosen: Vec<usize> = vec![0];
    for pair in [[1usize, 2], [3, 4]] {
        let score = |i: usize| -> (usize, usize) {
            (bends_facing(&projs[i].basis), holes_facing(&projs[i]))
        };
        let (s0, s1) = (score(pair[0]), score(pair[1]));
        if s0.0 + s0.1 + s1.0 + s1.1 == 0 {
            continue;
        }
        chosen.push(if s1.1 > s0.1 { pair[1] } else { pair[0] });
    }
    if chosen.len() == 1 {
        chosen.push(1);
    }

    let mut registry = Registry::default();
    let mut views: Vec<ViewPlan> = Vec::new();
    let mut tagged: Vec<bool> = vec![false; a.bends.len()];
    for &ci in &chosen {
        let place = candidates[ci];
        let proj = projs[ci].clone();
        let min = proj.min;
        let max = [proj.max[0] - proj.min[0], proj.max[1] - proj.min[1]];
        let shift = |p: P2| [p[0] - min[0], p[1] - min[1]];
        let shift_seg = |s: &Seg| match *s {
            Seg::Line { a, b } => Seg::Line {
                a: shift(a),
                b: shift(b),
            },
            Seg::Arc { c, r, start, end } => Seg::Arc {
                c: shift(c),
                r,
                start,
                end,
            },
            Seg::Circle { c, r } => Seg::Circle { c: shift(c), r },
        };
        let b = proj.basis;

        // The blank the features come from: the outline, then the holes
        // of every skin face that faces the viewer and is seen.
        let mut loops: Vec<Vec<Seg>> = Vec::new();
        let outline: Vec<Seg> = proj.outline.iter().map(shift_seg).collect();
        if outline.len() >= 3 || matches!(outline.as_slice(), [Seg::Circle { .. }]) {
            loops.push(outline);
        } else {
            loops.push(vec![
                Seg::Line {
                    a: [0.0, 0.0],
                    b: [max[0], 0.0],
                },
                Seg::Line {
                    a: [max[0], 0.0],
                    b: max,
                },
                Seg::Line {
                    a: max,
                    b: [0.0, max[1]],
                },
                Seg::Line {
                    a: [0.0, max[1]],
                    b: [0.0, 0.0],
                },
            ]);
        }
        for (fi, f) in solid.faces.iter().enumerate() {
            if f.role != "outerSkin" && f.role != "innerSkin" {
                continue;
            }
            let Some(Surface::Plane { normal, .. }) = solid.surfaces.get(f.surface) else {
                continue;
            };
            let n = unit(*normal);
            let n = if f.same_sense { n } else { mul(n, -1.0) };
            if dot(n, b.d) < 0.9999 || f.loops.len() < 2 {
                continue;
            }
            // Seen: most of its outer loop's vertices are visible.
            let (mut seen, mut total) = (0, 0);
            for le in &f.loops[0] {
                if let Some(e) = solid.edges.get(le.edge) {
                    total += 1;
                    if proj.verts[e.start].visible {
                        seen += 1;
                    }
                }
            }
            if seen * 2 < total {
                continue;
            }
            loops.extend(hole_loops(solid, fi, &proj, &shift));
        }
        // A roll seen along its axis: every full circle on it is a
        // diameter, the outline's own aside.
        for (origin, axis) in &a.rolls {
            if dot(*axis, b.d).abs() < 0.9999 {
                continue;
            }
            let c0 = shift(b.xy(*origin));
            let mut radii: Vec<f64> = Vec::new();
            for pc in &proj.circles {
                if !pc.full || pc.visible < 0.5 || dist(shift(pc.c), c0) > SAME {
                    continue;
                }
                let outline_r = match loops.first().map(|l| l.as_slice()) {
                    Some([Seg::Circle { r, .. }]) => (*r - pc.r).abs() <= SAME,
                    _ => false,
                };
                if outline_r || radii.iter().any(|r| (r - pc.r).abs() <= SAME) {
                    continue;
                }
                radii.push(pc.r);
            }
            for r in radii {
                loops.push(vec![Seg::Circle { c: c0, r }]);
            }
        }
        let blank = crate::dxf::blank::Blank {
            loops,
            bends: vec![],
            min: [0.0, 0.0],
            max,
        };
        let mut feat = super::features::features(&blank);
        minor += feat.minor;
        // Bend arcs carry no radius callout: the table has them.
        feat.arcs.retain(|arc| {
            !a.bends.iter().any(|bd| {
                let c = shift(b.xy(bd.origin));
                dist(c, arc.c) < SAME
                    && ((arc.r - bd.r_outer).abs() < SAME || (arc.r - bd.r_inner).abs() < SAME)
            })
        });
        feat.loops = proj
            .visible
            .iter()
            .chain(proj.smooth.iter())
            .chain(proj.hidden.iter())
            .map(|s| vec![shift_seg(s)])
            .collect();
        feat.max = max;
        for axis in 0..2 {
            let axis_w = if axis == 0 { b.r } else { b.u };
            let ords = if axis == 0 {
                &mut feat.xs
            } else {
                &mut feat.ys
            };
            // Across the sheet's thickness, or a bend radius away from
            // a flange's face, only the outer line counts.
            let mid = max[axis] / 2.0;
            let values: Vec<f64> = ords.iter().map(|o| o.value).collect();
            ords.retain(|o| {
                let v = o.value;
                let extent = o.sources.iter().any(|s| s == "extent");
                extent
                    || !values.iter().any(|w| {
                        let d = (w - v).abs();
                        d > SAME
                            && (d <= t + SAME || derived.iter().any(|x| (d - x).abs() <= SAME))
                            && (w - mid).abs() > (v - mid).abs()
                    })
            });
            ords.retain(|o| registry.claim(axis_w, o.value + min[axis]));
        }

        // Bend angles and tags where the bend's axis points at the viewer.
        let mut angles = Vec::new();
        let mut tags = Vec::new();
        for (bi, bd) in a.bends.iter().enumerate() {
            if dot(bd.axis, b.d).abs() < 0.9999 {
                continue;
            }
            let [pi, ci] = bd.flanges;
            let (pf, cf) = (&a.flanges[pi], &a.flanges[ci]);
            let offset = if bd.up { t } else { 0.0 };
            let line = |f: &Flange| -> (P2, P2) {
                let o = sub(f.origin, mul(f.n, offset));
                let dir = unit(cross(b.d, f.n));
                let q = shift(b.xy(o));
                let d2 = b.xy(dir);
                (q, unit2(d2))
            };
            let (pq, pd) = line(pf);
            let (cq, cd) = line(cf);
            let det = pd[0] * cd[1] - pd[1] * cd[0];
            if det.abs() < 1e-9 {
                continue;
            }
            let s = ((cq[0] - pq[0]) * cd[1] - (cq[1] - pq[1]) * cd[0]) / det;
            let corner = [pq[0] + pd[0] * s, pq[1] + pd[1] * s];
            let along = |q: P2, d: P2, p: P2| -> P2 {
                let k = (p[0] - q[0]) * d[0] + (p[1] - q[1]) * d[1];
                [q[0] + d[0] * k, q[1] + d[1] * k]
            };
            let pc = along(pq, pd, shift(b.xy(pf.centre)));
            let cc = along(cq, cd, shift(b.xy(cf.centre)));
            let a2 = unit2([corner[0] - pc[0], corner[1] - pc[1]]);
            let c2 = unit2([cc[0] - corner[0], cc[1] - corner[1]]);
            let value = (a2[0] * c2[0] + a2[1] * c2[1])
                .clamp(-1.0, 1.0)
                .acos()
                .to_degrees();
            let arc_c = shift(b.xy(bd.origin));
            let rc = bd.r_outer.max(bd.r_inner);
            let bis3 = add(pf.n, cf.n);
            let bis3 = if bd.up { mul(bis3, -1.0) } else { bis3 };
            let bis = unit2(b.xy(bis3));
            tags.push(BendTag {
                from: None,
                nr: bd.nr,
                on_arc: [arc_c[0] + bis[0] * rc, arc_c[1] + bis[1] * rc],
                dir: Some(bis),
                at: [0.0, 0.0],
            });
            tagged[bi] = true;
            if (value - 90.0).abs() > 0.05 && value > 0.05 {
                angles.push(AngleDim {
                    bend: bd.nr,
                    corner,
                    a: a2,
                    c: c2,
                    value,
                    text: text::deg(value),
                    radius: ANGLE_R,
                    at: [0.0, 0.0],
                    box_: [0.0; 4],
                });
            }
        }
        // A side view that adds nothing beyond the front view is dropped.
        let adds = place == Place::Centre
            || !feat.xs.is_empty()
            || !feat.ys.is_empty()
            || !feat.circles.is_empty()
            || !feat.arcs.is_empty()
            || !angles.is_empty();
        if !adds {
            for tg in &tags {
                if let Some(bi) = a.bends.iter().position(|bd| bd.nr == tg.nr) {
                    tagged[bi] = false;
                }
            }
            continue;
        }
        views.push(ViewPlan {
            place,
            proj,
            feat,
            angles,
            tags,
            layout: None,
            margin: [0.0; 4],
        });
    }

    // Every dimensionable vertex is located somewhere: the ones the
    // outlines and holes missed go to the first view that shows them,
    // hidden if need be. Across the thickness, the outer of a pair.
    let centre: P3 = {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in &solid.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ]
    };
    for (vi, p) in solid.vertices.iter().enumerate() {
        if !ok[vi] {
            continue;
        }
        let mut done: Vec<P3> = Vec::new();
        for v in views.iter_mut() {
            let b = v.proj.basis;
            let min = v.proj.min;
            for axis in 0..2 {
                let axis_w = if axis == 0 { b.r } else { b.u };
                let (c, _) = canon(axis_w);
                if done.iter().any(|d| dot(*d, c) > 0.9999) {
                    continue;
                }
                done.push(c);
                let value = dot(*p, c);
                let twin_farther = solid.vertices.iter().enumerate().any(|(wi, w)| {
                    wi != vi
                        && ok[wi]
                        && norm(sub(*w, *p)) <= t + SAME
                        && (dot(*w, c) - value).abs() > SAME
                        && (dot(*w, c) - dot(centre, c)).abs() > (value - dot(centre, c)).abs()
                });
                let world = dot(*p, axis_w);
                if twin_farther
                    || registry.shadowed(axis_w, world, dot(centre, axis_w), t, &derived)
                    || !registry.claim(axis_w, world)
                {
                    continue;
                }
                let q = b.xy(*p);
                let local = [q[0] - min[0], q[1] - min[1]];
                let ords = if axis == 0 {
                    &mut v.feat.xs
                } else {
                    &mut v.feat.ys
                };
                ords.push(Ordinate {
                    value: local[axis],
                    sources: vec![format!("vertex:{vi}")],
                    points: vec![local],
                });
                ords.sort_by(|p, q| p.value.total_cmp(&q.value));
            }
        }
    }

    // Bends no side view tagged get their tag in the front view, on the
    // middle of the bend's convex skin.
    for (bi, bd) in a.bends.iter().enumerate() {
        if tagged[bi] {
            continue;
        }
        let v = &mut views[0];
        let b = v.proj.basis;
        let min = v.proj.min;
        let [pi, ci] = bd.flanges;
        let bis3 = add(a.flanges[pi].n, a.flanges[ci].n);
        let bis3 = if bd.up { mul(bis3, -1.0) } else { bis3 };
        let bis3 = unit(bis3);
        let rc = bd.r_outer.max(bd.r_inner);
        // Middle of the zone along its axis, from the vertices of its face.
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for f in &solid.faces {
            if f.zone_id != bd.zone {
                continue;
            }
            for le in f.loops.iter().flatten() {
                if let Some(e) = solid.edges.get(le.edge) {
                    let h = dot(sub(solid.vertices[e.start], bd.origin), bd.axis);
                    lo = lo.min(h);
                    hi = hi.max(h);
                }
            }
        }
        if !lo.is_finite() {
            continue;
        }
        let mid = add(bd.origin, mul(bd.axis, (lo + hi) / 2.0));
        let on = add(mid, mul(bis3, rc));
        let q = b.xy(on);
        let d2 = b.xy(bis3);
        let dir = if (d2[0] * d2[0] + d2[1] * d2[1]).sqrt() > 0.3 {
            Some(unit2(d2))
        } else {
            None
        };
        v.tags.push(BendTag {
            from: None,
            nr: bd.nr,
            on_arc: [q[0] - min[0], q[1] - min[1]],
            dir,
            at: [0.0, 0.0],
        });
    }

    // The isometric, without dimensions.
    let proj = project(solid, topo, bases(Place::Iso), t);
    let mut feat = Features {
        max: [proj.max[0] - proj.min[0], proj.max[1] - proj.min[1]],
        ..Features::default()
    };
    let min = proj.min;
    for s in proj.visible.iter().chain(proj.smooth.iter()) {
        feat.loops.push(vec![match *s {
            Seg::Line { a, b } => Seg::Line {
                a: [a[0] - min[0], a[1] - min[1]],
                b: [b[0] - min[0], b[1] - min[1]],
            },
            Seg::Arc { c, r, start, end } => Seg::Arc {
                c: [c[0] - min[0], c[1] - min[1]],
                r,
                start,
                end,
            },
            Seg::Circle { c, r } => Seg::Circle {
                c: [c[0] - min[0], c[1] - min[1]],
                r,
            },
        }]);
    }
    views.push(ViewPlan {
        place: Place::Iso,
        proj,
        feat,
        angles: vec![],
        tags: vec![],
        layout: None,
        margin: [2.0; 4],
    });
    (views, minor)
}

fn unit2(v: P2) -> P2 {
    let l = (v[0] * v[0] + v[1] * v[1]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l]
}

/// Place the angle texts and bend tags of a view at scale `s`, extending
/// its margins; `lo` carries the running dimensions.
fn place_extras(v: &mut ViewPlan, s: f64) {
    let lo = v.layout.as_ref().unwrap();
    let mut obstacles: Vec<P2> = Vec::new();
    for seg in v.feat.loops.iter().flatten() {
        sample(seg, s, &mut obstacles);
    }
    for l in &lo.xs {
        sample_line([l.p, l.from[1] * s], [l.p, -dims::D], &mut obstacles);
    }
    for l in &lo.ys {
        sample_line([l.from[0] * s, l.p], [-dims::D, l.p], &mut obstacles);
    }
    let mut boxes: Vec<[f64; 4]> = dims::label_boxes(lo);
    boxes.extend(lo.callouts.iter().map(|c| {
        let tw = font::width(&c.text, H);
        [
            c.at[0] - 0.8,
            c.at[1] - 0.8,
            c.at[0] + tw + 0.8,
            c.at[1] + H + 0.8,
        ]
    }));
    let hit = |bx: [f64; 4], obstacles: &[P2], boxes: &[[f64; 4]]| -> bool {
        obstacles
            .iter()
            .any(|p| p[0] >= bx[0] && p[0] <= bx[2] && p[1] >= bx[1] && p[1] <= bx[3])
            || boxes
                .iter()
                .any(|b| bx[0] < b[2] && b[0] < bx[2] && bx[1] < b[3] && b[1] < bx[3])
    };
    let w = v.feat.max[0] * s;
    let h = v.feat.max[1] * s;
    let mut margin = lo.margin;
    for ang in &mut v.angles {
        let corner = [ang.corner[0] * s, ang.corner[1] * s];
        let bis = unit2([ang.a[0] + ang.c[0], ang.a[1] + ang.c[1]]);
        let tw = font::width(&ang.text, H);
        let mut r = ANGLE_R;
        for _ in 0..8 {
            let centre = [
                corner[0] + bis[0] * (r + 2.5 + tw / 2.0),
                corner[1] + bis[1] * (r + 2.5 + H / 2.0),
            ];
            let at = [centre[0] - tw / 2.0, centre[1] - H / 2.0];
            let bx = [at[0] - 0.8, at[1] - 0.8, at[0] + tw + 0.8, at[1] + H + 0.8];
            if !hit(bx, &obstacles, &boxes) {
                ang.at = at;
                ang.box_ = bx;
                break;
            }
            ang.at = at;
            ang.box_ = bx;
            r += 4.0;
        }
        ang.radius = r;
        boxes.push(ang.box_);
        // The arc and the extension line are obstacles too.
        let n = 24;
        let a0 = ang.a[1].atan2(ang.a[0]);
        let mut sweep = ang.c[1].atan2(ang.c[0]) - a0;
        while sweep > std::f64::consts::PI {
            sweep -= std::f64::consts::TAU;
        }
        while sweep < -std::f64::consts::PI {
            sweep += std::f64::consts::TAU;
        }
        for i in 0..=n {
            let t = a0 + sweep * i as f64 / n as f64;
            obstacles.push([corner[0] + r * t.cos(), corner[1] + r * t.sin()]);
        }
        sample_line(
            corner,
            [
                corner[0] + ang.a[0] * (r + 2.0),
                corner[1] + ang.a[1] * (r + 2.0),
            ],
            &mut obstacles,
        );
        margin[0] = margin[0].max(1.0 - ang.box_[0]);
        margin[1] = margin[1].max(1.0 - ang.box_[1]);
        margin[2] = margin[2].max(ang.box_[2] + 1.0 - w);
        margin[3] = margin[3].max(ang.box_[3] + 1.0 - h);
    }
    let mut placed: Vec<P2> = Vec::new();
    for tag in &mut v.tags {
        let on = [tag.on_arc[0] * s, tag.on_arc[1] * s];
        let centre = [w / 2.0, h / 2.0];
        let bis = tag
            .dir
            .unwrap_or_else(|| unit2([on[0] - centre[0], on[1] - centre[1]]));
        let mut at = [
            on[0] + bis[0] * (TAG_R + 4.0),
            on[1] + bis[1] * (TAG_R + 4.0),
        ];
        for _ in 0..40 {
            let bx = [at[0] - TAG_R, at[1] - TAG_R, at[0] + TAG_R, at[1] + TAG_R];
            let clash =
                hit(bx, &obstacles, &boxes) || obstacles.iter().any(|p| dist(*p, at) < TAG_R + 0.4);
            if !clash {
                break;
            }
            at = [at[0] + bis[0] * 2.0, at[1] + bis[1] * 2.0];
        }
        tag.at = at;
        // A leader through an earlier tag is drawn from that tag instead.
        tag.from = placed
            .iter()
            .filter(|q| seg_point_dist(on, at, **q) < TAG_R + 0.3)
            .max_by(|p, q| dist(on, **p).total_cmp(&dist(on, **q)))
            .copied();
        placed.push(at);
        boxes.push([at[0] - TAG_R, at[1] - TAG_R, at[0] + TAG_R, at[1] + TAG_R]);
        margin[0] = margin[0].max(TAG_R + 1.0 - at[0]);
        margin[1] = margin[1].max(TAG_R + 1.0 - at[1]);
        margin[2] = margin[2].max(at[0] + TAG_R + 1.0 - w);
        margin[3] = margin[3].max(at[1] + TAG_R + 1.0 - h);
    }
    v.margin = margin;
}

pub struct Plan {
    pub paper: Paper,
    pub s: f64,
    pub info_h: f64,
    /// Paper origin per view (model (0, 0)), and the isometric's scale.
    pub origins: Vec<P2>,
    pub iso_scale: f64,
}

fn table_h(bends: usize) -> f64 {
    if bends == 0 {
        0.0
    } else {
        TABLE_ROW * (bends as f64 + 1.0) + 1.0
    }
}

/// Lay the views out in first-angle arrangement and pick sheet and
/// scale.
fn plan(
    views: &mut [ViewPlan],
    note_lines: usize,
    bends: usize,
    lang: Lang,
) -> Result<Plan, String> {
    let info_h = table_h(bends).max(note_lines as f64 * NOTE_PITCH + 2.0);
    let n = views.len();
    let iso = n - 1;
    for s in SCALES {
        for v in views.iter_mut().take(iso) {
            let lo = dims::layout(&v.feat, s, lang);
            v.layout = Some(lo);
            place_extras(v, s);
        }
        for paper in Paper::ALL {
            let [x0, y0, x1, y1] = paper.view_area(info_h);
            let (aw, ah) = (x1 - x0, y1 - y0);
            // Column and row of each view.
            let col = |p: Place| match p {
                Place::Left => 0,
                Place::Right => 2,
                _ => 1,
            };
            let row = |p: Place| match p {
                Place::Above => 0,
                Place::Below => 2,
                _ => 1,
            };
            // Cell needs: width = left margin + w·s + right margin.
            let mut col_w = [0.0f64; 4];
            let mut row_h = [0.0f64; 4];
            let mut col_left = [0.0f64; 4];
            let mut row_bottom = [0.0f64; 4];
            let mut col_used = [false; 4];
            let mut row_used = [false; 4];
            for v in views[..iso].iter() {
                let (c, r) = (col(v.place), row(v.place));
                col_used[c] = true;
                row_used[r] = true;
                col_left[c] = col_left[c].max(v.margin[0]);
                row_bottom[r] = row_bottom[r].max(v.margin[1]);
            }
            for v in views[..iso].iter() {
                let (c, r) = (col(v.place), row(v.place));
                col_w[c] = col_w[c].max(col_left[c] + v.w() * s + v.margin[2]);
                row_h[r] = row_h[r].max(row_bottom[r] + v.h() * s + v.margin[3]);
            }
            // The isometric: a free corner, else a column of its own.
            let iso_v = &views[iso];
            let iso_sizes = |sc: f64| (iso_v.w() * sc + 4.0, iso_v.h() * sc + 4.0 + ISO_TITLE_H);
            let mut iso_cell: Option<(usize, usize)> = None;
            for (r, c) in [(2usize, 2usize), (0, 2), (2, 0), (0, 0)] {
                if row_used[r] && col_used[c] {
                    iso_cell = Some((r, c));
                    break;
                }
            }
            // The isometric wants half the main scale; a corner takes it
            // when it fits, else the corner's column and row grow to it,
            // else it shrinks to the corner.
            let target = SCALES
                .iter()
                .copied()
                .find(|sc| *sc <= s / 2.0 + 1e-9)
                .unwrap_or(s);
            let fits_in = |sc: f64, r: usize, c: usize, cw: &[f64; 4], rh: &[f64; 4]| {
                let (iw, ih) = iso_sizes(sc);
                iw <= cw[c] && ih <= rh[r]
            };
            let total = |cw: &[f64; 4], rh: &[f64; 4], cu: &[bool; 4], ru: &[bool; 4]| {
                let w: f64 = (0..3).filter(|c| cu[*c]).map(|c| cw[c]).sum::<f64>()
                    + GAP * (cu[..3].iter().filter(|u| **u).count() as f64 - 1.0);
                let h: f64 = (0..3).filter(|r| ru[*r]).map(|r| rh[r]).sum::<f64>()
                    + GAP * (ru[..3].iter().filter(|u| **u).count() as f64 - 1.0);
                (w, h)
            };
            let (iso_r, iso_c) = match iso_cell {
                Some(rc) => rc,
                None => {
                    if col_used[2] {
                        (2, 2)
                    } else {
                        (1, 2)
                    }
                }
            };
            let mut iso_scale = 0.0;
            if iso_cell.is_some() && fits_in(target, iso_r, iso_c, &col_w, &row_h) {
                iso_scale = target;
            } else {
                let (iw, ih) = iso_sizes(target);
                let mut cw = col_w;
                let mut rh = row_h;
                let mut cu = col_used;
                let mut ru = row_used;
                cu[iso_c] = true;
                ru[iso_r] = true;
                cw[iso_c] = cw[iso_c].max(iw);
                rh[iso_r] = rh[iso_r].max(ih);
                let (w, h) = total(&cw, &rh, &cu, &ru);
                if w <= aw && h <= ah {
                    iso_scale = target;
                    col_w = cw;
                    row_h = rh;
                    col_used = cu;
                    row_used = ru;
                } else if iso_cell.is_some() {
                    for sc in SCALES.iter().copied().filter(|sc| *sc < target) {
                        if fits_in(sc, iso_r, iso_c, &col_w, &row_h) {
                            iso_scale = sc;
                            break;
                        }
                    }
                } else {
                    for sc in SCALES.iter().copied().filter(|sc| *sc < target) {
                        let (iw, ih) = iso_sizes(sc);
                        let mut cw = col_w;
                        let mut rh = row_h;
                        cw[iso_c] = cw[iso_c].max(iw);
                        rh[iso_r] = rh[iso_r].max(ih);
                        let (w, h) = total(&cw, &rh, &cu, &ru);
                        if w <= aw && h <= ah {
                            iso_scale = sc;
                            col_w = cw;
                            row_h = rh;
                            col_used = cu;
                            row_used = ru;
                            break;
                        }
                    }
                }
            }
            let need_w: f64 = (0..3)
                .filter(|c| col_used[*c])
                .map(|c| col_w[c])
                .sum::<f64>()
                + GAP * (col_used[..3].iter().filter(|u| **u).count() as f64 - 1.0);
            let need_h: f64 = (0..3)
                .filter(|r| row_used[*r])
                .map(|r| row_h[r])
                .sum::<f64>()
                + GAP * (row_used[..3].iter().filter(|u| **u).count() as f64 - 1.0);
            if need_w > aw || need_h > ah {
                continue;
            }
            // Positions: columns left to right, rows top to bottom.
            let mut col_x = [0.0f64; 4];
            let mut x = x0 + (aw - need_w) / 2.0;
            for c in 0..3 {
                if col_used[c] {
                    col_x[c] = x;
                    x += col_w[c] + GAP;
                }
            }
            let mut row_y = [0.0f64; 4];
            let mut y = y1 - (ah - need_h) / 2.0;
            for r in 0..3 {
                if row_used[r] {
                    row_y[r] = y - row_h[r];
                    y -= row_h[r] + GAP;
                }
            }
            let mut origins = Vec::with_capacity(n);
            for v in views[..iso].iter() {
                let (c, r) = (col(v.place), row(v.place));
                origins.push([col_x[c] + col_left[c], row_y[r] + row_bottom[r]]);
            }
            // The isometric centred in its cell.
            let (iw, ih) = iso_sizes(iso_scale);
            origins.push([
                col_x[iso_c] + (col_w[iso_c] - iw) / 2.0 + 2.0,
                row_y[iso_r] + (row_h[iso_r] - ih) / 2.0 + 2.0,
            ]);
            return Ok(Plan {
                paper,
                s,
                info_h,
                origins,
                iso_scale,
            });
        }
    }
    Err("the views fit no sheet, not even A3 at 1:100".into())
}

fn notes(a: &Analysis, minor: usize, info: &SheetInfo, lang: Lang, bbox: P3) -> Vec<String> {
    let mut v = Vec::new();
    let mut mat = String::new();
    if !info.material.is_empty() {
        mat.push_str(info.material);
    }
    if info.thickness > 0.0 {
        if !mat.is_empty() {
            mat.push_str(", ");
        }
        mat.push_str(&format!("t = {}", text::mm(info.thickness)));
    }
    if !mat.is_empty() {
        v.push(format!("{}: {mat}", lang.t("material")));
    }
    v.push(format!(
        "{}: {} × {} × {}",
        lang.t("size"),
        text::mm(bbox[0]),
        text::mm(bbox[1]),
        text::mm(bbox[2])
    ));
    v.push(lang.t("outside_note").to_string());
    if !a.bends.is_empty() {
        let r0 = a.bends[0].r_inner.min(a.bends[0].r_outer);
        if a.bends
            .iter()
            .all(|b| (b.r_inner.min(b.r_outer) - r0).abs() < SAME)
        {
            v.push(format!("{} R{}", lang.t("radius_note"), text::mm(r0)));
        }
        v.push(lang.t("angle_note").to_string());
    }
    if minor > 0 {
        v.push(lang.t("minor_note_folded").to_string());
    }
    v.push(format!("{}. {}.", lang.t("mm"), lang.t("deburr")));
    v
}

/// Draw the sheet of one folded part into `doc` and describe it.
pub fn sheet(
    doc: &mut Doc,
    solid: &Solid,
    info: &SheetInfo,
    meta: &Meta,
    lang: Lang,
) -> Result<SheetReport, String> {
    let topo = topology(solid);
    let a = analyse(solid, &topo, info.thickness)?;
    let front = front_basis(solid, &a);
    let (mut views, minor) = plan_views(solid, &topo, &a, front);
    // Overall size along the front frame.
    let mut bbox = [0.0; 3];
    for (k, ax) in [front.r, front.u, front.d].iter().enumerate() {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in &solid.vertices {
            lo = lo.min(dot(*p, *ax));
            hi = hi.max(dot(*p, *ax));
        }
        bbox[k] = hi - lo;
    }
    // Bend zones reach past the vertices by their radius; the front view's
    // extents are exact.
    bbox[0] = views[0].feat.max[0];
    bbox[1] = views[0].feat.max[1];
    if let Some(v) = views
        .iter()
        .find(|v| v.place == Place::Right || v.place == Place::Left)
    {
        bbox[2] = v.feat.max[0];
    } else if let Some(v) = views
        .iter()
        .find(|v| v.place == Place::Below || v.place == Place::Above)
    {
        bbox[2] = v.feat.max[1];
    }
    let note_lines = notes(&a, minor, info, lang, bbox);
    let p = plan(&mut views, note_lines.len(), a.bends.len(), lang)?;
    let paper = p.paper;
    let s = p.s;
    let mut c = Content::new();
    c.caps_butt_joins_round();
    c.fill_gray(0.0);

    // Title block values.
    let v = |c: &mut Content, key: &str, h: f64, t: &str| sheet::value(c, paper, key, h, t);
    v(&mut c, "title", 5.0, info.title);
    let id = if meta.number.is_empty() {
        format!("P-{:03}", info.index)
    } else {
        format!("{}-{:03}", meta.number, info.index)
    };
    v(&mut c, "id", 5.0, &id);
    v(&mut c, "created", 2.5, &meta.created_by);
    v(&mut c, "approved", 2.5, &meta.approved_by);
    v(&mut c, "date", 2.5, &meta.date);
    v(&mut c, "material", 2.5, info.material);
    if info.thickness > 0.0 {
        v(
            &mut c,
            "thickness",
            2.5,
            &format!("{} mm", text::mm(info.thickness)),
        );
    }
    v(&mut c, "quantity", 2.5, &format!("{}", info.part_ids.len()));
    v(&mut c, "doctype", 2.5, lang.t("detail"));
    v(&mut c, "status", 2.5, &meta.status);
    v(&mut c, "revision", 2.5, &meta.revision);
    let scale_text = text::scale(s);
    v(&mut c, "scale", 3.5, &scale_text);
    v(
        &mut c,
        "sheet",
        3.5,
        &format!("{} / {}", info.index, info.total),
    );
    v(&mut c, "format", 3.5, paper.name());
    v(&mut c, "tolerances", 2.5, &meta.tolerances);
    v(
        &mut c,
        "blank",
        3.5,
        &format!(
            "{} × {} × {}",
            text::mm(bbox[0]),
            text::mm(bbox[1]),
            text::mm(bbox[2])
        ),
    );

    // Notes and bend table in the info strip.
    let (sx, sy, sw) = paper.info_strip();
    let th = table_h(a.bends.len());
    let tw: f64 = TABLE_COLS.iter().sum();
    let note_w = if th > 0.0 { sw - tw - 4.0 } else { sw };
    let mut y = sy + p.info_h - 1.0 - 2.5;
    for line in &note_lines {
        let (t, h) = sheet::shrink_to(line, 2.5, note_w - 2.0);
        c.text([sx + 1.0, y], h, 0.0, &t);
        y -= NOTE_PITCH;
    }
    let mut bend_reports = Vec::new();
    if th > 0.0 {
        let tx = sx + sw - tw;
        let ty = sy + p.info_h - th;
        c.width(NARROW);
        c.rect_stroke(tx, ty, tw, th - 1.0);
        let heads = ["bend", "angle", "direction", "inside_r"];
        let mut row_y = ty + th - 1.0 - TABLE_ROW;
        let mut x = tx;
        c.line([tx, row_y], [tx + tw, row_y]);
        for (i, k) in heads.iter().enumerate() {
            let (t, h) = sheet::shrink_to(lang.t(k), 2.5, TABLE_COLS[i] - 1.5);
            c.text([x + 0.8, row_y + 1.5], h, 0.0, &t);
            x += TABLE_COLS[i];
            if i + 1 < heads.len() {
                c.line([x, ty], [x, ty + th - 1.0]);
            }
        }
        for (i, b) in a.bends.iter().enumerate() {
            row_y -= TABLE_ROW;
            if i + 1 < a.bends.len() {
                c.line([tx, row_y], [tx + tw, row_y]);
            }
            let dir = if b.up { lang.t("up") } else { lang.t("down") };
            let r_in = b.r_inner.min(b.r_outer);
            let cells = [
                b.nr.to_string(),
                text::deg(b.angle_deg),
                dir.to_string(),
                text::mm(r_in),
            ];
            let mut x = tx;
            for (j, t) in cells.iter().enumerate() {
                c.text([x + 0.8, row_y + 1.5], 2.5, 0.0, t);
                x += TABLE_COLS[j];
            }
            bend_reports.push(BendReport {
                nr: b.nr,
                id: b.zone.clone(),
                angle_deg: b.angle_deg,
                direction: if b.up { "up" } else { "down" }.into(),
                inside_radius: r_in,
                allowance: 0.0,
            });
        }
    }

    // The views.
    let mut view_reports = Vec::new();
    let mut dimensions = Vec::new();
    let iso = views.len() - 1;
    for (i, view) in views.iter().enumerate() {
        let o = p.origins[i];
        let sv = if i == iso { p.iso_scale } else { s };
        if i == iso && sv <= 0.0 {
            continue;
        }
        let pm = |pt: P2| [o[0] + pt[0] * sv, o[1] + pt[1] * sv];
        let min = view.proj.min;
        let shift = |pt: P2| [pt[0] - min[0], pt[1] - min[1]];
        let draw = |c: &mut Content, segs: &[Seg]| {
            // Runs of lines end to end go out as one polyline, thinned to
            // what the paper can show.
            let mut run: Vec<P2> = Vec::new();
            let flush = |c: &mut Content, run: &mut Vec<P2>| {
                if run.len() >= 2 {
                    let t = super::project::thin(run, 0.04);
                    if t.len() >= 2 {
                        c.polyline(&t, false);
                    }
                }
                run.clear();
            };
            for seg in segs {
                match *seg {
                    Seg::Line { a, b } => {
                        let (pa, pb) = (pm(shift(a)), pm(shift(b)));
                        if dist(pa, pb) <= 0.02 {
                            continue;
                        }
                        if run.last().is_some_and(|l| dist(*l, pa) > 1e-6) {
                            flush(c, &mut run);
                        }
                        if run.is_empty() {
                            run.push(pa);
                        }
                        run.push(pb);
                        continue;
                    }
                    _ => flush(c, &mut run),
                }
                match *seg {
                    Seg::Line { .. } => {}
                    Seg::Arc {
                        c: cc,
                        r,
                        start,
                        end,
                    } => c.arc(pm(shift(cc)), r * sv, start, end),
                    Seg::Circle { c: cc, r } => c.circle(pm(shift(cc)), r * sv),
                }
            }
            flush(c, &mut run);
        };
        if i != iso {
            c.width(NARROW);
            c.dash(&[3.0, 1.0], 0.0);
            draw(&mut c, &view.proj.hidden);
            c.solid();
        }
        c.width(NARROW);
        draw(&mut c, &view.proj.smooth);
        c.width(if i == iso { NARROW } else { WIDE });
        draw(&mut c, &view.proj.visible);
        if i == iso {
            // Title with the scale when it differs.
            let title = if (sv - s).abs() > 1e-9 {
                format!("{} {}", lang.t("isometric"), text::scale(sv))
            } else {
                lang.t("isometric").to_string()
            };
            c.text([o[0], o[1] - 2.0 - 3.5], 3.5, 0.0, &title);
            view_reports.push(ViewReport {
                name: Place::Iso.name().into(),
                r: view.proj.basis.r,
                u: view.proj.basis.u,
                d: view.proj.basis.d,
                origin_world: view.proj.basis.world(min),
                origin: o,
                scale_factor: sv,
                size: view.feat.max,
                angles: vec![],
                tags: vec![],
            });
            continue;
        }
        let lo = view.layout.as_ref().unwrap();
        dims::draw(lo, &view.feat, &mut c, o);
        // Angle dimensions.
        c.width(NARROW);
        let mut angle_reports = Vec::new();
        for ang in &view.angles {
            let corner = pm(ang.corner);
            let r = ang.radius;
            c.line(
                corner,
                [
                    corner[0] + ang.a[0] * (r + 2.0),
                    corner[1] + ang.a[1] * (r + 2.0),
                ],
            );
            let a0 = ang.a[1].atan2(ang.a[0]).to_degrees();
            let a1 = ang.c[1].atan2(ang.c[0]).to_degrees();
            let mut sweep = a1 - a0;
            while sweep > 180.0 {
                sweep -= 360.0;
            }
            while sweep < -180.0 {
                sweep += 360.0;
            }
            let (start, end) = if sweep >= 0.0 { (a0, a1) } else { (a1, a0) };
            c.arc(corner, r, start, end);
            // Arrowheads tangent to the arc at both ends, pointing out.
            for (ang_deg, ccw) in [(start, false), (end, true)] {
                let t = ang_deg.to_radians();
                let tip = [corner[0] + r * t.cos(), corner[1] + r * t.sin()];
                let tangent = if ccw {
                    [-t.sin(), t.cos()]
                } else {
                    [t.sin(), -t.cos()]
                };
                arrow(&mut c, tip, tangent);
            }
            c.text([o[0] + ang.at[0], o[1] + ang.at[1]], H, 0.0, &ang.text);
            angle_reports.push(AngleReport {
                bend: ang.bend,
                value: ang.value,
                text: ang.text.clone(),
            });
            dimensions.push(DimReport {
                kind: "angle".into(),
                axis: None,
                value: ang.value,
                text: ang.text.clone(),
                count: None,
                sources: vec![format!("bend:{}", ang.bend)],
            });
        }
        // Bend tags.
        let mut tag_nrs = Vec::new();
        for tag in &view.tags {
            let at = [o[0] + tag.at[0], o[1] + tag.at[1]];
            let (on, gap) = match tag.from {
                Some(f) => ([o[0] + f[0], o[1] + f[1]], TAG_R),
                None => (pm(tag.on_arc), 0.0),
            };
            let d = [at[0] - on[0], at[1] - on[1]];
            let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
            if l > TAG_R + gap + 0.6 {
                let u = [d[0] / l, d[1] / l];
                c.line(
                    [on[0] + u[0] * gap, on[1] + u[1] * gap],
                    [at[0] - u[0] * TAG_R, at[1] - u[1] * TAG_R],
                );
            }
            c.circle(at, TAG_R);
            c.text_centred(at[0], at[1] - H / 2.0, H, &tag.nr.to_string());
            tag_nrs.push(tag.nr);
        }
        for (axis, labels) in [("x", &lo.xs), ("y", &lo.ys)] {
            for l in labels {
                dimensions.push(DimReport {
                    kind: "ordinate".into(),
                    axis: Some(format!("{}:{axis}", view.place.name())),
                    value: l.value,
                    text: l.text.clone(),
                    count: None,
                    sources: l.sources.clone(),
                });
            }
        }
        for co in &lo.callouts {
            dimensions.push(DimReport {
                kind: co.kind.into(),
                axis: Some(view.place.name().into()),
                value: co.value,
                text: co.text.clone(),
                count: Some(co.count),
                sources: vec![],
            });
        }
        view_reports.push(ViewReport {
            name: view.place.name().into(),
            r: view.proj.basis.r,
            u: view.proj.basis.u,
            d: view.proj.basis.d,
            origin_world: view.proj.basis.world(min),
            origin: o,
            scale_factor: sv,
            size: view.feat.max,
            angles: angle_reports,
            tags: tag_nrs,
        });
    }

    let (w, h) = paper.size();
    let label = format!("Pos {}", info.index);
    doc.add_page(w, h, &[paper.form_name()], c, &label);
    Ok(SheetReport {
        index: info.index,
        label,
        title: info.title.to_string(),
        part_ids: info.part_ids.to_vec(),
        quantity: info.part_ids.len(),
        paper: paper.name().into(),
        scale: scale_text,
        scale_factor: s,
        origin: p.origins[0],
        blank: [bbox[0], bbox[1]],
        material: info.material.to_string(),
        thickness: info.thickness,
        dimensions,
        bends: bend_reports,
        kind: "folded".into(),
        size: Some(bbox),
        views: view_reports,
        minor_bound: Some(minor_bound(solid)),
    })
}
