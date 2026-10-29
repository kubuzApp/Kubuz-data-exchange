//! The assembly sheet: every part folded and placed as the view places
//! it, drawn as one isometric pictorial with item balloons (ISO 6433)
//! pointing at each position, and the parts list beside it above the
//! title block. The balloons sit on a ring around the picture, spread so
//! none overlap; each leader ends in a dot on the part.

use serde::Serialize;

use super::iso::{Iso, Scene};
use super::lists::{self, Row, Table};
use super::section::P3;
use super::sheet::{self, Paper};
use super::text::{self, Lang};
use super::Meta;
use crate::dxf::blank::{Blank, P2};
use crate::pdf::Doc;
use crate::view::Solid;

/// What a placed part is drawn from: its flat blank folded (a bent
/// sheet part), a flat plate's solid, or any other solid as it is (a
/// bar, a tube, a part the view already shows folded).
///
/// A flat view places a sheet part by its neutral plane; placed and
/// folded, the kernel puts the part's outer skin at its origin. The
/// blank's [`Blank::skin_offset`] bridges the two, so the plates and the
/// folded parts of a flat view sit where the folded view has them.
pub enum Shape<'a> {
    Blank { blank: &'a Blank, thickness: f64 },
    Plate { solid: &'a Solid, blank: &'a Blank },
    Solid(&'a Solid),
}

/// A placed part: which row it belongs to and where the view puts it.
pub struct Instance<'a> {
    pub row: usize,
    pub shape: Shape<'a>,
    pub matrix: [P3; 3],
    pub position: P3,
}

impl Instance<'_> {
    /// The part's own point in the world.
    pub fn world(&self, q: P3) -> P3 {
        let (m, t) = (self.matrix, self.position);
        [
            m[0][0] * q[0] + m[0][1] * q[1] + m[0][2] * q[2] + t[0],
            m[1][0] * q[0] + m[1][1] * q[1] + m[1][2] * q[2] + t[1],
            m[2][0] * q[0] + m[2][1] * q[1] + m[2][2] * q[2] + t[2],
        ]
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalloonReport {
    pub pos: usize,
    /// Balloon centre on the sheet, mm.
    pub at: [f64; 2],
    /// Where its leader ends on the part.
    pub anchor: [f64; 2],
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssemblyReport {
    pub page: usize,
    pub paper: String,
    pub scale: String,
    pub scale_factor: f64,
    /// Where model `(0, 0)` of the picture plane sits on the sheet.
    pub origin: [f64; 2],
    pub bbox: [f64; 4],
    pub balloons: Vec<BalloonReport>,
}

const BALLOON_R: f64 = 4.0;
/// Clear ring between the picture and the balloons.
const RING: f64 = 11.0;
const SPACING: f64 = 9.5;

/// The scene of every instance, with the row each object of it belongs
/// to; `None` when nothing folds or places.
pub fn scene(instances: &[Instance]) -> Option<(Scene, Vec<usize>)> {
    let mut sc = Scene::new();
    let mut owner_row = Vec::new();
    for inst in instances {
        let added = match inst.shape {
            Shape::Blank { blank, thickness } => {
                let world = |p: P3| inst.world(blank.to_solid_at(p, 0.0));
                sc.add_blank(blank, thickness, &world)
            }
            Shape::Plate { solid, blank } => {
                let o = blank.skin_offset();
                let world = |p: P3| inst.world([p[0] - o[0], p[1] - o[1], p[2] - o[2]]);
                sc.add_solid(solid, &world)
            }
            Shape::Solid(solid) => sc.add_solid(solid, &|p| inst.world(p)),
        };
        if added.is_some() {
            owner_row.push(inst.row);
        }
    }
    if sc.objects() == 0 {
        return None;
    }
    Some((sc, owner_row))
}

/// The pictorial's lines on the sheet: outlines wide, tangent edges thin
/// (ISO 128-2).
pub fn draw_lines(c: &mut crate::pdf::Content, iso: &Iso, pm: &dyn Fn(P2) -> P2) {
    for thin in [false, true] {
        c.width(if thin { 0.18 } else { 0.35 });
        for (k, pl) in iso.polylines.iter().enumerate() {
            if iso.thin.get(k).copied().unwrap_or(false) != thin {
                continue;
            }
            let pts: Vec<P2> = super::flat::thin(pl.iter().map(|q| pm(*q)).collect());
            if pts.len() >= 2 {
                c.polyline(&pts, false);
            }
        }
    }
}

/// Points on a rectangle's perimeter by arc length from its lower-left
/// corner, counter-clockwise.
fn on_rect(r: [f64; 4], t: f64) -> P2 {
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    let p = 2.0 * (w + h);
    let t = t.rem_euclid(p);
    if t < w {
        [r[0] + t, r[1]]
    } else if t < w + h {
        [r[2], r[1] + (t - w)]
    } else if t < 2.0 * w + h {
        [r[2] - (t - w - h), r[3]]
    } else {
        [r[0], r[3] - (t - 2.0 * w - h)]
    }
}

/// Where the ray from the rectangle's centre through `a` leaves it, as
/// arc length.
fn ray_t(r: [f64; 4], a: P2) -> f64 {
    let c = [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0];
    let (dx, dy) = (a[0] - c[0], a[1] - c[1]);
    let (hw, hh) = ((r[2] - r[0]) / 2.0, (r[3] - r[1]) / 2.0);
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
        return w + h / 2.0;
    }
    // Scale the direction so the larger normalised component hits ±1.
    let kx = if dx.abs() > 1e-12 {
        hw / dx.abs()
    } else {
        f64::INFINITY
    };
    let ky = if dy.abs() > 1e-12 {
        hh / dy.abs()
    } else {
        f64::INFINITY
    };
    let k = kx.min(ky);
    let (px, py) = (dx * k, dy * k);
    if (py + hh).abs() < 1e-6 {
        px + hw
    } else if (px - hw).abs() < 1e-6 {
        w + (py + hh)
    } else if (py - hh).abs() < 1e-6 {
        w + h + (hw - px)
    } else {
        2.0 * w + h + (hh - py)
    }
}

/// Spread arc-length positions on a ring of perimeter `p` so neighbours
/// keep `gap`; keeps the order.
fn spread(ts: &mut [f64], p: f64, gap: f64) {
    let n = ts.len();
    if n < 2 || gap * n as f64 >= p {
        return;
    }
    for _ in 0..60 {
        let mut moved = false;
        for i in 0..n {
            let j = (i + 1) % n;
            let mut d = ts[j] - ts[i];
            if j == 0 {
                d += p;
            }
            if d < gap - 1e-6 {
                let push = (gap - d) / 2.0;
                ts[i] -= push;
                ts[j] += push;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    for t in ts.iter_mut() {
        *t = t.rem_euclid(p);
    }
}

/// Draw the assembly sheet into reserved page `page` (A3), with the
/// parts list rows `bom_range` of `bom` beside it. Returns the report.
#[allow(clippy::too_many_arguments)]
pub fn sheet(
    doc: &mut Doc,
    page: usize,
    pages: usize,
    meta: &Meta,
    lang: Lang,
    rows: &[Row],
    iso: &Iso,
    owner_row: &[usize],
    bom: &Table,
    bom_range: std::ops::Range<usize>,
) -> AssemblyReport {
    let paper = Paper::A3;
    let notes = [lang.t("balloon_note"), lang.t("order_note")];
    let (mut c, area) = lists::begin(paper, lang, lang.t("assembly"), "", &notes);
    // The parts list, right, above the title block.
    let (bx, _) = paper.title_block();
    let bom_w = bom.width();
    let bx = bx + sheet::TB_W - bom_w;
    bom.draw(&mut c, doc, page, bx, area[3], bom_range);
    // The picture area left of it, with room for the balloons.
    let px0 = area[0] + RING + BALLOON_R;
    let px1 = bx - 6.0 - RING - BALLOON_R;
    let py0 = area[1] + RING + BALLOON_R;
    let py1 = area[3] - RING - BALLOON_R;
    let (mw, mh) = (iso.bbox[2] - iso.bbox[0], iso.bbox[3] - iso.bbox[1]);
    let s = super::flat::SCALES
        .iter()
        .copied()
        .find(|s| mw * s <= px1 - px0 && mh * s <= py1 - py0)
        .unwrap_or(0.01);
    let scale_text = text::scale(s);
    let origin = [
        (px0 + px1) / 2.0 - (iso.bbox[0] + iso.bbox[2]) / 2.0 * s,
        (py0 + py1) / 2.0 - (iso.bbox[1] + iso.bbox[3]) / 2.0 * s,
    ];
    let pm = |q: P2| [origin[0] + q[0] * s, origin[1] + q[1] * s];
    let bbox = [
        origin[0] + iso.bbox[0] * s,
        origin[1] + iso.bbox[1] * s,
        origin[0] + iso.bbox[2] * s,
        origin[1] + iso.bbox[3] * s,
    ];
    draw_lines(&mut c, iso, &pm);
    // Scale under the caption, left.
    c.text([area[0] + 1.0, area[3] - 4.0], 3.5, 0.0, &scale_text);

    // Balloons: one per row that has visible lines, anchored at the
    // centroid of its first instance's visible points.
    let mut anchors: Vec<(usize, P2)> = Vec::new();
    for k in 0..iso.polylines.len() {
        let Some(&row) = owner_row.get(iso.owner[k]) else {
            continue;
        };
        if anchors.iter().any(|(r, _)| *r == row) {
            continue;
        }
        // All polylines of this blank.
        let blank = iso.owner[k];
        let mut sum = [0.0, 0.0];
        let mut n = 0.0;
        for (j, q) in iso.polylines.iter().enumerate() {
            if iso.owner[j] != blank {
                continue;
            }
            for p in q {
                sum[0] += p[0];
                sum[1] += p[1];
                n += 1.0;
            }
        }
        if n > 0.0 {
            anchors.push((row, pm([sum[0] / n, sum[1] / n])));
        }
    }
    anchors.sort_by_key(|(r, _)| rows[*r].pos);
    let ring = [
        bbox[0] - RING,
        bbox[1] - RING,
        bbox[2] + RING,
        bbox[3] + RING,
    ];
    let perimeter = 2.0 * (ring[2] - ring[0] + ring[3] - ring[1]);
    let mut order: Vec<usize> = (0..anchors.len()).collect();
    let mut ts: Vec<f64> = anchors.iter().map(|(_, a)| ray_t(ring, *a)).collect();
    order.sort_by(|a, b| ts[*a].total_cmp(&ts[*b]));
    let mut sorted: Vec<f64> = order.iter().map(|i| ts[*i]).collect();
    spread(&mut sorted, perimeter, SPACING);
    for (k, i) in order.iter().enumerate() {
        ts[*i] = sorted[k];
    }
    let mut balloons = Vec::new();
    c.width(0.25);
    for (i, (row, anchor)) in anchors.iter().enumerate() {
        let at = on_rect(ring, ts[i]);
        // Leader from the balloon's rim to the anchor, with a dot.
        let (dx, dy) = (anchor[0] - at[0], anchor[1] - at[1]);
        let len = dx.hypot(dy).max(1e-9);
        let rim = [at[0] + dx / len * BALLOON_R, at[1] + dy / len * BALLOON_R];
        c.line(rim, *anchor);
        c.circle_fill(*anchor, 0.6);
        c.width(0.5);
        c.circle(at, BALLOON_R);
        c.width(0.25);
        let pos = rows[*row].pos;
        c.text_centred(at[0], at[1] - 1.75, 3.5, &pos.to_string());
        balloons.push(BalloonReport {
            pos,
            at,
            anchor: *anchor,
        });
    }

    lists::finish(
        doc,
        page,
        paper,
        lang,
        meta,
        c,
        lang.t("assembly_title"),
        &scale_text,
        pages,
        lang.t("assembly_title"),
    );
    AssemblyReport {
        page,
        paper: paper.name().into(),
        scale: scale_text,
        scale_factor: s,
        origin,
        bbox,
        balloons,
    }
}

/// The parts list on an A4 page of its own (when there is no picture).
pub fn bom_only(
    doc: &mut Doc,
    first_page: usize,
    pages: usize,
    meta: &Meta,
    lang: Lang,
    bom: &Table,
) -> usize {
    lists::table_pages(
        doc,
        first_page,
        pages,
        meta,
        lang,
        bom,
        lang.t("bom"),
        lang.t("bom_title"),
        &[lang.t("balloon_note"), lang.t("order_note")],
    )
}
