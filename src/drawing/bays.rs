//! The bays of a cabinet or a rack, read from the placed parts: the
//! uprights (members standing the full height, narrow along x) divide the
//! front into bays, and the horizontal members inside a bay (shelves,
//! shelf frames, rails) are its levels. The general arrangement sheet
//! chains the upright centres above the front view and each bay's levels
//! from the floor inside the bay.

use super::iso::Scene;

/// An upright member reaches this share of the height.
const TALL: f64 = 0.8;
/// An upright member is at most this share of the length wide along x.
const NARROW: f64 = 0.05;
/// A horizontal member is at most this share of the height thick.
const FLAT: f64 = 0.1;
/// A level member covers at least this share of its bay's width.
const SPAN: f64 = 0.5;
/// Levels closer than this (a board on its frame) are one level, its top.
const STACK: f64 = 40.0;
/// Centres closer than this stand on one line.
const SAME: f64 = 1.0;

/// One bay: between two upright centres, its levels' tops from the
/// lowest up.
#[derive(Clone, Debug, PartialEq)]
pub struct Bay {
    pub x0: f64,
    pub x1: f64,
    /// The inner faces of its uprights, where the level members end.
    pub inner: [f64; 2],
    pub levels: Vec<f64>,
}

/// The uprights' centres along x, their top, and the bays between them;
/// `None` for fewer than two bays.
pub struct Bays {
    pub centres: Vec<f64>,
    pub top: f64,
    pub bays: Vec<Bay>,
}

pub fn find(scene: &Scene) -> Option<Bays> {
    let ext = scene.extents();
    let (lx, lz) = (ext[3] - ext[0], ext[5] - ext[2]);
    if lx <= 0.0 || lz <= 0.0 {
        return None;
    }
    let all: Vec<[f64; 6]> = (0..scene.objects()).map(|i| scene.bounds(i)).collect();
    let upright = |b: &[f64; 6]| b[5] - b[2] >= TALL * lz && b[3] - b[0] <= NARROW * lx;
    // Upright centres, merged, with each one's x extent.
    let mut cols: Vec<(f64, f64, f64)> = Vec::new();
    let mut top = f64::NEG_INFINITY;
    for b in all.iter().filter(|b| upright(b)) {
        let c = (b[0] + b[3]) / 2.0;
        top = top.max(b[5]);
        match cols.iter_mut().find(|(x, _, _)| (x - c).abs() <= SAME) {
            Some(col) => {
                col.1 = col.1.min(b[0]);
                col.2 = col.2.max(b[3]);
            }
            None => cols.push((c, b[0], b[3])),
        }
    }
    if cols.len() < 3 {
        return None;
    }
    cols.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut bays = Vec::new();
    for w in cols.windows(2) {
        let (x0, x1) = (w[0].0, w[1].0);
        let inner = [w[0].2, w[1].1];
        let width = inner[1] - inner[0];
        let mut tops: Vec<f64> = all
            .iter()
            .filter(|b| !upright(b) && b[5] - b[2] <= FLAT * lz)
            .filter(|b| {
                let cover = b[3].min(inner[1]) - b[0].max(inner[0]);
                cover >= SPAN * width && b[3] - b[0] <= 1.5 * (x1 - x0)
            })
            .map(|b| b[5])
            .collect();
        tops.sort_by(f64::total_cmp);
        // A board on its frame: the stack's top.
        let mut levels: Vec<f64> = Vec::new();
        for t in tops {
            match levels.last_mut() {
                Some(l) if t - *l <= STACK => *l = t,
                _ => levels.push(t),
            }
        }
        bays.push(Bay {
            x0,
            x1,
            inner,
            levels,
        });
    }
    // A row of balusters or posts under one rail is no cabinet: every
    // bay holds a level of its own.
    if bays.iter().any(|b: &Bay| b.levels.is_empty()) {
        return None;
    }
    Some(Bays {
        centres: cols.iter().map(|c| c.0).collect(),
        top,
        bays,
    })
}
