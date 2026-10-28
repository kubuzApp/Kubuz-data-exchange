//! Orthographic views of a solid with hidden lines found: every B-rep edge
//! projected along a view direction, split into the stretches that face
//! the viewer and the stretches behind other faces.
//!
//! Visibility comes from a depth buffer: each face is filled at a fine
//! pixel pitch with its depth, then every edge is sampled at half that
//! pitch and each sample compared with the buffer. A sample that is no
//! deeper than the nearest surface at its pixel (within a tolerance for
//! the face's slope) is visible. Edges along which two faces meet
//! smoothly (bend zones against flanges) are "smooth" and drawn thin;
//! the silhouette generators of cylinders are added where the view grazes
//! them. Hidden stretches that coincide with a visible line are dropped.
//!
//! The result is 2D geometry in view coordinates, where `x` runs along
//! the basis' `r`, `y` along `u`, and `d` points at the viewer.

use crate::dxf::blank::{Seg, P2};
use crate::step::writer::{any_perp, cross, sub, unit};
use crate::view::{Curve, Solid, Surface, P3};

pub fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn scale(a: P3, k: f64) -> P3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

pub fn norm(v: P3) -> f64 {
    dot(v, v).sqrt()
}

fn dist2(a: P2, b: P2) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Two faces meet smoothly when their normals agree within this cosine
/// (about 2.5°).
const SMOOTH_COS: f64 = 0.999;
/// Base depth tolerance in mm, plus the face's slope over a pixel.
const TOL_BASE: f64 = 0.3;
const MAX_SLOPE: f64 = 10.0;
/// Chord error of sampled curves on paper is well below this.
const THIN: f64 = 0.02;

/// A right-handed view frame: `r` to the right, `u` up, `d` at the viewer.
#[derive(Clone, Copy, Debug)]
pub struct Basis {
    pub r: P3,
    pub u: P3,
    pub d: P3,
}

impl Basis {
    /// The frame that looks along `-d` with `r_hint` (made perpendicular
    /// to `d`) to the right.
    pub fn new(d: P3, r_hint: P3) -> Basis {
        let d = unit(d);
        let r = unit(sub(r_hint, scale(d, dot(r_hint, d))));
        let u = cross(d, r);
        Basis { r, u, d }
    }

    pub fn xy(&self, p: P3) -> P2 {
        [dot(p, self.r), dot(p, self.u)]
    }

    /// Distance from the viewer: smaller is nearer.
    pub fn depth(&self, p: P3) -> f64 {
        -dot(p, self.d)
    }

    /// The world point of a view point at depth 0.
    pub fn world(&self, p: P2) -> P3 {
        add(scale(self.r, p[0]), scale(self.u, p[1]))
    }
}

/// What every view shares: which faces use an edge, which edges are
/// smooth, and every vertex's edges.
pub struct Topology {
    pub faces_of_edge: Vec<Vec<usize>>,
    pub smooth: Vec<bool>,
    pub edges_of_vertex: Vec<Vec<usize>>,
    /// Straight-line distance between an edge's ends.
    pub chord: Vec<f64>,
    /// Whether the vertex lies on any skin face (every vertex of a sheet
    /// does; a mesh solid's need not).
    pub on_skin: Vec<bool>,
    /// Whether two of the vertex's edges run through it with one tangent
    /// (the end of a fillet): such a vertex is no corner.
    pub tangent_junction: Vec<bool>,
}

pub fn topology(solid: &Solid) -> Topology {
    let ne = solid.edges.len();
    let mut faces_of_edge = vec![Vec::new(); ne];
    let mut edges_of_vertex = vec![Vec::new(); solid.vertices.len()];
    let mut on_skin = vec![false; solid.vertices.len()];
    for (fi, f) in solid.faces.iter().enumerate() {
        let skin = f.role == "outerSkin" || f.role == "innerSkin";
        for lp in &f.loops {
            for le in lp {
                if le.edge < ne {
                    faces_of_edge[le.edge].push(fi);
                    let e = solid.edges[le.edge];
                    for v in [e.start, e.end] {
                        if v < on_skin.len() {
                            if !edges_of_vertex[v].contains(&le.edge) {
                                edges_of_vertex[v].push(le.edge);
                            }
                            if skin {
                                on_skin[v] = true;
                            }
                        }
                    }
                }
            }
        }
    }
    let mut smooth = vec![false; ne];
    let mut chord = vec![0.0; ne];
    for (ei, e) in solid.edges.iter().enumerate() {
        let (Some(a), Some(b)) = (solid.vertices.get(e.start), solid.vertices.get(e.end)) else {
            continue;
        };
        chord[ei] = norm(sub(*b, *a));
        let fs = &faces_of_edge[ei];
        if fs.len() == 2 {
            let m = sample_edge(solid, ei, 1.0).mid();
            let n1 = face_normal(solid, fs[0], m);
            let n2 = face_normal(solid, fs[1], m);
            smooth[ei] = dot(n1, n2).abs() > SMOOTH_COS;
        } else if fs.len() == 1 && fs[0] < solid.faces.len() {
            // Used twice by one face (a seam): smooth.
            smooth[ei] = true;
        }
    }
    let coarse: Vec<Sampled> = (0..ne).map(|ei| sample_edge(solid, ei, 1.0)).collect();
    let tangent_junction = (0..solid.vertices.len())
        .map(|vi| {
            let es = &edges_of_vertex[vi];
            (0..es.len())
                .any(|a| (a + 1..es.len()).any(|b| tangent_at(solid, &coarse, es[a], vi, es[b])))
        })
        .collect();
    Topology {
        faces_of_edge,
        smooth,
        edges_of_vertex,
        chord,
        on_skin,
        tangent_junction,
    }
}

/// The surface normal of face `fi` at (or near) `p`, sign free.
pub fn face_normal(solid: &Solid, fi: usize, p: P3) -> P3 {
    match solid.surfaces.get(solid.faces[fi].surface) {
        Some(Surface::Plane { normal, .. }) => unit(*normal),
        Some(Surface::Cylinder { origin, axis, .. }) => {
            let a = unit(*axis);
            let v = sub(p, *origin);
            let rad = sub(v, scale(a, dot(v, a)));
            if norm(rad) < 1e-9 {
                any_perp(a)
            } else {
                unit(rad)
            }
        }
        Some(Surface::RuledBspline { top, bottom }) => {
            // The normal at the nearest of a few rulings.
            let (Some(t), Some(b)) = (solid.curves.get(*top), solid.curves.get(*bottom)) else {
                return [0.0, 0.0, 1.0];
            };
            let n: usize = 16;
            let mut best = (f64::INFINITY, [0.0, 0.0, 1.0]);
            for i in 0..=n {
                let s = i as f64 / n as f64;
                let pt = eval_curve(t, s);
                let pb = eval_curve(b, s);
                let s2 = ((i + 1).min(n)) as f64 / n as f64;
                let s1 = (i.saturating_sub(1)) as f64 / n as f64;
                let tangent = sub(eval_curve(t, s2), eval_curve(t, s1));
                let ruling = sub(pb, pt);
                let nn = cross(tangent, ruling);
                if norm(nn) < 1e-12 {
                    continue;
                }
                let dist = norm(sub(p, pt)).min(norm(sub(p, pb)));
                if dist < best.0 {
                    best = (dist, unit(nn));
                }
            }
            best.1
        }
        None => [0.0, 0.0, 1.0],
    }
}

/// A curve at parameter `s` in `[0, 1]` of its own range.
fn eval_curve(c: &Curve, s: f64) -> P3 {
    match c {
        Curve::Bspline {
            degree,
            control_points,
            knots,
            multiplicities,
        } => bspline(*degree, control_points, knots, multiplicities, s),
        _ => [0.0, 0.0, 0.0],
    }
}

/// De Boor on a B-spline with its knots given with multiplicities; `s` is
/// the fraction of the valid parameter range.
pub fn bspline(degree: usize, cp: &[P3], knots: &[f64], mults: &[usize], s: f64) -> P3 {
    let mut u: Vec<f64> = Vec::new();
    for (k, m) in knots.iter().zip(mults) {
        for _ in 0..*m {
            u.push(*k);
        }
    }
    let p = degree;
    let n = cp.len();
    if n == 0 {
        return [0.0, 0.0, 0.0];
    }
    if u.len() < n + p + 1 {
        // Malformed: fall back to the control polygon.
        let t = s * (n - 1) as f64;
        let i = (t.floor() as usize).min(n - 1);
        let j = (i + 1).min(n - 1);
        let f = t - i as f64;
        return add(scale(cp[i], 1.0 - f), scale(cp[j], f));
    }
    let lo = u[p];
    let hi = u[n];
    let t = lo + (hi - lo) * s.clamp(0.0, 1.0);
    // Knot span k with u[k] <= t < u[k+1], the last span closed at hi.
    let mut k = p;
    while k + 1 < n && t >= u[k + 1] {
        k += 1;
    }
    let mut d: Vec<P3> = (0..=p).map(|j| cp[j + k - p]).collect();
    for r in 1..=p {
        for j in (r..=p).rev() {
            let i = j + k - p;
            let den = u[i + p - r + 1] - u[i];
            let a = if den.abs() < 1e-15 {
                0.0
            } else {
                (t - u[i]) / den
            };
            d[j] = add(scale(d[j - 1], 1.0 - a), scale(d[j], a));
        }
    }
    d[p]
}

/// The kind of curve an edge was sampled from.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Line,
    /// Centre, unit axis, radius; the edge runs counter-clockwise about
    /// the axis.
    Circle {
        c: P3,
        axis: P3,
        r: f64,
    },
    Spline,
}

pub struct Sampled {
    pub pts: Vec<P3>,
    pub kind: Kind,
}

impl Sampled {
    fn mid(&self) -> P3 {
        self.pts[self.pts.len() / 2]
    }
}

/// Points along edge `ei` from its start to its end, about `step` apart
/// (curves also keep their chord error under [`THIN`]).
pub fn sample_edge(solid: &Solid, ei: usize, step: f64) -> Sampled {
    let e = solid.edges[ei];
    let a = solid.vertices[e.start];
    let b = solid.vertices[e.end];
    match solid.curves.get(e.curve) {
        Some(Curve::Circle {
            center,
            axis,
            radius,
        }) => {
            let ax = unit(*axis);
            let va = sub(a, *center);
            let e1 = {
                let rad = sub(va, scale(ax, dot(va, ax)));
                if norm(rad) < 1e-9 {
                    any_perp(ax)
                } else {
                    unit(rad)
                }
            };
            let e2 = cross(ax, e1);
            let vb = sub(b, *center);
            let mut sweep = dot(vb, e2).atan2(dot(vb, e1));
            if e.start == e.end || (sweep.abs() < 1e-9 && norm(sub(b, a)) < 1e-6) {
                sweep = std::f64::consts::TAU;
            } else if sweep <= 0.0 {
                sweep += std::f64::consts::TAU;
            }
            let r = *radius;
            let len = r * sweep;
            let by_chord = if r > THIN {
                sweep / (2.0 * (1.0 - THIN / r).acos())
            } else {
                1.0
            };
            let n = ((len / step).max(by_chord).ceil() as usize).max(4);
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = sweep * i as f64 / n as f64;
                pts.push(add(
                    *center,
                    add(scale(e1, r * t.cos()), scale(e2, r * t.sin())),
                ));
            }
            // Land exactly on the vertices.
            pts[0] = a;
            if e.start != e.end {
                pts[n] = b;
            }
            Sampled {
                pts,
                kind: Kind::Circle {
                    c: *center,
                    axis: ax,
                    r,
                },
            }
        }
        Some(Curve::Bspline {
            degree,
            control_points,
            knots,
            multiplicities,
        }) => {
            let mut poly = 0.0;
            for w in control_points.windows(2) {
                poly += norm(sub(w[1], w[0]));
            }
            let n = ((poly / step).ceil() as usize).clamp(8, 4000);
            let mut pts = Vec::with_capacity(n + 1);
            for i in 0..=n {
                pts.push(bspline(
                    *degree,
                    control_points,
                    knots,
                    multiplicities,
                    i as f64 / n as f64,
                ));
            }
            pts[0] = a;
            pts[n] = b;
            Sampled {
                pts,
                kind: Kind::Spline,
            }
        }
        _ => {
            let n = ((norm(sub(b, a)) / step).ceil() as usize).max(1);
            let pts = (0..=n)
                .map(|i| {
                    let t = i as f64 / n as f64;
                    add(scale(a, 1.0 - t), scale(b, t))
                })
                .collect();
            Sampled {
                pts,
                kind: Kind::Line,
            }
        }
    }
}

/// A depth buffer over the view's bounding box.
struct Buffer {
    x0: f64,
    y0: f64,
    px: f64,
    w: usize,
    h: usize,
    depth: Vec<f32>,
    mask: Vec<bool>,
}

impl Buffer {
    fn new(min: P2, max: P2) -> Buffer {
        let area = (max[0] - min[0]).max(1.0) * (max[1] - min[1]).max(1.0);
        let px = (area / 2.5e6).sqrt().clamp(0.1, 1.0);
        let w = ((max[0] - min[0]) / px).ceil() as usize + 4;
        let h = ((max[1] - min[1]) / px).ceil() as usize + 4;
        Buffer {
            x0: min[0] - 2.0 * px,
            y0: min[1] - 2.0 * px,
            px,
            w,
            h,
            depth: vec![f32::INFINITY; w * h],
            mask: vec![false; w * h],
        }
    }

    fn pixel(&self, p: P2) -> Option<(usize, usize)> {
        let i = ((p[0] - self.x0) / self.px).floor();
        let j = ((p[1] - self.y0) / self.px).floor();
        if i < 0.0 || j < 0.0 || i >= self.w as f64 || j >= self.h as f64 {
            None
        } else {
            Some((i as usize, j as usize))
        }
    }

    fn at(&self, p: P2) -> f64 {
        match self.pixel(p) {
            Some((i, j)) => f64::from(self.depth[j * self.w + i]),
            None => f64::INFINITY,
        }
    }

    fn masked(&self, p: P2) -> bool {
        match self.pixel(p) {
            Some((i, j)) => self.mask[j * self.w + i],
            None => false,
        }
    }

    fn mark(&mut self, p: P2) {
        if let Some((i, j)) = self.pixel(p) {
            for dj in -1i64..=1 {
                for di in -1i64..=1 {
                    let (ii, jj) = (i as i64 + di, j as i64 + dj);
                    if ii >= 0 && jj >= 0 && (ii as usize) < self.w && (jj as usize) < self.h {
                        self.mask[jj as usize * self.w + ii as usize] = true;
                    }
                }
            }
        }
    }

    /// Fill the even-odd interior of `loops` with the depth plane
    /// `depth = a·x + b·y + c`, clamped to `[lo, hi]`.
    fn fill(&mut self, loops: &[Vec<P2>], plane: [f64; 3], lo: f64, hi: f64) {
        // Edge table: (ymin, ymax, x at ymin, dx/dy).
        let mut edges: Vec<[f64; 4]> = Vec::new();
        for lp in loops {
            let n = lp.len();
            if n < 3 {
                continue;
            }
            for i in 0..n {
                let (p, q) = (lp[i], lp[(i + 1) % n]);
                if (p[1] - q[1]).abs() < 1e-12 {
                    continue;
                }
                let (lo_p, hi_p) = if p[1] < q[1] { (p, q) } else { (q, p) };
                edges.push([
                    lo_p[1],
                    hi_p[1],
                    lo_p[0],
                    (hi_p[0] - lo_p[0]) / (hi_p[1] - lo_p[1]),
                ]);
            }
        }
        if edges.is_empty() {
            return;
        }
        edges.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let ymin = edges[0][0];
        let ymax = edges.iter().map(|e| e[1]).fold(f64::MIN, f64::max);
        let j0 = (((ymin - self.y0) / self.px - 0.5).ceil().max(0.0)) as usize;
        let j1 = (((ymax - self.y0) / self.px - 0.5).floor()) as i64;
        if j1 < j0 as i64 {
            return;
        }
        let mut next = 0;
        let mut active: Vec<usize> = Vec::new();
        let mut xs: Vec<f64> = Vec::new();
        for j in j0..=(j1 as usize).min(self.h - 1) {
            let y = self.y0 + (j as f64 + 0.5) * self.px;
            while next < edges.len() && edges[next][0] <= y {
                active.push(next);
                next += 1;
            }
            active.retain(|&e| edges[e][1] > y);
            xs.clear();
            for &e in &active {
                let ed = edges[e];
                if ed[0] <= y {
                    xs.push(ed[2] + (y - ed[0]) * ed[3]);
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for pair in xs.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                let i0 = (((pair[0] - self.x0) / self.px - 0.5).ceil().max(0.0)) as usize;
                let i1 = (((pair[1] - self.x0) / self.px - 0.5).floor()) as i64;
                if i1 < i0 as i64 {
                    continue;
                }
                let row = j * self.w;
                for i in i0..=(i1 as usize).min(self.w - 1) {
                    let x = self.x0 + (i as f64 + 0.5) * self.px;
                    let z = (plane[0] * x + plane[1] * y + plane[2]).clamp(lo, hi) as f32;
                    let cell = &mut self.depth[row + i];
                    if z < *cell {
                        *cell = z;
                    }
                }
            }
        }
    }
}

/// The depth plane through three view points `(x, y, depth)`, or none
/// when they are collinear in the view.
fn depth_plane(p: [[f64; 3]; 3]) -> Option<[f64; 3]> {
    let [a, b, c] = p;
    let det = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
    if det.abs() < 1e-9 {
        return None;
    }
    let ka = ((b[2] - a[2]) * (c[1] - a[1]) - (c[2] - a[2]) * (b[1] - a[1])) / det;
    let kb = ((b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2])) / det;
    Some([ka, kb, a[2] - ka * a[0] - kb * a[1]])
}

/// Sutherland–Hodgman clip of a polygon to `lo <= v[axis] <= hi`.
fn clip_band(poly: &[P2], axis: usize, lo: f64, hi: f64) -> Vec<P2> {
    let a = clip_half(poly, axis, lo, true);
    clip_half(&a, axis, hi, false)
}

/// Keep the side of `v[axis] = at` that is greater (or smaller).
fn clip_half(poly: &[P2], axis: usize, at: f64, greater: bool) -> Vec<P2> {
    let keep = |p: P2| {
        if greater {
            p[axis] >= at
        } else {
            p[axis] <= at
        }
    };
    let cut = |p: P2, q: P2| -> P2 {
        let t = (at - p[axis]) / (q[axis] - p[axis]);
        [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]
    };
    let n = poly.len();
    let mut out = Vec::with_capacity(n + 2);
    for i in 0..n {
        let p = poly[i];
        let q = poly[(i + 1) % n];
        match (keep(p), keep(q)) {
            (true, true) => out.push(q),
            (true, false) => out.push(cut(p, q)),
            (false, true) => {
                out.push(cut(p, q));
                out.push(q);
            }
            (false, false) => {}
        }
    }
    out
}

/// Douglas–Peucker thinning of a polyline.
pub fn thin(pts: &[P2], eps: f64) -> Vec<P2> {
    if pts.len() <= 2 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let mut worst = (0.0, 0);
    for (i, p) in pts.iter().enumerate().skip(1).take(pts.len() - 2) {
        let dist = if l < 1e-12 {
            dist2(*p, a)
        } else {
            ((p[0] - a[0]) * d[1] - (p[1] - a[1]) * d[0]).abs() / l
        };
        if dist > worst.0 {
            worst = (dist, i);
        }
    }
    if worst.0 <= eps {
        return vec![a, b];
    }
    let mut left = thin(&pts[..=worst.1], eps);
    let right = thin(&pts[worst.1..], eps);
    left.pop();
    left.extend(right);
    left
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Visible,
    Hidden,
    Dropped,
}

/// A vertex in the view.
#[derive(Clone, Debug)]
pub struct PVert {
    pub p: P2,
    pub depth: f64,
    pub visible: bool,
}

/// A circle edge that lies in a plane facing the viewer, so it projects
/// true: in view coordinates, angles in degrees counter-clockwise.
#[derive(Clone, Copy, Debug)]
pub struct PCircle {
    pub edge: usize,
    pub c: P2,
    pub r: f64,
    pub full: bool,
    pub start: f64,
    pub end: f64,
    /// The share of the edge that is visible.
    pub visible: f64,
    /// Whether it bounds a planar face (a hole's rim, a tube's end).
    pub planar: bool,
    /// Whether it bounds a planar skin face (a hole or round in a flange).
    pub skin: bool,
    /// Whether both neighbours along its skin loop run into it tangent.
    pub fillet: bool,
    pub depth: f64,
}

#[derive(Clone, Debug)]
pub struct Projection {
    pub basis: Basis,
    pub min: P2,
    pub max: P2,
    /// Sharp visible edges: the outline, wide.
    pub visible: Vec<Seg>,
    /// Smooth visible edges (bend tangents), narrow.
    pub smooth: Vec<Seg>,
    /// Sharp hidden edges, dashed narrow.
    pub hidden: Vec<Seg>,
    pub verts: Vec<PVert>,
    pub circles: Vec<PCircle>,
    /// The silhouette's outer boundary as a closed counter-clockwise loop
    /// of exact lines and arcs, snapped to the projected geometry; empty
    /// when it could not be traced.
    pub outline: Vec<Seg>,
    /// Lines of every planar face seen edge-on: (point, unit direction).
    pub edge_lines: Vec<(P2, P2)>,
}

/// Depth slope of face `fi` at `p` in the view: how much deeper the
/// surface gets per millimetre across the paper, clamped.
fn slope(solid: &Solid, fi: usize, p: P3, basis: &Basis) -> f64 {
    match solid.surfaces.get(solid.faces[fi].surface) {
        Some(Surface::RuledBspline { .. }) => 3.0,
        _ => {
            let n = face_normal(solid, fi, p);
            let c = dot(n, basis.d).abs();
            ((1.0 - c * c).max(0.0).sqrt() / c.max(1e-3)).min(MAX_SLOPE)
        }
    }
}

/// Project `solid` along `basis`.
/// Project `solid` along `basis`. Hidden edges less than `thickness`
/// behind the surface in front of them (the inner skin behind the outer)
/// are left out.
pub fn project(solid: &Solid, topo: &Topology, basis: Basis, thickness: f64) -> Projection {
    // Coarse samples for the bounding box.
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let coarse: Vec<Sampled> = (0..solid.edges.len())
        .map(|ei| sample_edge(solid, ei, 2.0))
        .collect();
    for s in &coarse {
        for p in &s.pts {
            let q = basis.xy(*p);
            min[0] = min[0].min(q[0]);
            min[1] = min[1].min(q[1]);
            max[0] = max[0].max(q[0]);
            max[1] = max[1].max(q[1]);
        }
    }
    if !min[0].is_finite() {
        return Projection {
            basis,
            min: [0.0; 2],
            max: [0.0; 2],
            visible: vec![],
            smooth: vec![],
            hidden: vec![],
            verts: vec![],
            circles: vec![],
            outline: vec![],
            edge_lines: vec![],
        };
    }
    let mut buf = Buffer::new(min, max);
    let step = buf.px / 2.0;

    // Fine samples of every edge, in 3D and in the view.
    let fine: Vec<Sampled> = (0..solid.edges.len())
        .map(|ei| sample_edge(solid, ei, step))
        .collect();
    let fine2: Vec<Vec<P2>> = fine
        .iter()
        .map(|s| s.pts.iter().map(|p| basis.xy(*p)).collect())
        .collect();

    // Faces into the buffer.
    let mut silhouettes: Vec<Vec<P3>> = Vec::new();
    for f in &solid.faces {
        let loops2: Vec<Vec<P2>> = f
            .loops
            .iter()
            .map(|lp| {
                let mut v: Vec<P2> = Vec::new();
                for le in lp {
                    if le.edge >= fine2.len() {
                        continue;
                    }
                    let pts = &fine2[le.edge];
                    let n = pts.len();
                    if le.forward {
                        v.extend(pts[..n - 1].iter().copied());
                    } else {
                        v.extend(pts[1..].iter().rev().copied());
                    }
                }
                thin(&v, THIN / 2.0)
            })
            .collect();
        match solid.surfaces.get(f.surface) {
            Some(Surface::Plane { origin, normal }) => {
                let n = unit(*normal);
                let nd = dot(n, basis.d);
                if nd.abs() < 1e-6 {
                    continue;
                }
                // depth = -z, z = (n·o - x n·r - y n·u) / n·d
                let k = dot(n, *origin);
                let plane = [dot(n, basis.r) / nd, dot(n, basis.u) / nd, -k / nd];
                let (lo, hi) = face_depth_range(solid, f, &basis);
                buf.fill(&loops2, plane, lo, hi);
            }
            Some(Surface::Cylinder {
                origin,
                axis,
                radius,
            }) => {
                let ax = unit(*axis);
                let e1 = any_perp(ax);
                let e2 = cross(ax, e1);
                let r = *radius;
                let at = |th: f64, h: f64| -> P3 {
                    add(
                        *origin,
                        add(
                            scale(ax, h),
                            add(scale(e1, r * th.cos()), scale(e2, r * th.sin())),
                        ),
                    )
                };
                // Loops in (theta, h), unwrapped along each loop.
                let mut params: Vec<Vec<P2>> = Vec::new();
                for lp in &f.loops {
                    let mut v: Vec<P2> = Vec::new();
                    let mut prev: Option<f64> = None;
                    for le in lp {
                        if le.edge >= fine.len() {
                            continue;
                        }
                        let pts = &fine[le.edge].pts;
                        let n = pts.len();
                        let order: Vec<usize> = if le.forward {
                            (0..n - 1).collect()
                        } else {
                            (1..n).rev().collect()
                        };
                        for i in order {
                            let p = sub(pts[i], *origin);
                            let h = dot(p, ax);
                            let mut th = dot(p, e2).atan2(dot(p, e1));
                            if let Some(pv) = prev {
                                while th - pv > std::f64::consts::PI {
                                    th -= std::f64::consts::TAU;
                                }
                                while pv - th > std::f64::consts::PI {
                                    th += std::f64::consts::TAU;
                                }
                            }
                            prev = Some(th);
                            v.push([th, h]);
                        }
                    }
                    params.push(thin(&v, 1e-4));
                }
                let th_min = params
                    .iter()
                    .flatten()
                    .map(|p| p[0])
                    .fold(f64::INFINITY, f64::min);
                let th_max = params
                    .iter()
                    .flatten()
                    .map(|p| p[0])
                    .fold(f64::NEG_INFINITY, f64::max);
                if !th_min.is_finite() {
                    continue;
                }
                let dth = if r > THIN {
                    (2.0 * (1.0 - THIN / r).acos()).min(10f64.to_radians())
                } else {
                    10f64.to_radians()
                };
                let strips = (((th_max - th_min) / dth).ceil() as usize).max(1);
                let dth = (th_max - th_min) / strips as f64;
                for si in 0..strips {
                    let (a, b) = (th_min + dth * si as f64, th_min + dth * (si + 1) as f64);
                    let clipped: Vec<Vec<P2>> = params
                        .iter()
                        .map(|lp| clip_band(lp, 0, a, b))
                        .filter(|lp| lp.len() >= 3)
                        .collect();
                    if clipped.is_empty() {
                        continue;
                    }
                    let mut lo = f64::INFINITY;
                    let mut hi = f64::NEG_INFINITY;
                    let loops2: Vec<Vec<P2>> = clipped
                        .iter()
                        .map(|lp| {
                            lp.iter()
                                .map(|q| {
                                    let p = at(q[0], q[1]);
                                    let z = basis.depth(p);
                                    lo = lo.min(z);
                                    hi = hi.max(z);
                                    basis.xy(p)
                                })
                                .collect()
                        })
                        .collect();
                    let tri = [at(a, 0.0), at(b, 0.0), at(a, 1.0)].map(|p| {
                        let q = basis.xy(p);
                        [q[0], q[1], basis.depth(p)]
                    });
                    if let Some(plane) = depth_plane(tri) {
                        buf.fill(&loops2, plane, lo, hi);
                    }
                }
                // Silhouette generators where the view grazes the
                // cylinder, inside the face.
                if dot(ax, basis.d).abs() < 0.999 {
                    let th0 = (-dot(e1, basis.d)).atan2(dot(e2, basis.d));
                    for k in -2..=2 {
                        for half in [0.0, std::f64::consts::PI] {
                            let th = th0 + half + k as f64 * std::f64::consts::TAU;
                            if th <= th_min + 1e-3 || th >= th_max - 1e-3 {
                                continue;
                            }
                            // h intervals of the parameter polygon on theta = th.
                            let mut hs: Vec<f64> = Vec::new();
                            for lp in &params {
                                let n = lp.len();
                                for i in 0..n {
                                    let (p, q) = (lp[i], lp[(i + 1) % n]);
                                    if (p[0] <= th && th < q[0]) || (q[0] <= th && th < p[0]) {
                                        let t = (th - p[0]) / (q[0] - p[0]);
                                        hs.push(p[1] + (q[1] - p[1]) * t);
                                    }
                                }
                            }
                            hs.sort_by(|a, b| a.total_cmp(b));
                            for pair in hs.chunks(2) {
                                if pair.len() == 2 && pair[1] - pair[0] > 0.05 {
                                    let n = (((pair[1] - pair[0]) / step).ceil() as usize).max(1);
                                    silhouettes.push(
                                        (0..=n)
                                            .map(|i| {
                                                at(
                                                    th,
                                                    pair[0]
                                                        + (pair[1] - pair[0]) * i as f64 / n as f64,
                                                )
                                            })
                                            .collect(),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Some(Surface::RuledBspline { top, bottom }) => {
                let (Some(t), Some(b)) = (solid.curves.get(*top), solid.curves.get(*bottom)) else {
                    continue;
                };
                let n = 24;
                let pt: Vec<P3> = (0..=n)
                    .map(|i| eval_curve(t, i as f64 / n as f64))
                    .collect();
                let pb: Vec<P3> = (0..=n)
                    .map(|i| eval_curve(b, i as f64 / n as f64))
                    .collect();
                for i in 0..n {
                    for tri3 in [[pt[i], pt[i + 1], pb[i + 1]], [pt[i], pb[i + 1], pb[i]]] {
                        let v = tri3.map(|p| {
                            let q = basis.xy(p);
                            [q[0], q[1], basis.depth(p)]
                        });
                        if let Some(plane) = depth_plane(v) {
                            let lo = v.iter().map(|q| q[2]).fold(f64::INFINITY, f64::min);
                            let hi = v.iter().map(|q| q[2]).fold(f64::NEG_INFINITY, f64::max);
                            let poly: Vec<P2> = v.iter().map(|q| [q[0], q[1]]).collect();
                            buf.fill(&[poly], plane, lo, hi);
                        }
                    }
                }
            }
            None => {}
        }
    }

    // Classify every edge's samples.
    let mut states: Vec<Vec<State>> = Vec::with_capacity(fine.len());
    for (ei, s) in fine.iter().enumerate() {
        let faces = &topo.faces_of_edge[ei];
        let mut st = Vec::with_capacity(s.pts.len());
        for (i, p) in s.pts.iter().enumerate() {
            let sl = faces
                .iter()
                .map(|&fi| slope(solid, fi, *p, &basis))
                .fold(0.0, f64::max);
            let tol = TOL_BASE + 0.75 * buf.px * sl;
            let q = fine2[ei][i];
            let behind = basis.depth(*p) - buf.at(q);
            st.push(if behind <= tol {
                State::Visible
            } else if behind <= thickness + 0.1 {
                State::Dropped
            } else {
                State::Hidden
            });
        }
        smooth_states(&mut st);
        states.push(st);
    }
    let sil_states: Vec<Vec<State>> = silhouettes
        .iter()
        .map(|pts| {
            let tol = TOL_BASE + 0.75 * buf.px * MAX_SLOPE;
            let mut st: Vec<State> = pts
                .iter()
                .map(|p| {
                    if basis.depth(*p) - tol <= buf.at(basis.xy(*p)) {
                        State::Visible
                    } else {
                        State::Hidden
                    }
                })
                .collect();
            smooth_states(&mut st);
            st
        })
        .collect();
    // A face-on circle exactly behind another of the same centre and
    // radius (a tube's far rim) adds nothing: only the nearest is drawn.
    {
        let mut rims: Vec<(usize, P2, f64, f64)> = Vec::new();
        for (ei, e) in solid.edges.iter().enumerate() {
            if let Some(Curve::Circle {
                center,
                axis,
                radius,
            }) = solid.curves.get(e.curve)
            {
                if dot(unit(*axis), basis.d).abs() > 0.9999 && !fine[ei].pts.is_empty() {
                    rims.push((ei, basis.xy(*center), *radius, basis.depth(*center)));
                }
            }
        }
        for i in 0..rims.len() {
            let (ei, c, r, depth) = rims[i];
            let nearer = rims.iter().any(|&(ej, c2, r2, d2)| {
                ej != ei && dist2(c, c2) <= 0.05 && (r - r2).abs() <= 0.05 && d2 < depth - 0.05
            });
            if nearer {
                for st in states[ei].iter_mut() {
                    *st = State::Dropped;
                }
            }
        }
    }
    // Mask of the visible sharp lines; hidden and smooth samples under it
    // are dropped.
    for (ei, st) in states.iter().enumerate() {
        if topo.smooth[ei] {
            continue;
        }
        for (i, s) in st.iter().enumerate() {
            if *s == State::Visible {
                buf.mark(fine2[ei][i]);
            }
        }
    }
    for (k, st) in sil_states.iter().enumerate() {
        for (i, s) in st.iter().enumerate() {
            if *s == State::Visible {
                buf.mark(basis.xy(silhouettes[k][i]));
            }
        }
    }
    for (ei, st) in states.iter_mut().enumerate() {
        for (i, s) in st.iter_mut().enumerate() {
            if (*s == State::Hidden || topo.smooth[ei]) && buf.masked(fine2[ei][i]) {
                *s = State::Dropped;
            }
        }
        smooth_states(st);
    }

    // Runs into segments.
    let mut visible = Vec::new();
    let mut smooth = Vec::new();
    let mut hidden = Vec::new();
    let mut circles = Vec::new();
    for (ei, s) in fine.iter().enumerate() {
        let st = &states[ei];
        let pts2 = &fine2[ei];
        let true_circle = match s.kind {
            Kind::Circle { axis, .. } => dot(axis, basis.d).abs() > 0.999999,
            _ => false,
        };
        let edge_on_circle = match s.kind {
            Kind::Circle { axis, .. } => dot(axis, basis.d).abs() < 1e-6,
            _ => false,
        };
        let mut i = 0;
        while i < st.len() {
            let state = st[i];
            let mut j = i;
            while j + 1 < st.len() && st[j + 1] == state {
                j += 1;
            }
            let drawn = state == State::Visible || (state == State::Hidden && !topo.smooth[ei]);
            if drawn && j > i {
                let out = if topo.smooth[ei] {
                    &mut smooth
                } else if state == State::Visible {
                    &mut visible
                } else {
                    &mut hidden
                };
                match s.kind {
                    Kind::Line => out.push(Seg::Line {
                        a: pts2[i],
                        b: pts2[j],
                    }),
                    Kind::Circle { c, axis, r } if true_circle => {
                        let c2 = basis.xy(c);
                        let ccw = dot(axis, basis.d) > 0.0;
                        let ang = |p: P2| (p[1] - c2[1]).atan2(p[0] - c2[0]).to_degrees();
                        if i == 0
                            && j + 1 == st.len()
                            && solid.edges[ei].start == solid.edges[ei].end
                        {
                            out.push(Seg::Circle { c: c2, r });
                        } else {
                            let (a0, a1) = (ang(pts2[i]), ang(pts2[j]));
                            let (start, end) = if ccw { (a0, a1) } else { (a1, a0) };
                            out.push(Seg::Arc {
                                c: c2,
                                r,
                                start,
                                end,
                            });
                        }
                    }
                    _ if edge_on_circle => out.push(Seg::Line {
                        a: pts2[i],
                        b: pts2[j],
                    }),
                    _ => {
                        let t = thin(&pts2[i..=j], THIN);
                        for w in t.windows(2) {
                            if dist2(w[0], w[1]) > 1e-6 {
                                out.push(Seg::Line { a: w[0], b: w[1] });
                            }
                        }
                    }
                }
            }
            i = j + 1;
        }
        if let (Kind::Circle { c, axis, r }, true) = (s.kind, true_circle) {
            let c2 = basis.xy(c);
            let ccw = dot(axis, basis.d) > 0.0;
            let ang = |p: P2| (p[1] - c2[1]).atan2(p[0] - c2[0]).to_degrees();
            let n = pts2.len();
            let e = solid.edges[ei];
            let full = e.start == e.end;
            let (a0, a1) = (ang(pts2[0]), ang(pts2[n - 1]));
            let (start, end) = if ccw { (a0, a1) } else { (a1, a0) };
            let vis = st.iter().filter(|x| **x == State::Visible).count() as f64 / n as f64;
            let planar: Vec<usize> = topo.faces_of_edge[ei]
                .iter()
                .copied()
                .filter(|&fi| {
                    matches!(
                        solid.surfaces.get(solid.faces[fi].surface),
                        Some(Surface::Plane { .. })
                    )
                })
                .collect();
            let skin_faces: Vec<usize> = planar
                .iter()
                .copied()
                .filter(|&fi| {
                    let role = &solid.faces[fi].role;
                    role == "outerSkin" || role == "innerSkin"
                })
                .collect();
            let fillet = !full
                && skin_faces
                    .iter()
                    .any(|&fi| is_fillet(solid, topo, &fine, fi, ei));
            circles.push(PCircle {
                edge: ei,
                c: c2,
                r,
                full,
                start,
                end,
                visible: vis,
                planar: !planar.is_empty(),
                skin: !skin_faces.is_empty(),
                fillet,
                depth: basis.depth(c),
            });
        }
    }
    for (k, pts) in silhouettes.iter().enumerate() {
        let st = &sil_states[k];
        let mut i = 0;
        while i < st.len() {
            let state = st[i];
            let mut j = i;
            while j + 1 < st.len() && st[j + 1] == state {
                j += 1;
            }
            if j > i {
                let seg = Seg::Line {
                    a: basis.xy(pts[i]),
                    b: basis.xy(pts[j]),
                };
                if state == State::Visible {
                    visible.push(seg);
                } else if !buf.masked(basis.xy(pts[(i + j) / 2])) {
                    hidden.push(seg);
                }
            }
            i = j + 1;
        }
    }

    let verts: Vec<PVert> = solid
        .vertices
        .iter()
        .enumerate()
        .map(|(vi, p)| {
            let q = basis.xy(*p);
            let sl = topo.edges_of_vertex[vi]
                .iter()
                .flat_map(|ei| topo.faces_of_edge[*ei].iter())
                .map(|&fi| slope(solid, fi, *p, &basis))
                .fold(0.0, f64::max);
            let tol = TOL_BASE + 0.75 * buf.px * sl;
            PVert {
                p: q,
                depth: basis.depth(*p),
                visible: basis.depth(*p) - tol <= buf.at(q),
            }
        })
        .collect();

    let mut edge_lines: Vec<(P2, P2)> = Vec::new();
    for f in &solid.faces {
        if let Some(Surface::Plane { origin, normal }) = solid.surfaces.get(f.surface) {
            let n = unit(*normal);
            if dot(n, basis.d).abs() < 1e-6 {
                let dir = unit(cross(basis.d, n));
                let d2 = basis.xy(dir);
                edge_lines.push((basis.xy(*origin), d2));
            }
        }
    }
    let ext = (max[0] - min[0]).max(max[1] - min[1]);
    let bound = (0.02 * ext).clamp(6.0, 25.0);
    let outline = outline(&buf, &visible, &smooth, &circles, &edge_lines, bound);
    // A traced corner can sit a little outside the material where two
    // lines meet past a relief: the view's extent is that corner.
    let (mut min, mut max) = (min, max);
    for s in &outline {
        if let Seg::Line { a, b } = *s {
            for q in [a, b] {
                min[0] = min[0].min(q[0]);
                min[1] = min[1].min(q[1]);
                max[0] = max[0].max(q[0]);
                max[1] = max[1].max(q[1]);
            }
        }
    }
    Projection {
        basis,
        min,
        max,
        visible,
        smooth,
        hidden,
        verts,
        circles,
        outline,
        edge_lines,
    }
}

/// A line the outline may snap to: a point, a unit direction and, for a
/// projected edge, the span along it that it covers.
type Candidate = (P2, P2, Option<(f64, f64)>);

/// How one stretch of the traced outline was identified.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Snap {
    Line(usize),
    Circle(usize),
    Free,
}

/// Trace the covered pixels' outer boundary and snap it to the exact
/// lines and circles of the view.
fn outline(
    buf: &Buffer,
    visible: &[Seg],
    smooth: &[Seg],
    circles: &[PCircle],
    edge_lines: &[(P2, P2)],
    bound: f64,
) -> Vec<Seg> {
    let covered = |i: i64, j: i64| -> bool {
        i >= 0
            && j >= 0
            && (i as usize) < buf.w
            && (j as usize) < buf.h
            && buf.depth[j as usize * buf.w + i as usize].is_finite()
    };
    // Directed boundary edges, counter-clockwise around the covered area,
    // keyed by their start lattice point.
    let mut starts: std::collections::HashMap<(i64, i64), Vec<(i64, i64)>> =
        std::collections::HashMap::new();
    let mut count = 0usize;
    for j in 0..buf.h as i64 {
        for i in 0..buf.w as i64 {
            if !covered(i, j) {
                continue;
            }
            let mut push = |a: (i64, i64), b: (i64, i64)| {
                starts.entry(a).or_default().push(b);
                count += 1;
            };
            if !covered(i - 1, j) {
                push((i, j + 1), (i, j));
            }
            if !covered(i, j - 1) {
                push((i, j), (i + 1, j));
            }
            if !covered(i + 1, j) {
                push((i + 1, j), (i + 1, j + 1));
            }
            if !covered(i, j + 1) {
                push((i + 1, j + 1), (i, j + 1));
            }
        }
    }
    if count == 0 {
        return vec![];
    }
    // Chain into loops; keep the one with the largest area.
    let mut best: Vec<(i64, i64)> = Vec::new();
    let mut best_area = 0.0;
    let mut keys: Vec<(i64, i64)> = starts.keys().copied().collect();
    keys.sort_unstable();
    for k in keys {
        while let Some(first) = starts.get_mut(&k).and_then(|v| v.pop()) {
            let mut lp = vec![k, first];
            let mut cur = first;
            let mut prev_dir = (first.0 - k.0, first.1 - k.1);
            while cur != k {
                let Some(outs) = starts.get_mut(&cur) else {
                    break;
                };
                if outs.is_empty() {
                    break;
                }
                // At a pinch between diagonal pixels, turn right to keep
                // the union in one loop; else straight on.
                let right = (prev_dir.1, -prev_dir.0);
                let pick = outs
                    .iter()
                    .position(|n| (n.0 - cur.0, n.1 - cur.1) == right)
                    .or_else(|| {
                        outs.iter()
                            .position(|n| (n.0 - cur.0, n.1 - cur.1) == prev_dir)
                    })
                    .unwrap_or(0);
                let next = outs.remove(pick);
                prev_dir = (next.0 - cur.0, next.1 - cur.1);
                lp.push(next);
                cur = next;
                if lp.len() > count + 2 {
                    break;
                }
            }
            if cur != k {
                continue;
            }
            lp.pop();
            let mut area = 0.0;
            for i in 0..lp.len() {
                let (p, q) = (lp[i], lp[(i + 1) % lp.len()]);
                area += (p.0 * q.1 - q.0 * p.1) as f64;
            }
            if area.abs() > best_area {
                best_area = area.abs();
                best = lp;
            }
        }
    }
    if best.len() < 4 {
        return vec![];
    }
    let px = buf.px;
    let raw: Vec<P2> = best
        .iter()
        .map(|(i, j)| [buf.x0 + *i as f64 * px, buf.y0 + *j as f64 * px])
        .collect();
    let mut poly = thin(&raw, px * 1.5);
    if poly.len() > 1 && dist2(poly[0], poly[poly.len() - 1]) < 1e-9 {
        poly.pop();
    }
    let n = poly.len();
    if n < 3 {
        return vec![];
    }
    // Candidate lines: edge-on planes (unbounded) and projected straight
    // edges (near their span).
    let mut lines: Vec<Candidate> = Vec::new();
    for (p, d) in edge_lines {
        lines.push((*p, *d, None));
    }
    for seg in visible.iter().chain(smooth.iter()) {
        if let Seg::Line { a, b } = *seg {
            let l = dist2(a, b);
            if l > 2.0 * px {
                let d = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
                lines.push((a, d, Some((-2.0, l + 2.0))));
            }
        }
    }
    let tol = 2.2 * px;
    let groups = coincident_circles(circles, tol);
    let canon_circle = |ci: usize| -> usize {
        groups
            .iter()
            .find(|g| g.contains(&ci))
            .map(|g| g[0])
            .unwrap_or(ci)
    };
    let line_dist = |p: P2, li: usize| -> Option<f64> {
        let (o, d, span) = lines[li];
        let v = [p[0] - o[0], p[1] - o[1]];
        let along = v[0] * d[0] + v[1] * d[1];
        if let Some((lo, hi)) = span {
            if along < lo || along > hi {
                return None;
            }
        }
        Some((v[0] * d[1] - v[1] * d[0]).abs())
    };
    let mut snaps: Vec<Snap> = Vec::with_capacity(n);
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let l = dist2(a, b);
        let dir = [(b[0] - a[0]) / l.max(1e-12), (b[1] - a[1]) / l.max(1e-12)];
        let mut best: Option<(f64, Snap)> = None;
        for (li, cand) in lines.iter().enumerate() {
            let d = cand.1;
            if (dir[0] * d[0] + dir[1] * d[1]).abs() < 0.996 && l > 3.0 * px {
                continue;
            }
            if let (Some(da), Some(db)) = (line_dist(a, li), line_dist(b, li)) {
                let m = da.max(db);
                if m <= tol && best.map(|(bm, _)| m < bm).unwrap_or(true) {
                    best = Some((m, Snap::Line(li)));
                }
            }
        }
        if best.is_none() {
            for (ci, c) in circles.iter().enumerate() {
                if c.r < 2.0 * px {
                    continue;
                }
                let da = (dist2(a, c.c) - c.r).abs();
                let db = (dist2(b, c.c) - c.r).abs();
                let m = da.max(db);
                if m <= tol && best.map(|(bm, _)| m < bm).unwrap_or(true) {
                    best = Some((m, Snap::Circle(canon_circle(ci))));
                }
            }
        }
        snaps.push(best.map(|(_, s)| s).unwrap_or(Snap::Free));
    }
    // Stretches of equal snaps.
    let mut stretches: Vec<(usize, usize, Snap)> = Vec::new();
    let start = (0..n)
        .find(|&i| snaps[i] != snaps[(i + n - 1) % n])
        .unwrap_or(0);
    let mut i = start;
    loop {
        let s = snaps[i];
        let mut len = 1;
        while len < n && snaps[(i + len) % n] == s {
            len += 1;
        }
        stretches.push((i, len, s));
        i = (i + len) % n;
        if i == start || stretches.len() >= n {
            break;
        }
    }
    // Short free stretches (reliefs, tessellation) between identified
    // neighbours are dropped, and a stretch between two of the same
    // identification merges into it.
    loop {
        let m = stretches.len();
        if m < 3 {
            break;
        }
        let mut changed = false;
        for k in 0..m {
            let (i0, l0, s) = stretches[k];
            let prev = stretches[(k + m - 1) % m].2;
            let next = stretches[(k + 1) % m].2;
            let extent = (0..=l0)
                .map(|q| dist2(poly[i0], poly[(i0 + q) % n]))
                .fold(0.0, f64::max);
            // Anything shorter than a relief between two identified
            // neighbours that meet right there (a notch, a relief, the
            // tessellated end of a bend) gives way to their own junction.
            // A sheet's end across its thickness stays: its skins run
            // parallel and never meet.
            let mid = poly[(i0 + l0 / 2) % n];
            let meet = match (prev, next) {
                (Snap::Line(a), Snap::Line(b)) => {
                    let (oa, da, _) = lines[a];
                    let (ob, db, _) = lines[b];
                    let det = da[0] * db[1] - da[1] * db[0];
                    if det.abs() < 0.035 {
                        None
                    } else {
                        let t = ((ob[0] - oa[0]) * db[1] - (ob[1] - oa[1]) * db[0]) / det;
                        Some([oa[0] + da[0] * t, oa[1] + da[1] * t])
                    }
                }
                (Snap::Line(a), Snap::Circle(c)) | (Snap::Circle(c), Snap::Line(a)) => {
                    let (oa, da, _) = lines[a];
                    let cc = circles[c];
                    let p = line_circle(oa, da, cc.c, cc.r, mid);
                    // Only when the line really reaches the circle.
                    ((dist2(p, cc.c) - cc.r).abs() < tol).then_some(p)
                }
                _ => None,
            };
            let drop = extent < bound
                && !matches!(s, Snap::Circle(_))
                && s != prev
                && s != next
                && meet.map(|m| dist2(m, mid) < bound).unwrap_or(false);
            if drop {
                if prev == next {
                    // Merge prev, this and next into one stretch.
                    let kp = (k + m - 1) % m;
                    let kn = (k + 1) % m;
                    let (ip, lp, sp) = stretches[kp];
                    let ln = stretches[kn].1;
                    stretches[kp] = (ip, lp + l0 + ln, sp);
                    let mut rm = [k, kn];
                    rm.sort_unstable();
                    stretches.remove(rm[1]);
                    stretches.remove(rm[0]);
                } else {
                    // Give the samples to the previous stretch.
                    let kp = (k + m - 1) % m;
                    stretches[kp].1 += l0;
                    stretches.remove(k);
                }
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
    let m = stretches.len();
    if m < 2 {
        if let [(_, _, Snap::Circle(ci))] = stretches.as_slice() {
            let c = circles[*ci];
            return vec![Seg::Circle { c: c.c, r: c.r }];
        }
        return vec![];
    }
    // Junction points between consecutive stretches.
    let mut junctions: Vec<P2> = Vec::with_capacity(m);
    for k in 0..m {
        let (i0, l0, s0) = stretches[k];
        let (i1, _, s1) = stretches[(k + 1) % m];
        let _ = i0;
        let _ = l0;
        let raw_v = poly[i1 % n];
        let j = match (s0, s1) {
            (Snap::Line(a), Snap::Line(b)) => {
                let (oa, da, _) = lines[a];
                let (ob, db, _) = lines[b];
                let det = da[0] * db[1] - da[1] * db[0];
                if det.abs() < 0.035 {
                    project_on(raw_v, oa, da)
                } else {
                    let t = ((ob[0] - oa[0]) * db[1] - (ob[1] - oa[1]) * db[0]) / det;
                    [oa[0] + da[0] * t, oa[1] + da[1] * t]
                }
            }
            (Snap::Line(a), Snap::Circle(c)) | (Snap::Circle(c), Snap::Line(a)) => {
                let (oa, da, _) = lines[a];
                let cc = circles[c];
                line_circle(oa, da, cc.c, cc.r, raw_v)
            }
            (Snap::Circle(c), _) | (_, Snap::Circle(c)) => {
                let cc = circles[c];
                let v = [raw_v[0] - cc.c[0], raw_v[1] - cc.c[1]];
                let l = (v[0] * v[0] + v[1] * v[1]).sqrt().max(1e-12);
                [cc.c[0] + v[0] / l * cc.r, cc.c[1] + v[1] / l * cc.r]
            }
            (Snap::Line(a), Snap::Free) | (Snap::Free, Snap::Line(a)) => {
                let (oa, da, _) = lines[a];
                project_on(raw_v, oa, da)
            }
            (Snap::Free, Snap::Free) => raw_v,
        };
        junctions.push(j);
    }
    let mut out: Vec<Seg> = Vec::new();
    for k in 0..m {
        let (i0, l0, s) = stretches[k];
        let from = junctions[(k + m - 1) % m];
        let to = junctions[k];
        match s {
            Snap::Line(_) => {
                if dist2(from, to) > 1e-6 {
                    out.push(Seg::Line { a: from, b: to });
                }
            }
            Snap::Circle(ci) => {
                let c = circles[ci];
                let ang = |p: P2| (p[1] - c.c[1]).atan2(p[0] - c.c[0]).to_degrees();
                // The loop runs counter-clockwise; the arc's direction
                // follows the raw vertices.
                let mid = poly[(i0 + l0 / 2) % n];
                let (a0, a1, am) = (ang(from), ang(to), ang(mid));
                let ccw = (am - a0).rem_euclid(360.0) <= (a1 - a0).rem_euclid(360.0);
                let (start, end) = if ccw { (a0, a1) } else { (a1, a0) };
                if dist2(from, to) > 1e-6 {
                    out.push(Seg::Arc {
                        c: c.c,
                        r: c.r,
                        start,
                        end,
                    });
                }
            }
            Snap::Free => {
                let mut pts = vec![from];
                for k2 in 1..l0 {
                    pts.push(poly[(i0 + k2) % n]);
                }
                pts.push(to);
                for w in pts.windows(2) {
                    if dist2(w[0], w[1]) > 1e-6 {
                        out.push(Seg::Line { a: w[0], b: w[1] });
                    }
                }
            }
        }
    }
    out
}

fn project_on(p: P2, o: P2, d: P2) -> P2 {
    let t = (p[0] - o[0]) * d[0] + (p[1] - o[1]) * d[1];
    [o[0] + d[0] * t, o[1] + d[1] * t]
}

/// The intersection of a line and a circle nearest `near`, or the line's
/// point nearest the circle when they miss.
fn line_circle(o: P2, d: P2, c: P2, r: f64, near: P2) -> P2 {
    let v = [o[0] - c[0], o[1] - c[1]];
    let b = v[0] * d[0] + v[1] * d[1];
    let cc = v[0] * v[0] + v[1] * v[1] - r * r;
    let disc = b * b - cc;
    if disc < 0.0 {
        return project_on(c, o, d);
    }
    let s = disc.sqrt();
    let p1 = [o[0] + d[0] * (-b + s), o[1] + d[1] * (-b + s)];
    let p2 = [o[0] + d[0] * (-b - s), o[1] + d[1] * (-b - s)];
    if dist2(p1, near) <= dist2(p2, near) {
        p1
    } else {
        p2
    }
}

/// Depth range of a face from its edges' ends.
fn face_depth_range(solid: &Solid, f: &crate::view::Face, basis: &Basis) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for le in f.loops.iter().flatten() {
        if let Some(e) = solid.edges.get(le.edge) {
            for v in [e.start, e.end] {
                if let Some(p) = solid.vertices.get(v) {
                    let z = basis.depth(*p);
                    lo = lo.min(z);
                    hi = hi.max(z);
                }
            }
        }
    }
    if !lo.is_finite() {
        (f64::NEG_INFINITY, f64::INFINITY)
    } else {
        (lo - 0.05, hi + 0.05)
    }
}

/// Runs of fewer than three samples take their neighbours' state.
fn smooth_states(st: &mut [State]) {
    let n = st.len();
    if n < 4 {
        return;
    }
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && st[j + 1] == st[i] {
            j += 1;
        }
        if j - i < 2 && i > 0 && j + 1 < n && st[i - 1] == st[j + 1] {
            let s = st[i - 1];
            for v in st.iter_mut().take(j + 1).skip(i) {
                *v = s;
            }
        }
        i = j + 1;
    }
}

/// Whether circle edge `ei` on face `fi` is a fillet: the loop edges
/// before and after it run into it tangent at both ends.
fn is_fillet(solid: &Solid, topo: &Topology, fine: &[Sampled], fi: usize, ei: usize) -> bool {
    let f = &solid.faces[fi];
    for lp in &f.loops {
        let n = lp.len();
        if n < 2 {
            continue;
        }
        for (k, le) in lp.iter().enumerate() {
            if le.edge != ei {
                continue;
            }
            let prev = lp[(k + n - 1) % n].edge;
            let next = lp[(k + 1) % n].edge;
            if prev == ei || next == ei || (topo.smooth[prev] && topo.smooth[next]) {
                return false;
            }
            let e = solid.edges[ei];
            let (first, last) = if le.forward {
                (e.start, e.end)
            } else {
                (e.end, e.start)
            };
            return tangent_at(solid, fine, prev, first, ei)
                && tangent_at(solid, fine, next, last, ei);
        }
    }
    false
}

/// Whether edges `a` and `b` share vertex `v` with the same tangent there.
fn tangent_at(solid: &Solid, fine: &[Sampled], a: usize, v: usize, b: usize) -> bool {
    let dir = |ei: usize| -> Option<P3> {
        let e = solid.edges.get(ei)?;
        let pts = &fine.get(ei)?.pts;
        if pts.len() < 2 {
            return None;
        }
        if e.start == v {
            Some(unit(sub(pts[1], pts[0])))
        } else if e.end == v {
            Some(unit(sub(pts[pts.len() - 2], pts[pts.len() - 1])))
        } else {
            None
        }
    };
    match (dir(a), dir(b)) {
        (Some(da), Some(db)) => dot(da, db).abs() > 0.9962,
        _ => false,
    }
}

/// Group face-on circles that coincide in the view: same centre and
/// radius within `tol`. Returns, per group, the member indices.
pub fn coincident_circles(circles: &[PCircle], tol: f64) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, c) in circles.iter().enumerate() {
        let found = groups.iter().position(|g| {
            let f = &circles[g[0]];
            dist2(f.c, c.c) <= tol && (f.r - c.r).abs() <= tol
        });
        match found {
            Some(gi) => groups[gi].push(i),
            None => groups.push(vec![i]),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bspline_interpolates_ends_and_a_straight_polygon() {
        let cp = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
        ];
        let p0 = bspline(3, &cp, &[0.0, 1.0], &[4, 4], 0.0);
        let p1 = bspline(3, &cp, &[0.0, 1.0], &[4, 4], 1.0);
        let pm = bspline(3, &cp, &[0.0, 1.0], &[4, 4], 0.5);
        assert!((p0[0]).abs() < 1e-12 && (p1[0] - 3.0).abs() < 1e-12);
        assert!((pm[0] - 1.5).abs() < 1e-12);
    }

    #[test]
    fn depth_plane_recovers_a_plane() {
        let p = depth_plane([[0.0, 0.0, 1.0], [1.0, 0.0, 3.0], [0.0, 1.0, 2.0]]).unwrap();
        assert!((p[0] - 2.0).abs() < 1e-12 && (p[1] - 1.0).abs() < 1e-12);
        assert!((p[2] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn clip_band_keeps_the_middle() {
        let sq = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
        let c = clip_band(&sq, 0, 1.0, 2.0);
        let xs: Vec<f64> = c.iter().map(|p| p[0]).collect();
        assert!(xs
            .iter()
            .all(|x| (*x - 1.0).abs() < 1e-12 || (*x - 2.0).abs() < 1e-12));
        assert_eq!(c.len(), 4);
    }

    #[test]
    fn basis_is_right_handed() {
        let b = Basis::new([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        assert!((b.u[1] - 1.0).abs() < 1e-12);
        let c = cross(b.r, b.u);
        assert!((c[2] - 1.0).abs() < 1e-12);
    }
}
