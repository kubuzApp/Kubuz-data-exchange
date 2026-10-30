//! Item references on an assembly picture (ISO 6433, leaders per
//! ISO 128-22): where each leader ends on its part, and where the
//! balloons go so the leaders stay short and none cross.
//!
//! A leader ends with an arrowhead on a visible line of its part: the
//! point of that part's lines with the most room from every other part's
//! lines, and the shortest way out of the picture. The balloons stand in
//! columns left and right of the picture and in rows above and below it,
//! each on the side nearest its anchor, in the order of their anchors
//! along that side, spaced evenly. Pairs of leaders that still cross, or
//! run through a balloon, swap places until none do.

use std::collections::HashMap;

use super::iso::Iso;
use crate::dxf::blank::P2;

/// A side of the picture the marks stand on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Bottom,
    Top,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::Left, Side::Right, Side::Bottom, Side::Top];
}

/// A mark on its side, and where its leader ends.
#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub key: usize,
    /// The mark's reference point: a balloon's centre, or the end of a
    /// label's reference line nearest the picture.
    pub at: P2,
    pub anchor: P2,
    pub side: Side,
}

/// A point on a line and the key of its part.
type Sample = (P2, Option<usize>);

const CELL: f64 = 2.0;
const STEP: f64 = 0.5;
/// Room from other parts' lines worth having; more does not count.
const ENOUGH: f64 = 2.0;

fn dist(a: P2, b: P2) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The best anchor of every key: `key_of` maps a scene object to the key
/// it is marked under (`None`: not marked, its lines only keep others
/// away). `pm` maps the picture plane to the sheet; `pic` is the picture's
/// box on the sheet. Sorted by key.
pub fn anchors(
    iso: &Iso,
    key_of: &dyn Fn(usize) -> Option<usize>,
    pm: &dyn Fn(P2) -> P2,
    pic: [f64; 4],
) -> Vec<(usize, P2)> {
    anchors_where(iso, key_of, pm, pic, &|_| true)
}

/// [`anchors`], with the leaders ending only where `keep` holds (inside
/// a detail view's circle).
pub fn anchors_where(
    iso: &Iso,
    key_of: &dyn Fn(usize) -> Option<usize>,
    pm: &dyn Fn(P2) -> P2,
    pic: [f64; 4],
    keep: &dyn Fn(P2) -> bool,
) -> Vec<(usize, P2)> {
    // Every line sampled on the sheet, in a grid, with its key.
    let mut grid: HashMap<(i64, i64), Vec<Sample>> = HashMap::new();
    let cell = |p: P2| ((p[0] / CELL).floor() as i64, (p[1] / CELL).floor() as i64);
    let sheet: Vec<Vec<P2>> = iso
        .polylines
        .iter()
        .map(|pl| pl.iter().map(|q| pm(*q)).collect())
        .collect();
    for (k, pl) in sheet.iter().enumerate() {
        let key = iso.owner.get(k).and_then(|&o| key_of(o));
        for w in pl.windows(2) {
            let l = dist(w[0], w[1]);
            let n = (l / STEP).ceil().max(1.0) as usize;
            for i in 0..=n {
                let t = i as f64 / n as f64;
                let p = [
                    w[0][0] + (w[1][0] - w[0][0]) * t,
                    w[0][1] + (w[1][1] - w[0][1]) * t,
                ];
                grid.entry(cell(p)).or_default().push((p, key));
            }
        }
    }
    let room = |p: P2, key: usize| -> f64 {
        let (ci, cj) = cell(p);
        let r = (ENOUGH / CELL).ceil() as i64 + 1;
        let mut best = ENOUGH;
        for i in ci - r..=ci + r {
            for j in cj - r..=cj + r {
                if let Some(v) = grid.get(&(i, j)) {
                    for (q, k) in v {
                        if *k != Some(key) {
                            best = best.min(dist(p, *q));
                        }
                    }
                }
            }
        }
        best
    };
    let out_dist = |p: P2| {
        (p[0] - pic[0])
            .min(pic[2] - p[0])
            .min(p[1] - pic[1])
            .min(pic[3] - p[1])
            .max(0.0)
    };
    // Lines of 1 mm and more first; a part drawn smaller than that
    // (a cap or a nut at 1:20) takes its longest line.
    let mut best: HashMap<usize, (f64, P2)> = HashMap::new();
    let mut longest: HashMap<usize, (f64, P2)> = HashMap::new();
    for (k, pl) in sheet.iter().enumerate() {
        let Some(key) = iso.owner.get(k).and_then(|&o| key_of(o)) else {
            continue;
        };
        for w in pl.windows(2) {
            let l = dist(w[0], w[1]);
            let mid = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
            if !keep(mid) {
                continue;
            }
            let e = longest.entry(key).or_insert((-1.0, mid));
            if l > e.0 {
                *e = (l, mid);
            }
            if l < 1.0 {
                continue;
            }
            let ts: &[f64] = if l >= 6.0 { &[0.25, 0.5, 0.75] } else { &[0.5] };
            for &t in ts {
                let p = [
                    w[0][0] + (w[1][0] - w[0][0]) * t,
                    w[0][1] + (w[1][1] - w[0][1]) * t,
                ];
                if !keep(p) {
                    continue;
                }
                let score = room(p, key) * 4.0 - out_dist(p) * 0.05 + l.min(10.0) * 0.05;
                let e = best.entry(key).or_insert((f64::NEG_INFINITY, p));
                if score > e.0 + 1e-9 {
                    *e = (score, p);
                }
            }
        }
    }
    for (k, (_, p)) in longest {
        best.entry(k).or_insert((0.0, p));
    }
    let mut v: Vec<(usize, P2)> = best.into_iter().map(|(k, (_, p))| (k, p)).collect();
    v.sort_by_key(|(k, _)| *k);
    v
}

/// The line a side's marks stand on, and the range along it.
/// The columns own the corners; the rows stop a gap short of them.
fn side_line(
    side: Side,
    pic: [f64; 4],
    room: [f64; 4],
    ring: f64,
    gap: f64,
) -> Option<(f64, f64, f64)> {
    let (at, lo, hi) = match side {
        Side::Left => (
            pic[0] - ring,
            (pic[1] - ring).max(room[1]),
            (pic[3] + ring).min(room[3]),
        ),
        Side::Right => (
            pic[2] + ring,
            (pic[1] - ring).max(room[1]),
            (pic[3] + ring).min(room[3]),
        ),
        Side::Bottom => (
            pic[1] - ring,
            (pic[0] - ring + gap).max(room[0]),
            (pic[2] + ring - gap).min(room[2]),
        ),
        Side::Top => (
            pic[3] + ring,
            (pic[0] - ring + gap).max(room[0]),
            (pic[2] + ring - gap).min(room[2]),
        ),
    };
    let inside = match side {
        Side::Left | Side::Right => at >= room[0] - 1e-9 && at <= room[2] + 1e-9,
        Side::Bottom | Side::Top => at >= room[1] - 1e-9 && at <= room[3] + 1e-9,
    };
    (inside && hi > lo).then_some((at, lo, hi))
}

fn along(side: Side, p: P2) -> f64 {
    match side {
        Side::Left | Side::Right => p[1],
        Side::Bottom | Side::Top => p[0],
    }
}

fn point(side: Side, at: f64, t: f64) -> P2 {
    match side {
        Side::Left | Side::Right => [at, t],
        Side::Bottom | Side::Top => [t, at],
    }
}

/// Keep the order of `ts` and a gap of `gap` between neighbours, inside
/// `[lo, hi]`, moving each as little as the others allow.
fn spread(ts: &mut [f64], lo: f64, hi: f64, gap: f64) {
    let n = ts.len();
    if n == 0 {
        return;
    }
    for t in ts.iter_mut() {
        *t = t.clamp(lo, hi);
    }
    for _ in 0..4 {
        for i in 1..n {
            ts[i] = ts[i].max(ts[i - 1] + gap);
        }
        ts[n - 1] = ts[n - 1].min(hi);
        for i in (0..n - 1).rev() {
            ts[i] = ts[i].min(ts[i + 1] - gap);
        }
        ts[0] = ts[0].max(lo);
    }
}

fn crosses(a: P2, b: P2, c: P2, d: P2) -> bool {
    let o = |p: P2, q: P2, r: P2| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (d1, d2) = (o(a, b, c), o(a, b, d));
    let (d3, d4) = (o(c, d, a), o(c, d, b));
    d1 * d2 < -1e-12 && d3 * d4 < -1e-12
}

fn seg_point_dist(a: P2, b: P2, p: P2) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let t = if l2 < 1e-12 {
        0.0
    } else {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0)
    };
    dist(p, [a[0] + d[0] * t, a[1] + d[1] * t])
}

/// How many pairs of leaders cross, plus how many leaders run through
/// another mark (a circle of `r` around its reference point).
pub fn conflicts(slots: &[Slot], r: f64) -> usize {
    let mut n = 0;
    for i in 0..slots.len() {
        for j in 0..slots.len() {
            if i == j {
                continue;
            }
            let (a, b) = (&slots[i], &slots[j]);
            if i < j && crosses(a.at, a.anchor, b.at, b.anchor) {
                n += 1;
            }
            if seg_point_dist(a.at, a.anchor, b.at) < r + 0.3 {
                n += 1;
            }
        }
    }
    n
}

/// Place a mark for every anchor on `sides` of the picture `pic`, `ring`
/// away from it, inside `room`, `gap` apart along a side. Marks that find
/// no room are left out. Returns the slots in the anchors' order of keys.
pub fn place(
    anchors: &[(usize, P2)],
    pic: [f64; 4],
    room: [f64; 4],
    ring: f64,
    gap: f64,
    sides: &[Side],
    r: f64,
) -> Vec<Slot> {
    let lines: Vec<(Side, f64, f64, f64)> = sides
        .iter()
        .filter_map(|&s| side_line(s, pic, room, ring, gap).map(|(at, lo, hi)| (s, at, lo, hi)))
        .collect();
    let cap: Vec<usize> = lines
        .iter()
        .map(|&(_, _, lo, hi)| ((hi - lo) / gap).floor() as usize + 1)
        .collect();
    // Nearest sides first, while a side has room.
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (i, (_, a)) in anchors.iter().enumerate() {
        for (s, &(side, at, lo, hi)) in lines.iter().enumerate() {
            let perp = match side {
                Side::Left | Side::Right => (a[0] - at).abs(),
                Side::Bottom | Side::Top => (a[1] - at).abs(),
            };
            let t = along(side, *a);
            let off = (lo - t).max(t - hi).max(0.0);
            pairs.push((perp + off, i, s));
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut side_of: Vec<Option<usize>> = vec![None; anchors.len()];
    let mut used = vec![0usize; lines.len()];
    for (_, i, s) in pairs {
        if side_of[i].is_none() && used[s] < cap[s] {
            side_of[i] = Some(s);
            used[s] += 1;
        }
    }
    let mut slots: Vec<Slot> = Vec::new();
    for (s, &(side, at, lo, hi)) in lines.iter().enumerate() {
        let mut members: Vec<usize> = (0..anchors.len())
            .filter(|&i| side_of[i] == Some(s))
            .collect();
        members.sort_by(|&a, &b| along(side, anchors[a].1).total_cmp(&along(side, anchors[b].1)));
        let mut ts: Vec<f64> = members.iter().map(|&i| along(side, anchors[i].1)).collect();
        spread(&mut ts, lo, hi, gap);
        for (k, &i) in members.iter().enumerate() {
            slots.push(Slot {
                key: anchors[i].0,
                at: point(side, at, ts[k]),
                anchor: anchors[i].1,
                side,
            });
        }
    }
    // On each side, swap places while that shortens the leaders: the
    // shortest matching of marks to anchors has no crossing leaders.
    loop {
        let mut improved = false;
        for i in 0..slots.len() {
            for j in i + 1..slots.len() {
                if slots[i].side != slots[j].side {
                    continue;
                }
                let (a, b) = (&slots[i], &slots[j]);
                let now = dist(a.at, a.anchor) + dist(b.at, b.anchor);
                let swapped = dist(b.at, a.anchor) + dist(a.at, b.anchor);
                if swapped < now - 1e-6 {
                    let t = slots[i].at;
                    slots[i].at = slots[j].at;
                    slots[j].at = t;
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
    // Then swap places across sides while that removes a crossing, or
    // shortens the leaders without adding one.
    let len = |s: &[Slot]| s.iter().map(|x| dist(x.at, x.anchor)).sum::<f64>();
    let mut bad = conflicts(&slots, r);
    for _ in 0..200 {
        let mut improved = false;
        for i in 0..slots.len() {
            for j in i + 1..slots.len() {
                let before = len(&slots);
                let (ai, si) = (slots[i].at, slots[i].side);
                slots[i].at = slots[j].at;
                slots[i].side = slots[j].side;
                slots[j].at = ai;
                slots[j].side = si;
                let now = conflicts(&slots, r);
                let shorter = len(&slots) < before - 1.0;
                if now < bad || (now == bad && shorter) {
                    bad = now;
                    improved = true;
                } else {
                    slots[j].at = slots[i].at;
                    slots[j].side = slots[i].side;
                    slots[i].at = ai;
                    slots[i].side = si;
                }
            }
        }
        if !improved {
            break;
        }
    }
    slots.sort_by_key(|s| s.key);
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_keep_their_order_and_leaders_do_not_cross() {
        // Twelve anchors on a diagonal through a 100 × 100 picture.
        let anchors: Vec<(usize, P2)> = (0..12)
            .map(|i| (i, [5.0 + 8.0 * i as f64, 10.0 + 7.0 * i as f64]))
            .collect();
        let pic = [0.0, 0.0, 100.0, 100.0];
        let room = [-40.0, -40.0, 140.0, 140.0];
        let slots = place(&anchors, pic, room, 11.0, 9.5, &Side::ALL, 4.0);
        assert_eq!(slots.len(), 12);
        assert_eq!(conflicts(&slots, 4.0), 0);
        for a in &slots {
            for b in &slots {
                if a.key != b.key {
                    assert!(dist(a.at, b.at) >= 9.5 - 1e-6, "{a:?} {b:?}");
                }
            }
            // Every mark stands outside the picture, on its ring.
            assert!(
                a.at[0] <= -11.0 + 1e-9
                    || a.at[0] >= 111.0 - 1e-9
                    || a.at[1] <= -11.0 + 1e-9
                    || a.at[1] >= 111.0 - 1e-9
            );
        }
    }

    #[test]
    fn a_full_side_sends_the_rest_to_the_next_side() {
        // Twenty-four anchors crowded in the left third of a short
        // picture, whose left column takes seven.
        let anchors: Vec<(usize, P2)> = (0..24)
            .map(|i| (i, [1.0 + ((i * 7) % 19) as f64, 1.0 + 1.6 * i as f64]))
            .collect();
        let pic = [0.0, 0.0, 60.0, 40.0];
        let room = [-30.0, -30.0, 90.0, 70.0];
        let slots = place(&anchors, pic, room, 11.0, 9.5, &Side::ALL, 4.0);
        assert_eq!(slots.len(), 24);
        assert!(slots.iter().any(|s| s.side != Side::Left));
        assert_eq!(conflicts(&slots, 4.0), 0);
    }
}
