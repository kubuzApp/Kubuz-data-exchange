//! Running dimensions for the folded views (the first composer). A view gets running dimensions (ISO
//! 129-1 §8.4) from its lower-left corner along both axes: one dimension
//! line per axis with an origin circle, a projection line and an arrowhead
//! for every distinct coordinate, and the value written along the
//! projection line just outside the dimension line. Values that would
//! touch are staggered to a further level. Holes and radii get one callout
//! per size (`4× ⌀9`, `2× R5`) with a leader to one member; bend lines a
//! numbered tag beside their end.
//!
//! Everything here is in paper millimetres relative to the model origin's
//! place on the sheet; the scale `s` maps model to paper.

use super::features::{arc_mid, Features};
use super::text;
use crate::dxf::blank::{Seg, P2};
use crate::pdf::{font, Content};

/// Lettering height of dimensions (ISO 129-1: ten times the narrow line).
pub const H: f64 = 2.5;
/// Distance of the dimension line from the blank.
pub const D: f64 = 10.0;
/// Labels nearer than this along the dimension line share no level.
const MIN_SEP: f64 = 3.6;
/// Label box across the projection line.
const ARROW: f64 = 2.5;
const ARROW_HALF: f64 = 0.67;
const OVERSHOOT: f64 = 2.0;
const CALLOUT_PITCH: f64 = 6.0;
const CALLOUT_GAP: f64 = 12.0;
const TAG_R: f64 = 2.5;
const TAG_PITCH: f64 = 5.6;
pub const WIDE: f64 = 0.5;
pub const NARROW: f64 = 0.25;

#[derive(Clone, Debug)]
pub struct Label {
    pub value: f64,
    /// Paper coordinate along the axis.
    pub p: f64,
    pub level: usize,
    pub text: String,
    /// Where the projection line starts (model).
    pub from: P2,
    pub sources: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Callout {
    /// Paper point the leader starts from, at the text's edge.
    pub from: P2,
    pub text: String,
    pub kind: &'static str,
    pub value: f64,
    pub count: usize,
    /// Model point the leader's arrow lands on, and the centre it points at.
    pub anchor: P2,
    pub centre: P2,
    /// Paper start of the text (baseline).
    pub at: P2,
}

#[derive(Clone, Debug)]
pub struct Tag {
    pub nr: usize,
    /// Paper ends of the drawn (extended) bend line.
    pub line: [P2; 2],
    /// Paper centre of the circle.
    pub at: P2,
    /// Unit direction out along the line, in which the tag moves when it
    /// sits on something.
    pub u: P2,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub s: f64,
    pub xs: Vec<Label>,
    pub ys: Vec<Label>,
    pub x_levels: usize,
    pub y_levels: usize,
    pub x_lmax: f64,
    pub y_lmax: f64,
    pub callouts: Vec<Callout>,
    pub tags: Vec<Tag>,
    /// Paper mm beyond the scaled blank: left, bottom, right, top.
    pub margin: [f64; 4],
}

/// Callouts of one kind and size: (kind, value, (anchor, centre) members).
type Group = (&'static str, f64, Vec<(P2, P2)>);

/// How far a local callout's leader runs out from the feature's edge.
const LEADER: f64 = 5.0;

fn dist(a: P2, b: P2) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn inside(b: [f64; 4], p: P2) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}

fn overlap(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Points along a model segment, in paper mm, about one per millimetre.
pub(crate) fn sample(seg: &Seg, s: f64, out: &mut Vec<P2>) {
    match *seg {
        Seg::Line { a, b } => sample_line([a[0] * s, a[1] * s], [b[0] * s, b[1] * s], out),
        Seg::Arc { c, r, start, end } => {
            let sweep = (end - start).rem_euclid(360.0);
            let sweep = if sweep < 1e-9 { 360.0 } else { sweep };
            let n = ((r * s * sweep.to_radians()).ceil() as usize).max(2);
            for i in 0..=n {
                let a = (start + sweep * i as f64 / n as f64).to_radians();
                out.push([(c[0] + r * a.cos()) * s, (c[1] + r * a.sin()) * s]);
            }
        }
        Seg::Circle { c, r } => {
            let n = ((2.0 * std::f64::consts::PI * r * s).ceil() as usize).max(8);
            for i in 0..n {
                let a = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
                out.push([(c[0] + r * a.cos()) * s, (c[1] + r * a.sin()) * s]);
            }
        }
    }
}

pub(crate) fn sample_line(a: P2, b: P2, out: &mut Vec<P2>) {
    let n = (dist(a, b).ceil() as usize).max(1);
    for i in 0..=n {
        let t = i as f64 / n as f64;
        out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
    }
}

pub fn layout(f: &Features, s: f64, lang: text::Lang) -> Layout {
    let _ = lang;
    let (xs, x_levels, x_lmax) = labels(&f.xs, s, true);
    let (ys, y_levels, y_lmax) = labels(&f.ys, s, false);
    let w = f.max[0] * s;
    let h = f.max[1] * s;

    // Obstacles a callout text may not sit on: the outline, the bend
    // lines and the projection lines, sampled every millimetre on paper.
    let mut obstacles: Vec<P2> = Vec::new();
    for seg in f.loops.iter().flatten() {
        sample(seg, s, &mut obstacles);
    }
    for b in &f.bends {
        sample(&Seg::Line { a: b.a, b: b.b }, s, &mut obstacles);
    }
    for l in &xs {
        sample_line([l.p, l.from[1] * s], [l.p, -D], &mut obstacles);
    }
    for l in &ys {
        sample_line([l.from[0] * s, l.p], [-D, l.p], &mut obstacles);
    }
    let mut boxes: Vec<[f64; 4]> = Vec::new();

    // Callouts: one per size, next to its feature when a corner of the
    // sheet around it is free, else in a column down the right side.
    let mut callouts = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    for c in &f.circles {
        let d = (c.r * 2.0 * 10.0).round() / 10.0;
        let anchor = [c.c[0] + c.r, c.c[1]];
        push_group(&mut groups, "diameter", d, (anchor, c.c));
    }
    for a in &f.arcs {
        let r = (a.r * 10.0).round() / 10.0;
        push_group(&mut groups, "radius", r, (arc_mid(a), a.c));
    }
    let mut column = 0usize;
    for (kind, value, members) in &groups {
        let n = members.len();
        let sym = if *kind == "diameter" { "⌀" } else { "R" };
        let t = if n > 1 {
            format!("{n}× {sym}{}", text::mm(*value))
        } else {
            format!("{sym}{}", text::mm(*value))
        };
        let tw = font::width(&t, H);
        // The member nearest the top right, where a callout reads best.
        let (anchor, centre) = *members
            .iter()
            .max_by(|a, b| (a.1[0] + 0.25 * a.1[1]).total_cmp(&(b.1[0] + 0.25 * b.1[1])))
            .unwrap();
        let r = ((anchor[0] - centre[0]).powi(2) + (anchor[1] - centre[1]).powi(2)).sqrt() * s;
        let pc = [centre[0] * s, centre[1] * s];
        // Where the leader may leave the feature: a hole from four
        // diagonals, an arc only through the arc itself.
        let dirs: Vec<P2> = if *kind == "diameter" {
            let k = std::f64::consts::FRAC_1_SQRT_2;
            vec![[k, k], [-k, k], [k, -k], [-k, -k]]
        } else {
            let arc = f
                .arcs
                .iter()
                .find(|a| dist(a.c, centre) < 1e-9 && (a.r - r / s).abs() < 1e-6)
                .cloned();
            match arc {
                Some(a) => {
                    let sweep = (a.end - a.start).rem_euclid(360.0);
                    let sweep = if sweep < 1e-9 { 360.0 } else { sweep };
                    [0.5, 0.25, 0.75]
                        .iter()
                        .map(|t| {
                            let ang = (a.start + sweep * t).to_radians();
                            [ang.cos(), ang.sin()]
                        })
                        .collect()
                }
                None => vec![],
            }
        };
        let mut placed: Option<(P2, P2)> = None;
        for u in dirs {
            let end = [pc[0] + u[0] * (r + LEADER), pc[1] + u[1] * (r + LEADER)];
            let at = if u[0] >= 0.0 {
                [end[0] + 1.0, end[1] - H / 2.0]
            } else {
                [end[0] - 1.0 - tw, end[1] - H / 2.0]
            };
            let bx = [at[0] - 0.8, at[1] - 0.8, at[0] + tw + 0.8, at[1] + H + 0.8];
            if bx[0] < 0.0 || bx[1] < 0.0 {
                continue;
            }
            if obstacles.iter().any(|p| inside(bx, *p)) || boxes.iter().any(|b| overlap(bx, *b)) {
                continue;
            }
            let from = if u[0] >= 0.0 {
                [at[0] - 0.8, at[1] + H / 2.0]
            } else {
                [at[0] + tw + 0.8, at[1] + H / 2.0]
            };
            placed = Some((at, from));
            break;
        }
        let (at, from) = placed.unwrap_or_else(|| {
            let at = [w + CALLOUT_GAP, h - H - CALLOUT_PITCH * column as f64];
            column += 1;
            (at, [at[0] - 0.8, at[1] + H / 2.0])
        });
        boxes.push([at[0] - 0.8, at[1] - 0.8, at[0] + tw + 0.8, at[1] + H + 0.8]);
        callouts.push(Callout {
            from,
            text: t,
            kind,
            value: *value,
            count: n,
            anchor,
            centre,
            at,
        });
    }

    // Bend tags beyond the far end of each line, spread apart where lines
    // sit close together.
    let mut right: Vec<usize> = Vec::new();
    let mut top: Vec<usize> = Vec::new();
    let mut tags = Vec::new();
    for b in &f.bends {
        let (a, e) = ([b.a[0] * s, b.a[1] * s], [b.b[0] * s, b.b[1] * s]);
        let d = [e[0] - a[0], e[1] - a[1]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
        let u = [d[0] / l, d[1] / l];
        let horizontal = u[1].abs() < 1e-6;
        // The far end: right for a horizontal line, top otherwise.
        let far_first = if horizontal { a[0] > e[0] } else { a[1] > e[1] };
        let (near_end, far_end, u) = if far_first {
            (e, a, [-u[0], -u[1]])
        } else {
            (a, e, u)
        };
        let line = [
            [
                near_end[0] - u[0] * OVERSHOOT,
                near_end[1] - u[1] * OVERSHOOT,
            ],
            [far_end[0] + u[0] * OVERSHOOT, far_end[1] + u[1] * OVERSHOOT],
        ];
        let at = [
            far_end[0] + u[0] * (OVERSHOOT + 1.5 + TAG_R),
            far_end[1] + u[1] * (OVERSHOOT + 1.5 + TAG_R),
        ];
        if horizontal {
            right.push(tags.len());
        } else {
            top.push(tags.len());
        }
        tags.push(Tag {
            nr: b.nr,
            line,
            at,
            u,
        });
    }
    spread(&mut tags, &right, 1);
    spread(&mut tags, &top, 0);
    // Where a tag lands on another, on a callout or on the drawing, move
    // it further out along its line.
    for _ in 0..30 {
        let mut moved = false;
        for i in 0..tags.len() {
            let at = tags[i].at;
            let hit = (0..tags.len()).any(|j| j != i && dist(at, tags[j].at) < 2.0 * TAG_R + 0.5)
                || boxes.iter().any(|b| {
                    overlap(
                        *b,
                        [at[0] - TAG_R, at[1] - TAG_R, at[0] + TAG_R, at[1] + TAG_R],
                    )
                })
                || obstacles.iter().any(|p| dist(at, *p) < TAG_R + 0.4);
            if hit {
                let u = tags[i].u;
                tags[i].at = [at[0] + u[0] * 2.0, at[1] + u[1] * 2.0];
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }

    let mut left = D + 3.0 + y_levels as f64 * (y_lmax + 2.0) + 2.0;
    let bottom = D + 3.0 + x_levels as f64 * (x_lmax + 2.0) + 2.0;
    let mut right_m: f64 = 3.0;
    if !right.is_empty() {
        right_m = right_m.max(OVERSHOOT + 1.5 + 2.0 * TAG_R + 1.0);
    }
    for b in &boxes {
        right_m = right_m.max(b[2] + 1.0 - w);
    }
    let mut top_m: f64 = 3.0;
    for b in &boxes {
        top_m = top_m.max(b[3] + 1.0 - h);
    }
    for t in &tags {
        right_m = right_m.max(t.at[0] + TAG_R + 1.0 - w);
        top_m = top_m.max(t.at[1] + TAG_R + 1.0 - h);
    }
    let mut bottom_m = bottom;
    for b in &boxes {
        bottom_m = bottom_m.max(1.0 - b[1]);
    }
    for t in &tags {
        bottom_m = bottom_m.max(TAG_R + 1.0 - t.at[1]);
        left = left.max(TAG_R + 1.0 - t.at[0]);
    }
    Layout {
        s,
        xs,
        ys,
        x_levels,
        y_levels,
        x_lmax,
        y_lmax,
        callouts,
        tags,
        margin: [left, bottom_m, right_m, top_m],
    }
}

fn push_group(groups: &mut Vec<Group>, kind: &'static str, v: f64, m: (P2, P2)) {
    for g in groups.iter_mut() {
        if g.0 == kind && (g.1 - v).abs() < 1e-9 {
            g.2.push(m);
            return;
        }
    }
    groups.push((kind, v, vec![m]));
}

/// Labels along one axis with their levels: each takes the lowest level
/// no closer label already uses. Returns the labels, the level count and
/// the longest text.
fn labels(ords: &[super::features::Ordinate], s: f64, x_axis: bool) -> (Vec<Label>, usize, f64) {
    let mut out: Vec<Label> = Vec::new();
    let mut lmax: f64 = 0.0;
    for o in ords {
        let p = o.value * s;
        let text = text::mm(o.value);
        lmax = lmax.max(font::width(&text, H));
        let mut level = 0;
        loop {
            let clash = out
                .iter()
                .any(|l| l.level == level && (p - l.p).abs() < MIN_SEP);
            if !clash {
                break;
            }
            level += 1;
        }
        // Start the projection line at the source nearest the dimension
        // line: lowest for an x value, leftmost for a y value.
        let from = *o
            .points
            .iter()
            .min_by(|a, b| {
                let (ka, kb) = if x_axis { (a[1], b[1]) } else { (a[0], b[0]) };
                ka.total_cmp(&kb)
            })
            .unwrap();
        out.push(Label {
            value: o.value,
            p,
            level,
            text,
            from,
            sources: o.sources.clone(),
        });
    }
    let levels = out.iter().map(|l| l.level + 1).max().unwrap_or(1);
    (out, levels, lmax)
}

/// Push the tags in `which` apart along axis `axis` (0 = x, 1 = y) so
/// their circles never overlap.
fn spread(tags: &mut [Tag], which: &[usize], axis: usize) {
    let mut order: Vec<usize> = which.to_vec();
    order.sort_by(|a, b| tags[*a].at[axis].total_cmp(&tags[*b].at[axis]));
    for w in 1..order.len() {
        let prev = tags[order[w - 1]].at[axis];
        let i = order[w];
        if tags[i].at[axis] < prev + TAG_PITCH {
            tags[i].at[axis] = prev + TAG_PITCH;
        }
    }
}

/// The level of an x label decides how far below the dimension line its
/// text starts; the text runs upward along the projection line.
fn x_text_bottom(l: &Label, lo: &Layout) -> f64 {
    -D - 3.0 - (l.level as f64 + 1.0) * (lo.x_lmax + 2.0) + 2.0
}

fn y_text_right(l: &Label, lo: &Layout) -> f64 {
    -D - 3.0 - l.level as f64 * (lo.y_lmax + 2.0)
}

/// Paper boxes of the running dimensions' texts, relative to the model
/// origin, for whoever places more text on the view.
pub fn label_boxes(lo: &Layout) -> Vec<[f64; 4]> {
    let mut v = Vec::new();
    for l in &lo.xs {
        let bottom = x_text_bottom(l, lo);
        let w = font::width(&l.text, H);
        v.push([l.p - 0.5 - H, bottom, l.p - 0.5, bottom + w]);
    }
    for l in &lo.ys {
        let right = y_text_right(l, lo);
        let w = font::width(&l.text, H);
        v.push([right - w, l.p + 0.5, right, l.p + 0.5 + H]);
    }
    v
}

/// A closed filled arrowhead with its tip at `tip`, pointing along `u`.
pub fn arrow(c: &mut Content, tip: P2, u: P2) {
    let n = [-u[1], u[0]];
    let base = [tip[0] - u[0] * ARROW, tip[1] - u[1] * ARROW];
    c.move_to(tip);
    c.line_to([base[0] + n[0] * ARROW_HALF, base[1] + n[1] * ARROW_HALF]);
    c.line_to([base[0] - n[0] * ARROW_HALF, base[1] - n[1] * ARROW_HALF]);
    c.close();
    c.fill();
}

/// Draw the dimensions with the model origin at paper `o`.
pub fn draw(lo: &Layout, f: &Features, c: &mut Content, o: P2) {
    let s = lo.s;
    let pp = |p: P2| [o[0] + p[0], o[1] + p[1]];
    let pm = |p: P2| [o[0] + p[0] * s, o[1] + p[1] * s];
    let w = f.max[0] * s;
    let h = f.max[1] * s;
    c.save();
    c.width(NARROW);
    c.solid();

    // Centre marks on holes.
    for ci in &f.circles {
        let cc = pm(ci.c);
        let e = (ci.r * s + 1.5).max(2.0);
        c.line([cc[0] - e, cc[1]], [cc[0] + e, cc[1]]);
        c.line([cc[0], cc[1] - e], [cc[0], cc[1] + e]);
    }

    // X axis: dimension line below the blank, origin circle at x = 0.
    let yd = -D;
    c.line(pp([0.0, yd]), pp([w, yd]));
    c.circle(pp([0.0, yd]), 1.25);
    for l in &lo.xs {
        let from = pm(l.from);
        let start_y = if l.from[1] * s < 0.5 {
            yd + 0.0 - 0.0
        } else {
            from[1]
        };
        let top = if l.from[1] * s < 0.5 {
            o[1] - 1.0
        } else {
            start_y
        };
        c.line([o[0] + l.p, top], pp([l.p, yd - OVERSHOOT]));
        if l.p > 0.5 {
            arrow(c, pp([l.p, yd]), [1.0, 0.0]);
        }
        let bottom = x_text_bottom(l, lo);
        c.text(pp([l.p - 0.5, bottom]), H, 90.0, &l.text);
    }

    // Y axis: dimension line left of the blank, origin circle at y = 0.
    let xd = -D;
    c.line(pp([xd, 0.0]), pp([xd, h]));
    c.circle(pp([xd, 0.0]), 1.25);
    for l in &lo.ys {
        let from = pm(l.from);
        let right = if l.from[0] * s < 0.5 {
            o[0] - 1.0
        } else {
            from[0]
        };
        c.line([right, o[1] + l.p], pp([xd - OVERSHOOT, l.p]));
        if l.p > 0.5 {
            arrow(c, pp([xd, l.p]), [0.0, 1.0]);
        }
        c.text_right(o[0] + y_text_right(l, lo), o[1] + l.p + 0.5, H, &l.text);
    }

    // Callouts with leaders ending in an arrow on the feature.
    for co in &lo.callouts {
        let at = pp(co.at);
        let start = pp(co.from);
        let centre = pm(co.centre);
        let d = [centre[0] - start[0], centre[1] - start[1]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
        let u = [d[0] / l, d[1] / l];
        // The arrow lands where the leader meets the circle or arc.
        let r = ((co.anchor[0] - co.centre[0]).powi(2) + (co.anchor[1] - co.centre[1]).powi(2))
            .sqrt()
            * s;
        let tip = [centre[0] - u[0] * r, centre[1] - u[1] * r];
        c.line(start, tip);
        arrow(c, tip, u);
        c.text(at, H, 0.0, &co.text);
    }

    // Bend lines, extended past the outline, and their tags.
    for t in &lo.tags {
        c.line(pp(t.line[0]), pp(t.line[1]));
        let at = pp(t.at);
        let far = pp(t.line[1]);
        let d = [at[0] - far[0], at[1] - far[1]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if l > TAG_R + 0.6 {
            let u = [d[0] / l, d[1] / l];
            c.line(far, [at[0] - u[0] * TAG_R, at[1] - u[1] * TAG_R]);
        }
        c.circle(at, TAG_R);
        c.text_centred(at[0], at[1] - H / 2.0, H, &t.nr.to_string());
    }
    c.restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dxf::blank::{Blank, Seg};

    #[test]
    fn running_close_values_take_further_levels() {
        // A 50 × 30 plate whose top-right corner is cut back 2 mm: x = 48
        // and x = 50 are too close for one row of labels.
        let blank = Blank {
            loops: vec![vec![
                Seg::Line {
                    a: [0.0, 0.0],
                    b: [50.0, 0.0],
                },
                Seg::Line {
                    a: [50.0, 0.0],
                    b: [50.0, 20.0],
                },
                Seg::Line {
                    a: [50.0, 20.0],
                    b: [48.0, 30.0],
                },
                Seg::Line {
                    a: [48.0, 30.0],
                    b: [0.0, 30.0],
                },
                Seg::Line {
                    a: [0.0, 30.0],
                    b: [0.0, 0.0],
                },
            ]],
            bends: vec![],
            zones: vec![],
            min: [0.0, 0.0],
            max: [50.0, 30.0],
            ..Default::default()
        };
        let f = super::super::features::features(&blank);
        let lo = layout(&f, 1.0, text::Lang::Nl);
        assert_eq!(lo.x_levels, 2, "{:?}", lo.xs);
        assert_eq!(lo.y_levels, 1);
        assert!(lo.margin[1] > lo.margin[0]);
        let mut c = Content::new();
        draw(&lo, &f, &mut c, [50.0, 100.0]);
        assert!(c.as_str().contains(" Tj ET"));
    }
}
