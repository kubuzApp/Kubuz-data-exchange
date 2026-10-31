//! The general arrangement sheet (ISO 128-30, first angle): the placed
//! parts seen from the front, from above (below the front view) and from
//! the left (right of the front view), with the overall length, width
//! and height and, for every position whose parts stand at one pitch
//! along an axis, the run `n× p (=total)` from the first to the last
//! centre, chained to the edges (ISO 129-1). A cabinet or a rack
//! (uprights dividing the front into bays) gets the upright centres
//! chained above the front view and each bay's levels chained from the
//! floor inside the bay, in place of the height runs.

use serde::Serialize;

use super::bays::{self, Bays};
use super::dims::{self, D};
use super::iso::{Dir, Iso, Scene};
use super::lists::{self, Row};
use super::sheet::Paper;
use super::text::{self, Lang};
use super::Meta;
use crate::dxf::blank::P2;
use crate::pdf::{font, Content, Doc};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewReport {
    /// `front`, `top` or `left`.
    pub name: String,
    /// Where model `(0, 0)` of the view's picture plane sits on the
    /// sheet, mm.
    pub origin: [f64; 2],
    /// The view's extents on the sheet, mm.
    pub bbox: [f64; 4],
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DimReport {
    /// `overall` (the extents along the axis), `run` (equal spaces
    /// between the centres of one position's parts), `bay` (from one
    /// upright centre to the next) or `level` (a bay's floor or level to
    /// its next level; `first` is where it starts, `pitch` the bay's
    /// left upright centre).
    pub kind: String,
    /// `x`, `y` or `z`.
    pub axis: String,
    /// Which view carries it.
    pub view: String,
    pub text: String,
    /// The measured length.
    pub value: f64,
    /// For a run: the position, how many spaces, the pitch and the first
    /// centre's coordinate along the axis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArrangementReport {
    pub page: usize,
    pub paper: String,
    pub scale: String,
    pub scale_factor: f64,
    /// The assembly's extents `[x0, y0, z0, x1, y1, z1]`, mm.
    pub extents: [f64; 6],
    pub views: Vec<ViewReport>,
    pub dimensions: Vec<DimReport>,
    /// Where the bays and levels come from: `view` (the parts'
    /// `arrangement`) or `guessed` (the parts' proportions); missing
    /// when the sheet has no bays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bays: Option<String>,
}

/// A run of one position's parts at one pitch along an axis.
struct Run {
    pos: usize,
    axis: usize,
    first: f64,
    last: f64,
    count: usize,
    pitch: f64,
    /// The parts' extent across the axis the run is dimensioned from
    /// (where the extension lines start).
    edge: f64,
}

/// The three views and the runs, ready to lay out.
pub struct Plan {
    front: Iso,
    top: Iso,
    left: Iso,
    ext: [f64; 6],
    runs: Vec<Run>,
    bays: Option<Bays>,
    /// What the export should say about the bays.
    pub notes: Vec<super::Note>,
}

const GAP: f64 = 12.0;
const LABEL_H: f64 = 3.5;
/// A value's baseline above its dimension line.
const TEXT_UP: f64 = 1.5;
/// A dimension row: the line and its text.
const ROW: f64 = D;

/// Equal spaces are equal within this, and centres on one line within
/// ten times this.
const SAME: f64 = 0.05;

/// The runs: for every row, the groups of three or more parts whose
/// centres stand on one line along an axis at one pitch (an L-shaped
/// balustrade has a run on each leg).
fn runs(scene: &Scene, owner_row: &[usize], rows: &[Row]) -> Vec<Run> {
    let mut out = Vec::new();
    for (ri, row) in rows.iter().enumerate() {
        let objs: Vec<usize> = (0..owner_row.len())
            .filter(|&i| owner_row[i] == ri)
            .collect();
        if objs.len() < 3 {
            continue;
        }
        let bounds: Vec<[f64; 6]> = objs.iter().map(|&i| scene.bounds(i)).collect();
        let centres: Vec<[f64; 3]> = bounds
            .iter()
            .map(|b| {
                [
                    (b[0] + b[3]) / 2.0,
                    (b[1] + b[4]) / 2.0,
                    (b[2] + b[5]) / 2.0,
                ]
            })
            .collect();
        let mut taken = vec![false; centres.len()];
        for axis in 0..3 {
            let others = [(axis + 1) % 3, (axis + 2) % 3];
            // Lines along the axis: parts whose other two coordinates
            // agree.
            let mut lines: Vec<Vec<usize>> = Vec::new();
            for (i, c) in centres.iter().enumerate() {
                if taken[i] {
                    continue;
                }
                match lines.iter_mut().find(|l| {
                    let c0 = centres[l[0]];
                    others.iter().all(|&o| (c[o] - c0[o]).abs() <= 10.0 * SAME)
                }) {
                    Some(l) => l.push(i),
                    None => lines.push(vec![i]),
                }
            }
            for line in lines {
                if line.len() < 3 {
                    continue;
                }
                let mut along: Vec<f64> = line.iter().map(|&i| centres[i][axis]).collect();
                along.sort_by(f64::total_cmp);
                let pitch = (along[along.len() - 1] - along[0]) / (along.len() - 1) as f64;
                if pitch < 1.0 || along.windows(2).any(|w| (w[1] - w[0] - pitch).abs() > SAME) {
                    continue;
                }
                // The extension lines start at the parts' outer face
                // across the axis: above for x (the front view), left for
                // z (the front view), below for y (the left view).
                let bs = line.iter().map(|&i| bounds[i]);
                let edge = match axis {
                    0 => bs.map(|b| b[5]).fold(f64::NEG_INFINITY, f64::max),
                    2 => bs.map(|b| b[0]).fold(f64::INFINITY, f64::min),
                    _ => bs.map(|b| b[2]).fold(f64::INFINITY, f64::min),
                };
                for &i in &line {
                    taken[i] = true;
                }
                out.push(Run {
                    pos: row.pos,
                    axis,
                    first: along[0],
                    last: along[along.len() - 1],
                    count: along.len() - 1,
                    pitch,
                    edge,
                });
            }
        }
    }
    // Two legs with the same run along one axis (an L-shaped balustrade
    // seen from the front) are one dimension.
    let mut kept: Vec<Run> = Vec::new();
    for r in out {
        if !kept.iter().any(|k| {
            k.axis == r.axis
                && k.count == r.count
                && (k.first - r.first).abs() <= 10.0 * SAME
                && (k.pitch - r.pitch).abs() <= SAME
        }) {
            kept.push(r);
        }
    }
    kept.sort_by(|a, b| {
        (a.axis, a.pos)
            .cmp(&(b.axis, b.pos))
            .then(a.first.total_cmp(&b.first))
    });
    kept
}

/// The views of a scene of two or more placed parts, or `None`. `tags`
/// is each object's layout tag from the view.
pub fn plan(
    scene: &Scene,
    owner_row: &[usize],
    tags: &[Option<crate::view::Arrangement>],
    rows: &[Row],
) -> Option<Plan> {
    if scene.objects() < 2 {
        return None;
    }
    let front = scene.render(Dir::Front)?;
    let top = scene.render(Dir::Top)?;
    let left = scene.render(Dir::Left)?;
    let (bays, notes) = bays::find(scene, tags);
    let mut runs = runs(scene, owner_row, rows);
    if bays.is_some() {
        // The levels chains say the heights bay by bay.
        runs.retain(|r| r.axis != 2);
    }
    Some(Plan {
        front,
        top,
        left,
        ext: scene.extents(),
        runs,
        bays,
        notes,
    })
}

/// A chain of linear dimensions along `v` (unit, away from the part),
/// every line at `level` (paper coordinate along `v`), the pieces in
/// order from the chain's first end to its last. A value that does not
/// fit its piece goes beyond the chain's outer end for an end piece and
/// a text row higher for a piece in between (ISO 129-1).
fn chain(c: &mut Content, pieces: &[(P2, P2, String)], v: P2, level: f64) {
    let n = pieces.len();
    if n == 0 {
        return;
    }
    let along = |p: P2| p[0] * v[0] + p[1] * v[1];
    let on_line = |p: P2| {
        [
            p[0] + v[0] * (level - along(p)),
            p[1] + v[1] * (level - along(p)),
        ]
    };
    // The chain's own direction, from its first point to its last.
    let (a0, b1) = (on_line(pieces[0].0), on_line(pieces[n - 1].1));
    let l = (b1[0] - a0[0]).hypot(b1[1] - a0[1]).max(1e-9);
    let u = [(b1[0] - a0[0]) / l, (b1[1] - a0[1]) / l];
    let ur = dims::reading(u);
    let up = [-ur[1], ur[0]];
    let at = |p: P2, d: P2, k: f64| [p[0] + d[0] * k, p[1] + d[1] * k];
    for (i, (p, q, text)) in pieces.iter().enumerate() {
        let (a, b) = (on_line(*p), on_line(*q));
        c.line(at(*p, v, dims::GAP), at(a, v, dims::EXT));
        c.line(at(*q, v, dims::GAP), at(b, v, dims::EXT));
        c.line(a, b);
        dims::terminators(c, a, b);
        let tw = font::width(text, dims::H);
        let span = (b[0] - a[0]).hypot(b[1] - a[1]);
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let start = if tw + 2.0 <= span {
            at(at(mid, ur, -tw / 2.0), up, TEXT_UP)
        } else if i == 0 || i + 1 == n {
            // Beyond the outer end, clear of the arrowhead.
            let (end, out) = if i == 0 { (a, [-u[0], -u[1]]) } else { (b, u) };
            let clear = dims::ARROW_L + 1.5;
            let s = if ur[0] * out[0] + ur[1] * out[1] > 0.0 {
                at(end, out, clear)
            } else {
                at(end, out, clear + tw)
            };
            at(s, up, TEXT_UP)
        } else {
            at(at(mid, ur, -tw / 2.0), up, TEXT_UP + dims::H + 1.5)
        };
        c.text(start, dims::H, ur[1].atan2(ur[0]).to_degrees(), text);
    }
}

fn run_text(r: &Run) -> String {
    format!(
        "{}× {} (={})",
        r.count,
        text::mm_fine(r.pitch),
        text::mm(r.last - r.first)
    )
}

/// Draw the sheet into reserved page `page` (A3). Returns the report.
pub fn sheet(
    doc: &mut Doc,
    page: usize,
    pages: usize,
    meta: &Meta,
    lang: Lang,
    plan: &Plan,
) -> ArrangementReport {
    let paper = Paper::A3;
    let notes = [lang.t("arrangement_note")];
    let (mut c, area) = lists::begin(paper, lang, lang.t("arrangement"), "", &notes);
    let ext = plan.ext;
    let (lx, ly, lz) = (ext[3] - ext[0], ext[4] - ext[1], ext[5] - ext[2]);
    let nx = plan.runs.iter().filter(|r| r.axis == 0).count() as f64;
    let ny = plan.runs.iter().filter(|r| r.axis == 1).count() as f64;
    let nz = plan.runs.iter().filter(|r| r.axis == 2).count() as f64;
    // Paper space around the views: the label and the x runs above the
    // front view, the z runs and the height left of it, the length under
    // the top view, the y runs and the width under the left view.
    // A run row's text reaches dims::H + 1,5 above its line.
    let nb = if plan.bays.is_some() { 1.0 } else { 0.0 };
    let above = LABEL_H
        + 2.5
        + if nx + nb > 0.0 {
            (nx + nb) * ROW + 2.0 + 2.0 * (dims::H + 1.5) + 1.0
        } else {
            0.0
        };
    let left_of = (nz + 1.0) * ROW + 4.0;
    let below_top = ROW + 4.0;
    let below_left = (ny + 1.0) * ROW + 4.0;
    // Room for the scale text under the caption.
    let top = area[3] - 6.0;
    let (aw, ah) = (area[2] - area[0], top - area[1]);
    // A chain's end value that does not fit its piece is written beyond
    // the chain's end, into the gap between the views: the gaps grow to
    // take the widest such value (and the view name above the top view).
    let end_w = |r: &Run, first: bool| {
        let v = if first {
            match r.axis {
                0 => r.first - ext[0],
                1 => ext[4] - r.last,
                _ => r.first - ext[2],
            }
        } else {
            match r.axis {
                0 => ext[3] - r.last,
                1 => r.first - ext[1],
                _ => ext[5] - r.last,
            }
        };
        font::width(&text::mm(v), dims::H) + dims::ARROW_L + 1.5 + 1.5
    };
    let reach = |axis: usize, first: bool| {
        plan.runs
            .iter()
            .filter(|r| r.axis == axis)
            .map(|r| end_w(r, first))
            .fold(0.0, f64::max)
    };
    let gap_h = GAP.max(reach(1, true)).max(reach(0, false));
    // A bay's first level, from the floor, is often too short for its
    // value, which then stands under the floor line.
    let floor_w = plan
        .bays
        .iter()
        .flat_map(|b| &b.bays)
        .filter_map(|b| b.levels.first())
        .map(|z| font::width(&text::mm(z - ext[2]), dims::H) + dims::ARROW_L + 3.0)
        .fold(0.0, f64::max);
    let gap_v = GAP.max(reach(2, true).max(floor_w) + LABEL_H + 2.5);
    let fits = |s: f64| {
        let w = left_of + lx * s + gap_h + ly * s + 4.0;
        let h = above + lz * s + (gap_v + ly * s + below_top).max(below_left) + 1.0;
        w <= aw && h <= ah
    };
    let s = super::flat::SCALES
        .iter()
        .copied()
        .find(|&s| fits(s))
        .unwrap_or(0.01);
    let scale_text = text::scale(s);
    let w = left_of + lx * s + gap_h + ly * s + 4.0;
    let h = above + lz * s + (gap_v + ly * s + below_top).max(below_left) + 1.0;
    // Centred in the area.
    let x0 = area[0] + (aw - w).max(0.0) / 2.0;
    let y1 = top - (ah - h).max(0.0) / 2.0;
    // The front view: x right, z up.
    let fx0 = x0 + left_of;
    let fx1 = fx0 + lx * s;
    let fy1 = y1 - above;
    let fy0 = fy1 - lz * s;
    // The top view under it: x right, y up.
    let ty1 = fy0 - gap_v;
    let ty0 = ty1 - ly * s;
    // The left view right of it: -y right, z up.
    let lx0 = fx1 + gap_h;
    let lx1 = lx0 + ly * s;
    let origins = [
        (Dir::Front, [fx0 - ext[0] * s, fy0 - ext[2] * s]),
        (Dir::Top, [fx0 - ext[0] * s, ty0 - ext[1] * s]),
        (Dir::Left, [lx0 + ext[4] * s, fy0 - ext[2] * s]),
    ];
    let boxes = [
        [fx0, fy0, fx1, fy1],
        [fx0, ty0, fx1, ty1],
        [lx0, fy0, lx1, fy1],
    ];
    let mut views = Vec::new();
    for ((dir, o), bx) in origins.iter().zip(boxes) {
        let iso = match dir {
            Dir::Front => &plan.front,
            Dir::Top => &plan.top,
            _ => &plan.left,
        };
        super::assembly::draw_lines(
            &mut c,
            iso,
            &|q| [o[0] + q[0] * s, o[1] + q[1] * s],
            &|_| true,
            None,
        );
        views.push(ViewReport {
            name: dir.name().into(),
            origin: *o,
            bbox: bx,
        });
    }
    c.text([area[0] + 1.0, area[3] - 4.0], 3.5, 0.0, &scale_text);
    // View names above the views (ISO 128-30).
    let label_y = y1 - LABEL_H;
    c.text_centred((fx0 + fx1) / 2.0, label_y, LABEL_H, lang.t("view_front"));
    c.text_centred((lx0 + lx1) / 2.0, label_y, LABEL_H, lang.t("view_left"));
    c.text_centred((fx0 + fx1) / 2.0, ty1 + 2.5, LABEL_H, lang.t("view_top"));

    // Dimensions.
    c.width(dims::NARROW);
    let mut dimensions = Vec::new();
    let fp = |x: f64, z: f64| [fx0 + (x - ext[0]) * s, fy0 + (z - ext[2]) * s];
    let tp = |x: f64, y: f64| [fx0 + (x - ext[0]) * s, ty0 + (y - ext[1]) * s];
    let lp = |y: f64, z: f64| [lx0 + (ext[4] - y) * s, fy0 + (z - ext[2]) * s];
    let overall = |c: &mut crate::pdf::Content, axis: usize, p: P2, q: P2, v: P2, level: f64| {
        let value = [lx, ly, lz][axis];
        let t = text::mm(value);
        chain(c, &[(p, q, t.clone())], v, level);
        DimReport {
            kind: "overall".into(),
            axis: ["x", "y", "z"][axis].into(),
            view: match axis {
                0 => "top",
                1 => "left",
                _ => "front",
            }
            .into(),
            text: t,
            value,
            pos: None,
            count: None,
            pitch: None,
            first: None,
        }
    };
    // Length under the top view, width under the left view, height left
    // of the front view (outermost).
    dimensions.push(overall(
        &mut c,
        0,
        tp(ext[0], ext[1]),
        tp(ext[3], ext[1]),
        [0.0, -1.0],
        -(ty0 - ROW * 1.0 - 2.0),
    ));
    dimensions.push(overall(
        &mut c,
        1,
        lp(ext[4], ext[2]),
        lp(ext[1], ext[2]),
        [0.0, -1.0],
        -(fy0 - (ny + 1.0) * ROW - 2.0),
    ));
    dimensions.push(overall(
        &mut c,
        2,
        fp(ext[0], ext[2]),
        fp(ext[0], ext[5]),
        [-1.0, 0.0],
        -(fx0 - (nz + 1.0) * ROW - 2.0),
    ));
    // The runs, one row each, inside the overall dimension.
    let (mut kx, mut ky, mut kz) = (0.0, 0.0, 0.0);
    for r in &plan.runs {
        let t = run_text(r);
        let (pieces, v, level, view): (Vec<(P2, P2, String)>, P2, f64, &str) = match r.axis {
            0 => {
                kx += 1.0;
                (
                    vec![
                        (
                            fp(ext[0], ext[5]),
                            fp(r.first, r.edge),
                            text::mm(r.first - ext[0]),
                        ),
                        (fp(r.first, r.edge), fp(r.last, r.edge), t.clone()),
                        (
                            fp(r.last, r.edge),
                            fp(ext[3], ext[5]),
                            text::mm(ext[3] - r.last),
                        ),
                    ],
                    [0.0, 1.0],
                    fy1 + kx * ROW + 2.0,
                    "front",
                )
            }
            1 => {
                ky += 1.0;
                (
                    vec![
                        (
                            lp(ext[4], ext[2]),
                            lp(r.last, r.edge),
                            text::mm(ext[4] - r.last),
                        ),
                        (lp(r.last, r.edge), lp(r.first, r.edge), t.clone()),
                        (
                            lp(r.first, r.edge),
                            lp(ext[1], ext[2]),
                            text::mm(r.first - ext[1]),
                        ),
                    ],
                    [0.0, -1.0],
                    -(fy0 - ky * ROW - 2.0),
                    "left",
                )
            }
            _ => {
                kz += 1.0;
                (
                    vec![
                        (
                            fp(ext[0], ext[2]),
                            fp(r.edge, r.first),
                            text::mm(r.first - ext[2]),
                        ),
                        (fp(r.edge, r.first), fp(r.edge, r.last), t.clone()),
                        (
                            fp(r.edge, r.last),
                            fp(ext[0], ext[5]),
                            text::mm(ext[5] - r.last),
                        ),
                    ],
                    [-1.0, 0.0],
                    -(fx0 - kz * ROW - 2.0),
                    "front",
                )
            }
        };
        chain(&mut c, &pieces, v, level);
        dimensions.push(DimReport {
            kind: "run".into(),
            axis: ["x", "y", "z"][r.axis].into(),
            view: view.into(),
            text: t,
            value: r.last - r.first,
            pos: Some(r.pos),
            count: Some(r.count),
            pitch: Some(r.pitch),
            first: Some(r.first),
        });
    }

    if let Some(b) = &plan.bays {
        // The upright centres, chained above the runs.
        let pieces: Vec<(P2, P2, String)> = b
            .centres
            .windows(2)
            .map(|w| (fp(w[0], b.top), fp(w[1], b.top), text::mm(w[1] - w[0])))
            .collect();
        chain(&mut c, &pieces, [0.0, 1.0], fy1 + (nx + 1.0) * ROW + 2.0);
        for w in b.centres.windows(2) {
            dimensions.push(DimReport {
                kind: "bay".into(),
                axis: "x".into(),
                view: "front".into(),
                text: text::mm(w[1] - w[0]),
                value: w[1] - w[0],
                pos: None,
                count: None,
                pitch: None,
                first: Some(w[0]),
            });
        }
        // Each bay's levels from the floor, inside the bay near its left
        // upright, the extension lines along the levels themselves.
        for bay in &b.bays {
            if bay.levels.is_empty() {
                continue;
            }
            let room = (bay.inner[1] - bay.inner[0]) * s;
            let off = (0.25 * room).max(dims::H + 3.0).min(room / 2.0);
            let line = fp(bay.inner[0], 0.0)[0] + off;
            let mut from = ext[2];
            let mut pieces = Vec::new();
            for &z in &bay.levels {
                let t = text::mm(z - from);
                pieces.push((fp(bay.inner[0], from), fp(bay.inner[0], z), t.clone()));
                dimensions.push(DimReport {
                    kind: "level".into(),
                    axis: "z".into(),
                    view: "front".into(),
                    text: t,
                    value: z - from,
                    pos: None,
                    count: None,
                    pitch: Some(bay.x0),
                    first: Some(from),
                });
                from = z;
            }
            chain(&mut c, &pieces, [1.0, 0.0], line);
        }
    }

    lists::finish(
        doc,
        page,
        paper,
        lang,
        meta,
        c,
        lang.t("arrangement_title"),
        &scale_text,
        pages,
        lang.t("arrangement_title"),
    );
    ArrangementReport {
        page,
        paper: paper.name().into(),
        scale: scale_text,
        scale_factor: s,
        extents: ext,
        views,
        dimensions,
        bays: plan.bays.as_ref().map(|b| b.source.name().to_string()),
    }
}
