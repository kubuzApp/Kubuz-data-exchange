//! A page's content stream, written in hundredths of a millimetre with the
//! origin at the sheet's lower-left corner. Every coordinate is an integer,
//! which keeps the file small and the output exact: nothing here formats a
//! float. The page sets the matrix that maps these units to points.

use super::font;

/// Hundredths of a millimetre per point: the `cm` every page applies.
pub const UNITS_PER_MM: f64 = 100.0;
pub const MATRIX: &str = "0.028346457 0 0 0.028346457 0 0 cm";

pub type P2 = [f64; 2];

fn n(v: f64) -> i64 {
    let r = (v * UNITS_PER_MM).round();
    if r == 0.0 {
        0
    } else {
        r as i64
    }
}

#[derive(Clone, Debug, Default)]
pub struct Content {
    s: String,
}

impl Content {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn raw(&mut self, op: &str) {
        self.s.push_str(op);
        self.s.push('\n');
    }

    /// Line width in mm.
    pub fn width(&mut self, mm: f64) {
        self.raw(&format!("{} w", n(mm)));
    }

    /// Butt caps and round joins: dashes keep their length, corners no
    /// spikes.
    pub fn caps_butt_joins_round(&mut self) {
        self.raw("0 J 1 j");
    }

    pub fn caps_round(&mut self) {
        self.raw("1 J");
    }

    /// A dash pattern in mm, starting `phase` mm into it.
    pub fn dash(&mut self, pattern: &[f64], phase: f64) {
        let p: Vec<String> = pattern.iter().map(|v| n(*v).to_string()).collect();
        self.raw(&format!("[{}] {} d", p.join(" "), n(phase)));
    }

    pub fn solid(&mut self) {
        self.raw("[] 0 d");
    }

    pub fn save(&mut self) {
        self.raw("q");
    }

    pub fn restore(&mut self) {
        self.raw("Q");
    }

    /// Fill grey level 0..1 (0 = black).
    pub fn fill_gray(&mut self, g: f64) {
        self.raw(&format!("{} g", fmt_frac(g)));
    }

    /// Stroke grey level 0..1 (0 = black).
    pub fn stroke_gray(&mut self, g: f64) {
        self.raw(&format!("{} G", fmt_frac(g)));
    }

    pub fn fill_rgb(&mut self, rgb: [f64; 3]) {
        self.raw(&format!(
            "{} {} {} rg",
            fmt_frac(rgb[0]),
            fmt_frac(rgb[1]),
            fmt_frac(rgb[2])
        ));
    }

    pub fn move_to(&mut self, p: P2) {
        self.raw(&format!("{} {} m", n(p[0]), n(p[1])));
    }

    pub fn line_to(&mut self, p: P2) {
        self.raw(&format!("{} {} l", n(p[0]), n(p[1])));
    }

    pub fn curve_to(&mut self, c1: P2, c2: P2, p: P2) {
        self.raw(&format!(
            "{} {} {} {} {} {} c",
            n(c1[0]),
            n(c1[1]),
            n(c2[0]),
            n(c2[1]),
            n(p[0]),
            n(p[1])
        ));
    }

    pub fn close(&mut self) {
        self.raw("h");
    }

    pub fn stroke(&mut self) {
        self.raw("S");
    }

    pub fn fill(&mut self) {
        self.raw("f");
    }

    pub fn fill_even_odd(&mut self) {
        self.raw("f*");
    }

    pub fn line(&mut self, a: P2, b: P2) {
        self.move_to(a);
        self.line_to(b);
        self.stroke();
    }

    pub fn polyline(&mut self, pts: &[P2], closed: bool) {
        let Some((first, rest)) = pts.split_first() else {
            return;
        };
        self.move_to(*first);
        for p in rest {
            self.line_to(*p);
        }
        if closed {
            self.close();
        }
        self.stroke();
    }

    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.raw(&format!("{} {} {} {} re", n(x), n(y), n(w), n(h)));
    }

    pub fn rect_stroke(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.rect(x, y, w, h);
        self.stroke();
    }

    pub fn rect_fill(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.rect(x, y, w, h);
        self.fill();
    }

    /// Append a counter-clockwise arc from `start` to `end` degrees to the
    /// current path, as cubic Béziers of at most 90° each. The path must
    /// already stand at the arc's start.
    /// Append an arc of `sweep` degrees (negative: clockwise) from `start`
    /// about `c`, as Béziers, continuing the current path.
    pub fn arc_sweep(&mut self, c: P2, r: f64, start: f64, sweep: f64) {
        if sweep.abs() < 1e-9 {
            return;
        }
        let pieces = (sweep.abs() / 90.0).ceil().max(1.0) as usize;
        let step = (sweep / pieces as f64).to_radians();
        let mut a0 = start.to_radians();
        for _ in 0..pieces {
            let a1 = a0 + step;
            let k = 4.0 / 3.0 * ((a1 - a0) / 4.0).tan();
            let (s0, c0) = a0.sin_cos();
            let (s1, c1) = a1.sin_cos();
            let p1 = [c[0] + r * (c0 - k * s0), c[1] + r * (s0 + k * c0)];
            let p2 = [c[0] + r * (c1 + k * s1), c[1] + r * (s1 - k * c1)];
            let p = [c[0] + r * c1, c[1] + r * s1];
            self.curve_to(p1, p2, p);
            a0 = a1;
        }
    }

    pub fn arc_segments(&mut self, c: P2, r: f64, start: f64, end: f64) {
        let sweep = (end - start).rem_euclid(360.0);
        let sweep = if sweep == 0.0 { 360.0 } else { sweep };
        let pieces = (sweep / 90.0).ceil().max(1.0) as usize;
        let step = sweep / pieces as f64;
        let mut a0 = start.to_radians();
        for _ in 0..pieces {
            let a1 = a0 + step.to_radians();
            let k = 4.0 / 3.0 * ((a1 - a0) / 4.0).tan();
            let (s0, c0) = a0.sin_cos();
            let (s1, c1) = a1.sin_cos();
            let p1 = [c[0] + r * (c0 - k * s0), c[1] + r * (s0 + k * c0)];
            let p2 = [c[0] + r * (c1 + k * s1), c[1] + r * (s1 - k * c1)];
            let p = [c[0] + r * c1, c[1] + r * s1];
            self.curve_to(p1, p2, p);
            a0 = a1;
        }
    }

    pub fn arc(&mut self, c: P2, r: f64, start: f64, end: f64) {
        let t = start.to_radians();
        self.move_to([c[0] + r * t.cos(), c[1] + r * t.sin()]);
        self.arc_segments(c, r, start, end);
        self.stroke();
    }

    pub fn circle(&mut self, c: P2, r: f64) {
        self.move_to([c[0] + r, c[1]]);
        self.arc_segments(c, r, 0.0, 360.0);
        self.close();
        self.stroke();
    }

    pub fn circle_fill(&mut self, c: P2, r: f64) {
        self.move_to([c[0] + r, c[1]]);
        self.arc_segments(c, r, 0.0, 360.0);
        self.close();
        self.fill();
    }

    /// `text` with capitals `h` mm tall, its baseline starting at `p`,
    /// rotated `deg` counter-clockwise.
    pub fn text(&mut self, p: P2, h: f64, deg: f64, text: &str) {
        if text.is_empty() {
            return;
        }
        let size = n(h / font::CAP);
        let (s, c) = deg.to_radians().sin_cos();
        let m = if deg == 0.0 {
            format!("1 0 0 1 {} {} Tm", n(p[0]), n(p[1]))
        } else {
            format!(
                "{} {} {} {} {} {} Tm",
                fmt_frac(c),
                fmt_frac(s),
                fmt_frac(-s),
                fmt_frac(c),
                n(p[0]),
                n(p[1])
            )
        };
        self.raw(&format!(
            "BT /F1 {size} Tf {m} {} Tj ET",
            font::encode(text)
        ));
    }

    /// Horizontal text centred on `cx`, baseline at `y`.
    pub fn text_centred(&mut self, cx: f64, y: f64, h: f64, text: &str) {
        let w = font::width(text, h);
        self.text([cx - w / 2.0, y], h, 0.0, text);
    }

    /// Horizontal text ending at `x_end`, baseline at `y`.
    pub fn text_right(&mut self, x_end: f64, y: f64, h: f64, text: &str) {
        let w = font::width(text, h);
        self.text([x_end - w, y], h, 0.0, text);
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.s.into_bytes()
    }

    pub fn as_str(&self) -> &str {
        &self.s
    }
}

/// A fraction in 0..1 with four decimals, trailing zeros dropped.
fn fmt_frac(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    match s {
        "" | "-0" => "0".into(),
        _ => s.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_are_integers_in_hundredths_of_a_millimetre() {
        let mut c = Content::new();
        c.width(0.25);
        c.line([20.0, 10.004], [200.0, 287.0]);
        assert_eq!(c.as_str(), "25 w\n2000 1000 m\n20000 28700 l\nS\n");
        let mut t = Content::new();
        t.text([10.0, 10.0], 2.5, 0.0, "0");
        assert!(t
            .as_str()
            .starts_with("BT /F1 338 Tf 1 0 0 1 1000 1000 Tm <"));
    }

    #[test]
    fn a_full_circle_is_four_beziers() {
        let mut c = Content::new();
        c.circle([0.0, 0.0], 10.0);
        assert_eq!(c.as_str().matches(" c\n").count(), 4);
        assert!(c.as_str().contains("1000 0 m"));
    }
}
