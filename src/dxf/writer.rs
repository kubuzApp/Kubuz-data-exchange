//! DXF R12 (AC1009) text, the dialect every laser shop's CAM reads: lines,
//! arcs and circles on named layers, and no text at all, so nothing in the
//! file can be mistaken for a cut. Nothing newer is needed for a flat
//! pattern, and nothing newer is read as widely.

use super::blank::{Blank, Seg, P2};

pub const LAYER_CUT: &str = "CUT";
pub const LAYER_BEND: &str = "BEND";
pub const LAYER_ENGRAVE: &str = "ENGRAVE";

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
}

/// The whole file for one blank. The ENGRAVE layer is declared but empty:
/// the kernel has no markings yet, and the part's id travels in the file
/// name, not as text in the drawing.
pub fn write(blank: &Blank) -> String {
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
    }
    d.pair(0, "ENDSEC");
    d.pair(0, "EOF");
    d.out
}
