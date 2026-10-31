//! The bays of a cabinet or a rack: the uprights divide the front into
//! bays, and the horizontal members inside a bay (shelves, shelf frames,
//! rails) are its levels. The general arrangement sheet chains the
//! upright centres above the front view and each bay's levels from the
//! floor inside the bay.
//!
//! Which part is an upright and which a level comes from the view when
//! the template says so (a part's `arrangement`, [`crate::view::Arrangement`]).
//! A view that says nothing gets the old guess from the parts'
//! proportions, and a note that the layout is guessed: a product that
//! looks different from a cabinet can be guessed wrong.

use super::iso::Scene;
use super::Note;
use crate::view::{Arrangement, ArrangementRole};

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

/// The uprights' centres along x, their top, and the bays between them.
pub struct Bays {
    pub centres: Vec<f64>,
    pub top: f64,
    pub bays: Vec<Bay>,
    /// Whether the view said which parts are uprights and levels.
    pub source: Source,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The parts' `arrangement` in the view.
    View,
    /// Guessed from the parts' proportions.
    Guessed,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::View => "view",
            Source::Guessed => "guessed",
        }
    }
}

/// The bays of `scene`, whose object `i` is tagged `tags[i]`, and the
/// notes for the export. Any tag makes the view the source: then no
/// part is guessed. Without tags the proportions decide, and bays found
/// that way come with a `bays-guessed` note.
pub fn find(scene: &Scene, tags: &[Option<Arrangement>]) -> (Option<Bays>, Vec<Note>) {
    if tags.iter().any(Option::is_some) {
        return from_view(scene, tags);
    }
    match guess(scene) {
        Some(b) => (
            Some(b),
            vec![Note {
                code: "bays-guessed".into(),
                message: "The bays and levels on the general arrangement are guessed from the \
                          parts' proportions: no part in the view has an arrangement role \
                          (upright or level)."
                    .into(),
            }],
        ),
        None => (None, Vec::new()),
    }
}

/// Upright centres along x, merged, with each one's x extent, sorted;
/// and their top.
fn columns<'a>(bounds: impl Iterator<Item = &'a [f64; 6]>) -> (Vec<(f64, f64, f64)>, f64) {
    let mut cols: Vec<(f64, f64, f64)> = Vec::new();
    let mut top = f64::NEG_INFINITY;
    for b in bounds {
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
    cols.sort_by(|a, b| a.0.total_cmp(&b.0));
    (cols, top)
}

/// The bays between neighbouring columns, each with the tops of the
/// objects `level(bay index, object, x0, x1, inner)` keeps, a stack within
/// [`STACK`] counted once.
fn bays_between(
    cols: &[(f64, f64, f64)],
    all: &[[f64; 6]],
    level: impl Fn(usize, usize, f64, f64, [f64; 2]) -> bool,
) -> Vec<Bay> {
    let mut bays = Vec::new();
    for (k, w) in cols.windows(2).enumerate() {
        let (x0, x1) = (w[0].0, w[1].0);
        let inner = [w[0].2, w[1].1];
        let mut tops: Vec<f64> = (0..all.len())
            .filter(|&i| level(k, i, x0, x1, inner))
            .map(|i| all[i][5])
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
    bays
}

/// A level spans at least [`SPAN`] of its bay's inner width.
fn spans(b: &[f64; 6], inner: [f64; 2]) -> bool {
    let cover = b[3].min(inner[1]) - b[0].max(inner[0]);
    cover >= SPAN * (inner[1] - inner[0])
}

fn from_view(scene: &Scene, tags: &[Option<Arrangement>]) -> (Option<Bays>, Vec<Note>) {
    let all: Vec<[f64; 6]> = (0..scene.objects()).map(|i| scene.bounds(i)).collect();
    let role = |i: usize| tags.get(i).copied().flatten();
    let is = |i: usize, r: ArrangementRole| role(i).is_some_and(|a| a.role == r);
    let (cols, top) = columns(
        (0..all.len())
            .filter(|&i| is(i, ArrangementRole::Upright))
            .map(|i| &all[i]),
    );
    let mut notes = Vec::new();
    if cols.len() < 2 {
        notes.push(Note {
            code: "bays-incomplete".into(),
            message: format!(
                "The view marks {} upright line(s); bays need two or more, so the general \
                 arrangement has no bays.",
                cols.len()
            ),
        });
        return (None, notes);
    }
    let n = cols.len() - 1;
    let lost = (0..all.len())
        .filter(|&i| is(i, ArrangementRole::Level))
        .filter(|&i| role(i).and_then(|a| a.bay).is_some_and(|b| b >= n))
        .count();
    if lost > 0 {
        notes.push(Note {
            code: "bays-out-of-range".into(),
            message: format!(
                "{lost} level part(s) name a bay the view does not have ({n} bay(s) between the \
                 uprights); they are left off the general arrangement."
            ),
        });
    }
    let bays = bays_between(&cols, &all, |k, i, _, _, inner| {
        is(i, ArrangementRole::Level)
            && match role(i).and_then(|a| a.bay) {
                Some(bay) => bay == k,
                None => spans(&all[i], inner),
            }
    });
    (
        Some(Bays {
            centres: cols.iter().map(|c| c.0).collect(),
            top,
            bays,
            source: Source::View,
        }),
        notes,
    )
}

/// The old guess: uprights stand at least [`TALL`] of the height and
/// are at most [`NARROW`] of the length wide; levels are at most
/// [`FLAT`] of the height thick. Two bays at least, each with a level
/// (a row of balusters under one rail is no cabinet), or `None`.
fn guess(scene: &Scene) -> Option<Bays> {
    let ext = scene.extents();
    let (lx, lz) = (ext[3] - ext[0], ext[5] - ext[2]);
    if lx <= 0.0 || lz <= 0.0 {
        return None;
    }
    let all: Vec<[f64; 6]> = (0..scene.objects()).map(|i| scene.bounds(i)).collect();
    let upright = |b: &[f64; 6]| b[5] - b[2] >= TALL * lz && b[3] - b[0] <= NARROW * lx;
    let (cols, top) = columns(all.iter().filter(|b| upright(b)));
    if cols.len() < 3 {
        return None;
    }
    let bays = bays_between(&cols, &all, |_, i, x0, x1, inner| {
        let b = &all[i];
        !upright(b) && b[5] - b[2] <= FLAT * lz && spans(b, inner) && b[3] - b[0] <= 1.5 * (x1 - x0)
    });
    if bays.iter().any(|b| b.levels.is_empty()) {
        return None;
    }
    Some(Bays {
        centres: cols.iter().map(|c| c.0).collect(),
        top,
        bays,
        source: Source::Guessed,
    })
}
