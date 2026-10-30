//! The assembly sheets: every part folded and placed as the view places
//! it, drawn as one isometric pictorial with item balloons (ISO 6433)
//! pointing at each position, and the parts list beside it above the
//! title block.
//!
//! A complex assembly gets a sheet per group (sub-assembly): the whole
//! picture with that group's parts in wide lines and the rest narrow and
//! grey for context, balloons for that group's positions only, and its
//! own part of the parts list. The overview sheet then names the groups
//! instead of the positions. Parts too small to point at on the picture
//! get a detail view (ISO 128-30): an enlarged circle of the picture
//! beside the parts list, with their balloons around it.

use serde::Serialize;

use super::balloons::{self, Side, Slot};
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
#[derive(Clone, Copy)]
pub enum Shape<'a> {
    Blank { blank: &'a Blank, thickness: f64 },
    Plate { solid: &'a Solid, blank: &'a Blank },
    Solid(&'a Solid),
}

/// A placed part: which row it belongs to and where the view puts it.
#[derive(Clone, Copy)]
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

/// A group named on the overview: the label, where its reference line
/// ends and where its leader ends on one of the group's parts.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalloutReport {
    pub group: String,
    pub text: String,
    pub at: [f64; 2],
    pub anchor: [f64; 2],
    /// The page (0-based) of the group's sheet.
    pub page: usize,
}

/// An enlarged circle of the picture: where it is on the main picture
/// (centre and radius on the sheet), where its copy is drawn and at what
/// scale, and the balloons around it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailReport {
    pub letter: String,
    pub scale: String,
    pub scale_factor: f64,
    /// The circle on the main picture, sheet mm.
    pub centre: [f64; 2],
    pub radius: f64,
    /// Where model `(0, 0)` of the picture plane sits in the detail.
    pub origin: [f64; 2],
    /// The detail's circle on the sheet.
    pub at: [f64; 2],
    pub at_radius: f64,
    pub balloons: Vec<BalloonReport>,
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
    /// The group this sheet is about; none on the overview.
    pub group: Option<String>,
    pub callouts: Vec<CalloutReport>,
    pub detail: Option<DetailReport>,
}

const BALLOON_R: f64 = 4.0;
/// Clear ring between the picture and the balloons.
const RING: f64 = 11.0;
const SPACING: f64 = 9.5;
/// A part smaller than this on the sheet (mm, the larger side of its
/// picture) goes to the detail view.
const TINY: f64 = 4.0;
const LABEL_H: f64 = 3.5;
const LABEL_GAP: f64 = 7.5;

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

/// What one assembly sheet shows.
pub struct Page<'a> {
    pub caption: String,
    pub sub: String,
    /// The title block's description and the page label.
    pub description: String,
    pub label: String,
    /// The group drawn wide; the others narrow and grey. `None`: all wide.
    pub focus: Option<usize>,
    /// The rows (indices) that get a balloon.
    pub marked: Vec<usize>,
    /// Groups named on the picture instead: group index, text, its page.
    pub callouts: Vec<(usize, String, usize)>,
    /// The group of every row.
    pub row_group: &'a [usize],
    /// The group key, for the report.
    pub group_key: Option<String>,
}

/// The pictorial's lines on the sheet: outlines wide, tangent edges thin
/// (ISO 128-2); lines `wide` says are context come narrow and grey.
pub fn draw_lines(
    c: &mut crate::pdf::Content,
    iso: &Iso,
    pm: &dyn Fn(P2) -> P2,
    wide: &dyn Fn(usize) -> bool,
    clip: Option<(P2, f64)>,
) {
    for pass in 0..3 {
        // Context first, then tangent edges, then outlines on top.
        match pass {
            0 => {
                c.width(0.13);
                c.stroke_gray(0.55);
            }
            1 => {
                c.width(0.18);
                c.stroke_gray(0.0);
            }
            _ => c.width(0.35),
        }
        for (k, pl) in iso.polylines.iter().enumerate() {
            let focus = wide(iso.owner[k]);
            let thin = iso.thin.get(k).copied().unwrap_or(false);
            let this = if !focus {
                0
            } else if thin {
                1
            } else {
                2
            };
            if this != pass {
                continue;
            }
            let pts: Vec<P2> = super::flat::thin(pl.iter().map(|q| pm(*q)).collect());
            match clip {
                None => {
                    if pts.len() >= 2 {
                        c.polyline(&pts, false);
                    }
                }
                Some((cc, r)) => {
                    for piece in clip_circle(&pts, cc, r) {
                        if piece.len() >= 2 {
                            c.polyline(&piece, false);
                        }
                    }
                }
            }
        }
    }
    c.stroke_gray(0.0);
}

/// The pieces of a polyline inside a circle.
fn clip_circle(pts: &[P2], c: P2, r: f64) -> Vec<Vec<P2>> {
    let inside = |p: P2| (p[0] - c[0]).hypot(p[1] - c[1]) <= r;
    let mut out: Vec<Vec<P2>> = Vec::new();
    let mut cur: Vec<P2> = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let f = [a[0] - c[0], a[1] - c[1]];
        let qa = d[0] * d[0] + d[1] * d[1];
        let qb = 2.0 * (f[0] * d[0] + f[1] * d[1]);
        let qc = f[0] * f[0] + f[1] * f[1] - r * r;
        let disc = qb * qb - 4.0 * qa * qc;
        let (t0, t1) = if qa < 1e-12 || disc <= 0.0 {
            if inside(a) {
                (0.0, 1.0)
            } else {
                (1.0, 0.0)
            }
        } else {
            let sq = disc.sqrt();
            (
                ((-qb - sq) / (2.0 * qa)).max(0.0),
                ((-qb + sq) / (2.0 * qa)).min(1.0),
            )
        };
        if t0 >= t1 {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let at = |t: f64| [a[0] + d[0] * t, a[1] + d[1] * t];
        if cur.is_empty() || t0 > 1e-9 {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(at(t0));
        }
        cur.push(at(t1));
        if t1 < 1.0 - 1e-9 {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A balloon with its leader: from the rim toward the anchor, ending in
/// an arrowhead on the part's line (ISO 128-22).
fn balloon(c: &mut crate::pdf::Content, at: P2, anchor: P2, text: &str) {
    let (dx, dy) = (anchor[0] - at[0], anchor[1] - at[1]);
    let len = dx.hypot(dy).max(1e-9);
    let u = [dx / len, dy / len];
    let rim = [at[0] + u[0] * BALLOON_R, at[1] + u[1] * BALLOON_R];
    c.width(0.25);
    c.line(rim, anchor);
    super::dims::arrow(c, anchor, u);
    c.width(0.5);
    c.circle(at, BALLOON_R);
    c.text_centred(at[0], at[1] - 1.75, 3.5, text);
}

/// The sheet box of each row's largest instance on the picture.
fn row_sizes(iso: &Iso, owner_row: &[usize], pm: &dyn Fn(P2) -> P2, n_rows: usize) -> Vec<f64> {
    let mut boxes: std::collections::HashMap<usize, [f64; 4]> = std::collections::HashMap::new();
    for (k, pl) in iso.polylines.iter().enumerate() {
        let b = boxes
            .entry(iso.owner[k])
            .or_insert([f64::MAX, f64::MAX, f64::MIN, f64::MIN]);
        for q in pl {
            let p = pm(*q);
            b[0] = b[0].min(p[0]);
            b[1] = b[1].min(p[1]);
            b[2] = b[2].max(p[0]);
            b[3] = b[3].max(p[1]);
        }
    }
    let mut size = vec![0.0f64; n_rows];
    for (obj, b) in boxes {
        if let Some(&row) = owner_row.get(obj) {
            size[row] = size[row].max((b[2] - b[0]).max(b[3] - b[1]));
        }
    }
    size
}

/// Draw assembly sheet `pg` into reserved page `page` (A3), with the
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
    bom: Option<(&Table, std::ops::Range<usize>)>,
    pg: &Page,
) -> AssemblyReport {
    let paper = Paper::A3;
    let notes = [lang.t("balloon_note"), lang.t("order_note")];
    let (mut c, area) = lists::begin(paper, lang, &pg.caption, &pg.sub, &notes);
    // The parts list, right, above the title block; without one the
    // picture takes the sheet's width.
    let (tbx, _) = paper.title_block();
    let (bx, bom_bottom) = match bom {
        Some((bom, range)) => {
            let bx = tbx + sheet::TB_W - bom.width();
            (bx, bom.draw(&mut c, doc, page, bx, area[3], range))
        }
        None => (area[2] + 6.0, area[1]),
    };
    let row_group = |obj: usize| owner_row.get(obj).map(|&r| pg.row_group[r]);
    let wide = |obj: usize| match pg.focus {
        None => true,
        Some(g) => row_group(obj) == Some(g),
    };

    // Callout labels need their width beside the picture.
    let label_w = pg
        .callouts
        .iter()
        .map(|(_, t, _)| crate::pdf::font::width(t, LABEL_H) + 2.0)
        .fold(0.0, f64::max);
    let mx = if pg.callouts.is_empty() {
        RING + BALLOON_R
    } else {
        RING + label_w
    };
    let my = if pg.callouts.is_empty() {
        RING + BALLOON_R
    } else {
        2.0
    };
    // The picture area left of the parts list.
    let px0 = area[0] + mx;
    let px1 = bx - 6.0 - mx;
    let py0 = area[1] + my;
    let py1 = area[3] - my;
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
    draw_lines(&mut c, iso, &pm, &wide, None);
    // Scale under the caption, left.
    c.text([area[0] + 1.0, area[3] - 4.0], 3.5, 0.0, &scale_text);

    // Which marked rows are too small to point at: they go to a detail
    // view when there is room for one beside the parts list.
    let size = row_sizes(iso, owner_row, &pm, rows.len());
    let tiny: Vec<usize> = pg
        .marked
        .iter()
        .copied()
        .filter(|&r| size[r] < TINY)
        .collect();
    let free = [bx, area[1], area[2], bom_bottom - 8.0];
    let detail_d = (free[2] - free[0]).min(free[3] - free[1]) - 2.0 * (RING + BALLOON_R) - 8.0;
    let detail_on = !tiny.is_empty() && detail_d >= 40.0;
    let on_main: Vec<usize> = pg
        .marked
        .iter()
        .copied()
        .filter(|r| !detail_on || !tiny.contains(r))
        .collect();

    // The balloons on the main picture.
    let room = [area[0], area[1], bx - 4.0, area[3]];
    let mut balloons_out = Vec::new();
    let marked_key =
        |obj: usize, set: &[usize]| owner_row.get(obj).copied().filter(|r| set.contains(r));
    let anchors = balloons::anchors(iso, &|o| marked_key(o, &on_main), &pm, bbox);
    let slots = balloons::place(&anchors, bbox, room, RING, SPACING, &Side::ALL, BALLOON_R);
    for sl in &slots {
        let pos = rows[sl.key].pos;
        balloon(&mut c, sl.at, sl.anchor, &pos.to_string());
        balloons_out.push(BalloonReport {
            pos,
            at: sl.at,
            anchor: sl.anchor,
        });
    }

    // Group callouts: a label on a reference line, its leader to one of
    // the group's parts.
    let mut callouts_out = Vec::new();
    if !pg.callouts.is_empty() {
        let groups: Vec<usize> = pg.callouts.iter().map(|(g, _, _)| *g).collect();
        let key = |obj: usize| row_group(obj).filter(|g| groups.contains(g));
        let anchors = balloons::anchors(iso, &key, &pm, bbox);
        let slots: Vec<Slot> = balloons::place(
            &anchors,
            bbox,
            [area[0], area[1], bx - 4.0, area[3]],
            RING,
            LABEL_GAP,
            &[Side::Left, Side::Right],
            1.0,
        );
        c.width(0.25);
        for sl in &slots {
            let Some((_, text, gpage)) = pg.callouts.iter().find(|(g, _, _)| *g == sl.key) else {
                continue;
            };
            let w = crate::pdf::font::width(text, LABEL_H);
            let (x0, x1) = match sl.side {
                Side::Left => (sl.at[0] - w - 1.0, sl.at[0]),
                _ => (sl.at[0], sl.at[0] + w + 1.0),
            };
            c.line([x0, sl.at[1]], [x1, sl.at[1]]);
            c.text([x0 + 0.5, sl.at[1] + 0.8], LABEL_H, 0.0, text);
            let (dx, dy) = (sl.anchor[0] - sl.at[0], sl.anchor[1] - sl.at[1]);
            let len = dx.hypot(dy).max(1e-9);
            c.line(sl.at, sl.anchor);
            super::dims::arrow(&mut c, sl.anchor, [dx / len, dy / len]);
            doc.add_link(
                page,
                [x0, sl.at[1] - 0.5, x1, sl.at[1] + LABEL_H + 1.0],
                *gpage,
            );
            callouts_out.push(CalloutReport {
                group: sl.key.to_string(),
                text: text.clone(),
                at: sl.at,
                anchor: sl.anchor,
                page: *gpage,
            });
        }
    }

    // The detail view: the circle around the spot where most of the
    // small parts sit together, enlarged beside the parts list.
    let mut detail = None;
    if detail_on {
        // Candidate centres: every small part's anchor, by instance.
        let tiny_anchor = balloons::anchors(iso, &|o| marked_key(o, &tiny), &pm, bbox);
        // Model radius around a centre on the picture plane: enough for a
        // few of the small parts, at least a hand's width.
        let biggest = tiny.iter().map(|&r| size[r]).fold(0.0, f64::max) / s;
        let rm = (biggest * 4.0).clamp(30.0, 150.0);
        let inv = |p: P2| [(p[0] - origin[0]) / s, (p[1] - origin[1]) / s];
        // Pick the centre that holds the most different small parts.
        let mut best: Option<(usize, P2)> = None;
        for (_, a) in &tiny_anchor {
            let cm = inv(*a);
            let mut held = std::collections::HashSet::new();
            for (k, pl) in iso.polylines.iter().enumerate() {
                let Some(&row) = owner_row.get(iso.owner[k]) else {
                    continue;
                };
                if !tiny.contains(&row) {
                    continue;
                }
                if pl
                    .iter()
                    .any(|q| (q[0] - cm[0]).hypot(q[1] - cm[1]) < rm * 0.7)
                {
                    held.insert(row);
                }
            }
            if best.as_ref().is_none_or(|(n, _)| held.len() > *n) {
                best = Some((held.len(), cm));
            }
        }
        if let Some((_, cm)) = best {
            let ds = super::flat::SCALES
                .iter()
                .copied()
                .find(|x| 2.0 * rm * x <= detail_d)
                .unwrap_or(s);
            let ar = rm * ds;
            let at = [(free[0] + free[2]) / 2.0, (free[1] + free[3]) / 2.0 + 2.0];
            let dorigin = [at[0] - cm[0] * ds, at[1] - cm[1] * ds];
            let dm = |q: P2| [dorigin[0] + q[0] * ds, dorigin[1] + q[1] * ds];
            draw_lines(&mut c, iso, &dm, &wide, Some((at, ar)));
            c.width(0.25);
            c.circle(at, ar);
            // The circle on the main picture, with its letter.
            let centre = pm(cm);
            let radius = rm * s;
            c.width(0.25);
            c.circle(centre, radius);
            let letter = "A".to_string();
            let lp = letter_spot(
                iso,
                &pm,
                centre,
                radius,
                crate::pdf::font::width(&letter, 5.0),
                5.0,
            );
            c.text(lp, 5.0, 0.0, &letter);
            let caption = format!("{} {} ({})", lang.t("detail_view"), letter, text::scale(ds));
            c.text_centred(at[0], at[1] + ar + RING + BALLOON_R + 1.5, 3.5, &caption);
            // Balloons around the detail for the small parts it shows.
            let dbox = [at[0] - ar, at[1] - ar, at[0] + ar, at[1] + ar];
            let inside = |p: P2| (p[0] - at[0]).hypot(p[1] - at[1]) < ar - 0.5;
            let key = |obj: usize| marked_key(obj, &tiny);
            let anchors = balloons::anchors_where(iso, &key, &dm, dbox, &|p| inside(p));
            let slots = balloons::place(&anchors, dbox, free, RING, SPACING, &Side::ALL, BALLOON_R);
            let mut dballoons = Vec::new();
            for sl in &slots {
                let pos = rows[sl.key].pos;
                balloon(&mut c, sl.at, sl.anchor, &pos.to_string());
                dballoons.push(BalloonReport {
                    pos,
                    at: sl.at,
                    anchor: sl.anchor,
                });
            }
            // Small parts the detail does not show still get a balloon on
            // the main picture.
            let shown: Vec<usize> = slots.iter().map(|sl| sl.key).collect();
            let left: Vec<usize> = tiny
                .iter()
                .copied()
                .filter(|r| !shown.contains(r))
                .collect();
            if !left.is_empty() {
                let mut all = on_main.clone();
                all.extend(&left);
                let anchors = balloons::anchors(iso, &|o| marked_key(o, &all), &pm, bbox);
                let extra: Vec<(usize, P2)> = anchors
                    .into_iter()
                    .filter(|(k, _)| left.contains(k))
                    .collect();
                let taken: Vec<P2> = balloons_out.iter().map(|b| b.at).collect();
                let slots = balloons::place(
                    &extra,
                    bbox,
                    room,
                    RING + 2.0 * BALLOON_R + 2.0,
                    SPACING,
                    &Side::ALL,
                    BALLOON_R,
                );
                for sl in &slots {
                    if taken
                        .iter()
                        .any(|t| (t[0] - sl.at[0]).hypot(t[1] - sl.at[1]) < SPACING)
                    {
                        continue;
                    }
                    let pos = rows[sl.key].pos;
                    balloon(&mut c, sl.at, sl.anchor, &pos.to_string());
                    balloons_out.push(BalloonReport {
                        pos,
                        at: sl.at,
                        anchor: sl.anchor,
                    });
                }
            }
            detail = Some(DetailReport {
                letter,
                scale: text::scale(ds),
                scale_factor: ds,
                centre,
                radius,
                origin: dorigin,
                at,
                at_radius: ar,
                balloons: dballoons,
            });
        }
    }

    lists::finish(
        doc,
        page,
        paper,
        lang,
        meta,
        c,
        &pg.description,
        &scale_text,
        pages,
        &pg.label,
    );
    AssemblyReport {
        page,
        paper: paper.name().into(),
        scale: scale_text,
        scale_factor: s,
        origin,
        bbox,
        balloons: balloons_out,
        group: pg.group_key.clone(),
        callouts: callouts_out,
        detail,
    }
}

/// Where a detail circle's letter goes: just outside the circle, on the
/// side where the fewest of the picture's lines cross it. Returns the
/// text's start (lower left).
fn letter_spot(iso: &Iso, pm: &dyn Fn(P2) -> P2, centre: P2, radius: f64, w: f64, h: f64) -> P2 {
    let hits = |b: [f64; 4]| {
        let inside = |p: P2| p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3];
        iso.polylines
            .iter()
            .flat_map(|pl| pl.windows(2))
            .filter(|seg| {
                let (a, q) = (pm(seg[0]), pm(seg[1]));
                (0..=8).any(|i| {
                    let t = i as f64 / 8.0;
                    inside([a[0] + (q[0] - a[0]) * t, a[1] + (q[1] - a[1]) * t])
                })
            })
            .count()
    };
    let mut best = (usize::MAX, [centre[0] + radius, centre[1] + radius]);
    for k in 0..8 {
        let ang = std::f64::consts::FRAC_PI_4 * (k as f64 + 0.5);
        let (dx, dy) = (ang.cos(), ang.sin());
        let p = [
            centre[0] + dx * (radius + 1.0),
            centre[1] + dy * (radius + 1.0),
        ];
        // The box's corner nearest the circle at p.
        let x0 = if dx >= 0.0 { p[0] } else { p[0] - w };
        let y0 = if dy >= 0.0 { p[1] } else { p[1] - h };
        let n = hits([x0 - 0.5, y0 - 0.5, x0 + w + 0.5, y0 + h + 0.5]);
        if n < best.0 {
            best = (n, [x0, y0]);
        }
    }
    best.1
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
