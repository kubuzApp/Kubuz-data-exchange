//! The sheet around a drawing: ISO 5457 sizes, margins, frame, centring
//! marks (no trimming marks), a grid reference (numbers along the top and bottom,
//! letters down the sides), and a title block of 180 × 48 mm in the
//! Kubuz layout: who drew and approved it and when, scale, size and
//! projection symbol, material, thickness, weight and folded extents, the
//! part's number and description, the customer, the owner's address and
//! the ownership line. The captions are drawn once per format (in a Form
//! XObject); the values each page writes itself.

use super::logo::Logo;
use super::text::Lang;
use crate::pdf::{font, Content};

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
/// The four caption rows at the top, the big row, the ownership line.
const SUB: f64 = 5.0;
const ROW_A: f64 = 4.0 * SUB;
const ROW_C: f64 = 4.0;
const ROW_B: f64 = TB_H - ROW_A - ROW_C;
/// Column edges of the top row: captions left, scale, size, projection
/// symbol, captions right.
const COLS: [f64; 6] = [0.0, 62.0, 84.0, 104.0, 128.0, TB_W];
const LABEL_H: f64 = 2.2;
/// Where a value starts after its caption, in the left and right columns.
const VALUE_X: [f64; 2] = [23.0, 19.0];

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
        // The top centring mark reaches 5 mm into the frame.
        [x0 + 2.0, bottom + 3.0, x1 - 2.0, y1 - 6.0]
    }
}

/// The title block's cells: caption key, lower-left corner relative to
/// the block, width, height, and where the value starts (relative to the
/// cell) with its height.
struct Cell {
    key: &'static str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    /// Caption at the cell's top left when true; else no caption.
    caption: bool,
    /// Value origin (relative), height, centred when `centre`.
    vx: f64,
    vy: f64,
    vh: f64,
    centre: bool,
}

fn cells() -> Vec<Cell> {
    let mut v = Vec::new();
    let left = ["designer", "design_date", "approved", "approved_date"];
    let right = ["material", "thickness", "weight", "extents"];
    for (i, k) in left.iter().enumerate() {
        v.push(Cell {
            key: k,
            x: COLS[0],
            y: TB_H - SUB * (i as f64 + 1.0),
            w: COLS[1] - COLS[0],
            h: SUB,
            caption: true,
            vx: VALUE_X[0],
            vy: 1.4,
            vh: 2.5,
            centre: false,
        });
        v.push(Cell {
            key: right[i],
            x: COLS[4],
            y: TB_H - SUB * (i as f64 + 1.0),
            w: COLS[5] - COLS[4],
            h: SUB,
            caption: true,
            vx: VALUE_X[1],
            vy: 1.4,
            vh: 2.5,
            centre: false,
        });
    }
    v.push(Cell {
        key: "scale",
        x: COLS[1],
        y: TB_H - ROW_A,
        w: COLS[2] - COLS[1],
        h: ROW_A,
        caption: true,
        vx: (COLS[2] - COLS[1]) / 2.0,
        vy: 6.0,
        vh: 4.0,
        centre: true,
    });
    v.push(Cell {
        key: "size",
        x: COLS[2],
        y: TB_H - ROW_A,
        w: COLS[3] - COLS[2],
        h: ROW_A,
        caption: true,
        vx: (COLS[3] - COLS[2]) / 2.0,
        vy: 6.0,
        vh: 4.0,
        centre: true,
    });
    v.push(Cell {
        key: "projection",
        x: COLS[3],
        y: TB_H - ROW_A,
        w: COLS[4] - COLS[3],
        h: ROW_A,
        caption: false,
        vx: 0.0,
        vy: 0.0,
        vh: 0.0,
        centre: false,
    });
    // The big row: id, description and customer left; the owner right.
    v.push(Cell {
        key: "id",
        x: COLS[0],
        y: ROW_C,
        w: COLS[4],
        h: ROW_B,
        caption: false,
        vx: COLS[4] / 2.0,
        vy: ROW_B - 7.0,
        vh: 5.0,
        centre: true,
    });
    v.push(Cell {
        key: "owner",
        x: COLS[4],
        y: ROW_C,
        w: COLS[5] - COLS[4],
        h: ROW_B,
        caption: false,
        vx: (COLS[5] - COLS[4]) / 2.0,
        vy: ROW_B - 5.5,
        vh: 3.0,
        centre: true,
    });
    v.push(Cell {
        key: "property",
        x: 0.0,
        y: 0.0,
        w: TB_W,
        h: ROW_C,
        caption: false,
        vx: TB_W / 2.0,
        vy: 1.2,
        vh: 1.8,
        centre: true,
    });
    v
}

fn cell(key: &str) -> Cell {
    cells()
        .into_iter()
        .find(|c| c.key == key)
        .unwrap_or_else(|| panic!("no title block cell {key}"))
}

/// The values one page writes into its title block.
#[derive(Clone, Debug, Default)]
pub struct TitleValues {
    pub id: String,
    pub description: String,
    pub customer: String,
    pub quantity: usize,
    pub sheet: usize,
    pub sheets: usize,
    pub designer: String,
    pub date: String,
    pub approved: String,
    pub approved_date: String,
    pub scale: String,
    pub material: String,
    pub thickness: String,
    pub weight: String,
    pub extents: String,
}

/// Where a captioned cell's value starts: after its caption in `lang`,
/// never before the column's usual value position.
fn value_x(cell: &Cell, lang: Lang) -> f64 {
    if !cell.caption || cell.centre {
        return cell.vx;
    }
    let caption = font::width(&format!("{}:", lang.t(cell.key)), LABEL_H);
    cell.vx.max(1.0 + caption + 1.5)
}

/// Write the values of one page.
pub fn title_values(c: &mut Content, paper: Paper, lang: Lang, v: &TitleValues) {
    let (bx, by) = paper.title_block();
    let put = |c: &mut Content, key: &str, text: &str| {
        let cell = cell(key);
        let vx = value_x(&cell, lang);
        let room = if cell.centre {
            cell.w - 2.0
        } else {
            cell.w - vx - 1.0
        };
        let (t, h) = shrink_to(text, cell.vh, room);
        let x = bx + cell.x + vx;
        let y = by + cell.y + cell.vy;
        if cell.centre {
            c.text_centred(x, y, h, &t);
        } else {
            c.text([x, y], h, 0.0, &t);
        }
    };
    put(c, "designer", &v.designer);
    put(c, "design_date", &v.date);
    put(c, "approved", &v.approved);
    put(c, "approved_date", &v.approved_date);
    put(c, "scale", &v.scale);
    put(c, "size", paper.name());
    put(c, "material", &v.material);
    put(c, "thickness", &v.thickness);
    put(c, "weight", &v.weight);
    put(c, "extents", &v.extents);
    put(c, "id", &v.id);
    // Description under the id, customer and count at the bottom.
    let id = cell("id");
    let (t, h) = shrink_to(&v.description, 3.0, id.w - 2.0);
    c.text_centred(bx + id.x + id.w / 2.0, by + id.y + id.vy - 5.5, h, &t);
    let (t, h) = shrink_to(&v.customer, 3.0, id.w * 0.6);
    c.text([bx + id.x + 1.5, by + id.y + 1.6], h, 0.0, &t);
    let count = if v.quantity > 0 {
        format!(
            "{} {} · {} {}/{}",
            lang.t("quantity"),
            v.quantity,
            lang.t("sheet"),
            v.sheet,
            v.sheets
        )
    } else {
        format!("{} {}/{}", lang.t("sheet"), v.sheet, v.sheets)
    };
    c.text_right(bx + id.x + id.w - 1.5, by + id.y + 1.6, 2.2, &count);
}

/// The id cell of the title block on the page, `[x0, y0, x1, y1]`: where
/// a part sheet links back to the parts list.
pub fn id_rect(paper: Paper) -> [f64; 4] {
    let (bx, by) = paper.title_block();
    let id = cell("id");
    [bx + id.x, by + id.y, bx + id.x + id.w, by + id.y + id.h]
}

/// The Form XObject of one format: frame, marks, grid reference, title
/// block grid and captions, owner block, logo and projection symbol.
pub fn frame_form(
    paper: Paper,
    lang: Lang,
    logo: Option<&Logo>,
    owner: &str,
    address: &[String],
) -> Result<Content, String> {
    let mut c = Content::new();
    let (w, h) = paper.size();
    let [x0, y0, x1, y1] = paper.frame();
    c.caps_butt_joins_round();
    c.fill_gray(0.0);

    // No trimming marks: sheets print on their own format, nothing to trim.
    // Centring marks, 0,7 mm, from the sheet edge to 5 mm inside the frame.
    c.width(0.7);
    // The bottom mark stops at the frame where the title block is.
    let bottom_in = if paper.title_block().0 <= w / 2.0 {
        0.0
    } else {
        5.0
    };
    c.line([w / 2.0, 0.0], [w / 2.0, y0 + bottom_in]);
    c.line([w / 2.0, h], [w / 2.0, y1 - 5.0]);
    c.line([0.0, h / 2.0], [x0 + 5.0, h / 2.0]);
    c.line([w, h / 2.0], [x1 - 5.0, h / 2.0]);
    // The frame.
    c.rect_stroke(x0, y0, x1 - x0, y1 - y0);

    // Grid reference (ISO 5457): 50 mm fields counted from the centring
    // marks, numbers along the top and bottom, letters down the sides.
    c.width(0.35);
    let gx0 = x0 - 5.0;
    let gx1 = x1 + 5.0;
    let gy0 = y0 - 5.0;
    let gy1 = y1 + 5.0;
    c.rect_stroke(gx0, gy0, gx1 - gx0, gy1 - gy0);
    let fields = |lo: f64, hi: f64, mid: f64| -> Vec<f64> {
        let mut v = vec![lo];
        let n = ((mid - lo) / 50.0).ceil() as i64;
        let mut x = mid - 50.0 * n as f64;
        while x < hi - 1.0 {
            if x > lo + 1.0 {
                v.push(x);
            }
            x += 50.0;
        }
        v.push(hi);
        v
    };
    let xs = fields(gx0, gx1, w / 2.0);
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
    let ys = fields(gy0, gy1, h / 2.0);
    for yv in &ys[1..ys.len() - 1] {
        c.line([gx0, *yv], [x0, *yv]);
        c.line([x1, *yv], [gx1, *yv]);
    }
    let letters = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K'];
    for (i, pair) in ys.iter().rev().collect::<Vec<_>>().windows(2).enumerate() {
        let mid = (pair[0] + pair[1]) / 2.0;
        let s = letters[i.min(letters.len() - 1)].to_string();
        c.text_centred(gx0 - 2.5, mid - 1.75, 3.5, &s);
        c.text_centred(gx1 + 2.5, mid - 1.75, 3.5, &s);
    }

    // Title block grid and captions.
    let (bx, by) = paper.title_block();
    c.width(0.5);
    c.rect_stroke(bx, by, TB_W, TB_H);
    c.width(0.25);
    for cell in cells() {
        c.rect_stroke(bx + cell.x, by + cell.y, cell.w, cell.h);
        if cell.caption {
            c.text(
                [bx + cell.x + 1.0, by + cell.y + cell.h - LABEL_H - 1.0],
                LABEL_H,
                0.0,
                &format!("{}:", lang.t(cell.key)),
            );
        }
    }
    // Owner block: logo above the name when there is one, address under.
    let o = cell("owner");
    let (ox, oy) = (bx + o.x, by + o.y);
    let mut lines: Vec<&str> = vec![owner];
    lines.extend(address.iter().map(|s| s.as_str()));
    let lh = 2.2;
    let block_h = 3.0 + lh * (lines.len() as f64 - 1.0) + 1.0;
    let mut top = oy + o.h;
    if let Some(l) = logo {
        let lh_logo = (o.h - block_h - 2.0).max(5.0);
        super::logo::draw(l, &mut c, ox + 4.0, top - lh_logo - 1.0, o.w - 8.0, lh_logo)?;
        top -= lh_logo + 1.5;
    } else {
        top -= (o.h - block_h) / 2.0;
    }
    let mut y = top - 3.0;
    for (i, line) in lines.iter().enumerate() {
        let h = if i == 0 { 3.0 } else { lh };
        let (t, h) = shrink_to(line, h, o.w - 2.0);
        c.text_centred(ox + o.w / 2.0, y, h, &t);
        y -= if i == 0 { lh + 1.0 } else { lh + 0.6 };
    }
    // Ownership line.
    let p = cell("property");
    let note = lang.t("property").replace("{}", owner);
    let (t, h) = shrink_to(&note, p.vh, p.w - 2.0);
    c.text_centred(bx + p.x + p.w / 2.0, by + p.y + p.vy, h, &t);

    // First-angle projection symbol (ISO 5456-2): the truncated cone's
    // side view with its large end left, the circles to its right.
    let pr = cell("projection");
    let base = by + pr.y + pr.h / 2.0 - 2.5;
    let sx = bx + pr.x + pr.w / 2.0 - 8.0;
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

/// `text` at height `h`, or smaller (never under 1,8 mm), or cut with
/// three dots, so it fits `w` mm.
pub fn shrink_to(text: &str, h: f64, w: f64) -> (String, f64) {
    let width = font::width;
    let mut h = h;
    while width(text, h) > w && h > 1.8 {
        h = (h - 0.25).max(1.8);
    }
    if width(text, h) <= w {
        return (text.to_string(), h);
    }
    let mut s: String = text.to_string();
    // The font subset has no ellipsis glyph; three dots read the same.
    while !s.is_empty() && width(&format!("{s}..."), h) > w {
        s.pop();
    }
    (format!("{}...", s.trim_end()), h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_tile_the_title_block() {
        for p in Paper::ALL {
            let mut area = 0.0;
            for c in cells() {
                assert!(c.x >= -1e-9 && c.x + c.w <= TB_W + 1e-9, "{}", c.key);
                assert!(c.y >= -1e-9 && c.y + c.h <= TB_H + 1e-9, "{}", c.key);
                area += c.w * c.h;
            }
            assert!((area - TB_W * TB_H).abs() < 1e-6);
            let f = frame_form(p, Lang::Nl, None, "Kubuz", &["Bosbeek 56".into()]).unwrap();
            assert!(f.as_str().contains(" Tj ET"));
            let mut c = Content::new();
            title_values(
                &mut c,
                p,
                Lang::Nl,
                &TitleValues {
                    id: "Top".into(),
                    quantity: 2,
                    sheet: 1,
                    sheets: 3,
                    ..TitleValues::default()
                },
            );
            assert!(c.as_str().contains("Tj"));
        }
    }

    #[test]
    fn long_values_shrink_then_truncate() {
        let (t, h) = shrink_to("1.4301 (X5CrNi18-10) EN 10088-2 2B", 3.5, 28.0);
        assert!(h < 3.5);
        assert!(t.ends_with("...") || t.len() == 34);
        assert_eq!(shrink_to("A4", 3.5, 28.0), ("A4".to_string(), 3.5));
        let (cut, _) = shrink_to("mdf_moisture_resistant_board_of_eighteen", 2.5, 20.0);
        assert!(cut.ends_with("..."), "{cut}");
        assert!(cut.chars().all(font::has), "{cut}");
    }

    #[test]
    fn values_start_after_their_caption_in_every_language() {
        for lang in [Lang::Nl, Lang::En, Lang::Fr] {
            for cell in cells().iter().filter(|c| c.caption && !c.centre) {
                let caption = font::width(&format!("{}:", lang.t(cell.key)), LABEL_H);
                assert!(
                    value_x(cell, lang) >= 1.0 + caption + 1.0,
                    "{} in {lang:?}",
                    cell.key
                );
            }
        }
        let nl = cell("approved_date");
        assert!(value_x(&nl, Lang::Nl) > VALUE_X[0]);
    }
}
