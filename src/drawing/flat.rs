//! One sheet per flat blank: the flat pattern seen from the outer skin,
//! running dimensions, hole and radius callouts, numbered bend lines with
//! a bend table, notes, and the title block's values. The sheet size and
//! the scale are the smallest sheet and the largest ISO 5455 scale on
//! which all of that fits.

use serde::Serialize;

use super::dims::{self, Layout, NARROW, WIDE};
use super::features::{features, Features};
use super::sheet::{self, Paper};
use super::text::{self, Lang};
use super::Meta;
use crate::dxf::blank::{Blank, Seg, P2};
use crate::pdf::{font, Content, Doc};

/// ISO 5455 scales, largest first.
const SCALES: [f64; 9] = [5.0, 2.0, 1.0, 0.5, 0.2, 0.1, 0.05, 0.02, 0.01];
const VIEW_TITLE_H: f64 = 9.0;
const TABLE_ROW: f64 = 5.0;
const NOTE_PITCH: f64 = 3.6;
const TABLE_COLS: [f64; 5] = [8.0, 14.0, 14.0, 12.0, 14.0];

/// What one sheet says about itself, for the judge and the CPQ.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetReport {
    /// 1-based position in the dossier.
    pub index: usize,
    pub label: String,
    pub title: String,
    pub part_ids: Vec<String>,
    pub quantity: usize,
    pub paper: String,
    pub scale: String,
    pub scale_factor: f64,
    /// Where the blank's origin sits on the sheet, in mm from the lower
    /// left corner.
    pub origin: [f64; 2],
    pub blank: [f64; 2],
    pub material: String,
    pub thickness: f64,
    pub dimensions: Vec<DimReport>,
    pub bends: Vec<BendReport>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DimReport {
    /// `ordinate`, `diameter` or `radius`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    pub value: f64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BendReport {
    pub nr: usize,
    pub id: String,
    pub angle_deg: f64,
    pub direction: String,
    pub inside_radius: f64,
    pub allowance: f64,
}

/// The values the title block and the outline need for one sheet.
pub struct SheetInfo<'a> {
    pub index: usize,
    pub total: usize,
    pub title: &'a str,
    pub part_ids: &'a [String],
    pub material: &'a str,
    pub thickness: f64,
}

pub struct Plan {
    pub paper: Paper,
    pub s: f64,
    pub origin: P2,
    pub layout: Layout,
    pub info_h: f64,
}

fn table_h(bends: usize) -> f64 {
    if bends == 0 {
        0.0
    } else {
        TABLE_ROW * (bends as f64 + 1.0) + 1.0
    }
}

fn notes(blank: &Blank, f: &Features, info: &SheetInfo, lang: Lang) -> Vec<String> {
    let mut v = Vec::new();
    let mut mat = String::new();
    if !info.material.is_empty() {
        mat.push_str(info.material);
    }
    if info.thickness > 0.0 {
        if !mat.is_empty() {
            mat.push_str(", ");
        }
        mat.push_str(&format!("t = {}", text::mm(info.thickness)));
    }
    if !mat.is_empty() {
        v.push(format!("{}: {mat}", lang.t("material")));
    }
    v.push(format!(
        "{}: {} × {}",
        lang.t("blank"),
        text::mm(blank.max[0]),
        text::mm(blank.max[1])
    ));
    v.push(lang.t("dim_note").to_string());
    if f.minor > 0 {
        v.push(lang.t("minor_note").to_string());
    }
    if !f.curves.is_empty() {
        v.push(lang.t("curve_note").to_string());
    }
    if blank.bends.is_empty() {
        v.push(lang.t("no_bends").to_string());
    } else {
        v.push(lang.t("bend_note").to_string());
    }
    v.push(format!("{}. {}.", lang.t("mm"), lang.t("deburr")));
    v.push(format!("DXF: {}.dxf", info.part_ids[0]));
    v
}

/// Choose sheet and scale.
pub fn plan(f: &Features, blank: &Blank, note_lines: usize, lang: Lang) -> Result<Plan, String> {
    let info_h = table_h(blank.bends.len()).max(note_lines as f64 * NOTE_PITCH + 2.0);
    for s in SCALES {
        for paper in Paper::ALL {
            let lo = dims::layout(f, s, lang);
            let [x0, y0, x1, y1] = paper.view_area(info_h);
            let need_w = f.max[0] * s + lo.margin[0] + lo.margin[2];
            let need_h = f.max[1] * s + lo.margin[1] + lo.margin[3] + VIEW_TITLE_H;
            let (aw, ah) = (x1 - x0, y1 - y0);
            if need_w <= aw && need_h <= ah {
                let ox = x0 + lo.margin[0] + (aw - need_w) / 2.0;
                let oy = y0 + lo.margin[1] + (ah - need_h) / 2.0;
                return Ok(Plan {
                    paper,
                    s,
                    origin: [ox, oy],
                    layout: lo,
                    info_h,
                });
            }
        }
    }
    Err(format!(
        "blank of {} × {} mm fits no sheet, not even A3 at 1:100",
        text::mm(f.max[0]),
        text::mm(f.max[1])
    ))
}

/// Draw the sheet into `doc` and describe it.
pub fn sheet(
    doc: &mut Doc,
    blank: &Blank,
    info: &SheetInfo,
    meta: &Meta,
    lang: Lang,
) -> Result<SheetReport, String> {
    let f = features(blank);
    let note_lines = notes(blank, &f, info, lang);
    let p = plan(&f, blank, note_lines.len(), lang)?;
    let paper = p.paper;
    let o = p.origin;
    let s = p.s;
    let mut c = Content::new();
    c.caps_butt_joins_round();
    c.fill_gray(0.0);

    // Title block values.
    let v = |c: &mut Content, key: &str, h: f64, t: &str| sheet::value(c, paper, key, h, t);
    v(&mut c, "title", 5.0, info.title);
    let id = if meta.number.is_empty() {
        format!("P-{:03}", info.index)
    } else {
        format!("{}-{:03}", meta.number, info.index)
    };
    v(&mut c, "id", 5.0, &id);
    v(&mut c, "created", 2.5, &meta.created_by);
    v(&mut c, "approved", 2.5, &meta.approved_by);
    v(&mut c, "date", 2.5, &meta.date);
    v(&mut c, "material", 2.5, info.material);
    if info.thickness > 0.0 {
        v(
            &mut c,
            "thickness",
            2.5,
            &format!("{} mm", text::mm(info.thickness)),
        );
    }
    v(&mut c, "quantity", 2.5, &format!("{}", info.part_ids.len()));
    v(&mut c, "doctype", 2.5, lang.t("detail"));
    v(&mut c, "status", 2.5, &meta.status);
    v(&mut c, "revision", 2.5, &meta.revision);
    let scale_text = text::scale(s);
    v(&mut c, "scale", 3.5, &scale_text);
    v(
        &mut c,
        "sheet",
        3.5,
        &format!("{} / {}", info.index, info.total),
    );
    v(&mut c, "format", 3.5, paper.name());
    v(&mut c, "tolerances", 2.5, &meta.tolerances);
    v(
        &mut c,
        "blank",
        3.5,
        &format!("{} × {}", text::mm(blank.max[0]), text::mm(blank.max[1])),
    );

    // Notes and bend table in the info strip.
    let (sx, sy, sw) = paper.info_strip();
    let th = table_h(blank.bends.len());
    let tw: f64 = TABLE_COLS.iter().sum();
    let note_w = if th > 0.0 { sw - tw - 4.0 } else { sw };
    let mut y = sy + p.info_h - 1.0 - 2.5;
    for line in &note_lines {
        let (t, h) = sheet::shrink_to(line, 2.5, note_w - 2.0);
        c.text([sx + 1.0, y], h, 0.0, &t);
        y -= NOTE_PITCH;
    }
    let mut bend_reports = Vec::new();
    if th > 0.0 {
        let tx = sx + sw - tw;
        let ty = sy + p.info_h - th;
        c.width(NARROW);
        c.rect_stroke(tx, ty, tw, th - 1.0);
        let heads = ["bend", "angle", "direction", "inside_r", "allowance"];
        let mut row_y = ty + th - 1.0 - TABLE_ROW;
        let mut x = tx;
        c.line([tx, row_y], [tx + tw, row_y]);
        for (i, k) in heads.iter().enumerate() {
            let (t, h) = sheet::shrink_to(lang.t(k), 2.5, TABLE_COLS[i] - 1.5);
            c.text([x + 0.8, row_y + 1.5], h, 0.0, &t);
            x += TABLE_COLS[i];
            if i + 1 < heads.len() {
                c.line([x, ty], [x, ty + th - 1.0]);
            }
        }
        for (i, b) in blank.bends.iter().enumerate() {
            row_y -= TABLE_ROW;
            if i + 1 < blank.bends.len() {
                c.line([tx, row_y], [tx + tw, row_y]);
            }
            let dir = match b.direction {
                crate::view::BendDirection::Up => lang.t("up"),
                crate::view::BendDirection::Down => lang.t("down"),
            };
            let cells = [
                (i + 1).to_string(),
                text::deg(b.angle_deg),
                dir.to_string(),
                text::mm(b.inside_radius),
                text::mm(b.allowance),
            ];
            let mut x = tx;
            for (j, t) in cells.iter().enumerate() {
                c.text([x + 0.8, row_y + 1.5], 2.5, 0.0, t);
                x += TABLE_COLS[j];
            }
            bend_reports.push(BendReport {
                nr: i + 1,
                id: b.id.clone(),
                angle_deg: b.angle_deg,
                direction: b.direction.label().to_lowercase(),
                inside_radius: b.inside_radius,
                allowance: b.allowance,
            });
        }
    }

    // View title, below the centring mark that reaches 5 mm into the frame.
    let [ax0, _, _, ay1] = paper.view_area(p.info_h);
    c.text([ax0 + 1.0, ay1 - 7.5], 5.0, 0.0, lang.t("flat"));
    c.text(
        [
            ax0 + 1.0 + font::width(lang.t("flat"), 5.0) + 3.0,
            ay1 - 7.5,
        ],
        2.5,
        0.0,
        lang.t("flat_sub"),
    );

    // The outline, wide.
    c.width(WIDE);
    let pm = |pt: P2| [o[0] + pt[0] * s, o[1] + pt[1] * s];
    for lp in &f.loops {
        draw_loop(&mut c, lp, &pm, s);
    }
    // Dimensions, callouts, bend lines and tags, narrow.
    dims::draw(&p.layout, &f, &mut c, o);

    let (w, h) = paper.size();
    let label = format!("Pos {}", info.index);
    doc.add_page(w, h, &[paper.form_name()], c, &label);

    let mut dimensions = Vec::new();
    for (axis, labels) in [("x", &p.layout.xs), ("y", &p.layout.ys)] {
        for l in labels {
            dimensions.push(DimReport {
                kind: "ordinate".into(),
                axis: Some(axis.into()),
                value: l.value,
                text: l.text.clone(),
                count: None,
                sources: l.sources.clone(),
            });
        }
    }
    for co in &p.layout.callouts {
        dimensions.push(DimReport {
            kind: co.kind.into(),
            axis: None,
            value: co.value,
            text: co.text.clone(),
            count: Some(co.count),
            sources: vec![],
        });
    }
    Ok(SheetReport {
        index: info.index,
        label,
        title: info.title.to_string(),
        part_ids: info.part_ids.to_vec(),
        quantity: info.part_ids.len(),
        paper: paper.name().into(),
        scale: scale_text,
        scale_factor: s,
        origin: o,
        blank: blank.max,
        material: info.material.to_string(),
        thickness: info.thickness,
        dimensions,
        bends: bend_reports,
    })
}

/// One closed loop as a single path, so corners join cleanly.
fn draw_loop(c: &mut Content, lp: &[Seg], pm: &impl Fn(P2) -> P2, s: f64) {
    let Some(first) = lp.first() else {
        return;
    };
    if let Seg::Circle { c: cc, r } = first {
        c.circle(pm(*cc), r * s);
        return;
    }
    let (start, _) = super::features::ends(first);
    c.move_to(pm(start));
    let mut cur = start;
    for seg in lp {
        let (a, b) = super::features::ends(seg);
        // Loops run one way, but an arc is stored counter-clockwise: enter
        // it at whichever end we stand on.
        let forward = (a[0] - cur[0]).abs() + (a[1] - cur[1]).abs()
            <= (b[0] - cur[0]).abs() + (b[1] - cur[1]).abs();
        match *seg {
            Seg::Line { a, b } => {
                let next = if forward { b } else { a };
                c.line_to(pm(next));
                cur = next;
            }
            Seg::Arc {
                c: cc,
                r,
                start,
                end,
            } => {
                let pc = pm(cc);
                if forward {
                    c.arc_segments(pc, r * s, start, end);
                    cur = b;
                } else {
                    // Clockwise from end to start: draw the pieces reversed.
                    arc_reverse(c, pc, r * s, start, end);
                    cur = a;
                }
            }
            Seg::Circle { .. } => {}
        }
    }
    c.close();
    c.stroke();
}

/// Append the arc from `end` back to `start` (clockwise) as Béziers.
fn arc_reverse(c: &mut Content, centre: P2, r: f64, start: f64, end: f64) {
    let sweep = (end - start).rem_euclid(360.0);
    let sweep = if sweep == 0.0 { 360.0 } else { sweep };
    let pieces = (sweep / 90.0).ceil().max(1.0) as usize;
    let step = sweep / pieces as f64;
    let mut a0 = end.to_radians();
    for _ in 0..pieces {
        let a1 = a0 - step.to_radians();
        let k = 4.0 / 3.0 * ((a0 - a1) / 4.0).tan();
        let (s0, c0) = a0.sin_cos();
        let (s1, c1) = a1.sin_cos();
        let p1 = [centre[0] + r * (c0 + k * s0), centre[1] + r * (s0 - k * c0)];
        let p2 = [centre[0] + r * (c1 - k * s1), centre[1] + r * (s1 + k * c1)];
        let p = [centre[0] + r * c1, centre[1] + r * s1];
        c.curve_to(p1, p2, p);
        a0 = a1;
    }
}
