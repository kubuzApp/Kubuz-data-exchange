//! The sheet around a drawing: ISO 5457 sizes, margins, frame, centring
//! and trimming marks, and an ISO 7200 title block of 180 mm whose captions
//! are drawn once per format (in a Form XObject) and whose values each
//! page writes itself.

use super::logo::Logo;
use super::text::Lang;
use crate::pdf::Content;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paper {
    /// Portrait, the only way ISO 5457 allows it.
    A4,
    /// Landscape.
    A3,
}

/// Title block size.
pub const TB_W: f64 = 180.0;
pub const TB_H: f64 = 48.0;
const ROW: f64 = 12.0;
const LABEL_H: f64 = 1.8;

impl Paper {
    pub const ALL: [Paper; 2] = [Paper::A4, Paper::A3];

    pub fn name(self) -> &'static str {
        match self {
            Paper::A4 => "A4",
            Paper::A3 => "A3",
        }
    }

    pub fn form_name(self) -> &'static str {
        match self {
            Paper::A4 => "FrA4",
            Paper::A3 => "FrA3",
        }
    }

    pub fn size(self) -> (f64, f64) {
        match self {
            Paper::A4 => (210.0, 297.0),
            Paper::A3 => (420.0, 297.0),
        }
    }

    /// The frame: left margin 20 (filing), the rest 10.
    pub fn frame(self) -> [f64; 4] {
        let (w, h) = self.size();
        [20.0, 10.0, w - 10.0, h - 10.0]
    }

    /// Lower-left corner of the title block.
    pub fn title_block(self) -> (f64, f64) {
        let [_, y0, x1, _] = self.frame();
        (x1 - TB_W, y0)
    }

    /// The strip for the bend table and notes: above the title block on
    /// A4, beside it on A3. Returns its lower-left corner and width.
    pub fn info_strip(self) -> (f64, f64, f64) {
        let [x0, y0, x1, _] = self.frame();
        match self {
            Paper::A4 => (x0, y0 + TB_H + 2.0, x1 - x0),
            Paper::A3 => (x0, y0, x1 - x0 - TB_W - 4.0),
        }
    }

    /// Where the views go once the info strip is `info_h` tall:
    /// `[x0, y0, x1, y1]`.
    pub fn view_area(self, info_h: f64) -> [f64; 4] {
        let [x0, _, x1, y1] = self.frame();
        let (_, sy, _) = self.info_strip();
        let bottom = match self {
            Paper::A4 => sy + info_h,
            Paper::A3 => sy + info_h.max(TB_H),
        };
        [x0 + 2.0, bottom + 3.0, x1 - 2.0, y1 - 2.0]
    }
}

/// What the title block's cells are, in reading order: (label key, column
/// start, width, row) with rows counted from the top of the block.
fn cells() -> Vec<(&'static str, f64, f64, usize)> {
    let mut v = vec![
        ("owner", 0.0, 60.0, 0),
        ("title", 60.0, 60.0, 0),
        ("id", 120.0, 60.0, 0),
    ];
    for (i, k) in [
        "created",
        "approved",
        "date",
        "material",
        "thickness",
        "quantity",
    ]
    .iter()
    .enumerate()
    {
        v.push((k, 30.0 * i as f64, 30.0, 1));
    }
    for (i, k) in ["doctype", "status", "revision", "scale", "sheet", "format"]
        .iter()
        .enumerate()
    {
        v.push((k, 30.0 * i as f64, 30.0, 2));
    }
    v.push(("tolerances", 0.0, 90.0, 3));
    v.push(("projection", 90.0, 30.0, 3));
    v.push(("blank", 120.0, 60.0, 3));
    v
}

/// The cell's lower-left corner and size on the sheet.
pub fn cell(paper: Paper, key: &str) -> (f64, f64, f64, f64) {
    let (bx, by) = paper.title_block();
    for (k, x, w, row) in cells() {
        if k == key {
            return (bx + x, by + TB_H - ROW * (row as f64 + 1.0), w, ROW);
        }
    }
    panic!("no title block cell {key}")
}

/// The Form XObject of one format: frame, marks, title block grid and
/// captions, logo and projection symbol.
pub fn frame_form(
    paper: Paper,
    lang: Lang,
    logo: Option<&Logo>,
    owner: &str,
) -> Result<Content, String> {
    let mut c = Content::new();
    let (w, h) = paper.size();
    let [x0, y0, x1, y1] = paper.frame();
    c.caps_butt_joins_round();
    c.fill_gray(0.0);

    // Trimming marks: two overlapping 10 × 5 rectangles in each corner.
    for (cx, sx) in [(0.0, 1.0), (w, -1.0)] {
        for (cy, sy) in [(0.0, 1.0), (h, -1.0)] {
            c.rect_fill(cx.min(cx + sx * 10.0), cy.min(cy + sy * 5.0), 10.0, 5.0);
            c.rect_fill(cx.min(cx + sx * 5.0), cy.min(cy + sy * 10.0), 5.0, 10.0);
        }
    }
    // Centring marks, 0,7 mm, from the sheet edge to 5 mm inside the frame.
    c.width(0.7);
    c.line([w / 2.0, 0.0], [w / 2.0, y0 + 5.0]);
    c.line([w / 2.0, h], [w / 2.0, y1 - 5.0]);
    c.line([0.0, h / 2.0], [x0 + 5.0, h / 2.0]);
    c.line([w, h / 2.0], [x1 - 5.0, h / 2.0]);
    // The frame.
    c.rect_stroke(x0, y0, x1 - x0, y1 - y0);

    // Grid reference on A3: 50 mm fields from the centring marks, letters
    // down the sides and numbers along the top and bottom.
    if paper == Paper::A3 {
        c.width(0.35);
        let gx0 = x0 - 5.0;
        let gx1 = x1 + 5.0;
        let gy0 = y0 - 5.0;
        let gy1 = y1 + 5.0;
        c.rect_stroke(gx0, gy0, gx1 - gx0, gy1 - gy0);
        let mut xs: Vec<f64> = vec![gx0];
        let mut x = w / 2.0 - 150.0;
        while x < gx1 - 1.0 {
            if x > gx0 + 1.0 {
                xs.push(x);
            }
            x += 50.0;
        }
        xs.push(gx1);
        for xv in &xs[1..xs.len() - 1] {
            c.line([*xv, gy0], [*xv, y0]);
            c.line([*xv, y1], [*xv, gy1]);
        }
        for (i, pair) in xs.windows(2).enumerate() {
            let mid = (pair[0] + pair[1]) / 2.0;
            let n = (i + 1).to_string();
            c.text_centred(mid, gy1 + 0.8, 3.5, &n);
            c.text_centred(mid, gy0 - 4.3, 3.5, &n);
        }
        let mut ys: Vec<f64> = vec![gy1];
        let mut y = h / 2.0 + 100.0;
        while y > gy0 + 1.0 {
            if y < gy1 - 1.0 {
                ys.push(y);
            }
            y -= 50.0;
        }
        ys.push(gy0);
        for yv in &ys[1..ys.len() - 1] {
            c.line([gx0, *yv], [x0, *yv]);
            c.line([x1, *yv], [gx1, *yv]);
        }
        let letters = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K'];
        for (i, pair) in ys.windows(2).enumerate() {
            let mid = (pair[0] + pair[1]) / 2.0;
            let s = letters[i.min(letters.len() - 1)].to_string();
            c.text_centred(gx0 - 2.5, mid - 1.75, 3.5, &s);
            c.text_centred(gx1 + 2.5, mid - 1.75, 3.5, &s);
        }
    }

    // Title block grid.
    let (bx, by) = paper.title_block();
    c.width(0.5);
    c.rect_stroke(bx, by, TB_W, TB_H);
    c.width(0.25);
    for (key, x, w, row) in cells() {
        let y = by + TB_H - ROW * (row as f64 + 1.0);
        c.rect_stroke(bx + x, y, w, ROW);
        c.text([bx + x + 1.0, y + ROW - 2.8], LABEL_H, 0.0, lang.t(key));
    }
    // Owner: logo or name.
    let (ox, oy, ow, oh) = cell(paper, "owner");
    match logo {
        Some(l) => super::logo::draw(l, &mut c, ox + 2.0, oy + 1.0, ow - 4.0, oh - 5.0)?,
        None => c.text([ox + 1.5, oy + 2.5], 5.0, 0.0, owner),
    }
    // First-angle projection symbol (ISO 5456-2): the truncated cone's
    // side view with its large end left, the circles to its right.
    let (px, py, pw, _) = cell(paper, "projection");
    let base = py + 2.0;
    let sx = px + pw / 2.0 - 8.0;
    c.width(0.35);
    c.move_to([sx, base]);
    c.line_to([sx, base + 5.0]);
    c.line_to([sx + 6.0, base + 3.75]);
    c.line_to([sx + 6.0, base + 1.25]);
    c.close();
    c.stroke();
    let cc = [sx + 11.5, base + 2.5];
    c.circle(cc, 2.5);
    c.circle(cc, 1.25);
    c.width(0.18);
    c.dash(&[4.0, 0.75, 0.0, 0.75], 0.0);
    c.caps_round();
    c.line([sx - 1.5, base + 2.5], [cc[0] + 4.0, base + 2.5]);
    c.solid();
    c.caps_butt_joins_round();
    Ok(c)
}

/// Write `value` into the title block cell `key`, with capitals `h` tall.
pub fn value(c: &mut Content, paper: Paper, key: &str, h: f64, text: &str) {
    let (x, y, w, _) = cell(paper, key);
    let fit = shrink_to(text, h, w - 2.0);
    c.text([x + 1.0, y + 2.5], fit.1, 0.0, &fit.0);
}

/// `text` at height `h`, or smaller (never under 1,8 mm), or cut with an
/// ellipsis, so it fits `w` mm.
pub fn shrink_to(text: &str, h: f64, w: f64) -> (String, f64) {
    use crate::pdf::font::width;
    let mut h = h;
    while width(text, h) > w && h > 1.8 {
        h = (h - 0.25).max(1.8);
    }
    if width(text, h) <= w {
        return (text.to_string(), h);
    }
    let mut s: String = text.to_string();
    while !s.is_empty() && width(&format!("{s}…"), h) > w {
        s.pop();
    }
    (format!("{s}…"), h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_tile_the_title_block() {
        for p in Paper::ALL {
            let (bx, by) = p.title_block();
            let mut area = 0.0;
            for (k, _, _, _) in cells() {
                let (x, y, w, h) = cell(p, k);
                assert!(x >= bx - 1e-9 && x + w <= bx + TB_W + 1e-9, "{k}");
                assert!(y >= by - 1e-9 && y + h <= by + TB_H + 1e-9, "{k}");
                area += w * h;
            }
            assert!((area - TB_W * TB_H).abs() < 1e-6);
            let f = frame_form(p, Lang::Nl, None, "Kubuz").unwrap();
            assert!(f.as_str().contains(" Tj ET"));
        }
    }

    #[test]
    fn long_values_shrink_then_truncate() {
        let (t, h) = shrink_to("1.4301 (X5CrNi18-10) EN 10088-2 2B", 3.5, 28.0);
        assert!(h < 3.5);
        assert!(t.ends_with('…') || t.len() == 34);
        assert_eq!(shrink_to("A4", 3.5, 28.0), ("A4".to_string(), 3.5));
    }
}
