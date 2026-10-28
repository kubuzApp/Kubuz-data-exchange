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
pub(crate) const SCALES: [f64; 9] = [5.0, 2.0, 1.0, 0.5, 0.2, 0.1, 0.05, 0.02, 0.01];
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
    /// The isometric pictorial of the folded part, when it found room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isometric: Option<IsoReport>,
    /// `flat` or `folded`.
    pub kind: String,
    /// Overall size of a folded part along the front view's frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<super::folded::ViewReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minor_bound: Option<f64>,
    /// The sheet's page in the dossier, 0-based.
    pub page: usize,
    /// `laser`, `laser+bend` or `folded`.
    pub route: String,
    /// The position this blank is the mirror image of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror_of: Option<usize>,
    /// Per piece, kg, when the density is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    /// Outline plus holes, mm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cut_length: Option<f64>,
    /// Net, mm².
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holes: Option<usize>,
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
pub struct IsoReport {
    pub scale: String,
    pub scale_factor: f64,
    /// Paper position of the picture plane's origin, in mm from the lower
    /// left corner.
    pub origin: [f64; 2],
    /// Paper box of the pictorial.
    pub bbox: [f64; 4],
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
    /// Position number.
    pub index: usize,
    pub total: usize,
    /// This sheet's page in the dossier (0-based) and the page count.
    pub page: usize,
    pub pages: usize,
    pub title: &'a str,
    pub part_ids: &'a [String],
    pub material: &'a str,
    pub thickness: f64,
    /// The position this blank is the mirror image of.
    pub mirror_of: Option<usize>,
    /// The parts list's page, which the title block links back to.
    pub bom_page: Option<usize>,
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
    /// The isometric pictorial's scale and paper origin, when it fits.
    pub iso: Option<(f64, P2)>,
    pub iso_lines: Option<super::iso::Iso>,
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
    if let Some(e) = section::folded_extents(blank, info.thickness) {
        v.push(format!(
            "{}: {} × {} × {}",
            lang.t("folded_size"),
            text::mm(e[0]),
            text::mm(e[1]),
            text::mm(e[2])
        ));
    }
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
    if let Some(m) = info.mirror_of {
        v.push(format!(
            "{}.",
            lang.t("mirror_of").replace("{}", &m.to_string())
        ));
    }
    v.push(format!("DXF: {}.dxf", info.part_ids[0]));
    v
}

/// Choose sheet and scale: the largest scale at which the flat pattern
/// and its sections (at the same scale) fit, the smaller sheet preferred.
pub fn plan(
    f: &Features,
    blank: &Blank,
    t: f64,
    note_lines: usize,
    lang: Lang,
) -> Result<Plan, String> {
    let info_h = table_h(blank.bends.len()).max(note_lines as f64 * NOTE_PITCH + 2.0);
    let iso = super::iso::isometric(blank, t);
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
                            iso: None,
                            iso_lines: None,
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
            // The sections share the flat pattern's scale.
            for p in &profiles {
                let mut choice: Option<(f64, P2, SectionPlan, bool)> = None;
                for ss in [s] {
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
            // The pictorial where room is left, at the flat pattern's
            // scale or up to two steps smaller; never on a bigger sheet
            // for its sake.
            let mut iso_at = None;
            if let Some(iso) = &iso {
                for &ss in SCALES
                    .iter()
                    .filter(|&&ss| ss <= s + 1e-12 && ss >= s / 4.0 - 1e-12)
                {
                    let w = (iso.bbox[2] - iso.bbox[0]) * ss + 2.0;
                    let h = (iso.bbox[3] - iso.bbox[1]) * ss + SECTION_TITLE_H;
                    if right_x0 + w <= x1 && right_top - h >= y0 {
                        iso_at = Some((
                            ss,
                            [
                                right_x0 + 1.0 - iso.bbox[0] * ss,
                                right_top - h - iso.bbox[1] * ss,
                            ],
                        ));
                        break;
                    }
                    if bottom_x + w <= x1 && bottom_y1 - h >= y0 {
                        iso_at = Some((
                            ss,
                            [
                                bottom_x + 1.0 - iso.bbox[0] * ss,
                                bottom_y1 - h - iso.bbox[1] * ss,
                            ],
                        ));
                        break;
                    }
                }
            }
            if iso_at.is_some() {
                score += 0.005;
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
                        iso: iso_at,
                        iso_lines: None,
                    },
                ));
            }
        }
    }
    best.map(|(_, mut p)| {
        if p.iso.is_some() {
            p.iso_lines = iso;
        }
        p
    })
    .ok_or_else(|| {
        format!(
            "blank of {} × {} mm fits no sheet, not even A3 at 1:100",
            text::mm(f.max[0]),
            text::mm(f.max[1])
        )
    })
}

/// Weight from the blank's area, thickness and the material's density
/// (kg/m³), when the meta knows it.
pub(crate) fn weight(
    blank: &Blank,
    f: &Features,
    t: f64,
    material: &str,
    meta: &Meta,
) -> Option<f64> {
    let density = match meta.densities.get(material) {
        Some(d) => *d,
        None => density(material)?,
    };
    if t <= 0.0 {
        return None;
    }
    let _ = blank;
    Some(net_area(f) * 1e-6 * t * 1e-3 * density)
}

/// The blank's area less its holes, mm².
pub(crate) fn net_area(f: &Features) -> f64 {
    let outer = super::features::outer_loop(&f.loops);
    let mut area = 0.0;
    for (li, lp) in f.loops.iter().enumerate() {
        let pts = super::features::loop_points(lp, 1.0);
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
    area
}

/// Densities in kg/m³ by material name, for the weight when the meta
/// gives none.
pub fn density(material: &str) -> Option<f64> {
    let m: String = material
        .to_ascii_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect();
    if m.is_empty() {
        return None;
    }
    let has = |needles: &[&str]| needles.iter().any(|n| m.contains(n));
    let starts = |needles: &[&str]| needles.iter().any(|n| m.starts_with(n));
    // Stainless before steel: "1.4301" and "304" are not "S355".
    if has(&["INOX", "RVS", "STAINLESS", "EDELSTAHL"])
        || starts(&["1.43", "1.44", "1.45", "1.46", "X"])
        || [
            "301", "303", "304", "304L", "316", "316L", "316TI", "321", "430", "441", "2205",
            "2507", "904L",
        ]
        .iter()
        .any(|g| m == *g || m.starts_with(&format!("{g}/")) || m.starts_with(&format!("AISI{g}")))
    {
        return Some(if has(&["430", "441", "1.4016", "1.4509"]) {
            7700.0
        } else {
            7900.0
        });
    }
    if has(&["ALU", "ALMG", "ALSI", "ALZN", "ENAW", "EN AW"])
        || starts(&["AW", "AL", "5754", "5083", "6061", "6082", "1050", "3003"])
    {
        return Some(2700.0);
    }
    if has(&["CU", "COPPER", "KOPER", "CUIVRE"]) && !has(&["CUZN", "BRASS", "MESSING", "LAITON"]) {
        return Some(8940.0);
    }
    if has(&["CUZN", "BRASS", "MESSING", "LAITON", "MS58", "MS63"]) {
        return Some(8500.0);
    }
    if has(&["ZINC", "ZINK", "TITANZINK"]) {
        return Some(7140.0);
    }
    if has(&["TITAN", "TI6AL4V"]) || starts(&["TI"]) {
        return Some(4430.0);
    }
    if has(&[
        "S235",
        "S275",
        "S355",
        "DC01",
        "DC03",
        "DC04",
        "DX51",
        "DX52",
        "DX53",
        "DX54",
        "S250",
        "S280",
        "S320",
        "ST37",
        "ST52",
        "HARDOX",
        "STRENX",
        "DOMEX",
        "STEEL",
        "STAAL",
        "ACIER",
        "SENDZIMIR",
        "ZM",
        "CORTEN",
        "S460",
        "S500",
        "S700",
        "QSTE",
        "GALVA",
    ]) || starts(&[
        "1.0", "1.1", "1.2", "1.5", "1.7", "1.8", "S", "DC", "DX", "HX", "HC", "C45", "C",
    ]) {
        return Some(7850.0);
    }
    None
}

pub(crate) fn kg(w: f64) -> String {
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
        sheet: info.page + 1,
        sheets: info.pages,
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
        extents: format!("{} × {}", text::mm(blank.max[0]), text::mm(blank.max[1])),
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

    // The isometric pictorial, thin lines, titled.
    let mut iso_report = None;
    if let (Some((ss, io)), Some(iso)) = (p.iso, &p.iso_lines) {
        c.width(0.35);
        for pl in &iso.polylines {
            let simple = thin(
                pl.iter()
                    .map(|q| [io[0] + q[0] * ss, io[1] + q[1] * ss])
                    .collect(),
            );
            if simple.len() >= 2 {
                c.polyline(&simple, false);
            }
        }
        let mut title = lang.t("isometric").to_string();
        if (ss - s).abs() > 1e-9 {
            title.push_str(&format!(" ({})", text::scale(ss)));
        }
        let bbox = [
            io[0] + iso.bbox[0] * ss,
            io[1] + iso.bbox[1] * ss,
            io[0] + iso.bbox[2] * ss,
            io[1] + iso.bbox[3] * ss,
        ];
        c.text_centred((bbox[0] + bbox[2]) / 2.0, bbox[3] + 2.0, 4.0, &title);
        iso_report = Some(IsoReport {
            scale: text::scale(ss),
            scale_factor: ss,
            origin: io,
            bbox,
        });
    }

    let (w, h) = paper.size();
    let label = format!("Pos {}", info.index);
    let page = doc.add_page(w, h, &[paper.form_name()], c, &label);
    if let Some(bom) = info.bom_page {
        doc.add_link(page, sheet::id_rect(paper), bom);
    }

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
        isometric: iso_report,
        kind: "flat".into(),
        size: None,
        views: vec![],
        minor_bound: None,
        page: info.page,
        route: String::new(),
        mirror_of: info.mirror_of,
        weight: None,
        cut_length: None,
        area: None,
        holes: None,
    })
}

/// Thin a polyline on paper: points closer than 0,15 mm add nothing, nor
/// does a point within 0,05 mm of the line through its neighbours; a
/// polyline shorter than 0,3 mm is dropped.
pub(crate) fn thin(pts: Vec<P2>) -> Vec<P2> {
    let n = pts.len();
    let mut kept: Vec<P2> = Vec::with_capacity(n);
    for (i, q) in pts.iter().enumerate() {
        let keep = match kept.last() {
            Some(l) => (q[0] - l[0]).hypot(q[1] - l[1]) >= 0.15 || i + 1 == n,
            None => true,
        };
        if keep {
            kept.push(*q);
        }
    }
    let mut simple: Vec<P2> = Vec::with_capacity(kept.len());
    for (i, q) in kept.iter().enumerate() {
        if i == 0 || i + 1 == kept.len() {
            simple.push(*q);
            continue;
        }
        let a = *simple.last().unwrap();
        let b = kept[i + 1];
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy).max(1e-9);
        let dev = ((q[0] - a[0]) * dy - (q[1] - a[1]) * dx).abs() / len;
        if dev >= 0.05 {
            simple.push(*q);
        }
    }
    let length: f64 = simple
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum();
    if simple.len() < 2 || length < 0.3 {
        return Vec::new();
    }
    simple
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
    // Enter the first segment at the end the second does not share, so
    // an arc at the start of the loop runs the loop's way.
    let (fa, fb) = super::features::ends(first);
    let start = match lp.get(1) {
        Some(second) if !matches!(second, Seg::Circle { .. }) => {
            let (sa, sb) = super::features::ends(second);
            let near = |p: P2, q: P2| (p[0] - q[0]).abs() + (p[1] - q[1]).abs();
            let fb_shared = near(fb, sa).min(near(fb, sb));
            let fa_shared = near(fa, sa).min(near(fa, sb));
            if fb_shared <= fa_shared {
                fa
            } else {
                fb
            }
        }
        _ => fa,
    };
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
