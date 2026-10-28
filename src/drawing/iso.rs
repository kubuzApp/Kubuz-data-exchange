//! An isometric pictorial of the folded part, from the blank's zones and
//! bends: the outer skin as a shell (flanges as planes, bends as
//! cylinders), seen from above the outer skin, visible lines only.
//!
//! The lines are the zones' outlines (the sheet's edges and the tangent
//! lines where a flange meets its bend) and the holes. A piece of a line
//! is hidden when another zone's surface lies in front of its midpoint.

use super::features::outer_loop;
use super::section::{fold, P3};
use crate::dxf::blank::{Blank, P2};

/// The pictorial, on the picture plane in model millimetres.
#[derive(Clone, Debug, Default)]
pub struct Iso {
    pub polylines: Vec<Vec<P2>>,
    pub bbox: [f64; 4],
}

/// Isometric axes: the viewer at (-1, -1, 1), the blank's x to the right
/// and up, its y to the left and up, the skin's normal up.
fn project(p: P3) -> (P2, f64) {
    let sx = (p[0] - p[1]) / 2f64.sqrt();
    let sy = (p[0] + p[1] + 2.0 * p[2]) / 6f64.sqrt();
    let depth = (-p[0] - p[1] + p[2]) / 3f64.sqrt();
    ([sx, sy], depth)
}

struct Facet {
    /// Which zone it belongs to (lines of that zone are never hidden by
    /// it).
    zone: usize,
    /// Projected outline and holes.
    poly: Vec<P2>,
    holes: Vec<Vec<P2>>,
    /// Depth as an affine function of the picture point.
    plane: [f64; 3],
    bbox: [f64; 4],
}

fn bbox_of(pts: &[P2]) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in pts {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    }
    b
}

fn inside(poly: &[P2], p: P2) -> bool {
    let mut hit = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if x > p[0] {
                hit = !hit;
            }
        }
    }
    hit
}

/// The depth plane through projected points `pts` with depths `ds`:
/// `depth = a·x + b·y + c`, from the three points spanning the largest
/// triangle.
fn depth_plane(pts: &[P2], ds: &[f64]) -> Option<[f64; 3]> {
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let i = 0;
    let mut j = 1;
    let mut best = 0.0;
    for k in 1..n {
        let d = (pts[k][0] - pts[i][0]).hypot(pts[k][1] - pts[i][1]);
        if d > best {
            best = d;
            j = k;
        }
    }
    let mut k = j;
    best = 0.0;
    for l in 1..n {
        let a = (pts[j][0] - pts[i][0]) * (pts[l][1] - pts[i][1])
            - (pts[j][1] - pts[i][1]) * (pts[l][0] - pts[i][0]);
        if a.abs() > best {
            best = a.abs();
            k = l;
        }
    }
    if best < 1e-6 {
        return None;
    }
    // Solve [x y 1] [a b c]^T = d for the three points.
    let m = [
        [pts[i][0], pts[i][1], 1.0],
        [pts[j][0], pts[j][1], 1.0],
        [pts[k][0], pts[k][1], 1.0],
    ];
    let r = [ds[i], ds[j], ds[k]];
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d0 = det(m);
    if d0.abs() < 1e-12 {
        return None;
    }
    let mut out = [0.0; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let mut mc = m;
        for (row, rv) in mc.iter_mut().zip(r) {
            row[c] = rv;
        }
        *o = det(mc) / d0;
    }
    Some(out)
}

/// Subdivide a 2D polygon's edges into pieces no longer than `step`.
fn densify(poly: &[P2], step: f64, closed: bool) -> Vec<P2> {
    let mut out = Vec::new();
    let n = poly.len();
    let last = if closed { n } else { n - 1 };
    for i in 0..last {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let k = ((len / step).ceil() as usize).max(1);
        for j in 0..k {
            let f = j as f64 / k as f64;
            out.push([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]);
        }
    }
    if !closed {
        out.push(poly[n - 1]);
    }
    out
}

/// The pictorial of the folded blank, or `None` for a flat plate or a
/// blank that does not fold.
pub fn isometric(blank: &Blank, t: f64) -> Option<Iso> {
    if blank.bends.is_empty() {
        return None;
    }
    let fd = fold(blank, t)?;
    let bend_of = |zone: &str| blank.bends.iter().position(|b| b.zone_id == zone);
    // A point of zone `z` on the folded outer skin.
    let lift = |z: usize, p: P2| -> Option<P3> {
        match bend_of(&blank.zones[z].id) {
            Some(bi) => fd.bends[bi].map(|bf| bf.apply(p, 0)),
            None => fd.transform[z].map(|tr| tr.apply([p[0], p[1], 0.0])),
        }
    };
    // Holes by the zone they lie in.
    let outer = outer_loop(&blank.loops);
    let mut holes: Vec<(usize, Vec<P2>)> = Vec::new();
    for (li, lp) in blank.loops.iter().enumerate() {
        if li == outer {
            continue;
        }
        // Model coordinates, thinned to a point every 3 mm (a hole's
        // curve needs no finer in a pictorial).
        let fine = super::features::loop_points(lp, 1.0);
        let mut pts: Vec<P2> = Vec::new();
        for p in fine {
            if pts
                .last()
                .is_none_or(|q| (p[0] - q[0]).hypot(p[1] - q[1]) >= 3.0)
            {
                pts.push(p);
            }
        }
        if pts.is_empty() {
            continue;
        }
        let Some(z) = (0..blank.zones.len())
            .filter(|&z| bend_of(&blank.zones[z].id).is_none())
            .find(|&z| inside(&blank.zones[z].points, pts[0]))
        else {
            continue;
        };
        holes.push((z, pts));
    }

    // Occluders: flanges as one facet each (with their holes), bends as
    // eight facets along the turn.
    let mut facets: Vec<Facet> = Vec::new();
    let mut lines: Vec<(usize, Vec<P2>, bool)> = Vec::new();
    for (z, zone) in blank.zones.iter().enumerate() {
        if zone.points.len() < 3 {
            continue;
        }
        match bend_of(&zone.id) {
            None => {
                let Some(tr) = fd.transform[z] else {
                    continue;
                };
                let pts3: Vec<P3> = zone
                    .points
                    .iter()
                    .map(|p| tr.apply([p[0], p[1], 0.0]))
                    .collect();
                let (poly, ds): (Vec<P2>, Vec<f64>) = pts3.iter().map(|p| project(*p)).unzip();
                let Some(plane) = depth_plane(&poly, &ds) else {
                    continue;
                };
                let hs: Vec<Vec<P2>> = holes
                    .iter()
                    .filter(|(hz, _)| *hz == z)
                    .map(|(_, pts)| {
                        pts.iter()
                            .map(|p| project(tr.apply([p[0], p[1], 0.0])).0)
                            .collect()
                    })
                    .collect();
                let bbox = bbox_of(&poly);
                facets.push(Facet {
                    zone: z,
                    poly,
                    holes: hs,
                    plane,
                    bbox,
                });
                lines.push((z, zone.points.clone(), true));
                for (_, pts) in holes.iter().filter(|(hz, _)| *hz == z) {
                    lines.push((z, pts.clone(), true));
                }
            }
            Some(bi) => {
                let Some(bf) = fd.bends[bi] else {
                    continue;
                };
                let b = &blank.bends[bi];
                let (e0, e1) = (
                    dot(b.a, bf.e).min(dot(b.b, bf.e)),
                    dot(b.a, bf.e).max(dot(b.b, bf.e)),
                );
                let k = 8;
                for i in 0..k {
                    let f0 = bf.n_start + bf.allowance * i as f64 / k as f64;
                    let f1 = bf.n_start + bf.allowance * (i + 1) as f64 / k as f64;
                    let corners = [[e0, f0], [e1, f0], [e1, f1], [e0, f1]].map(|[ae, an]| {
                        let p = [bf.e[0] * ae + bf.n[0] * an, bf.e[1] * ae + bf.n[1] * an];
                        bf.apply(p, 0)
                    });
                    let (poly, ds): (Vec<P2>, Vec<f64>) =
                        corners.iter().map(|p| project(*p)).unzip();
                    let Some(plane) = depth_plane(&poly, &ds) else {
                        continue;
                    };
                    let bbox = bbox_of(&poly);
                    facets.push(Facet {
                        zone: z,
                        poly,
                        holes: vec![],
                        plane,
                        bbox,
                    });
                }
                lines.push((z, zone.points.clone(), true));
            }
        }
    }
    if facets.is_empty() {
        return None;
    }

    // Visibility of every line, piece by piece.
    let step = 3.0;
    let mut polylines: Vec<Vec<P2>> = Vec::new();
    for (z, pts, closed) in &lines {
        let dense = densify(pts, step, *closed);
        let n = dense.len();
        let count = if *closed { n } else { n - 1 };
        let mut run: Vec<P2> = Vec::new();
        for i in 0..count {
            let (a, b) = (dense[i], dense[(i + 1) % n]);
            let (Some(a3), Some(b3)) = (lift(*z, a), lift(*z, b)) else {
                continue;
            };
            let (pa, _) = project(a3);
            let (pb, _) = project(b3);
            let mid3 = [
                (a3[0] + b3[0]) / 2.0,
                (a3[1] + b3[1]) / 2.0,
                (a3[2] + b3[2]) / 2.0,
            ];
            let (pm, dm) = project(mid3);
            let hidden = facets.iter().any(|fc| {
                fc.zone != *z
                    && pm[0] > fc.bbox[0]
                    && pm[0] < fc.bbox[2]
                    && pm[1] > fc.bbox[1]
                    && pm[1] < fc.bbox[3]
                    && inside(&fc.poly, pm)
                    && !fc.holes.iter().any(|h| inside(h, pm))
                    && fc.plane[0] * pm[0] + fc.plane[1] * pm[1] + fc.plane[2] > dm + 0.05
            });
            if hidden {
                if run.len() >= 2 {
                    polylines.push(std::mem::take(&mut run));
                } else {
                    run.clear();
                }
                continue;
            }
            if run.is_empty() {
                run.push(pa);
            }
            run.push(pb);
        }
        if run.len() >= 2 {
            polylines.push(run);
        }
    }
    // Merge collinear points so the PDF stays small.
    for pl in &mut polylines {
        let mut out: Vec<P2> = Vec::with_capacity(pl.len());
        for &p in pl.iter() {
            if out.len() >= 2 {
                let a = out[out.len() - 2];
                let b = out[out.len() - 1];
                let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                let len = (p[0] - a[0]).hypot(p[1] - a[1]);
                if cross.abs() < 1e-3 * len.max(1e-9) {
                    *out.last_mut().unwrap() = p;
                    continue;
                }
            }
            out.push(p);
        }
        *pl = out;
    }
    if polylines.is_empty() {
        return None;
    }
    let all: Vec<P2> = polylines.iter().flatten().copied().collect();
    Some(Iso {
        bbox: bbox_of(&all),
        polylines,
    })
}

fn dot(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
