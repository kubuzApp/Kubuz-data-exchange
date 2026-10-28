//! One sheet per flat blank: the flat pattern seen from the outer skin as
//! the main view, with chain dimensions, hole and radius callouts, bend
//! lines with their notes and numbered tags; one section per direction of
//! bend lines showing the folded profile with its flange lengths and
//! angles; the bend table and the notes along the bottom; the title
//! block's values. The sheet size and the scales are chosen so that the
//! flat pattern and the sections together are as large as the sheet
//! allows.

use serde::Serialize;

use super::dims::{self, Layout, NARROW, WIDE};
use super::features::{features, Features};
use super::section::{self, Piece, Profile, SectionPlan};
use super::sheet::{self, Paper, TitleValues};
use super::text::{self, Lang};
use super::Meta;
use crate::dxf::blank::{Blank, Seg, P2};
use crate::pdf::{font, Content, Doc};

/// ISO 5455 scales, largest first.
const SCALES: [f64; 9] = [5.0, 2.0, 1.0, 0.5, 0.2, 0.1, 0.05, 0.02, 0.01];
const VIEW_TITLE_H: f64 = 8.0;
const SECTION_TITLE_H: f64 = 7.0;
const TABLE_ROW: f64 = 5.0;
const NOTE_PITCH: f64 = 3.6;
const TABLE_COLS: [f64; 5] = [8.0, 14.0, 14.0, 12.0, 14.0];
/// Between the flat pattern's box and a section's.
const GAP: f64 = 6.0;

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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub chains: Vec<ChainReport>,
    pub bends: Vec<BendReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<SectionReport>,
    /// Folded extents L × W × H along the base flange's axes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extents: Option<[f64; 3]>,
    /// `flat` or `folded`.
    pub kind: String,
    /// Overall size of a folded part along the front view's frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<super::folded::ViewReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minor_bound: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DimReport {
    /// `diameter`, `radius`, `ordinate`, `flange`, `width`, `height` or
    /// `angle`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    pub value: f64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    pub sources: Vec<String>,
    /// The section's letter for a section's dimension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
}

/// One chain of dimensions: along `direction`, from the first station to
/// the last, each value the distance between two consecutive stations.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainReport {
    /// `bend`, `outline`, `hole` or `overall`.
    pub kind: String,
    /// `x`, `y` or `oblique`.
    pub axis: String,
    pub direction: [f64; 2],
    /// `-1`: below or left of the blank; `1`: above or right.
    pub side: f64,
    pub stations: Vec<StationReport>,
    pub values: Vec<f64>,
    pub texts: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StationReport {
    /// Model mm along the chain's direction.
    pub t: f64,
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
    /// The note written along the line, when it fit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionReport {
    pub letter: String,
    /// The cut on the blank (model), entry to exit.
    pub cut: [[f64; 2]; 2],
    pub direction: [f64; 2],
    pub bends: Vec<usize>,
    pub pieces: Vec<PieceReport>,
    pub scale: String,
    pub scale_factor: f64,
    /// Paper position of the walk's origin (the cut's entry, outer skin).
    pub origin: [f64; 2],
    pub width: f64,
    pub height: f64,
    pub dimensions: Vec<DimReport>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PieceReport {
    /// `flat` or `bend`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
    /// The drawn length of an interrupted flange.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shown: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bend: Option<usize>,
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

/// One placed section.
pub struct Placed {
    pub profile: Profile,
    pub plan: SectionPlan,
    /// Paper position of the walk's origin.
    pub origin: P2,
}

pub struct Plan {
    pub paper: Paper,
    pub s: f64,
    pub origin: P2,
    pub layout: Layout,
    pub info_h: f64,
    pub sections: Vec<Placed>,
}

fn table_h(bends: usize) -> f64 {
    if bends == 0 {
        0.0
    } else {
        TABLE_ROW * (bends as f64 + 1.0) + 1.0
    }
}

fn notes(blank: &Blank, f: &Features, info: &SheetInfo, lang: Lang, sections: bool) -> Vec<String> {
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
    v.push(lang.t("chain_note").to_string());
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
    if sections {
        v.push(lang.t("section_note").to_string());
    }
    v.push(format!("{}. {}.", lang.t("mm"), lang.t("deburr")));
    v.push(format!("DXF: {}.dxf", info.part_ids[0]));
    v
}

/// Choose sheet and scales: the largest scale for the flat pattern first,
/// then the largest scales for its sections (at most two ISO steps above
/// the flat pattern's, at most one below), then the smaller sheet.
pub fn plan(
    f: &Features,
    blank: &Blank,
    t: f64,
    note_lines: usize,
    lang: Lang,
) -> Result<Plan, String> {
    let info_h = table_h(blank.bends.len()).max(note_lines as f64 * NOTE_PITCH + 2.0);
    let mut best: Option<(f64, Plan)> = None;
    for s in SCALES {
        for paper in Paper::ALL {
            let lo = dims::layout(f, &blank.bends, s, lang);
            let [x0, y0, x1, y1] = paper.view_area(info_h);
            let need_w = f.max[0] * s + lo.margin[0] + lo.margin[2];
            let need_h = f.max[1] * s + lo.margin[1] + lo.margin[3] + VIEW_TITLE_H;
            let (aw, ah) = (x1 - x0, y1 - y0);
            if need_w > aw || need_h > ah {
                continue;
            }
            let profiles = section::profiles(blank, f, &lo, s, t);
            if profiles.is_empty() {
                // Centre the flat pattern.
                let ox = x0 + lo.margin[0] + (aw - need_w) / 2.0;
                let oy = y0 + lo.margin[1] + (ah - need_h) / 2.0;
                let score = 10.0 * s.ln() - if paper == Paper::A3 { 0.01 } else { 0.0 };
                if best.as_ref().is_none_or(|(b, _)| score > *b + 1e-9) {
                    best = Some((
                        score,
                        Plan {
                            paper,
                            s,
                            origin: [ox, oy],
                            layout: lo,
                            info_h,
                            sections: vec![],
                        },
                    ));
                }
                continue;
            }
            // The cut markers widen the flat pattern's box.
            let ext = [
                -lo.margin[0],
                -lo.margin[1],
                f.max[0] * s + lo.margin[2],
                f.max[1] * s + lo.margin[3],
            ];
            let mut grown = ext;
            let mut avoid = lo.boxes.clone();
            for p in &profiles {
                let (_, _, _, bx) = section::cut_marker(p, s, ext, &avoid, &lo.lines);
                avoid.push(bx);
                grown[0] = grown[0].min(bx[0]);
                grown[1] = grown[1].min(bx[1]);
                grown[2] = grown[2].max(bx[2]);
                grown[3] = grown[3].max(bx[3]);
            }
            let need_w = grown[2] - grown[0];
            let need_h = grown[3] - grown[1] + VIEW_TITLE_H;
            if need_w > aw || need_h > ah {
                continue;
            }
            // Flat pattern at the top left; sections in the strip to its
            // right (stacked down) or below it (left to right).
            let flat_box = [x0, y1 - need_h, x0 + need_w, y1];
            let origin = [x0 - grown[0], flat_box[1] - grown[1]];
            let mut right_top = y1;
            let right_x0 = flat_box[2] + GAP;
            let bottom_y1 = flat_box[1] - GAP;
            let mut bottom_x = x0;
            let mut placed = Vec::new();
            let mut score = 10.0 * s.ln() - if paper == Paper::A3 { 0.01 } else { 0.0 };
            let mut ok = true;
            // A section may be two ISO steps larger than the flat pattern
            // and one step smaller.
            let si = SCALES
                .iter()
                .position(|&x| (x - s).abs() < 1e-12)
                .unwrap_or(0);
            let floor = SCALES[(si + 1).min(SCALES.len() - 1)];
            let ceil = SCALES[si.saturating_sub(2)];
            for p in &profiles {
                let mut choice: Option<(f64, P2, SectionPlan, bool)> = None;
                for &ss in SCALES
                    .iter()
                    .filter(|&&ss| ss >= floor - 1e-12 && ss <= ceil + 1e-12)
                {
                    let sp = section::plan(p, ss);
                    let w = sp.bbox[2] - sp.bbox[0];
                    let h = sp.bbox[3] - sp.bbox[1] + SECTION_TITLE_H;
                    // Right strip.
                    if right_x0 + w <= x1 && right_top - h >= y0 {
                        let o = [right_x0 - sp.bbox[0], right_top - h - sp.bbox[1]];
                        choice = Some((ss, o, sp, true));
                        break;
                    }
                    if bottom_x + w <= x1 && bottom_y1 - h >= y0 {
                        let o = [bottom_x - sp.bbox[0], bottom_y1 - h - sp.bbox[1]];
                        choice = Some((ss, o, sp, false));
                        break;
                    }
                }
                let Some((ss, o, sp, right)) = choice else {
                    ok = false;
                    break;
                };
                if right {
                    right_top -= sp.bbox[3] - sp.bbox[1] + SECTION_TITLE_H + GAP;
                } else {
                    bottom_x += sp.bbox[2] - sp.bbox[0] + GAP;
                }
                score += ss.ln() / profiles.len() as f64;
                placed.push(Placed {
                    profile: p.clone(),
                    plan: sp,
                    origin: o,
                });
            }
            if !ok {
                continue;
            }
            if best.as_ref().is_none_or(|(b, _)| score > *b + 1e-9) {
                best = Some((
                    score,
                    Plan {
                        paper,
                        s,
                        origin,
                        layout: lo,
                        info_h,
                        sections: placed,
                    },
                ));
            }
        }
    }
    best.map(|(_, p)| p).ok_or_else(|| {
        format!(
            "blank of {} × {} mm fits no sheet, not even A3 at 1:100",
            text::mm(f.max[0]),
            text::mm(f.max[1])
        )
    })
}

/// Weight from the blank's area, thickness and the material's density
/// (kg/m³), when the meta knows it.
fn weight(blank: &Blank, f: &Features, t: f64, material: &str, meta: &Meta) -> Option<f64> {
    let density = *meta.densities.get(material)?;
    if t <= 0.0 {
        return None;
    }
    let outer = super::features::outer_loop(&f.loops);
    let mut area = 0.0;
    for (li, lp) in f.loops.iter().enumerate() {
        let mut pts = Vec::new();
        for seg in lp {
            dims::sample(seg, 1.0, &mut pts);
        }
        let n = pts.len();
        let mut a = 0.0;
        for i in 0..n {
            let (p, q) = (pts[i], pts[(i + 1) % n]);
            a += p[0] * q[1] - q[0] * p[1];
        }
        let a = a.abs() / 2.0;
        if li == outer {
            area += a;
        } else {
            area -= a;
        }
    }
    let _ = blank;
    Some(area * t * density * 1e-9)
}

fn kg(w: f64) -> String {
    let v = if w >= 10.0 {
        (w * 10.0).round() / 10.0
    } else {
        (w * 100.0).round() / 100.0
    };
    let mut s = format!("{v}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    format!("{} kg", s.replace('.', ","))
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
    let t = info.thickness;
    let note_lines = notes(blank, &f, info, lang, !blank.bends.is_empty());
    let p = plan(&f, blank, t, note_lines.len(), lang)?;
    let paper = p.paper;
    let o = p.origin;
    let s = p.s;
    let mut c = Content::new();
    c.caps_butt_joins_round();
    c.fill_gray(0.0);

    // Title block values.
    let extents = section::folded_extents(blank, t);
    let scale_text = text::scale(s);
    let tv = TitleValues {
        id: info.title.to_string(),
        description: meta.title.clone(),
        customer: meta.customer.clone(),
        quantity: info.part_ids.len(),
        sheet: info.index,
        sheets: info.total,
        designer: meta.created_by.clone(),
        date: meta.date.clone(),
        approved: meta.approved_by.clone(),
        approved_date: meta.approved_date.clone(),
        scale: scale_text.clone(),
        material: info.material.to_string(),
        thickness: if t > 0.0 {
            format!("{} mm", text::mm(t))
        } else {
            String::new()
        },
        weight: weight(blank, &f, t, info.material, meta)
            .map(kg)
            .unwrap_or_default(),
        extents: match extents {
            Some(e) => format!(
                "{} × {} × {}",
                text::mm(e[0]),
                text::mm(e[1]),
                text::mm(e[2])
            ),
            None => format!("{} × {}", text::mm(blank.max[0]), text::mm(blank.max[1])),
        },
    };
    sheet::title_values(&mut c, paper, lang, &tv);

    // Notes bottom left, the bend table beside them on the same baseline.
    let (sx, sy, sw) = paper.info_strip();
    let th = table_h(blank.bends.len());
    let tw: f64 = TABLE_COLS.iter().sum();
    let note_w = if th > 0.0 { sw - tw - 6.0 } else { sw };
    let mut y = sy + 1.0 + (note_lines.len() as f64 - 1.0) * NOTE_PITCH + 0.5;
    for line in &note_lines {
        let (tt, h) = sheet::shrink_to(line, 2.5, note_w - 2.0);
        c.text([sx + 1.0, y], h, 0.0, &tt);
        y -= NOTE_PITCH;
    }
    let mut bend_reports = Vec::new();
    for (i, b) in blank.bends.iter().enumerate() {
        bend_reports.push(BendReport {
            nr: i + 1,
            id: b.id.clone(),
            angle_deg: b.angle_deg,
            direction: b.direction.label().to_lowercase(),
            inside_radius: b.inside_radius,
            allowance: b.allowance,
            note: p
                .layout
                .tags
                .iter()
                .find(|tg| tg.nr == i + 1)
                .and_then(|tg| tg.note.as_ref().map(|n| n.0.clone())),
        });
    }
    if th > 0.0 {
        let tx = sx + note_w + 6.0;
        let ty = sy + 1.0;
        c.width(NARROW);
        c.rect_stroke(tx, ty, tw, th - 1.0);
        let heads = ["bend", "direction", "angle", "inside_r", "allowance"];
        let mut row_y = ty + th - 1.0 - TABLE_ROW;
        let mut x = tx;
        c.line([tx, row_y], [tx + tw, row_y]);
        for (i, k) in heads.iter().enumerate() {
            let (tt, h) = sheet::shrink_to(lang.t(k), 2.5, TABLE_COLS[i] - 1.5);
            c.text([x + 0.8, row_y + 1.5], h, 0.0, &tt);
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
                dir.to_string(),
                text::deg(b.angle_deg),
                text::mm(b.inside_radius),
                text::mm(b.allowance),
            ];
            let mut x = tx;
            for (j, tt) in cells.iter().enumerate() {
                c.text([x + 0.8, row_y + 1.5], 2.5, 0.0, tt);
                x += TABLE_COLS[j];
            }
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
    // Dimensions, callouts, bend lines, notes and tags, narrow.
    dims::draw(&p.layout, &f, &mut c, o);

    // The sections and their cutting lines.
    let ext = [
        -p.layout.margin[0],
        -p.layout.margin[1],
        blank.max[0] * s + p.layout.margin[2],
        blank.max[1] * s + p.layout.margin[3],
    ];
    let mut section_reports = Vec::new();
    let mut dimensions = Vec::new();
    let mut avoid: Vec<[f64; 4]> = p.layout.boxes.clone();
    for pl in &p.sections {
        let placed = section::draw_cut(&pl.profile, &mut c, o, s, ext, &avoid, &p.layout.lines);
        avoid.extend(placed);
        section::draw(&pl.plan, &mut c, pl.origin);
        let title = section::title(&pl.profile, lang, pl.plan.s, s);
        let cx = pl.origin[0] + (pl.plan.bbox[0] + pl.plan.bbox[2]) / 2.0;
        c.text_centred(cx, pl.origin[1] + pl.plan.bbox[3] + 2.0, 4.0, &title);
        let mut dims_r = Vec::new();
        for d in &pl.plan.dims {
            dims_r.push(DimReport {
                kind: d.kind.into(),
                axis: None,
                value: d.value,
                text: d.text.clone(),
                count: None,
                sources: d
                    .piece
                    .map(|i| vec![format!("piece:{i}")])
                    .unwrap_or_default(),
                section: Some(pl.profile.letter.to_string()),
            });
        }
        for a in &pl.plan.angles {
            dims_r.push(DimReport {
                kind: "angle".into(),
                axis: None,
                value: a.value,
                text: a.text.clone(),
                count: None,
                sources: vec![format!("bend:{}", a.nr)],
                section: Some(pl.profile.letter.to_string()),
            });
        }
        dimensions.extend(dims_r.iter().cloned());
        section_reports.push(SectionReport {
            letter: pl.profile.letter.to_string(),
            cut: pl.profile.cut,
            direction: pl.profile.c,
            bends: pl.profile.bends.clone(),
            pieces: pl
                .profile
                .pieces
                .iter()
                .enumerate()
                .map(|(i, pc)| match pc {
                    Piece::Flat { len } => PieceReport {
                        kind: "flat".into(),
                        length: Some(*len),
                        shown: pl
                            .plan
                            .breaks
                            .iter()
                            .find(|b| b.0 == i)
                            .map(|_| pl.plan.shown[i]),
                        bend: None,
                    },
                    Piece::Bend { nr, .. } => PieceReport {
                        kind: "bend".into(),
                        length: None,
                        shown: None,
                        bend: Some(*nr),
                    },
                })
                .collect(),
            scale: text::scale(pl.plan.s),
            scale_factor: pl.plan.s,
            origin: pl.origin,
            width: pl.plan.width,
            height: pl.plan.height,
            dimensions: dims_r,
        });
    }

    let (w, h) = paper.size();
    let label = format!("Pos {}", info.index);
    doc.add_page(w, h, &[paper.form_name()], c, &label);

    let mut chains = Vec::new();
    for ch in &p.layout.chains {
        let axis = if ch.n[1].abs() < 1e-9 {
            "x"
        } else if ch.n[0].abs() < 1e-9 {
            "y"
        } else {
            "oblique"
        };
        chains.push(ChainReport {
            kind: ch.kind.into(),
            axis: axis.into(),
            direction: ch.n,
            side: ch.side,
            stations: ch
                .stations
                .iter()
                .map(|st| StationReport {
                    t: st.t,
                    sources: st.sources.clone(),
                })
                .collect(),
            values: ch.texts.iter().map(|t| t.value).collect(),
            texts: ch.texts.iter().map(|t| t.text.clone()).collect(),
        });
    }
    for co in &p.layout.callouts {
        dimensions.push(DimReport {
            kind: co.kind.into(),
            axis: None,
            value: co.value,
            text: co.text.clone(),
            count: Some(co.count),
            sources: vec![],
            section: None,
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
        thickness: t,
        dimensions,
        chains,
        bends: bend_reports,
        sections: section_reports,
        extents,
        kind: "flat".into(),
        size: None,
        views: vec![],
        minor_bound: None,
    })
}

/// One closed loop as a single path, so corners join cleanly.
pub(crate) fn draw_loop(c: &mut Content, lp: &[Seg], pm: &impl Fn(P2) -> P2, s: f64) {
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
                let sweep = (end - start).rem_euclid(360.0);
                let sweep = if sweep == 0.0 { 360.0 } else { sweep };
                if forward {
                    c.arc_sweep(pc, r * s, start, sweep);
                    cur = b;
                } else {
                    c.arc_sweep(pc, r * s, end, -sweep);
                    cur = a;
                }
            }
            Seg::Circle { .. } => {}
        }
    }
    c.close();
    c.stroke();
}
