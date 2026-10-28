//! DXF R12 (AC1009) text, the dialect every laser shop's CAM reads: lines,
//! arcs, circles and text on named layers. Nothing newer is needed for a
//! flat pattern, and nothing newer is read as widely.

use super::blank::{Blank, Seg, P2};

pub const LAYER_CUT: &str = "CUT";
pub const LAYER_BEND: &str = "BEND";
pub const LAYER_ENGRAVE: &str = "ENGRAVE";

/// Height of the text next to a bend line and of the part mark, in mm.
const BEND_TEXT: f64 = 2.5;
const MARK_TEXT: f64 = 5.0;

/// A number as DXF writes it: nine decimals (an arc of a few metres'
/// radius still meets its neighbour within a micron), trailing zeros
/// dropped, no negative zero.
pub fn num(x: f64) -> String {
    let s = format!("{x:.9}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    match s {
        "" | "-0" => "0".into(),
        _ => s.into(),
    }
}

pub struct Dxf {
    out: String,
}

impl Dxf {
    fn pair(&mut self, code: i32, value: &str) {
        self.out.push_str(&format!("{code}\n{value}\n"));
    }

    fn point(&mut self, base: i32, p: P2) {
        self.pair(base, &num(p[0]));
        self.pair(base + 10, &num(p[1]));
        self.pair(base + 20, "0");
    }

    fn line(&mut self, layer: &str, a: P2, b: P2) {
        self.pair(0, "LINE");
        self.pair(8, layer);
        self.point(10, a);
        self.point(11, b);
    }

    fn arc(&mut self, layer: &str, c: P2, r: f64, start: f64, end: f64) {
        self.pair(0, "ARC");
        self.pair(8, layer);
        self.point(10, c);
        self.pair(40, &num(r));
        self.pair(50, &num(start));
        self.pair(51, &num(end));
    }

    fn circle(&mut self, layer: &str, c: P2, r: f64) {
        self.pair(0, "CIRCLE");
        self.pair(8, layer);
        self.point(10, c);
        self.pair(40, &num(r));
    }

    /// Text centred on `at`, turned by `rotation` degrees.
    fn text(&mut self, layer: &str, at: P2, height: f64, rotation: f64, text: &str) {
        self.pair(0, "TEXT");
        self.pair(8, layer);
        self.point(10, at);
        self.pair(40, &num(height));
        self.pair(1, text);
        if rotation.abs() > 1e-9 {
            self.pair(50, &num(rotation));
        }
        self.pair(72, "1");
        self.point(11, at);
    }
}

/// The whole file for one blank, marked `mark` (the part's id) on the
/// ENGRAVE layer.
pub fn write(blank: &Blank, mark: &str) -> String {
    let mut d = Dxf { out: String::new() };
    d.pair(0, "SECTION");
    d.pair(2, "HEADER");
    d.pair(9, "$ACADVER");
    d.pair(1, "AC1009");
    d.pair(9, "$INSUNITS");
    d.pair(70, "4");
    d.pair(9, "$EXTMIN");
    d.point(10, blank.min);
    d.pair(9, "$EXTMAX");
    d.point(10, blank.max);
    d.pair(0, "ENDSEC");

    d.pair(0, "SECTION");
    d.pair(2, "TABLES");
    d.pair(0, "TABLE");
    d.pair(2, "LTYPE");
    d.pair(70, "2");
    for (name, desc, pattern) in [
        ("CONTINUOUS", "Solid line", vec![]),
        ("DASHED", "Dashed __ __ __", vec![5.0, -2.5]),
    ] {
        d.pair(0, "LTYPE");
        d.pair(2, name);
        d.pair(70, "64");
        d.pair(3, desc);
        d.pair(72, "65");
        d.pair(73, &pattern.len().to_string());
        let total: f64 = pattern.iter().map(|x: &f64| x.abs()).sum();
        d.pair(40, &num(total));
        for x in pattern {
            d.pair(49, &num(x));
        }
    }
    d.pair(0, "ENDTAB");
    d.pair(0, "TABLE");
    d.pair(2, "LAYER");
    d.pair(70, "4");
    for (name, colour, ltype) in [
        ("0", 7, "CONTINUOUS"),
        (LAYER_CUT, 7, "CONTINUOUS"),
        (LAYER_BEND, 1, "DASHED"),
        (LAYER_ENGRAVE, 3, "CONTINUOUS"),
    ] {
        d.pair(0, "LAYER");
        d.pair(2, name);
        d.pair(70, "64");
        d.pair(62, &colour.to_string());
        d.pair(6, ltype);
    }
    d.pair(0, "ENDTAB");
    d.pair(0, "ENDSEC");

    d.pair(0, "SECTION");
    d.pair(2, "ENTITIES");
    for s in blank.loops.iter().flatten() {
        match *s {
            Seg::Line { a, b } => d.line(LAYER_CUT, a, b),
            Seg::Arc { c, r, start, end } => d.arc(LAYER_CUT, c, r, start, end),
            Seg::Circle { c, r } => d.circle(LAYER_CUT, c, r),
        }
    }
    for b in &blank.bends {
        d.line(LAYER_BEND, b.a, b.b);
        // The note reads along the line, upright, just beside it.
        let (dx, dy) = (b.b[0] - b.a[0], b.b[1] - b.a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-9 {
            continue;
        }
        let mut rot = dy.atan2(dx).to_degrees();
        let mut side = [-dy / len, dx / len];
        if rot > 90.0 + 1e-9 || rot <= -90.0 + 1e-9 {
            rot += 180.0;
            side = [-side[0], -side[1]];
        }
        let rot = rot.rem_euclid(360.0);
        let mid = [(b.a[0] + b.b[0]) / 2.0, (b.a[1] + b.b[1]) / 2.0];
        let off = BEND_TEXT * 0.8;
        let at = [mid[0] + side[0] * off, mid[1] + side[1] * off];
        d.text(LAYER_BEND, at, BEND_TEXT, rot, &bend_note(b));
    }
    if !mark.is_empty() {
        d.text(LAYER_ENGRAVE, blank.mark, MARK_TEXT, 0.0, mark);
    }
    d.pair(0, "ENDSEC");
    d.pair(0, "EOF");
    d.out
}

/// `DOWN 90%%d R1.5`: direction, angle (`%%d` is DXF's degree sign) and
/// inside radius.
pub fn bend_note(b: &super::blank::BendLine) -> String {
    format!(
        "{} {}%%d R{}",
        b.direction.label(),
        num((b.angle_deg * 100.0).round() / 100.0),
        num((b.inside_radius * 100.0).round() / 100.0)
    )
}
