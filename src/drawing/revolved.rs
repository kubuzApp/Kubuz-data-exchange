//! Cones and tori in the drawings: the two surfaces of revolution the
//! kernel writes for transitions, roll lips and hems on a round rim.
//!
//! Both are parametrised by `u`, the angle about the axis, and `v`: the
//! height along the axis on a cone, the angle round the tube on a torus.
//! The hidden-line buffer gets each face as small planar cells of that
//! (u, v) grid; the silhouette is where the surface normal turns square
//! to the view.

use std::f64::consts::{PI, TAU};

use super::project::{add, clip_band, depth_plane, dot, norm, scale, Basis};
use crate::dxf::blank::P2;
use crate::step::writer::{any_perp, cross, sub, unit};
use crate::view::{Face, Surface, P3};

/// Takes one cell's projected loops, its depth plane and depth range.
pub type Fill<'a> = dyn FnMut(&[Vec<P2>], [f64; 3], f64, f64) + 'a;

/// A cone or a torus with its own frame.
#[derive(Clone, Copy, Debug)]
pub struct Rev {
    origin: P3,
    ax: P3,
    e1: P3,
    e2: P3,
    kind: Kind,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    /// Radius `r0` at `v = 0`, `tan` more per millimetre of `v`.
    Cone {
        r0: f64,
        tan: f64,
        cos: f64,
        sin: f64,
    },
    Torus {
        major: f64,
        minor: f64,
    },
}

impl Rev {
    pub fn of(s: &Surface) -> Option<Rev> {
        let (origin, axis, kind) = match *s {
            Surface::Cone {
                origin,
                axis,
                radius,
                semi_angle_deg,
            } => {
                let a = semi_angle_deg.to_radians();
                (
                    origin,
                    axis,
                    Kind::Cone {
                        r0: radius,
                        tan: a.tan(),
                        cos: a.cos(),
                        sin: a.sin(),
                    },
                )
            }
            Surface::Torus {
                center,
                axis,
                major_radius,
                minor_radius,
            } => (
                center,
                axis,
                Kind::Torus {
                    major: major_radius,
                    minor: minor_radius,
                },
            ),
            _ => return None,
        };
        let ax = unit(axis);
        let e1 = any_perp(ax);
        let e2 = cross(ax, e1);
        Some(Rev {
            origin,
            ax,
            e1,
            e2,
            kind,
        })
    }

    /// The same surface moved by the rigid map `world` (a part's pose);
    /// (u, v) parameters stay the same.
    pub fn mapped(&self, world: &dyn Fn(P3) -> P3) -> Rev {
        let o = world(self.origin);
        let dir = |v: P3| sub(world(add(self.origin, v)), o);
        Rev {
            origin: o,
            ax: dir(self.ax),
            e1: dir(self.e1),
            e2: dir(self.e2),
            kind: self.kind,
        }
    }

    fn radial(&self, u: f64) -> P3 {
        add(scale(self.e1, u.cos()), scale(self.e2, u.sin()))
    }

    pub fn at(&self, u: f64, v: f64) -> P3 {
        let rad = self.radial(u);
        match self.kind {
            Kind::Cone { r0, tan, .. } => add(
                self.origin,
                add(scale(self.ax, v), scale(rad, r0 + v * tan)),
            ),
            Kind::Torus { major, minor } => add(
                self.origin,
                add(
                    scale(rad, major + minor * v.cos()),
                    scale(self.ax, minor * v.sin()),
                ),
            ),
        }
    }

    /// Outward unit normal at (u, v).
    pub fn normal_uv(&self, u: f64, v: f64) -> P3 {
        let rad = self.radial(u);
        match self.kind {
            Kind::Cone { cos, sin, .. } => add(scale(rad, cos), scale(self.ax, -sin)),
            Kind::Torus { .. } => add(scale(rad, v.cos()), scale(self.ax, v.sin())),
        }
    }

    /// (u, v) of a point on (or near) the surface; u in (-π, π].
    pub fn param(&self, p: P3) -> P2 {
        let q = sub(p, self.origin);
        let h = dot(q, self.ax);
        let x = dot(q, self.e1);
        let y = dot(q, self.e2);
        let u = y.atan2(x);
        match self.kind {
            Kind::Cone { .. } => [u, h],
            Kind::Torus { major, .. } => {
                let rho = (x * x + y * y).sqrt();
                [u, h.atan2(rho - major)]
            }
        }
    }

    pub fn normal_at(&self, p: P3) -> P3 {
        let [u, v] = self.param(p);
        self.normal_uv(u, v)
    }

    /// Cells along `v` one strip of `u` needs: a cone's generators are
    /// straight, a torus bends both ways.
    fn v_cells(&self, v_span: f64, dth: f64) -> usize {
        match self.kind {
            Kind::Cone { .. } => 1,
            Kind::Torus { .. } => ((v_span / dth).ceil() as usize).max(1),
        }
    }

    /// Angle step that keeps the chord error under `tol` on this surface.
    fn angle_step(&self, tol: f64) -> f64 {
        let r = match self.kind {
            Kind::Cone { r0, .. } => r0.abs().max(1.0),
            Kind::Torus { major, minor } => (major + minor).max(minor),
        };
        if r > tol {
            (2.0 * (1.0 - tol / r).acos()).min(10f64.to_radians())
        } else {
            10f64.to_radians()
        }
    }
}

/// The silhouette circles of a torus seen along its axis `d` (its outer
/// and inner equators): centre and radius. None for anything else.
pub fn axial_circles(s: &Surface, d: P3) -> Vec<(P3, f64)> {
    match *s {
        Surface::Torus {
            center,
            axis,
            major_radius,
            minor_radius,
        } if dot(unit(axis), d).abs() > 1.0 - 1e-9 => {
            let mut out = vec![(center, major_radius + minor_radius)];
            if major_radius > minor_radius {
                out.push((center, major_radius - minor_radius));
            }
            out
        }
        _ => Vec::new(),
    }
}

/// The face's loops in (u, v), each unwrapped so it runs on without
/// jumps. A loop that goes once round the axis (the rim of a full ring)
/// does not close in (u, v); two such loops are joined into one polygon
/// across the seam.
pub fn face_params(
    rev: &Rev,
    f: &Face,
    edge_pts: &dyn Fn(usize) -> Option<Vec<P3>>,
) -> Vec<Vec<P2>> {
    let mut closed: Vec<Vec<P2>> = Vec::new();
    let mut open: Vec<Vec<P2>> = Vec::new();
    for lp in &f.loops {
        let mut v: Vec<P2> = Vec::new();
        let mut prev: Option<P2> = None;
        for le in lp {
            let Some(pts) = edge_pts(le.edge) else {
                continue;
            };
            let n = pts.len();
            if n < 2 {
                continue;
            }
            let order: Vec<usize> = if le.forward {
                (0..n - 1).collect()
            } else {
                (1..n).rev().collect()
            };
            for i in order {
                let mut q = rev.param(pts[i]);
                if let Some(pv) = prev {
                    for k in 0..2 {
                        while q[k] - pv[k] > PI {
                            q[k] -= TAU;
                        }
                        while pv[k] - q[k] > PI {
                            q[k] += TAU;
                        }
                    }
                }
                // A cone's v is a length, not an angle: undo the wrap.
                if matches!(rev.kind, Kind::Cone { .. }) {
                    q[1] = rev.param(pts[i])[1];
                }
                prev = Some(q);
                v.push(q);
            }
        }
        if v.len() < 2 {
            continue;
        }
        let (first, last) = (v[0], v[v.len() - 1]);
        // The last point is one sample before the first: compare the gap
        // with a full turn.
        if (last[0] - first[0]).abs() > PI {
            open.push(v);
        } else {
            closed.push(v);
        }
    }
    // Two rims that each go once round: one runs +u, the other -u.
    if open.len() == 2 {
        let a = open[0].clone();
        let mut b = open[1].clone();
        let shift = a[a.len() - 1][0] - b[0][0];
        let turns = (shift / TAU).round() * TAU;
        for q in &mut b {
            q[0] += turns;
        }
        let mut poly = a;
        poly.extend(b);
        closed.push(poly);
    } else {
        closed.extend(open);
    }
    closed
}

/// The face cut into small (u, v) cells: each cell's bounds `[u0, u1,
/// v0, v1]` and the face's loops clipped to it. A cell is near enough to
/// planar for a facet, with chord error under `tol`.
pub fn cells(rev: &Rev, params: &[Vec<P2>], tol: f64) -> Vec<([f64; 4], Vec<Vec<P2>>)> {
    let mut out = Vec::new();
    let Some([lo_u, hi_u, lo_v, hi_v]) = param_bounds(params) else {
        return out;
    };
    let dth = rev.angle_step(tol);
    let nu = (((hi_u - lo_u) / dth).ceil() as usize).max(1);
    let nv = rev.v_cells(hi_v - lo_v, dth);
    let du = (hi_u - lo_u) / nu as f64;
    let dv = (hi_v - lo_v) / nv as f64;
    for i in 0..nu {
        let (a, b) = (lo_u + du * i as f64, lo_u + du * (i + 1) as f64);
        let strip: Vec<Vec<P2>> = params
            .iter()
            .map(|lp| clip_band(lp, 0, a, b))
            .filter(|lp| lp.len() >= 3)
            .collect();
        if strip.is_empty() {
            continue;
        }
        for j in 0..nv {
            let (c, d) = (lo_v + dv * j as f64, lo_v + dv * (j + 1) as f64);
            let cell: Vec<Vec<P2>> = strip
                .iter()
                .map(|lp| clip_band(lp, 1, c, d))
                .filter(|lp| lp.len() >= 3)
                .collect();
            if !cell.is_empty() {
                out.push(([a, b, c, d], cell));
            }
        }
    }
    out
}

fn param_bounds(params: &[Vec<P2>]) -> Option<[f64; 4]> {
    let mut b = [
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in params.iter().flatten() {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].max(p[0]);
        b[2] = b[2].min(p[1]);
        b[3] = b[3].max(p[1]);
    }
    (b[0].is_finite() && b[1] - b[0] > 1e-9 && b[3] - b[2] > 1e-9).then_some(b)
}

/// Puts one cone or torus face into the hidden-line buffer through
/// `fill(loops, depth_plane, lo, hi)` and adds its silhouette lines.
#[allow(clippy::too_many_arguments)]
pub fn project_face(
    rev: &Rev,
    params: &[Vec<P2>],
    basis: &Basis,
    tol: f64,
    step: f64,
    fill: &mut Fill,
    silhouettes: &mut Vec<Vec<P3>>,
) {
    for ([a, b, c, d], cell) in cells(rev, params, tol) {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        let loops2: Vec<Vec<P2>> = cell
            .iter()
            .map(|lp| {
                lp.iter()
                    .map(|q| {
                        let p = rev.at(q[0], q[1]);
                        let z = basis.depth(p);
                        lo = lo.min(z);
                        hi = hi.max(z);
                        basis.xy(p)
                    })
                    .collect()
            })
            .collect();
        let tri = [rev.at(a, c), rev.at(b, c), rev.at(a, d)].map(|p| {
            let q = basis.xy(p);
            [q[0], q[1], basis.depth(p)]
        });
        if let Some(plane) = depth_plane(tri) {
            fill(&loops2, plane, lo, hi);
        }
    }
    silhouette(
        rev,
        params,
        basis.d,
        &|p| basis.xy(p),
        tol,
        step,
        silhouettes,
    );
}

fn inside(params: &[Vec<P2>], q: P2) -> bool {
    let mut c = false;
    for lp in params {
        let n = lp.len();
        for i in 0..n {
            let (a, b) = (lp[i], lp[(i + 1) % n]);
            if (a[1] > q[1]) != (b[1] > q[1])
                && q[0] < a[0] + (q[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
            {
                c = !c;
            }
        }
    }
    c
}

/// The parts of the piece a–b inside the face.
fn clip_to_face(params: &[Vec<P2>], a: P2, b: P2) -> Vec<(P2, P2)> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let mut ts = vec![0.0, 1.0];
    for lp in params {
        let n = lp.len();
        for i in 0..n {
            let (p, q) = (lp[i], lp[(i + 1) % n]);
            let e = [q[0] - p[0], q[1] - p[1]];
            let den = d[0] * e[1] - d[1] * e[0];
            if den.abs() < 1e-15 {
                continue;
            }
            let w = [p[0] - a[0], p[1] - a[1]];
            let t = (w[0] * e[1] - w[1] * e[0]) / den;
            let s = (w[0] * d[1] - w[1] * d[0]) / den;
            if t > 0.0 && t < 1.0 && (0.0..=1.0).contains(&s) {
                ts.push(t);
            }
        }
    }
    ts.sort_by(f64::total_cmp);
    let at = |t: f64| [a[0] + d[0] * t, a[1] + d[1] * t];
    ts.windows(2)
        .filter(|w| w[1] - w[0] > 1e-12)
        .filter(|w| {
            let m = at((w[0] + w[1]) / 2.0);
            // On the face's own edge the edge itself is drawn.
            inside(params, m) && !on_boundary(params, m, 1e-6)
        })
        .map(|w| (at(w[0]), at(w[1])))
        .collect()
}

/// Whether `q` lies within `eps` of a loop edge.
fn on_boundary(params: &[Vec<P2>], q: P2, eps: f64) -> bool {
    params.iter().any(|lp| {
        let n = lp.len();
        (0..n).any(|i| {
            let (a, b) = (lp[i], lp[(i + 1) % n]);
            let ab = [b[0] - a[0], b[1] - a[1]];
            let l2 = ab[0] * ab[0] + ab[1] * ab[1];
            let t = if l2 > 0.0 {
                (((q[0] - a[0]) * ab[0] + (q[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let d = [q[0] - a[0] - ab[0] * t, q[1] - a[1] - ab[1] * t];
            d[0] * d[0] + d[1] * d[1] <= eps * eps
        })
    })
}

/// Where n · d changes sign inside the face, traced on a (u, v) grid, as
/// one short piece per grid cell.
#[allow(clippy::too_many_arguments)]
pub fn silhouette(
    rev: &Rev,
    params: &[Vec<P2>],
    view: P3,
    flat: &dyn Fn(P3) -> P2,
    tol: f64,
    step: f64,
    out: &mut Vec<Vec<P3>>,
) {
    let Some([lo_u, hi_u, lo_v, hi_v]) = param_bounds(params) else {
        return;
    };
    let dth = rev.angle_step(tol);
    let nu = (((hi_u - lo_u) / (dth / 2.0)).ceil() as usize).clamp(2, 720);
    let nv = match rev.kind {
        Kind::Cone { .. } => {
            // Straight generators: fine enough to follow the rim.
            let len = norm(sub(rev.at(lo_u, hi_v), rev.at(lo_u, lo_v)));
            ((len / step.max(0.1)).ceil() as usize).clamp(2, 400)
        }
        Kind::Torus { .. } => (((hi_v - lo_v) / (dth / 2.0)).ceil() as usize).clamp(2, 720),
    };
    // One cell more on every side, so a silhouette that leaves the face
    // is followed up to its edge.
    let (du, dv) = ((hi_u - lo_u) / nu as f64, (hi_v - lo_v) / nv as f64);
    let (lo_u, hi_u, nu) = (lo_u - du, hi_u + du, nu + 2);
    let (lo_v, hi_v, nv) = (lo_v - dv, hi_v + dv, nv + 2);
    let g = |i: usize, j: usize| -> (P2, f64) {
        let q = [
            lo_u + (hi_u - lo_u) * i as f64 / nu as f64,
            lo_v + (hi_v - lo_v) * j as f64 / nv as f64,
        ];
        (q, dot(rev.normal_uv(q[0], q[1]), view))
    };
    // Zero crossings on grid edges, as segments per cell (marching squares).
    let mut segs: Vec<(P2, P2)> = Vec::new();
    for i in 0..nu {
        for j in 0..nv {
            let c = [g(i, j), g(i + 1, j), g(i + 1, j + 1), g(i, j + 1)];
            let mut hits: Vec<P2> = Vec::new();
            for k in 0..4 {
                let (p, fp) = c[k];
                let (q, fq) = c[(k + 1) % 4];
                if (fp < 0.0) != (fq < 0.0) {
                    let t = fp / (fp - fq);
                    hits.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
                }
            }
            if hits.len() == 2 {
                segs.push((hits[0], hits[1]));
            } else if hits.len() == 4 {
                segs.push((hits[0], hits[1]));
                segs.push((hits[2], hits[3]));
            }
        }
    }
    let segs: Vec<(P2, P2)> = segs
        .into_iter()
        .flat_map(|(a, b)| clip_to_face(params, a, b))
        .collect();
    // Straight pieces: the drawing turns each run of points into one
    // straight line, so a curved silhouette is cut into pieces that each
    // stay within `tol` of the curve on paper (`flat`).
    let mut done: Vec<(P3, P3)> = Vec::new();
    for chain in chains(segs) {
        let pts: Vec<P3> = chain.iter().map(|q| rev.at(q[0], q[1])).collect();
        let flat_pts: Vec<P3> = pts
            .iter()
            .map(|p| {
                let q = flat(*p);
                [q[0], q[1], 0.0]
            })
            .collect();
        let keep = simplify(&flat_pts, tol);
        for w in keep.windows(2) {
            let (a, b) = (pts[w[0]], pts[w[1]]);
            let same = |p: P3, q: P3| norm(sub(p, q)) < 1e-6;
            if done
                .iter()
                .any(|&(p, q)| (same(p, a) && same(q, b)) || (same(p, b) && same(q, a)))
            {
                continue;
            }
            done.push((a, b));
            // The points on the surface, about `step` apart, so the
            // drawing can test each stretch for visibility.
            let mut piece = vec![a];
            for k in w[0]..w[1] {
                let (p, q) = (pts[k], pts[k + 1]);
                let n = ((norm(sub(q, p)) / step.max(0.1)).ceil() as usize).max(1);
                piece.extend((1..=n).map(|i| add(p, scale(sub(q, p), i as f64 / n as f64))));
            }
            out.push(piece);
        }
    }
}

/// Joins pieces that share an end into chains of points.
fn chains(mut segs: Vec<(P2, P2)>) -> Vec<Vec<P2>> {
    let near = |a: P2, b: P2| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
    let mut out = Vec::new();
    while let Some((a, b)) = segs.pop() {
        let mut chain = vec![a, b];
        loop {
            let tail = chain[chain.len() - 1];
            let head = chain[0];
            let Some(k) = segs.iter().position(|(p, q)| {
                near(*p, tail) || near(*q, tail) || near(*p, head) || near(*q, head)
            }) else {
                break;
            };
            let (p, q) = segs.swap_remove(k);
            if near(p, tail) {
                chain.push(q);
            } else if near(q, tail) {
                chain.push(p);
            } else if near(p, head) {
                chain.insert(0, q);
            } else {
                chain.insert(0, p);
            }
        }
        out.push(chain);
    }
    out
}

/// Douglas–Peucker: the indices of the points that keep the polyline
/// within `tol` of the original.
fn simplify(pts: &[P3], tol: f64) -> Vec<usize> {
    if pts.len() < 3 {
        return (0..pts.len()).collect();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0, pts.len() - 1)];
    while let Some((i, j)) = stack.pop() {
        let (a, b) = (pts[i], pts[j]);
        let ab = sub(b, a);
        let l2 = dot(ab, ab);
        let mut worst = (0.0, 0);
        for (k, p) in pts.iter().enumerate().take(j).skip(i + 1) {
            let ap = sub(*p, a);
            let t = if l2 > 0.0 {
                (dot(ap, ab) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let d = norm(sub(ap, scale(ab, t)));
            if d > worst.0 {
                worst = (d, k);
            }
        }
        if worst.0 > tol {
            keep[worst.1] = true;
            stack.push((i, worst.1));
            stack.push((worst.1, j));
        }
    }
    (0..pts.len()).filter(|&k| keep[k]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P3, b: P3) -> bool {
        norm(sub(a, b)) < 1e-9
    }

    #[test]
    fn param_finds_the_point_back() {
        let cone = Rev::of(&Surface::Cone {
            origin: [1.0, 2.0, 3.0],
            axis: [0.0, 0.0, 1.0],
            radius: 50.0,
            semi_angle_deg: -26.0,
        })
        .unwrap();
        let torus = Rev::of(&Surface::Torus {
            center: [0.0, 0.0, 0.0],
            axis: [0.0, 1.0, 1.0],
            major_radius: 50.0,
            minor_radius: 10.0,
        })
        .unwrap();
        for rev in [cone, torus] {
            for (u, v) in [(0.3, 0.2), (-2.0, 1.4), (3.0, -2.5)] {
                let p = rev.at(u, v);
                let [pu, pv] = rev.param(p);
                assert!(close(rev.at(pu, pv), p), "{rev:?} {u} {v}");
            }
        }
    }

    #[test]
    fn the_normal_is_square_to_the_surface() {
        let cone = Rev::of(&Surface::Cone {
            origin: [0.0, 0.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            radius: 50.0,
            semi_angle_deg: -30.0,
        })
        .unwrap();
        let h = 1e-6;
        let (u, v) = (0.7, 12.0);
        let du = sub(cone.at(u + h, v), cone.at(u - h, v));
        let dv = sub(cone.at(u, v + h), cone.at(u, v - h));
        let n = cone.normal_uv(u, v);
        assert!(dot(n, du).abs() < 1e-9 && dot(n, dv).abs() < 1e-9);
        // Outward: away from the axis.
        assert!(dot(n, cone.radial(u)) > 0.0);
    }

    #[test]
    fn a_straight_chain_simplifies_to_its_ends() {
        let pts: Vec<P3> = (0..10).map(|i| [i as f64, 2.0 * i as f64, 0.0]).collect();
        assert_eq!(simplify(&pts, 0.01), vec![0, 9]);
        let arc: Vec<P3> = (0..=90)
            .map(|i| {
                let a = (i as f64).to_radians();
                [10.0 * a.cos(), 10.0 * a.sin(), 0.0]
            })
            .collect();
        let keep = simplify(&arc, 0.05);
        assert!(keep.len() > 3 && keep.len() < 20, "{keep:?}");
    }

    #[test]
    fn pieces_join_into_chains_either_way_round() {
        let segs = vec![
            ([1.0, 0.0], [2.0, 0.0]),
            ([0.0, 0.0], [1.0, 0.0]),
            ([3.0, 0.0], [2.0, 0.0]),
        ];
        let c = chains(segs);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].len(), 4);
    }
}
