//! A pictorial of placed parts with hidden lines removed: the isometric
//! of a folded blank on its own sheet, and the assembly views (isometric,
//! front, top and left side) of every placed part.
//!
//! A scene holds facets and lines in model space. A blank contributes its
//! zones folded on the outer skin (flanges as planes, bends as chord
//! strips) and the outlines and holes as lines. Any other solid (a bar, a
//! tube, a plate with holes) contributes every face as facets (planes as
//! their loops, cylinders and ruled surfaces as chord strips) and every
//! edge that is not a smooth junction as a line; cylinders also get their
//! silhouette generators per view. A piece of a line is hidden when a
//! facet of another zone lies in front of its midpoint.

use super::features::outer_loop;
use super::project::{self, bspline, sample_edge};
use super::section::{fold, P3};
use crate::dxf::blank::{Blank, P2};
use crate::step::writer::{any_perp, cross, sub, unit};
use crate::view::{Curve, Solid, Surface};

/// The pictorial, on the picture plane in model millimetres.
#[derive(Clone, Debug, Default)]
pub struct Iso {
    pub polylines: Vec<Vec<P2>>,
    /// Which object of the scene each polyline belongs to.
    pub owner: Vec<usize>,
    /// Whether the polyline is a tangent edge (drawn thin, ISO 128-2).
    pub thin: Vec<bool>,
    pub bbox: [f64; 4],
}

/// A view direction: where the picture plane's axes and the depth (toward
/// the viewer) come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// The viewer at (-1, -1, 1): x to the right and up, y to the left and
    /// up, z up.
    Iso,
    /// Seen from the front (the viewer at -y): x right, z up.
    Front,
    /// Seen from above: x right, y up (first angle: the object's back
    /// against the front view above it).
    Top,
    /// Seen from the left (the viewer at -x): -y right, z up (first angle:
    /// drawn right of the front view).
    Left,
}

impl Dir {
    /// The picture point and the depth (larger is nearer the viewer).
    pub fn project(self, p: P3) -> (P2, f64) {
        match self {
            Dir::Iso => {
                let sx = (p[0] - p[1]) / 2f64.sqrt();
                let sy = (p[0] + p[1] + 2.0 * p[2]) / 6f64.sqrt();
                let depth = (-p[0] - p[1] + p[2]) / 3f64.sqrt();
                ([sx, sy], depth)
            }
            Dir::Front => ([p[0], p[2]], -p[1]),
            Dir::Top => ([p[0], p[1]], p[2]),
            Dir::Left => ([-p[1], p[2]], -p[0]),
        }
    }

    /// Unit vector toward the viewer.
    pub fn toward(self) -> P3 {
        match self {
            Dir::Iso => unit([-1.0, -1.0, 1.0]),
            Dir::Front => [0.0, -1.0, 0.0],
            Dir::Top => [0.0, 0.0, 1.0],
            Dir::Left => [-1.0, 0.0, 0.0],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Dir::Iso => "iso",
            Dir::Front => "front",
            Dir::Top => "top",
            Dir::Left => "left",
        }
    }
}

/// Isometric axes, as [`Dir::Iso`].
pub fn project(p: P3) -> (P2, f64) {
    Dir::Iso.project(p)
}

/// A planar piece of surface in model space.
struct Facet {
    /// Which zone it belongs to (lines of that zone are never hidden by
    /// it).
    zone: usize,
    poly: Vec<P3>,
    holes: Vec<Vec<P3>>,
}

/// A projected facet: outline and holes on the picture plane, the depth
/// as an affine function of the picture point, and its bounds.
struct Flat {
    zone: usize,
    poly: Ring,
    holes: Vec<Ring>,
    plane: [f64; 3],
    bbox: [f64; 4],
}

/// A projected outline or hole, ready for many point-in-polygon tests: a
/// point outside its bounds is never inside, and a long outline (a
/// grating floor's, a slotted plate's) keeps its edges in horizontal
/// bands so a test walks only the edges at the point's height. The
/// answer is the same as `inside` on the points.
struct Ring {
    pts: Vec<P2>,
    bbox: [f64; 4],
    /// Edge indices per band of height `band`, from `bbox[1]`; empty for
    /// a short ring.
    bands: Vec<Vec<u32>>,
    band: f64,
}

/// Rings with more edges than this get bands.
const BAND_EDGES: usize = 48;

impl Ring {
    fn new(pts: Vec<P2>) -> Ring {
        let bbox = bbox_of(&pts);
        let n = pts.len();
        let mut ring = Ring {
            pts,
            bbox,
            bands: Vec::new(),
            band: 0.0,
        };
        if n > BAND_EDGES {
            let count = (n / 8).clamp(4, 256);
            ring.band = ((bbox[3] - bbox[1]) / count as f64).max(1e-9);
            ring.bands = vec![Vec::new(); count];
            for i in 0..n {
                let (a, b) = (ring.pts[i], ring.pts[(i + 1) % n]);
                let (lo, hi) = (ring.band_of(a[1].min(b[1])), ring.band_of(a[1].max(b[1])));
                for band in &mut ring.bands[lo..=hi] {
                    band.push(i as u32);
                }
            }
        }
        ring
    }

    fn band_of(&self, y: f64) -> usize {
        let k = ((y - self.bbox[1]) / self.band).floor().max(0.0) as usize;
        k.min(self.bands.len() - 1)
    }

    fn contains(&self, p: P2) -> bool {
        if !in_box(&self.bbox, p) {
            return false;
        }
        if self.bands.is_empty() {
            return inside(&self.pts, p);
        }
        // An edge crosses the point's height only when its low end is at
        // or under it and its high end above, so it is in this band.
        let n = self.pts.len();
        let mut hit = false;
        for &i in &self.bands[self.band_of(p[1])] {
            let (a, b) = (self.pts[i as usize], self.pts[(i as usize + 1) % n]);
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > p[0] {
                    hit = !hit;
                }
            }
        }
        hit
    }
}

/// A line of the scene: the zones it lies on (never hidden by their
/// facets), its points in model space, and the object it came from.
struct Line {
    zones: [usize; 2],
    pts: Vec<P3>,
    closed: bool,
    object: usize,
    /// A tangent edge (two faces meeting smoothly): drawn thin.
    thin: bool,
}

/// A cylindrical face whose silhouette generators depend on the view.
struct Cyl {
    zone: usize,
    object: usize,
    c: P3,
    axis: P3,
    r: f64,
    h: [f64; 2],
    /// Angular range from `e1` (full turn when `None`).
    ang: Option<[f64; 2]>,
    e1: P3,
    e2: P3,
}

/// Placed parts, ready to render.
#[derive(Default)]
pub struct Scene {
    facets: Vec<Facet>,
    lines: Vec<Line>,
    cyls: Vec<Cyl>,
    zones: usize,
    objects: usize,
    /// Each object's bounds in model space.
    bounds: Vec<[f64; 6]>,
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

fn grow(b: &mut [f64; 6], p: P3) {
    for i in 0..3 {
        b[i] = b[i].min(p[i]);
        b[i + 3] = b[i + 3].max(p[i]);
    }
}

/// `p` within `b`, edges included: outside it `inside` is false for any
/// polygon `b` bounds.
fn in_box(b: &[f64; 4], p: P2) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
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

/// The projected facets bucketed by picture area, so the visibility test
/// of a point looks only at the facets whose bounds can cover it (a
/// 200-part staircase has tens of thousands of facets).
struct Grid {
    origin: P2,
    cell: P2,
    n: [usize; 2],
    cells: Vec<Vec<u32>>,
}

impl Grid {
    fn new(flats: &[Flat]) -> Grid {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for f in flats {
            b[0] = b[0].min(f.bbox[0]);
            b[1] = b[1].min(f.bbox[1]);
            b[2] = b[2].max(f.bbox[2]);
            b[3] = b[3].max(f.bbox[3]);
        }
        // About as many cells as facets, at most 256 a side.
        let side = ((flats.len() as f64).sqrt().ceil() as usize).clamp(1, 256);
        let n = [side, side];
        let cell = [
            ((b[2] - b[0]) / side as f64).max(1e-9),
            ((b[3] - b[1]) / side as f64).max(1e-9),
        ];
        let mut g = Grid {
            origin: [b[0], b[1]],
            cell,
            n,
            cells: vec![Vec::new(); side * side],
        };
        for (i, f) in flats.iter().enumerate() {
            let (x0, y0) = g.index([f.bbox[0], f.bbox[1]]);
            let (x1, y1) = g.index([f.bbox[2], f.bbox[3]]);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    g.cells[y * g.n[0] + x].push(i as u32);
                }
            }
        }
        g
    }

    fn index(&self, p: P2) -> (usize, usize) {
        let ix = ((p[0] - self.origin[0]) / self.cell[0]).floor();
        let iy = ((p[1] - self.origin[1]) / self.cell[1]).floor();
        (
            (ix.max(0.0) as usize).min(self.n[0] - 1),
            (iy.max(0.0) as usize).min(self.n[1] - 1),
        )
    }

    /// The facets whose bounds may hold `p`.
    fn at(&self, p: P2) -> &[u32] {
        let (x, y) = self.index(p);
        &self.cells[y * self.n[0] + x]
    }
}

/// Chord slices of a cylinder or a bend, in degrees.
const SLICE_DEG: f64 = 7.5;

impl Scene {
    pub fn new() -> Self {
        Scene::default()
    }

    /// How many objects were added.
    pub fn objects(&self) -> usize {
        self.objects
    }

    /// Object `i`'s bounds `[x0, y0, z0, x1, y1, z1]` in model space.
    pub fn bounds(&self, i: usize) -> [f64; 6] {
        self.bounds[i]
    }

    /// The bounds of everything in model space.
    pub fn extents(&self) -> [f64; 6] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for o in &self.bounds {
            for i in 0..3 {
                b[i] = b[i].min(o[i]);
                b[i + 3] = b[i + 3].max(o[i + 3]);
            }
        }
        b
    }

    fn begin_object(&mut self) -> usize {
        self.bounds.push([
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ]);
        self.objects += 1;
        self.objects - 1
    }

    fn push_facet(&mut self, object: usize, zone: usize, poly: Vec<P3>, holes: Vec<Vec<P3>>) {
        for p in poly.iter().chain(holes.iter().flatten()) {
            grow(&mut self.bounds[object], *p);
        }
        self.facets.push(Facet { zone, poly, holes });
    }

    fn push_line(&mut self, object: usize, zones: [usize; 2], pts: Vec<P3>, closed: bool) {
        self.push_line_weight(object, zones, pts, closed, false);
    }

    fn push_line_weight(
        &mut self,
        object: usize,
        zones: [usize; 2],
        pts: Vec<P3>,
        closed: bool,
        thin: bool,
    ) {
        if pts.len() < 2 {
            return;
        }
        for p in &pts {
            grow(&mut self.bounds[object], *p);
        }
        self.lines.push(Line {
            zones,
            pts,
            closed,
            object,
            thin,
        });
    }

    /// Add `blank` folded with thickness `t`, every folded point mapped by
    /// `world` (identity for a part on its own). Returns the blank's index
    /// in the scene, or `None` when it does not fold (a flat plate is
    /// added as its one zone).
    pub fn add_blank(&mut self, blank: &Blank, t: f64, world: &dyn Fn(P3) -> P3) -> Option<usize> {
        let fd = fold(blank, t);
        let flat = blank.bends.is_empty();
        if !flat && fd.is_none() {
            return None;
        }
        let bend_of = |zone: &str| blank.bends.iter().position(|b| b.zone_id == zone);
        // A point of zone `z` on the folded outer skin, in the world.
        let lift = |z: usize, p: P2| -> Option<P3> {
            if flat {
                return Some(world([p[0], p[1], 0.0]));
            }
            let fd = fd.as_ref()?;
            match bend_of(&blank.zones[z].id) {
                Some(bi) => fd.bends[bi].map(|bf| world(bf.apply(p, 0))),
                None => fd.transform[z].map(|tr| world(tr.apply([p[0], p[1], 0.0]))),
            }
        };
        // Zones; a plate without zones is its outline.
        let outline: Vec<P2>;
        let zones: Vec<(String, &[P2])> = if blank.zones.is_empty() {
            let outer = outer_loop(&blank.loops);
            let lp = blank.loops.get(outer)?;
            outline = super::features::loop_points(lp, 1.0);
            vec![(String::new(), outline.as_slice())]
        } else {
            blank
                .zones
                .iter()
                .map(|z| (z.id.clone(), z.points.as_slice()))
                .collect()
        };
        let zone_lift = |z: usize, p: P2| -> Option<P3> {
            if blank.zones.is_empty() {
                Some(world([p[0], p[1], 0.0]))
            } else {
                lift(z, p)
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
            let Some(z) = (0..zones.len())
                .filter(|&z| bend_of(&zones[z].0).is_none())
                .find(|&z| inside(zones[z].1, pts[0]))
            else {
                continue;
            };
            holes.push((z, pts));
        }

        let base = self.zones;
        let object = self.begin_object();
        let mut added = 0;
        let step = 3.0;
        for (z, (zid, points)) in zones.iter().enumerate() {
            if points.len() < 3 {
                continue;
            }
            let gz = base + z;
            let is_bend = !blank.zones.is_empty() && bend_of(zid).is_some();
            if !is_bend {
                let pts3: Option<Vec<P3>> = points.iter().map(|p| zone_lift(z, *p)).collect();
                let Some(pts3) = pts3 else {
                    continue;
                };
                let mut hs: Vec<Vec<P3>> = Vec::new();
                for (_, pts) in holes.iter().filter(|(hz, _)| *hz == z) {
                    let h: Option<Vec<P3>> = pts.iter().map(|p| zone_lift(z, *p)).collect();
                    if let Some(h) = h {
                        hs.push(h);
                    }
                }
                self.push_facet(object, gz, pts3, hs);
                let mut outline_pts = Vec::new();
                for p in densify(points, step, true) {
                    if let Some(q) = zone_lift(z, p) {
                        outline_pts.push(q);
                    }
                }
                self.push_line(object, [gz, gz], outline_pts, true);
                added += 1;
                for (_, pts) in holes.iter().filter(|(hz, _)| *hz == z) {
                    let mut hp = Vec::new();
                    for p in densify(pts, step, true) {
                        if let Some(q) = zone_lift(z, p) {
                            hp.push(q);
                        }
                    }
                    self.push_line(object, [gz, gz], hp, true);
                }
            } else {
                let bi = bend_of(zid).unwrap();
                let Some(bf) = fd.as_ref().and_then(|fd| fd.bends[bi]) else {
                    continue;
                };
                let b = &blank.bends[bi];
                let (e0, e1) = (
                    dot2(b.a, bf.e).min(dot2(b.b, bf.e)),
                    dot2(b.a, bf.e).max(dot2(b.b, bf.e)),
                );
                let k = ((b.angle_deg / SLICE_DEG).ceil() as usize).max(4);
                for i in 0..k {
                    let f0 = bf.n_start + bf.allowance * i as f64 / k as f64;
                    let f1 = bf.n_start + bf.allowance * (i + 1) as f64 / k as f64;
                    let corners = [[e0, f0], [e1, f0], [e1, f1], [e0, f1]].map(|[ae, an]| {
                        let p = [bf.e[0] * ae + bf.n[0] * an, bf.e[1] * ae + bf.n[1] * an];
                        world(bf.apply(p, 0))
                    });
                    self.push_facet(object, gz, corners.to_vec(), vec![]);
                }
                let mut outline_pts = Vec::new();
                for p in densify(points, step, true) {
                    if let Some(q) = lift(z, p) {
                        outline_pts.push(q);
                    }
                }
                self.push_line(object, [gz, gz], outline_pts, true);
                added += 1;
            }
        }
        self.zones += zones.len();
        if added == 0 {
            self.objects -= 1;
            self.bounds.pop();
            return None;
        }
        // The inner skin counts for the object's bounds (the pictorial
        // draws the outer skin only).
        if let Some(fd) = fd.as_ref() {
            for (z, tr) in fd.transform.iter().enumerate() {
                let Some(tr) = tr else {
                    continue;
                };
                for p in &blank.zones[z].points {
                    grow(&mut self.bounds[object], world(tr.apply([p[0], p[1], -t])));
                }
            }
            for (bi, b) in blank.bends.iter().enumerate() {
                let Some(bf) = fd.bends[bi] else {
                    continue;
                };
                for k in 0..=8 {
                    let f = k as f64 / 8.0;
                    for end in [b.a, b.b] {
                        let along = dot2(end, bf.n);
                        let p = [
                            end[0] + bf.n[0] * (bf.n_start + bf.allowance * f - along),
                            end[1] + bf.n[1] * (bf.n_start + bf.allowance * f - along),
                        ];
                        grow(&mut self.bounds[object], world(bf.apply(p, 1)));
                    }
                }
            }
        } else if t > 0.0 {
            for lp in &blank.loops {
                for p in super::features::loop_points(lp, 1.0) {
                    grow(&mut self.bounds[object], world([p[0], p[1], -t]));
                }
            }
        }
        Some(object)
    }

    /// Add a B-rep solid as it is (a bar, a tube, a plate with holes, a
    /// folded part), every point mapped by `world`. Returns the object's
    /// index, or `None` when the solid has no faces.
    pub fn add_solid(&mut self, solid: &Solid, world: &dyn Fn(P3) -> P3) -> Option<usize> {
        if solid.faces.is_empty() || solid.edges.is_empty() {
            return None;
        }
        let topo = project::topology(solid);
        let base = self.zones;
        let object = self.begin_object();
        let step = 4.0;
        // Every edge once, as a line on both its faces: a tangent edge
        // (a flange meeting its bend) thin, a seam (one face twice) not
        // at all.
        let mut seen = vec![false; solid.edges.len()];
        for (fi, f) in solid.faces.iter().enumerate() {
            for lp in &f.loops {
                for le in lp {
                    let ei = le.edge;
                    if ei >= solid.edges.len() || seen[ei] {
                        continue;
                    }
                    seen[ei] = true;
                    let fs = &topo.faces_of_edge[ei];
                    let Some(other) = fs.iter().copied().find(|&g| g != fi) else {
                        continue;
                    };
                    let pts: Vec<P3> = sample_edge(solid, ei, step)
                        .pts
                        .into_iter()
                        .map(world)
                        .collect();
                    self.push_line_weight(
                        object,
                        [base + fi, base + other],
                        pts,
                        false,
                        topo.smooth[ei],
                    );
                }
            }
        }
        // Every face as facets.
        for (fi, f) in solid.faces.iter().enumerate() {
            let zone = base + fi;
            let loops: Vec<Vec<P3>> = f
                .loops
                .iter()
                .map(|lp| loop_points(solid, lp, step))
                .filter(|l| l.len() >= 3)
                .collect();
            match solid.surfaces.get(f.surface) {
                Some(Surface::Plane { .. }) => {
                    let mut it = loops.into_iter();
                    let Some(outer) = it.next() else {
                        continue;
                    };
                    let poly: Vec<P3> = outer.into_iter().map(world).collect();
                    let holes: Vec<Vec<P3>> =
                        it.map(|h| h.into_iter().map(world).collect()).collect();
                    self.push_facet(object, zone, poly, holes);
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
                    let all: Vec<P3> = loops.iter().flatten().copied().collect();
                    if all.is_empty() {
                        continue;
                    }
                    let mut h = [f64::INFINITY, f64::NEG_INFINITY];
                    let mut angs: Vec<f64> = Vec::with_capacity(all.len());
                    for p in &all {
                        let v = sub(*p, *origin);
                        let hh = project::dot(v, ax);
                        h = [h[0].min(hh), h[1].max(hh)];
                        angs.push(project::dot(v, e2).atan2(project::dot(v, e1)));
                    }
                    // A loop edge that is a full circle makes the face a
                    // full turn; else the covered range is the complement
                    // of the largest gap between the sampled angles.
                    let full = f.loops.iter().flatten().any(|le| {
                        let e = solid.edges[le.edge];
                        e.start == e.end
                            && matches!(solid.curves.get(e.curve), Some(Curve::Circle { .. }))
                    });
                    let ang = if full {
                        None
                    } else {
                        angs.sort_by(f64::total_cmp);
                        let mut gap = (0.0, 0);
                        for i in 0..angs.len() {
                            let next = if i + 1 < angs.len() {
                                angs[i + 1]
                            } else {
                                angs[0] + std::f64::consts::TAU
                            };
                            if next - angs[i] > gap.0 {
                                gap = (next - angs[i], i);
                            }
                        }
                        // The range runs from the angle after the gap to
                        // the angle before it.
                        let a0 = angs[(gap.1 + 1) % angs.len()];
                        let mut a1 = angs[gap.1];
                        if a1 < a0 {
                            a1 += std::f64::consts::TAU;
                        }
                        if a1 - a0 < 1e-6 {
                            continue;
                        }
                        Some([a0, a1])
                    };
                    let (a0, a1) = match ang {
                        None => (0.0, std::f64::consts::TAU),
                        Some([a0, a1]) => (a0, a1),
                    };
                    let k = (((a1 - a0).to_degrees() / SLICE_DEG).ceil() as usize).max(2);
                    let at = |a: f64, hh: f64| -> P3 {
                        let (ca, sa) = (a.cos(), a.sin());
                        world([
                            origin[0] + ax[0] * hh + r * (e1[0] * ca + e2[0] * sa),
                            origin[1] + ax[1] * hh + r * (e1[1] * ca + e2[1] * sa),
                            origin[2] + ax[2] * hh + r * (e1[2] * ca + e2[2] * sa),
                        ])
                    };
                    for i in 0..k {
                        let b0 = a0 + (a1 - a0) * i as f64 / k as f64;
                        let b1 = a0 + (a1 - a0) * (i + 1) as f64 / k as f64;
                        let quad = vec![at(b0, h[0]), at(b1, h[0]), at(b1, h[1]), at(b0, h[1])];
                        self.push_facet(object, zone, quad, vec![]);
                    }
                    // The world frame of the cylinder, for its silhouette.
                    let o = world(*origin);
                    let wax = sub(
                        world([origin[0] + ax[0], origin[1] + ax[1], origin[2] + ax[2]]),
                        o,
                    );
                    let we1 = sub(
                        world([origin[0] + e1[0], origin[1] + e1[1], origin[2] + e1[2]]),
                        o,
                    );
                    let we2 = sub(
                        world([origin[0] + e2[0], origin[1] + e2[1], origin[2] + e2[2]]),
                        o,
                    );
                    self.cyls.push(Cyl {
                        zone,
                        object,
                        c: o,
                        axis: wax,
                        r,
                        h,
                        ang,
                        e1: we1,
                        e2: we2,
                    });
                }
                Some(Surface::RuledBspline { top, bottom }) => {
                    let (Some(t), Some(b)) = (solid.curves.get(*top), solid.curves.get(*bottom))
                    else {
                        continue;
                    };
                    let n = 24;
                    let ev = |c: &Curve, s: f64| -> Option<P3> {
                        match c {
                            Curve::Bspline {
                                degree,
                                control_points,
                                knots,
                                multiplicities,
                            } => Some(bspline(*degree, control_points, knots, multiplicities, s)),
                            _ => None,
                        }
                    };
                    let mut quads = Vec::with_capacity(n);
                    for i in 0..n {
                        let (s0, s1) = (i as f64 / n as f64, (i + 1) as f64 / n as f64);
                        let (Some(t0), Some(t1), Some(b0), Some(b1)) =
                            (ev(t, s0), ev(t, s1), ev(b, s0), ev(b, s1))
                        else {
                            break;
                        };
                        quads.push(vec![world(t0), world(t1), world(b1), world(b0)]);
                    }
                    for q in quads {
                        self.push_facet(object, zone, q, vec![]);
                    }
                }
                None => {}
            }
        }
        self.zones += solid.faces.len();
        Some(object)
    }

    /// The silhouette generators of the cylinders seen along `dir`: the
    /// two rulings where the surface normal is square to the view, when
    /// they lie on the face.
    fn silhouettes(&self, dir: Dir) -> Vec<Line> {
        let v = dir.toward();
        let mut out = Vec::new();
        for cy in &self.cyls {
            // Normal at angle a is e1 cos a + e2 sin a; square to v when
            // tan a = -(v·e1)/(v·e2).
            let (d1, d2) = (project::dot(v, cy.e1), project::dot(v, cy.e2));
            if d1.abs() < 1e-9 && d2.abs() < 1e-9 {
                continue;
            }
            let a = (-d1).atan2(d2);
            for ang in [a, a + std::f64::consts::PI] {
                let on = match cy.ang {
                    None => true,
                    Some([a0, a1]) => {
                        let mut t = ang.rem_euclid(std::f64::consts::TAU);
                        if t < a0 {
                            t += std::f64::consts::TAU;
                        }
                        t > a0 + 1e-6 && t < a1 - 1e-6
                    }
                };
                if !on {
                    continue;
                }
                let (ca, sa) = (ang.cos(), ang.sin());
                let rim = [
                    cy.r * (cy.e1[0] * ca + cy.e2[0] * sa),
                    cy.r * (cy.e1[1] * ca + cy.e2[1] * sa),
                    cy.r * (cy.e1[2] * ca + cy.e2[2] * sa),
                ];
                let at = |hh: f64| -> P3 {
                    [
                        cy.c[0] + cy.axis[0] * hh + rim[0],
                        cy.c[1] + cy.axis[1] * hh + rim[1],
                        cy.c[2] + cy.axis[2] * hh + rim[2],
                    ]
                };
                out.push(Line {
                    zones: [cy.zone, cy.zone],
                    pts: vec![at(cy.h[0]), at(cy.h[1])],
                    closed: false,
                    object: cy.object,
                    thin: false,
                });
            }
        }
        out
    }

    /// The scene's visible lines seen along `dir`, or `None` when there
    /// is nothing to draw.
    pub fn render(&self, dir: Dir) -> Option<Iso> {
        if self.facets.is_empty() {
            return None;
        }
        let flats: Vec<Flat> = self
            .facets
            .iter()
            .filter_map(|fc| {
                let (poly, ds): (Vec<P2>, Vec<f64>) =
                    fc.poly.iter().map(|p| dir.project(*p)).unzip();
                let plane = depth_plane(&poly, &ds)?;
                let holes = fc
                    .holes
                    .iter()
                    .map(|h| Ring::new(h.iter().map(|p| dir.project(*p).0).collect()))
                    .collect();
                Some(Flat {
                    zone: fc.zone,
                    bbox: bbox_of(&poly),
                    poly: Ring::new(poly),
                    holes,
                    plane,
                })
            })
            .collect();
        // Sub-segments about this long are tested for visibility: fine
        // enough that a sliver of a hidden stretch stays under a paper
        // line's width at the sheet's scale.
        let ext = self.extents();
        let span = (0..3).map(|i| ext[i + 3] - ext[i]).fold(0.0, f64::max);
        let step = (span / 400.0).clamp(2.0, 20.0);
        let grid = Grid::new(&flats);
        let hidden = |a3: P3, b3: P3, zones: [usize; 2]| -> bool {
            let mid3 = [
                (a3[0] + b3[0]) / 2.0,
                (a3[1] + b3[1]) / 2.0,
                (a3[2] + b3[2]) / 2.0,
            ];
            let (pm, dm) = dir.project(mid3);
            grid.at(pm).iter().any(|&f| {
                let fc = &flats[f as usize];
                fc.zone != zones[0]
                    && fc.zone != zones[1]
                    && pm[0] > fc.bbox[0]
                    && pm[0] < fc.bbox[2]
                    && pm[1] > fc.bbox[1]
                    && pm[1] < fc.bbox[3]
                    && fc.plane[0] * pm[0] + fc.plane[1] * pm[1] + fc.plane[2] > dm + 0.05
                    && fc.poly.contains(pm)
                    && !fc.holes.iter().any(|h| h.contains(pm))
            })
        };
        let mut polylines: Vec<Vec<P2>> = Vec::new();
        let mut owner: Vec<usize> = Vec::new();
        let mut thin: Vec<bool> = Vec::new();
        let extra = self.silhouettes(dir);
        for line in self.lines.iter().chain(extra.iter()) {
            let n = line.pts.len();
            if n < 2 {
                continue;
            }
            let count = if line.closed { n } else { n - 1 };
            let mut run: Vec<P2> = Vec::new();
            for i in 0..count {
                let (a3, b3) = (line.pts[i], line.pts[(i + 1) % n]);
                let len = project::norm(sub(b3, a3));
                let k = ((len / step).ceil() as usize).max(1);
                for j in 0..k {
                    let f0 = j as f64 / k as f64;
                    let f1 = (j + 1) as f64 / k as f64;
                    let p3 = lerp(a3, b3, f0);
                    let q3 = lerp(a3, b3, f1);
                    let (pa, _) = dir.project(p3);
                    let (pb, _) = dir.project(q3);
                    if hidden(p3, q3, line.zones) {
                        if run.len() >= 2 {
                            polylines.push(std::mem::take(&mut run));
                            owner.push(line.object);
                            thin.push(line.thin);
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
            }
            if run.len() >= 2 {
                polylines.push(run);
                owner.push(line.object);
                thin.push(line.thin);
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
                    // Straight on, not back on itself (an edge-on plate
                    // outline runs out and back).
                    let ahead = (b[0] - a[0]) * (p[0] - b[0]) + (b[1] - a[1]) * (p[1] - b[1]);
                    if cross.abs() < 1e-3 * len.max(1e-9) && ahead >= 0.0 {
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
            owner,
            thin,
        })
    }
}

fn lerp(a: P3, b: P3, f: f64) -> P3 {
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}

/// The points of a face loop in order, each edge sampled about `step`
/// apart and run the way the loop uses it, without the repeated
/// junctions.
fn loop_points(solid: &Solid, lp: &[crate::view::LoopEdge], step: f64) -> Vec<P3> {
    let mut out: Vec<P3> = Vec::new();
    for le in lp {
        if le.edge >= solid.edges.len() {
            continue;
        }
        let mut s = sample_edge(solid, le.edge, step).pts;
        if !le.forward {
            s.reverse();
        }
        for p in s {
            if out.last().is_none_or(|q| project::norm(sub(p, *q)) > 1e-9) {
                out.push(p);
            }
        }
    }
    if out.len() > 1 && project::norm(sub(out[0], out[out.len() - 1])) <= 1e-9 {
        out.pop();
    }
    out
}

/// The pictorial of one folded blank, or `None` for a flat plate or a
/// blank that does not fold.
pub fn isometric(blank: &Blank, t: f64) -> Option<Iso> {
    if blank.bends.is_empty() {
        return None;
    }
    let mut scene = Scene::new();
    scene.add_blank(blank, t, &|p| p)?;
    scene.render(Dir::Iso)
}

fn dot2(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ring answers as `inside` on its points, banded or not: on
    /// random points, on vertices and on edge heights.
    #[test]
    fn a_ring_contains_what_inside_says() {
        let mut seed = 7u64;
        let mut rnd = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for n in [3usize, 12, 47, 49, 200, 1000] {
            // A star: radius varies, so many edges share heights.
            let pts: Vec<P2> = (0..n)
                .map(|i| {
                    let a = i as f64 / n as f64 * std::f64::consts::TAU;
                    let r = 20.0 + 15.0 * rnd();
                    [r * a.cos(), (r * a.sin() * 4.0).round() / 4.0]
                })
                .collect();
            let ring = Ring::new(pts.clone());
            assert_eq!(ring.bands.is_empty(), n <= BAND_EDGES);
            let mut probes: Vec<P2> = (0..4000)
                .map(|_| [rnd() * 80.0 - 40.0, (rnd() * 320.0 - 160.0).round() / 4.0])
                .collect();
            probes.extend(pts.iter().copied());
            for p in probes {
                assert_eq!(ring.contains(p), inside(&pts, p), "n {n} at {p:?}");
            }
        }
    }

    /// The grid hands every facet whose bounds hold a point to the test.
    #[test]
    fn the_grid_finds_every_facet_over_a_point() {
        let flat = |x0: f64, y0: f64, w: f64, h: f64| {
            let poly = vec![[x0, y0], [x0 + w, y0], [x0 + w, y0 + h], [x0, y0 + h]];
            Flat {
                zone: 0,
                bbox: bbox_of(&poly),
                poly: Ring::new(poly),
                holes: Vec::new(),
                plane: [0.0; 3],
            }
        };
        let flats: Vec<Flat> = (0..60)
            .map(|i| {
                let f = i as f64;
                flat(
                    f * 7.0 % 90.0,
                    f * 13.0 % 70.0,
                    3.0 + f % 40.0,
                    2.0 + f % 25.0,
                )
            })
            .collect();
        let grid = Grid::new(&flats);
        for i in 0..120 {
            for j in 0..100 {
                let p = [i as f64 - 5.0 + 0.37, j as f64 - 5.0 + 0.61];
                let near = grid.at(p);
                for (k, f) in flats.iter().enumerate() {
                    if in_box(&f.bbox, p) {
                        assert!(near.contains(&(k as u32)), "facet {k} at {p:?}");
                    }
                }
            }
        }
    }
}
