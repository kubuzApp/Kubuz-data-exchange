//! The flat pattern of one solid: the outline and holes of its outer skin
//! as lines and arcs in the skin's own plane, seen from outside, with the
//! kernel's bend lines projected onto it.
//!
//! The kernel's flat view has every zone as its own planar face, and two
//! neighbouring zones share the edges on their tangent line. So the blank's
//! outline is the set of outer-skin edges that only one outer-skin face
//! uses, chained by their shared vertices; a hole that was split across a
//! tangent line comes back whole the same way.

use std::collections::HashMap;

use crate::step::writer::{any_perp, cross, unit};
use crate::view::{BendDirection, Curve, Solid, Surface, P3};

pub type P2 = [f64; 2];

/// One piece of the outline, in blank coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Line {
        a: P2,
        b: P2,
    },
    /// Counter-clockwise from `start` to `end` degrees about `c`.
    Arc {
        c: P2,
        r: f64,
        start: f64,
        end: f64,
    },
    Circle {
        c: P2,
        r: f64,
    },
}

#[derive(Clone, Debug)]
pub struct BendLine {
    pub id: String,
    /// The bend zone's id on the skin.
    pub zone_id: String,
    pub angle_deg: f64,
    pub direction: BendDirection,
    pub inside_radius: f64,
    /// The bend allowance: what the blank is longer by than the two
    /// flanges' flat lengths.
    pub allowance: f64,
    pub a: P2,
    pub b: P2,
}

/// One zone of the outer skin (a flange or a bend zone), as the polygon
/// of its outer loop, sampled.
#[derive(Clone, Debug, Default)]
pub struct Zone {
    pub id: String,
    pub points: Vec<P2>,
}

/// Where the blank's plane sits in the solid it came from: blank
/// `(x, y)` is `(dot(p, u) · sx · scale, dot(p, v) · scale) + shift` for
/// a solid point `p`, and the outer skin is the plane `dot(p, n) = d`
/// with `n` pointing out of the metal.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub u: P3,
    pub v: P3,
    pub n: P3,
    pub d: f64,
    /// `-1` for a mirrored part.
    pub sx: f64,
    pub scale: f64,
    pub shift: P2,
}

impl Default for Frame {
    fn default() -> Self {
        Frame {
            u: [1.0, 0.0, 0.0],
            v: [0.0, 1.0, 0.0],
            n: [0.0, 0.0, 1.0],
            d: 0.0,
            sx: 1.0,
            scale: 1.0,
            shift: [0.0, 0.0],
        }
    }
}

/// A flat pattern: closed loops of segments (the outline and the holes,
/// all cut), the bend lines, and the zones the kernel cut the skin into.
#[derive(Clone, Debug, Default)]
pub struct Blank {
    pub loops: Vec<Vec<Seg>>,
    pub bends: Vec<BendLine>,
    pub zones: Vec<Zone>,
    pub min: P2,
    pub max: P2,
    pub frame: Frame,
}

impl Blank {
    /// A point `[x, y, w]` of the blank (`w` along the skin's normal, in
    /// the blank's scaled millimetres) back in the solid's own
    /// coordinates.
    pub fn to_solid(&self, p: P3) -> P3 {
        self.to_solid_at(p, self.frame.d)
    }

    /// As [`Blank::to_solid`] with the outer skin put at `d` along the
    /// skin's normal instead of where the solid has it.
    pub fn to_solid_at(&self, p: P3, d: f64) -> P3 {
        let f = &self.frame;
        let a = f.sx * (p[0] - f.shift[0]) / f.scale;
        let b = (p[1] - f.shift[1]) / f.scale;
        let c = d + p[2] / f.scale;
        [
            f.u[0] * a + f.v[0] * b + f.n[0] * c,
            f.u[1] * a + f.v[1] * b + f.n[1] * c,
            f.u[2] * a + f.v[2] * b + f.n[2] * c,
        ]
    }

    /// Where the solid's outer skin sits along its normal, as a vector
    /// in the solid's own coordinates. A flat view has a part's neutral
    /// plane at its origin and its outer skin this far from it; placed
    /// (folded), the kernel puts the outer skin at the origin.
    pub fn skin_offset(&self) -> P3 {
        let f = &self.frame;
        [f.n[0] * f.d, f.n[1] * f.d, f.n[2] * f.d]
    }
}

/// The blank of `solid` seen from its outer skin. `mirror` flips it (a
/// part placed by a mirror image) and `scale` sizes it (a part placed by a
/// uniform scale); bend directions stay, because the skin a mirrored part
/// is seen from is the same skin.
pub fn flat_blank(solid: &Solid, mirror: bool, scale: f64) -> Result<Blank, String> {
    if solid.mesh.is_some() {
        return Err("a mesh has no flat pattern".into());
    }
    let outer: Vec<usize> = (0..solid.faces.len())
        .filter(|&i| solid.faces[i].role == "outerSkin")
        .collect();
    if outer.is_empty() {
        return Err("no outer skin: not a sheet".into());
    }
    let mut normal: Option<P3> = None;
    for &fi in &outer {
        let f = &solid.faces[fi];
        let Some(Surface::Plane { normal: n, .. }) = solid.surfaces.get(f.surface) else {
            return Err("the outer skin is not flat (is the view folded?)".into());
        };
        let n = unit(*n);
        let n = if f.same_sense {
            n
        } else {
            [-n[0], -n[1], -n[2]]
        };
        match normal {
            None => normal = Some(n),
            Some(m) => {
                if dot(m, n) < 1.0 - 1e-6 {
                    return Err("the outer skin is not one plane (is the view folded?)".into());
                }
            }
        }
    }
    let n = normal.unwrap();
    // A right-handed frame with the skin's normal toward the viewer.
    let u = if n[2].abs() > 0.999_999 {
        [1.0, 0.0, 0.0]
    } else {
        unit(any_perp(n))
    };
    let v = cross(n, u);
    let to2 = |p: P3| -> P2 {
        let x = dot(p, u) * scale;
        let y = dot(p, v) * scale;
        [if mirror { -x } else { x }, y]
    };

    // Edges used once across the outer skin bound the blank.
    let mut uses: HashMap<usize, usize> = HashMap::new();
    for &fi in &outer {
        for le in solid.faces[fi].loops.iter().flatten() {
            *uses.entry(le.edge).or_default() += 1;
        }
    }
    let mut by_start: HashMap<usize, Vec<(usize, bool)>> = HashMap::new();
    let mut boundary = 0usize;
    for &fi in &outer {
        for le in solid.faces[fi].loops.iter().flatten() {
            if uses[&le.edge] != 1 {
                continue;
            }
            let e = solid
                .edges
                .get(le.edge)
                .ok_or_else(|| format!("edge {} missing", le.edge))?;
            let from = if le.forward { e.start } else { e.end };
            by_start
                .entry(from)
                .or_default()
                .push((le.edge, le.forward));
            boundary += 1;
        }
    }
    if boundary == 0 {
        return Err("the outer skin has no outline".into());
    }

    // Chain them into closed loops.
    let mut loops = Vec::new();
    let mut taken = vec![false; solid.edges.len()];
    let mut starts: Vec<usize> = by_start.keys().copied().collect();
    starts.sort_unstable();
    for s0 in starts {
        while let Some(&(e0, f0)) = by_start[&s0].iter().find(|(e, _)| !taken[*e]) {
            let mut lp = Vec::new();
            let (mut e, mut fwd) = (e0, f0);
            loop {
                taken[e] = true;
                let edge = solid.edges[e];
                lp.push(segment(solid, e, fwd, &to2, n, mirror, scale)?);
                let to = if fwd { edge.end } else { edge.start };
                if to == s0 {
                    break;
                }
                let next = by_start
                    .get(&to)
                    .and_then(|c| c.iter().find(|(e, _)| !taken[*e]))
                    .ok_or_else(|| format!("the outline is open at vertex {to}"))?;
                (e, fwd) = *next;
            }
            loops.push(lp);
        }
    }

    let d = outer
        .iter()
        .flat_map(|&fi| solid.faces[fi].loops.iter().flatten())
        .filter_map(|le| solid.edges.get(le.edge))
        .filter_map(|e| solid.vertices.get(e.start))
        .map(|p| dot(*p, n))
        .next()
        .unwrap_or(0.0);
    let mut blank = Blank {
        loops,
        frame: Frame {
            u,
            v,
            n,
            d,
            sx: if mirror { -1.0 } else { 1.0 },
            scale,
            shift: [0.0, 0.0],
        },
        ..Blank::default()
    };
    // The zones: each outer-skin face's outer loop, sampled on the blank.
    for &fi in &outer {
        let f = &solid.faces[fi];
        let Some(lp) = f.loops.first() else {
            continue;
        };
        let mut points = Vec::new();
        for le in lp {
            let seg = segment(solid, le.edge, le.forward, &to2, n, mirror, scale)?;
            match seg {
                Seg::Line { a, .. } => points.push(a),
                Seg::Arc { c, r, start, end } => {
                    let sweep = (end - start).rem_euclid(360.0);
                    let k = ((sweep / 15.0).ceil() as usize).max(1);
                    let (first, last) = {
                        let a = to2(solid.vertices[if le.forward {
                            solid.edges[le.edge].start
                        } else {
                            solid.edges[le.edge].end
                        }]);
                        let ccw = (angle_deg(c, a) - start).rem_euclid(360.0) < 1e-6;
                        if ccw {
                            (start, start + sweep)
                        } else {
                            (end, end - sweep)
                        }
                    };
                    for i in 0..k {
                        let t = first + (last - first) * i as f64 / k as f64;
                        points.push(arc_point(c, r, t));
                    }
                }
                Seg::Circle { .. } => {}
            }
        }
        if points.len() >= 3 {
            blank.zones.push(Zone {
                id: f.zone_id.clone(),
                points,
            });
        }
    }
    for b in &solid.bends {
        blank.bends.push(BendLine {
            id: b.id.clone(),
            zone_id: b.zone_id.clone(),
            angle_deg: b.angle_deg,
            direction: b.direction,
            inside_radius: b.inside_radius * scale,
            allowance: b.allowance * scale,
            a: to2(b.line[0]),
            b: to2(b.line[1]),
        });
    }
    blank.normalise();
    Ok(blank)
}

fn segment(
    solid: &Solid,
    e: usize,
    fwd: bool,
    to2: &impl Fn(P3) -> P2,
    n: P3,
    mirror: bool,
    scale: f64,
) -> Result<Seg, String> {
    let edge = solid.edges[e];
    let (vs, ve) = if fwd {
        (edge.start, edge.end)
    } else {
        (edge.end, edge.start)
    };
    let a = to2(solid.vertices[vs]);
    let b = to2(solid.vertices[ve]);
    match solid
        .curves
        .get(edge.curve)
        .ok_or_else(|| format!("curve {} missing", edge.curve))?
    {
        Curve::Line => Ok(Seg::Line { a, b }),
        Curve::Circle {
            center,
            axis,
            radius,
        } => {
            let c = to2(*center);
            let r = radius * scale;
            if edge.start == edge.end {
                return Ok(Seg::Circle { c, r });
            }
            // The curve runs counter-clockwise about its axis from the
            // edge's start to its end; in the blank, seen with the skin's
            // normal toward the viewer, that is counter-clockwise when the
            // axis points the same way. The loop direction and a mirror
            // each turn it round, but a DXF arc always runs
            // counter-clockwise, so only which end is first changes.
            let (pa, pb) = if fwd { (a, b) } else { (b, a) };
            let n_up = (dot(*axis, n) > 0.0) ^ mirror;
            let (first, last) = if n_up { (pa, pb) } else { (pb, pa) };
            Ok(Seg::Arc {
                c,
                r,
                start: angle_deg(c, first),
                end: angle_deg(c, last),
            })
        }
        Curve::Bspline { .. } => {
            Err("a spline on the outline is not written yet (lines and arcs only)".into())
        }
    }
}

pub fn angle_deg(c: P2, p: P2) -> f64 {
    let a = (p[1] - c[1]).atan2(p[0] - c[0]).to_degrees();
    if a < 0.0 {
        a + 360.0
    } else {
        a
    }
}

fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl Blank {
    /// Move the blank so its lower-left corner sits at the origin, and
    /// record its bounds.
    fn normalise(&mut self) {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        let mut take = |p: P2| {
            for i in 0..2 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        };
        for s in self.loops.iter().flatten() {
            match *s {
                Seg::Line { a, b } => {
                    take(a);
                    take(b);
                }
                Seg::Arc { c, r, start, end } => {
                    take(arc_point(c, r, start));
                    take(arc_point(c, r, end));
                    // The axis crossings the arc passes through.
                    let sweep = (end - start).rem_euclid(360.0);
                    for q in [0.0, 90.0, 180.0, 270.0] {
                        if (q - start).rem_euclid(360.0) <= sweep {
                            take(arc_point(c, r, q));
                        }
                    }
                }
                Seg::Circle { c, r } => {
                    take([c[0] - r, c[1] - r]);
                    take([c[0] + r, c[1] + r]);
                }
            }
        }
        let shift = [-lo[0], -lo[1]];
        self.frame.shift = shift;
        let mv = |p: &mut P2| {
            p[0] += shift[0];
            p[1] += shift[1];
        };
        for s in self.loops.iter_mut().flatten() {
            match s {
                Seg::Line { a, b } => {
                    mv(a);
                    mv(b);
                }
                Seg::Arc { c, .. } | Seg::Circle { c, .. } => mv(c),
            }
        }
        for b in &mut self.bends {
            mv(&mut b.a);
            mv(&mut b.b);
        }
        for z in &mut self.zones {
            for p in &mut z.points {
                mv(p);
            }
        }
        self.min = [0.0, 0.0];
        self.max = [hi[0] - lo[0], hi[1] - lo[1]];
    }
}

pub fn arc_point(c: P2, r: f64, deg: f64) -> P2 {
    let t = deg.to_radians();
    [c[0] + r * t.cos(), c[1] + r * t.sin()]
}
