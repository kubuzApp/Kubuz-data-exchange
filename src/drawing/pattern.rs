//! Rectangular patterns of equal cut-outs: a grating's mesh, a perforated
//! panel. A drafter does not chain every hole of such a field; the field
//! is located by the near edge of its first cell and the far edge of its
//! last, and one note gives the cell, the count and the pitch
//! (`261× 29 × 29`, `9 × 29, steek 33,33`).

use super::features::{ends, extremes, outer_loop, seg_len};
use super::text::{self, Lang};
use crate::dxf::blank::{Seg, P2};

/// Fewer cut-outs than this are dimensioned one by one.
const MIN_MEMBERS: usize = 6;
/// A field needs this many cells along at least one axis.
const MIN_RUN: usize = 3;
/// Centres and pitches agree to this (mm).
const GRID_TOL: f64 = 0.02;

#[derive(Clone, Debug)]
pub struct Pattern {
    /// Cells along x and y.
    pub nx: usize,
    pub ny: usize,
    /// Centre to centre along x and y (0 along an axis with one cell).
    pub pitch: P2,
    /// The near corner of the first cell and the far corner of the last.
    pub lo: P2,
    pub hi: P2,
    /// The cell's width and depth.
    pub cell: P2,
    /// The radius when the cells are round holes.
    pub circle: Option<f64>,
    /// Every cell's centre, x fastest.
    pub centres: Vec<P2>,
    /// The loops the field is made of.
    pub loops: Vec<usize>,
}

impl Pattern {
    pub fn count(&self) -> usize {
        self.centres.len()
    }

    /// What one cell is, and how many: `261× 29 × 29`, `24× ⌀10`.
    pub fn cell_text(&self) -> String {
        let n = self.count();
        match self.circle {
            Some(r) => format!("{n}× ⌀{}", text::mm(2.0 * r)),
            None => format!(
                "{n}× {} × {}",
                text::mm(self.cell[0]),
                text::mm(self.cell[1])
            ),
        }
    }

    /// The grid: `9 × 29, steek 33,33`, or `steek 30 × 40` when the
    /// pitches differ.
    pub fn grid_text(&self, lang: Lang) -> String {
        let [px, py] = self.pitch;
        let pitch = if self.nx > 1 && self.ny > 1 && (px - py).abs() > GRID_TOL {
            format!("{} × {}", text::mm_fine(px), text::mm_fine(py))
        } else {
            text::mm_fine(if self.nx > 1 { px } else { py })
        };
        format!("{} × {}, {} {pitch}", self.nx, self.ny, lang.t("pitch"))
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
        if members.len() < MIN_MEMBERS {
            continue;
        }
        let boxes: Vec<(P2, P2)> = members.iter().map(|&i| bbox(&loops[i])).collect();
        let centres: Vec<P2> = boxes
            .iter()
            .map(|(lo, hi)| [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0])
            .collect();
        let xs: Vec<f64> = centres.iter().map(|c| c[0]).collect();
        let ys: Vec<f64> = centres.iter().map(|c| c[1]).collect();
        let (Some((cx, px)), Some((cy, py))) = (axis(&xs), axis(&ys)) else {
            continue;
        };
        let (nx, ny) = (cx.len(), cy.len());
        if nx * ny != members.len() || nx.max(ny) < MIN_RUN {
            continue;
        }
        let cell = [boxes[0].1[0] - boxes[0].0[0], boxes[0].1[1] - boxes[0].0[1]];
        // Cells must not touch: a pitch below the cell is no field of holes.
        if (nx > 1 && px <= cell[0] + GRID_TOL) || (ny > 1 && py <= cell[1] + GRID_TOL) {
            continue;
        }
        let circle = match loops[members[0]].as_slice() {
            [Seg::Circle { r, .. }] => Some(*r),
            _ => None,
        };
        let mut order: Vec<usize> = (0..members.len()).collect();
        order.sort_by(|&a, &b| {
            centres[a][1]
                .total_cmp(&centres[b][1])
                .then(centres[a][0].total_cmp(&centres[b][0]))
        });
        out.push(Pattern {
            nx,
            ny,
            pitch: [px, py],
            lo: [cx[0] - cell[0] / 2.0, cy[0] - cell[1] / 2.0],
            hi: [cx[nx - 1] + cell[0] / 2.0, cy[ny - 1] + cell[1] / 2.0],
            cell,
            circle,
            centres: order.iter().map(|&k| centres[k]).collect(),
            loops: order.iter().map(|&k| members[k]).collect(),
        });
    }
    out
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
        assert_eq!((p.nx, p.ny, p.count()), (9, 29, 261));
        assert!((p.pitch[0] - 33.33).abs() < 1e-9 && (p.pitch[1] - 33.33).abs() < 1e-9);
        assert!((p.lo[0] - 15.0).abs() < 1e-9 && (p.cell[0] - 29.0).abs() < 1e-9);
        assert_eq!(p.cell_text(), "261× 29 × 29");
        assert_eq!(p.grid_text(Lang::Nl), "9 × 29, steek 33,33");
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
}
