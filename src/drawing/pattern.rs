//! Repeated cut-outs read as a drafter writes them (ISO 129-1:2018, 7.8):
//! a field of equal holes or cut-outs on a grid (a grating's mesh, a
//! perforated panel) is located by the centres of its first and last
//! cells, the spaces between them written once as `8× 33,33 (=266,6)`;
//! holes on a pitch circle (a flange) are located by the circle's centre
//! and given by `⌀150` on the circle, `8× ⌀18` on one hole and the
//! angular spacing `8× 45°`; a slot is located by its centre and given by
//! its width and the distance between its end centres.

use super::features::{ends, extremes, outer_loop, seg_len};
use super::text::{self, Lang};
use crate::dxf::blank::{Seg, P2};

/// Fewer cut-outs than this are dimensioned one by one.
const MIN_MEMBERS: usize = 6;
/// A field needs this many cells along at least one axis.
const MIN_RUN: usize = 3;
/// A pitch circle needs this many holes.
const MIN_POLAR: usize = 3;
/// Centres and pitches agree to this (mm).
const GRID_TOL: f64 = 0.02;
/// Angular steps agree to this (degrees).
const ANGLE_TOL: f64 = 0.05;

#[derive(Clone, Debug)]
pub enum Kind {
    /// Cells along x and y, and the centre-to-centre pitch along each
    /// (0 along an axis with one cell).
    Grid { nx: usize, ny: usize, pitch: P2 },
    /// Holes on a pitch circle: its centre and radius, the first hole's
    /// angle from +x (degrees, the hole nearest to +x counter-clockwise)
    /// and the step between holes.
    Polar {
        centre: P2,
        r: f64,
        start_deg: f64,
        step_deg: f64,
    },
}

#[derive(Clone, Debug)]
pub struct Pattern {
    pub kind: Kind,
    /// The cell's box: width and depth.
    pub cell: P2,
    /// The radius when the cells are round holes.
    pub circle: Option<f64>,
    /// Width and centre distance when the cells are slots.
    pub slot: Option<(f64, f64)>,
    /// Every cell's centre: a grid x fastest, a circle counter-clockwise
    /// from the first hole.
    pub centres: Vec<P2>,
    /// The loops the field is made of, in the order of `centres`.
    pub loops: Vec<usize>,
}

/// A slot: two parallel lines closed by two semicircles.
#[derive(Clone, Debug)]
pub struct Slot {
    pub lp: usize,
    /// Its middle, which locates it.
    pub centre: P2,
    /// The centres of its round ends.
    pub ends: [P2; 2],
    pub width: f64,
}

impl Slot {
    /// Distance between the end centres.
    pub fn between(&self) -> f64 {
        dist(self.ends[0], self.ends[1])
    }
}

impl Pattern {
    pub fn count(&self) -> usize {
        self.centres.len()
    }

    /// The first and last cell's centres.
    pub fn first(&self) -> P2 {
        self.centres[0]
    }

    pub fn last(&self) -> P2 {
        self.centres[self.centres.len() - 1]
    }

    /// What one cell is, and how many: `261× 29 × 29`, `24× ⌀10`,
    /// `4× 12 × 28 h.o.h.`.
    pub fn cell_text(&self, lang: Lang) -> String {
        let n = self.count();
        if let Some(r) = self.circle {
            return format!("{n}× ⌀{}", text::mm(2.0 * r));
        }
        if let Some((w, l)) = self.slot {
            return format!("{n}× {}", slot_text(w, l, lang));
        }
        format!(
            "{n}× {} × {}",
            text::mm(self.cell[0]),
            text::mm(self.cell[1])
        )
    }

    /// The angular spacing of a pitch circle: `8× 45°`.
    pub fn step_text(&self) -> Option<String> {
        match self.kind {
            Kind::Polar { step_deg, .. } => {
                Some(format!("{}× {}", self.count(), text::deg(step_deg)))
            }
            Kind::Grid { .. } => None,
        }
    }

    /// The cell a leader points at, nearest the top right: (a point on
    /// its edge, its centre).
    pub fn leader_cell(&self) -> (P2, P2) {
        let c = *self
            .centres
            .iter()
            .max_by(|a, b| (a[0] + 0.25 * a[1]).total_cmp(&(b[0] + 0.25 * b[1])))
            .unwrap();
        let r = self.circle.unwrap_or(self.cell[0].min(self.cell[1]) / 2.0);
        ([c[0] + r, c[1]], c)
    }
}

/// A slot's size as written: width × distance between its end centres,
/// with the local word for centre to centre.
pub fn slot_text(w: f64, l: f64, lang: Lang) -> String {
    format!("{} × {} {}", text::mm(w), text::mm(l), lang.t("cc"))
}

fn dist(a: P2, b: P2) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// The box around a loop.
fn bbox(lp: &[Seg]) -> (P2, P2) {
    let mut pts: Vec<P2> = extremes(lp.iter(), &[]);
    for s in lp {
        let (a, b) = ends(s);
        pts.push(a);
        pts.push(b);
        if let Seg::Circle { c, r } = *s {
            pts.push([c[0] - r, c[1] - r]);
            pts.push([c[0] + r, c[1] + r]);
        }
    }
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in pts {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

/// The slot a loop is, if it is one: two parallel lines and two
/// semicircles of the same radius, each arc's chord equal to the width.
pub fn slot_of(lp: usize, segs: &[Seg]) -> Option<Slot> {
    if segs.len() != 4 {
        return None;
    }
    let arcs: Vec<(P2, f64)> = segs
        .iter()
        .filter_map(|s| match *s {
            Seg::Arc { c, r, start, end } => {
                let sweep = (end - start).rem_euclid(360.0);
                ((sweep - 180.0).abs() < 0.5).then_some((c, r))
            }
            _ => None,
        })
        .collect();
    let lines = segs
        .iter()
        .filter(|s| matches!(s, Seg::Line { .. }))
        .count();
    if arcs.len() != 2 || lines != 2 || (arcs[0].1 - arcs[1].1).abs() > GRID_TOL {
        return None;
    }
    let (c0, c1) = (arcs[0].0, arcs[1].0);
    if dist(c0, c1) < GRID_TOL {
        return None;
    }
    // The first end is the lower-left one, so the slot reads the same
    // whichever way the loop runs.
    let lower = |p: P2, q: P2| {
        if (p[1] - q[1]).abs() > GRID_TOL {
            p[1] < q[1]
        } else {
            p[0] <= q[0]
        }
    };
    let (a, b) = if lower(c0, c1) { (c0, c1) } else { (c1, c0) };
    Some(Slot {
        lp,
        centre: [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0],
        ends: [a, b],
        width: 2.0 * arcs[0].1,
    })
}

/// Equally spaced values: (distinct values, pitch), when every gap agrees.
fn axis(values: &[f64]) -> Option<(Vec<f64>, f64)> {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let mut distinct: Vec<f64> = Vec::new();
    for x in v {
        if distinct.last().is_none_or(|l| (x - l).abs() > GRID_TOL) {
            distinct.push(x);
        }
    }
    if distinct.len() < 2 {
        return Some((distinct, 0.0));
    }
    let pitch = (distinct[distinct.len() - 1] - distinct[0]) / (distinct.len() - 1) as f64;
    distinct
        .windows(2)
        .all(|w| (w[1] - w[0] - pitch).abs() <= GRID_TOL)
        .then_some((distinct, pitch))
}

/// The grid the centres fill, if they fill one: (nx, ny, pitch, order of
/// the members, x fastest).
fn grid(centres: &[P2]) -> Option<(usize, usize, P2, Vec<usize>)> {
    let xs: Vec<f64> = centres.iter().map(|c| c[0]).collect();
    let ys: Vec<f64> = centres.iter().map(|c| c[1]).collect();
    let (Some((cx, px)), Some((cy, py))) = (axis(&xs), axis(&ys)) else {
        return None;
    };
    let (nx, ny) = (cx.len(), cy.len());
    if nx * ny != centres.len() || nx.max(ny) < MIN_RUN {
        return None;
    }
    // Rows then columns, by the grid value a centre snaps to: the
    // kernel's centres carry noise that would shuffle a raw sort.
    let snap = |v: f64, axis: &[f64]| axis.iter().position(|a| (v - a).abs() <= GRID_TOL);
    let mut order: Vec<usize> = (0..centres.len()).collect();
    order.sort_by_key(|&i| (snap(centres[i][1], &cy), snap(centres[i][0], &cx)));
    Some((nx, ny, [px, py], order))
}

/// The pitch circle the centres sit on, if they do, equally spaced:
/// (centre, radius, start angle, step, order counter-clockwise from the
/// hole nearest to +x).
fn polar(centres: &[P2]) -> Option<(P2, f64, f64, f64, Vec<usize>)> {
    let n = centres.len();
    if n < MIN_POLAR {
        return None;
    }
    // The centre of equally spaced points is their mean.
    let c = centres
        .iter()
        .fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
    let c = [c[0] / n as f64, c[1] / n as f64];
    let r = dist(c, centres[0]);
    if r < 1.0 || centres.iter().any(|p| (dist(c, *p) - r).abs() > GRID_TOL) {
        return None;
    }
    let angle = |p: P2| {
        (p[1] - c[1])
            .atan2(p[0] - c[0])
            .to_degrees()
            .rem_euclid(360.0)
    };
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| angle(centres[a]).total_cmp(&angle(centres[b])));
    let step = 360.0 / n as f64;
    let start = angle(centres[order[0]]);
    for (k, &i) in order.iter().enumerate() {
        let want = (start + step * k as f64).rem_euclid(360.0);
        if (angle(centres[i]) - want).abs() > ANGLE_TOL {
            return None;
        }
    }
    // A hole within a tolerance of +x starts at 0.
    let start = if start < ANGLE_TOL || 360.0 - start < ANGLE_TOL {
        0.0
    } else {
        start
    };
    Some((c, r, start, step, order))
}

/// The fields among the cut-outs of a blank.
pub fn patterns(loops: &[Vec<Seg>]) -> Vec<Pattern> {
    let outer = outer_loop(loops);
    // Equal cut-outs: same box, same number of segments, same perimeter.
    type Key = (i64, i64, usize, i64, bool);
    let mut groups: Vec<(Key, Vec<usize>)> = Vec::new();
    for (i, lp) in loops.iter().enumerate() {
        if i == outer || lp.is_empty() {
            continue;
        }
        let (lo, hi) = bbox(lp);
        let round = |x: f64| (x * 100.0).round() as i64;
        let perimeter: f64 = lp.iter().map(seg_len).sum();
        let circle = matches!(lp.as_slice(), [Seg::Circle { .. }]);
        let key = (
            round(hi[0] - lo[0]),
            round(hi[1] - lo[1]),
            lp.len(),
            (perimeter * 10.0).round() as i64,
            circle,
        );
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(i),
            None => groups.push((key, vec![i])),
        }
    }
    let mut out = Vec::new();
    for (_, members) in groups {
        let boxes: Vec<(P2, P2)> = members.iter().map(|&i| bbox(&loops[i])).collect();
        let circle = match loops[members[0]].as_slice() {
            [Seg::Circle { r, .. }] => Some(*r),
            _ => None,
        };
        let slot = slot_of(members[0], &loops[members[0]]).map(|s| (s.width, s.between()));
        let centres: Vec<P2> = match slot {
            Some(_) => members
                .iter()
                .map(|&i| slot_of(i, &loops[i]).unwrap().centre)
                .collect(),
            None => boxes
                .iter()
                .map(|(lo, hi)| [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0])
                .collect(),
        };
        let cell = [boxes[0].1[0] - boxes[0].0[0], boxes[0].1[1] - boxes[0].0[1]];
        let (kind, order) = if circle.is_some() && members.len() >= MIN_POLAR {
            match polar(&centres) {
                Some((centre, r, start_deg, step_deg, order)) => (
                    Kind::Polar {
                        centre,
                        r,
                        start_deg,
                        step_deg,
                    },
                    order,
                ),
                None => match grid_field(&centres, cell, members.len()) {
                    Some(g) => g,
                    None => continue,
                },
            }
        } else {
            match grid_field(&centres, cell, members.len()) {
                Some(g) => g,
                None => continue,
            }
        };
        out.push(Pattern {
            kind,
            cell,
            circle,
            slot,
            centres: order.iter().map(|&k| centres[k]).collect(),
            loops: order.iter().map(|&k| members[k]).collect(),
        });
    }
    out
}

fn grid_field(centres: &[P2], cell: P2, n: usize) -> Option<(Kind, Vec<usize>)> {
    if n < MIN_MEMBERS {
        return None;
    }
    let (nx, ny, pitch, order) = grid(centres)?;
    // Cells must not touch: a pitch below the cell is no field of holes.
    if (nx > 1 && pitch[0] <= cell[0] + GRID_TOL) || (ny > 1 && pitch[1] <= cell[1] + GRID_TOL) {
        return None;
    }
    Some((Kind::Grid { nx, ny, pitch }, order))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(c: P2, h: f64) -> Vec<Seg> {
        let p = [
            [c[0] - h, c[1] - h],
            [c[0] + h, c[1] - h],
            [c[0] + h, c[1] + h],
            [c[0] - h, c[1] + h],
        ];
        (0..4)
            .map(|i| Seg::Line {
                a: p[i],
                b: p[(i + 1) % 4],
            })
            .collect()
    }

    #[test]
    fn a_grating_mesh_is_one_field() {
        let mut loops = vec![square([152.5, 497.0], 497.0)];
        for j in 0..29 {
            for i in 0..9 {
                loops.push(square(
                    [29.5 + 33.33 * i as f64, 29.5 + 33.33 * j as f64],
                    14.5,
                ));
            }
        }
        let p = patterns(&loops);
        assert_eq!(p.len(), 1);
        let p = &p[0];
        let Kind::Grid { nx, ny, pitch } = p.kind else {
            panic!("grid");
        };
        assert_eq!((nx, ny, p.count()), (9, 29, 261));
        assert!((pitch[0] - 33.33).abs() < 1e-9 && (pitch[1] - 33.33).abs() < 1e-9);
        assert!((p.first()[0] - 29.5).abs() < 1e-9 && (p.cell[0] - 29.0).abs() < 1e-9);
        assert_eq!(p.cell_text(Lang::Nl), "261× 29 × 29");
    }

    #[test]
    fn bolt_holes_sit_on_a_pitch_circle() {
        let mut loops = vec![square([110.0, 110.0], 110.0)];
        for k in 0..8 {
            let a = (22.5 + 45.0 * k as f64).to_radians();
            loops.push(vec![Seg::Circle {
                c: [110.0 + 75.0 * a.cos(), 110.0 + 75.0 * a.sin()],
                r: 9.0,
            }]);
        }
        let p = patterns(&loops);
        assert_eq!(p.len(), 1);
        let Kind::Polar {
            centre,
            r,
            start_deg,
            step_deg,
        } = p[0].kind
        else {
            panic!("polar");
        };
        assert!((centre[0] - 110.0).abs() < 1e-9 && (r - 75.0).abs() < 1e-9);
        assert!((start_deg - 22.5).abs() < 1e-9 && (step_deg - 45.0).abs() < 1e-9);
        assert_eq!(p[0].cell_text(Lang::En), "8× ⌀18");
        assert_eq!(p[0].step_text().as_deref(), Some("8× 45°"));
    }

    #[test]
    fn a_few_holes_or_an_irregular_set_are_no_field() {
        let mut loops = vec![square([100.0, 100.0], 100.0)];
        for i in 0..4 {
            loops.push(square([20.0 + 30.0 * i as f64, 20.0], 5.0));
        }
        assert!(patterns(&loops).is_empty());
        for i in 0..4 {
            loops.push(square([20.0 + 31.0 * i as f64, 60.0], 5.0));
        }
        assert!(patterns(&loops).is_empty());
    }

    #[test]
    fn a_slot_is_read_from_its_ends() {
        let lp = vec![
            Seg::Line {
                a: [10.0, 4.0],
                b: [30.0, 4.0],
            },
            Seg::Arc {
                c: [30.0, 10.0],
                r: 6.0,
                start: 270.0,
                end: 90.0,
            },
            Seg::Line {
                a: [30.0, 16.0],
                b: [10.0, 16.0],
            },
            Seg::Arc {
                c: [10.0, 10.0],
                r: 6.0,
                start: 90.0,
                end: 270.0,
            },
        ];
        let s = slot_of(1, &lp).unwrap();
        assert_eq!(s.centre, [20.0, 10.0]);
        assert_eq!((s.width, s.between()), (12.0, 20.0));
        assert_eq!(slot_text(s.width, s.between(), Lang::Nl), "12 × 20 h.o.h.");
    }
}
