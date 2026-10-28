//! The dossier's front matter: the cover sheet, the parts list (ISO
//! 7573), the laser list and the bend list, as tables on ISO sheets with
//! the same title block as the drawings. Every row that names a part
//! links to its sheet.

use super::sheet::{self, Paper, TitleValues};
use super::text::{self, Lang};
use super::Meta;
use crate::pdf::{Content, Doc};
use crate::view::BendDirection;

/// Which machine a part goes to, in dossier order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Route {
    /// Cut only.
    Laser,
    /// Cut, then bent.
    LaserBend,
    /// Drawn from a folded view: no flat pattern to cut.
    Folded,
}

impl Route {
    pub fn name(self) -> &'static str {
        match self {
            Route::Laser => "laser",
            Route::LaserBend => "laser+bend",
            Route::Folded => "folded",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BendRow {
    pub nr: usize,
    pub angle_deg: f64,
    pub direction: BendDirection,
    pub inside_radius: f64,
    pub allowance: f64,
    /// Along the bend line.
    pub length: f64,
}

/// One line of the parts list: a sheet of the dossier and what the lists
/// say about it.
#[derive(Clone, Debug)]
pub struct Row {
    pub pos: usize,
    pub id: String,
    pub part_ids: Vec<String>,
    pub material: String,
    pub thickness: f64,
    /// Flat pattern L × W.
    pub blank: Option<[f64; 2]>,
    /// Folded L × W × H.
    pub folded: Option<[f64; 3]>,
    /// Per piece, kg.
    pub weight: Option<f64>,
    pub cut_length: Option<f64>,
    /// Net, mm².
    pub area: Option<f64>,
    pub holes: Option<usize>,
    pub bends: Vec<BendRow>,
    pub route: Route,
    pub mirror_of: Option<usize>,
    /// The page (0-based) of its sheet.
    pub page: usize,
}

impl Row {
    pub fn quantity(&self) -> usize {
        self.part_ids.len()
    }
}

const ROW_H: f64 = 5.0;
const HEAD_H: f64 = 6.0;
const TEXT_H: f64 = 2.5;
const PAD: f64 = 0.8;
const NOTE_PITCH: f64 = 3.6;

pub struct Col {
    pub title: String,
    pub w: f64,
    pub right: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A band naming the group the rows under it belong to.
    Group,
    Normal,
    Total,
}

pub struct TRow {
    pub kind: Kind,
    pub cells: Vec<String>,
    /// The page this row links to.
    pub link: Option<usize>,
}

pub struct Table {
    pub cols: Vec<Col>,
    pub rows: Vec<TRow>,
}

fn col(title: &str, w: f64, right: bool) -> Col {
    Col {
        title: title.to_string(),
        w,
        right,
    }
}

fn group(text: String) -> TRow {
    TRow {
        kind: Kind::Group,
        cells: vec![text],
        link: None,
    }
}

impl Table {
    pub fn width(&self) -> f64 {
        self.cols.iter().map(|c| c.w).sum()
    }

    /// How many rows fit under the header in `h` mm.
    pub fn capacity(h: f64) -> usize {
        ((h - HEAD_H) / ROW_H).floor().max(0.0) as usize
    }

    /// Split the rows over pages that take `first` rows, then `rest`
    /// rows each; a group band never ends a page.
    pub fn split(&self, first: usize, rest: usize) -> Vec<std::ops::Range<usize>> {
        let mut pages = Vec::new();
        let n = self.rows.len();
        let mut at = 0;
        while at < n {
            let cap = if pages.is_empty() { first } else { rest }.max(2);
            let mut end = (at + cap).min(n);
            if end < n && self.rows[end - 1].kind == Kind::Group {
                end -= 1;
            }
            pages.push(at..end);
            at = end;
        }
        if pages.is_empty() {
            pages.push(0..0);
        }
        pages
    }

    /// Draw the header and `rows[range]` with the top-left corner at
    /// `(x, y_top)`, registering links on `page`. Returns the bottom.
    pub fn draw(
        &self,
        c: &mut Content,
        doc: &mut Doc,
        page: usize,
        x: f64,
        y_top: f64,
        range: std::ops::Range<usize>,
    ) -> f64 {
        let w = self.width();
        let rows = &self.rows[range];
        let h = HEAD_H + ROW_H * rows.len() as f64;
        let y0 = y_top - h;
        c.width(0.5);
        c.rect_stroke(x, y0, w, h);
        c.width(0.25);
        // Header.
        let hy = y_top - HEAD_H;
        c.line([x, hy], [x + w, hy]);
        // Column separators run through the header and the ordinary rows,
        // never through a group band.
        let separators = |c: &mut Content, y0: f64, y1: f64| {
            let mut cx = x;
            for (i, cl) in self.cols.iter().enumerate() {
                cx += cl.w;
                if i + 1 < self.cols.len() {
                    c.line([cx, y0], [cx, y1]);
                }
            }
        };
        separators(c, hy, y_top);
        let mut cx = x;
        for cl in &self.cols {
            let (t, th) = sheet::shrink_to(&cl.title, TEXT_H, cl.w - 2.0 * PAD);
            if cl.right {
                c.text_right(cx + cl.w - PAD, hy + 1.8, th, &t);
            } else {
                c.text([cx + PAD, hy + 1.8], th, 0.0, &t);
            }
            cx += cl.w;
        }
        // Rows.
        let mut ry = hy;
        for (k, r) in rows.iter().enumerate() {
            ry -= ROW_H;
            match r.kind {
                Kind::Group => {
                    c.fill_gray(0.9);
                    c.rect_fill(x + 0.3, ry + 0.3, w - 0.6, ROW_H - 0.6);
                    c.fill_gray(0.0);
                    let (t, th) = sheet::shrink_to(&r.cells[0], TEXT_H, w - 2.0 * PAD);
                    c.text([x + PAD, ry + 1.5], th, 0.0, &t);
                }
                Kind::Normal | Kind::Total => {
                    separators(c, ry, ry + ROW_H);
                    if r.kind == Kind::Total {
                        c.width(0.5);
                        c.line([x, ry + ROW_H], [x + w, ry + ROW_H]);
                        c.width(0.25);
                    }
                    let mut cx = x;
                    for (i, cl) in self.cols.iter().enumerate() {
                        if let Some(s) = r.cells.get(i) {
                            if !s.is_empty() {
                                let (t, th) = sheet::shrink_to(s, TEXT_H, cl.w - 2.0 * PAD);
                                if cl.right {
                                    c.text_right(cx + cl.w - PAD, ry + 1.5, th, &t);
                                } else {
                                    c.text([cx + PAD, ry + 1.5], th, 0.0, &t);
                                }
                            }
                        }
                        cx += cl.w;
                    }
                }
            }
            if k + 1 < rows.len() {
                c.line([x, ry], [x + w, ry]);
            }
            if let Some(p) = r.link {
                doc.add_link(page, [x, ry, x + w, ry + ROW_H], p);
            }
        }
        y0
    }
}

/// A list page's caption, subtitle and notes; returns the content and
/// the table area `[x0, y0, x1, y1]` left between them.
pub fn begin(
    paper: Paper,
    _lang: Lang,
    caption: &str,
    sub: &str,
    notes: &[&str],
) -> (Content, [f64; 4]) {
    let mut c = Content::new();
    c.caps_butt_joins_round();
    c.fill_gray(0.0);
    let [x0, _, x1, y1] = paper.frame();
    let top = y1 - 6.0 - 7.5;
    c.text([x0 + 3.0, top], 5.0, 0.0, caption);
    if !sub.is_empty() {
        let (t, h) = sheet::shrink_to(sub, TEXT_H, x1 - x0 - 6.0);
        c.text([x0 + 3.0, top - 5.0], h, 0.0, &t);
    }
    let (sx, sy, sw) = paper.info_strip();
    let bottom = match paper {
        Paper::A4 => {
            let mut y = sy + 1.0;
            for n in notes {
                let (t, h) = sheet::shrink_to(n, TEXT_H, sw - 2.0);
                c.text([sx + 1.0, y], h, 0.0, &t);
                y += NOTE_PITCH;
            }
            y + 1.5
        }
        Paper::A3 => {
            let mut y = sy + 1.0;
            for n in notes {
                let (t, h) = sheet::shrink_to(n, TEXT_H, sw - 2.0);
                c.text([sx + 1.0, y], h, 0.0, &t);
                y += NOTE_PITCH;
            }
            sy + sheet::TB_H + 4.0
        }
    };
    let table_top = if sub.is_empty() {
        top - 5.0
    } else {
        top - 10.0
    };
    (c, [x0 + 2.0, bottom, x1 - 2.0, table_top])
}

/// Write the title block values and fill reserved page `page`.
#[allow(clippy::too_many_arguments)]
pub fn finish(
    doc: &mut Doc,
    page: usize,
    paper: Paper,
    lang: Lang,
    meta: &Meta,
    mut c: Content,
    description: &str,
    scale: &str,
    pages: usize,
    label: &str,
) {
    let tv = TitleValues {
        id: if meta.number.is_empty() {
            dossier_title(meta, lang)
        } else {
            meta.number.clone()
        },
        description: description.to_string(),
        customer: meta.customer.clone(),
        quantity: 0,
        sheet: page + 1,
        sheets: pages,
        designer: meta.created_by.clone(),
        date: meta.date.clone(),
        approved: meta.approved_by.clone(),
        approved_date: meta.approved_date.clone(),
        scale: scale.to_string(),
        ..TitleValues::default()
    };
    sheet::title_values(&mut c, paper, lang, &tv);
    let (w, h) = paper.size();
    doc.fill_page(page, w, h, &[paper.form_name()], c, label);
}

pub fn dossier_title(meta: &Meta, lang: Lang) -> String {
    if meta.title.is_empty() {
        lang.t("dossier").to_string()
    } else {
        meta.title.clone()
    }
}

fn lw(v: [f64; 2]) -> String {
    format!("{} × {}", text::mm(v[0]), text::mm(v[1]))
}

fn lwh(v: [f64; 3]) -> String {
    format!(
        "{} × {} × {}",
        text::mm(v[0]),
        text::mm(v[1]),
        text::mm(v[2])
    )
}

/// A weight in kg as the lists print it: two decimals under 10 kg, one
/// above, no trailing zero, decimal comma.
pub fn kg(w: f64) -> String {
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
    s.replace('.', ",")
}

/// `dm²` to a hundredth.
fn dm2(mm2: f64) -> String {
    let v = (mm2 / 10_000.0 * 100.0).round() / 100.0;
    let mut s = format!("{v:.2}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s.replace('.', ",")
}

fn material_line(material: &str, t: f64, lang: Lang) -> String {
    let mut s = String::new();
    if !material.is_empty() {
        s.push_str(material);
    }
    if t > 0.0 {
        if !s.is_empty() {
            s.push_str(" · ");
        }
        s.push_str(&format!("t = {} mm", text::mm(t)));
    }
    if s.is_empty() {
        s.push_str(lang.t("material"));
    }
    s
}

/// The parts list, one row per sheet, with a total.
pub fn bom_table(rows: &[Row], lang: Lang) -> Table {
    let cols = vec![
        col(lang.t("pos"), 10.0, true),
        col(lang.t("quantity"), 12.0, true),
        col(lang.t("name"), 44.0, false),
        col(lang.t("material"), 22.0, false),
        col("t", 10.0, true),
        col(lang.t("blank"), 30.0, false),
        col(lang.t("folded_size"), 32.0, false),
        col("kg", 16.0, true),
    ];
    let mut trows = Vec::new();
    let mut total_q = 0;
    let mut total_w = 0.0;
    let mut all_weighed = true;
    for r in rows {
        let q = r.quantity();
        total_q += q;
        match r.weight {
            Some(w) => total_w += w * q as f64,
            None => all_weighed = false,
        }
        let mut name = r.id.clone();
        if let Some(m) = r.mirror_of {
            name.push_str(&format!(
                " ({})",
                lang.t("mirror_of").replace("{}", &m.to_string())
            ));
        }
        trows.push(TRow {
            kind: Kind::Normal,
            cells: vec![
                r.pos.to_string(),
                q.to_string(),
                name,
                r.material.clone(),
                if r.thickness > 0.0 {
                    text::mm(r.thickness)
                } else {
                    String::new()
                },
                r.blank.map(lw).unwrap_or_default(),
                r.folded.map(lwh).unwrap_or_default(),
                r.weight.map(kg).unwrap_or_default(),
            ],
            link: Some(r.page),
        });
    }
    trows.push(TRow {
        kind: Kind::Total,
        cells: vec![
            String::new(),
            total_q.to_string(),
            lang.t("total").to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            if all_weighed && total_q > 0 {
                kg(total_w)
            } else {
                String::new()
            },
        ],
        link: None,
    });
    Table { cols, rows: trows }
}

/// The laser list: per material and thickness, every blank with its cut
/// length, holes, net area and weight.
pub fn laser_table(rows: &[Row], lang: Lang) -> Table {
    let cols = vec![
        col(lang.t("pos"), 10.0, true),
        col(lang.t("quantity"), 12.0, true),
        col(lang.t("part"), 44.0, false),
        col(lang.t("blank"), 30.0, false),
        col(&format!("{} mm", lang.t("cut_length")), 24.0, true),
        col(lang.t("holes"), 12.0, true),
        col(lang.t("area"), 16.0, true),
        col(lang.t("per_piece"), 14.0, true),
        col(lang.t("kg_total"), 14.0, true),
    ];
    let mut trows = Vec::new();
    let mut i = 0;
    let cut: Vec<&Row> = rows.iter().filter(|r| r.blank.is_some()).collect();
    let mut grand = (0usize, 0.0f64, 0usize, 0.0f64, 0.0f64, true);
    let mut groups = 0;
    while i < cut.len() {
        let (m, t) = (cut[i].material.clone(), cut[i].thickness);
        let mut j = i;
        while j < cut.len() && cut[j].material == m && (cut[j].thickness - t).abs() < 1e-9 {
            j += 1;
        }
        trows.push(group(material_line(&m, t, lang)));
        let mut sum = (0usize, 0.0f64, 0usize, 0.0f64, 0.0f64, true);
        for r in &cut[i..j] {
            let q = r.quantity();
            let cl = r.cut_length.unwrap_or(0.0);
            let holes = r.holes.unwrap_or(0);
            let area = r.area.unwrap_or(0.0);
            sum.0 += q;
            sum.1 += cl * q as f64;
            sum.2 += holes * q;
            sum.3 += area * q as f64;
            match r.weight {
                Some(w) => sum.4 += w * q as f64,
                None => sum.5 = false,
            }
            trows.push(TRow {
                kind: Kind::Normal,
                cells: vec![
                    r.pos.to_string(),
                    q.to_string(),
                    r.id.clone(),
                    r.blank.map(lw).unwrap_or_default(),
                    text::mm(cl),
                    holes.to_string(),
                    dm2(area),
                    r.weight.map(kg).unwrap_or_default(),
                    r.weight.map(|w| kg(w * q as f64)).unwrap_or_default(),
                ],
                link: Some(r.page),
            });
        }
        trows.push(TRow {
            kind: Kind::Total,
            cells: vec![
                String::new(),
                sum.0.to_string(),
                lang.t("total").to_string(),
                String::new(),
                text::mm(sum.1),
                sum.2.to_string(),
                dm2(sum.3),
                String::new(),
                if sum.5 { kg(sum.4) } else { String::new() },
            ],
            link: None,
        });
        grand.0 += sum.0;
        grand.1 += sum.1;
        grand.2 += sum.2;
        grand.3 += sum.3;
        grand.4 += sum.4;
        grand.5 &= sum.5;
        groups += 1;
        i = j;
    }
    if groups > 1 {
        trows.push(TRow {
            kind: Kind::Total,
            cells: vec![
                String::new(),
                grand.0.to_string(),
                format!(
                    "{} ({})",
                    lang.t("total"),
                    lang.t("laser_title").to_lowercase()
                ),
                String::new(),
                text::mm(grand.1),
                grand.2.to_string(),
                dm2(grand.3),
                String::new(),
                if grand.5 { kg(grand.4) } else { String::new() },
            ],
            link: None,
        });
    }
    Table { cols, rows: trows }
}

/// The bend list: per thickness and inside radius (the tool), every
/// bend of every bent part.
pub fn bend_table(rows: &[Row], lang: Lang) -> Table {
    let cols = vec![
        col(lang.t("pos"), 10.0, true),
        col(lang.t("quantity"), 12.0, true),
        col(lang.t("part"), 50.0, false),
        col(lang.t("bend"), 12.0, true),
        col(lang.t("angle"), 14.0, true),
        col(lang.t("direction"), 18.0, false),
        col(lang.t("inside_r"), 16.0, true),
        col(lang.t("allowance"), 18.0, true),
        col(lang.t("bend_length"), 26.0, true),
    ];
    // Tools: (thickness, radius) in ascending order.
    let mut tools: Vec<(f64, f64)> = Vec::new();
    for r in rows {
        for b in &r.bends {
            let key = (r.thickness, b.inside_radius);
            if !tools
                .iter()
                .any(|(t, rr)| (t - key.0).abs() < 1e-6 && (rr - key.1).abs() < 1e-6)
            {
                tools.push(key);
            }
        }
    }
    tools.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut trows = Vec::new();
    for (t, radius) in tools {
        let mut band = String::new();
        if t > 0.0 {
            band.push_str(&format!("t = {} mm · ", text::mm(t)));
        }
        band.push_str(&format!("{} {} mm", lang.t("inside_r"), text::mm(radius)));
        trows.push(group(band));
        let mut count = 0;
        for r in rows {
            let mine: Vec<&BendRow> = r
                .bends
                .iter()
                .filter(|b| {
                    (r.thickness - t).abs() < 1e-6 && (b.inside_radius - radius).abs() < 1e-6
                })
                .collect();
            for (k, b) in mine.iter().enumerate() {
                let dir = match b.direction {
                    BendDirection::Up => lang.t("up"),
                    BendDirection::Down => lang.t("down"),
                };
                trows.push(TRow {
                    kind: Kind::Normal,
                    cells: vec![
                        if k == 0 {
                            r.pos.to_string()
                        } else {
                            String::new()
                        },
                        if k == 0 {
                            r.quantity().to_string()
                        } else {
                            String::new()
                        },
                        if k == 0 { r.id.clone() } else { String::new() },
                        b.nr.to_string(),
                        text::deg(b.angle_deg),
                        dir.to_string(),
                        text::mm(b.inside_radius),
                        text::mm(b.allowance),
                        text::mm(b.length),
                    ],
                    link: Some(r.page),
                });
            }
            count += mine.len() * r.quantity();
        }
        trows.push(TRow {
            kind: Kind::Total,
            cells: vec![
                String::new(),
                String::new(),
                format!("{count} {}", lang.t("bends_total")),
            ],
            link: None,
        });
    }
    Table { cols, rows: trows }
}

/// What the cover says about the dossier: the key-value block, the
/// totals and the contents. `sections` are (title, first page, last page).
pub struct CoverInfo<'a> {
    pub rows: &'a [Row],
    pub sections: Vec<(String, usize, usize)>,
}

/// Draw the cover into reserved page `page`.
pub fn cover(doc: &mut Doc, page: usize, pages: usize, meta: &Meta, lang: Lang, info: &CoverInfo) {
    let paper = Paper::A4;
    let (mut c, [x0, y0, x1, y1]) = begin(paper, lang, lang.t("dossier_caps"), "", &[]);
    let title = dossier_title(meta, lang);
    let (t, h) = sheet::shrink_to(&title, 7.0, x1 - x0 - 2.0);
    let mut y = y1 - 6.0;
    c.text([x0 + 1.0, y], h, 0.0, &t);
    y -= 9.0;
    // Key-value block.
    let mut kv: Vec<(&str, String)> = Vec::new();
    if !meta.number.is_empty() {
        kv.push(("number", meta.number.clone()));
    }
    kv.push(("revision", meta.revision.clone()));
    if !meta.customer.is_empty() {
        kv.push(("customer", meta.customer.clone()));
    }
    kv.push(("date", meta.date.clone()));
    kv.push(("created", meta.created_by.clone()));
    if !meta.approved_by.is_empty() {
        kv.push(("approved", meta.approved_by.clone()));
    }
    if !meta.status.is_empty() {
        kv.push(("status", meta.status.clone()));
    }
    kv.push(("tolerances", meta.tolerances.clone()));
    for (k, v) in &kv {
        c.text([x0 + 1.0, y], TEXT_H, 0.0, &format!("{}:", lang.t(k)));
        let (t, h) = sheet::shrink_to(v, TEXT_H, x1 - x0 - 46.0);
        c.text([x0 + 45.0, y], h, 0.0, &t);
        y -= 4.5;
    }
    y -= 3.0;
    // Totals.
    let total_q: usize = info.rows.iter().map(Row::quantity).sum();
    let weighed = info.rows.iter().all(|r| r.weight.is_some());
    let total_w: f64 = info
        .rows
        .iter()
        .map(|r| r.weight.unwrap_or(0.0) * r.quantity() as f64)
        .sum();
    let mut totals: Vec<(String, String)> = vec![
        (lang.t("parts_total").to_string(), total_q.to_string()),
        (
            lang.t("unique_parts").to_string(),
            info.rows.len().to_string(),
        ),
    ];
    if weighed && total_q > 0 {
        totals.push((
            lang.t("total_weight").to_string(),
            format!("{} kg", kg(total_w)),
        ));
    }
    // Per material and thickness.
    let mut mats: Vec<(String, f64, usize, f64, bool)> = Vec::new();
    for r in info.rows {
        let q = r.quantity();
        match mats
            .iter_mut()
            .find(|m| m.0 == r.material && (m.1 - r.thickness).abs() < 1e-9)
        {
            Some(m) => {
                m.2 += q;
                match r.weight {
                    Some(w) => m.3 += w * q as f64,
                    None => m.4 = false,
                }
            }
            None => mats.push((
                r.material.clone(),
                r.thickness,
                q,
                r.weight.unwrap_or(0.0) * q as f64,
                r.weight.is_some(),
            )),
        }
    }
    for m in &mats {
        let mut v = format!("{} {}", m.2, lang.t("pieces"));
        if m.4 {
            v.push_str(&format!(" · {} kg", kg(m.3)));
        }
        totals.push((material_line(&m.0, m.1, lang), v));
    }
    for (k, v) in &totals {
        let (t, h) = sheet::shrink_to(k, TEXT_H, 43.0);
        c.text([x0 + 1.0, y], h, 0.0, &t);
        let (t, h) = sheet::shrink_to(v, TEXT_H, x1 - x0 - 46.0);
        c.text([x0 + 45.0, y], h, 0.0, &t);
        y -= 4.5;
    }
    y -= 3.0;
    // Contents.
    c.text([x0 + 1.0, y], 3.5, 0.0, lang.t("contents"));
    y -= 3.0;
    let mut table = Table {
        cols: vec![
            col("", 126.0, false),
            col(lang.t("sheet"), 24.0, true),
            col(lang.t("route"), 26.0, false),
        ],
        rows: Vec::new(),
    };
    for (title, first, last) in &info.sections {
        let pages_text = if first == last {
            format!("{}", first + 1)
        } else {
            format!("{}–{}", first + 1, last + 1)
        };
        table.rows.push(TRow {
            kind: Kind::Normal,
            cells: vec![title.clone(), pages_text, String::new()],
            link: Some(*first),
        });
    }
    let room = Table::capacity(y - y0);
    let listed = info.rows.len() <= room.saturating_sub(table.rows.len() + 1);
    if listed {
        for r in info.rows {
            let mut name = format!("    {} {} · {}", lang.t("pos"), r.pos, r.id);
            if r.quantity() > 1 {
                name.push_str(&format!(" ({}×)", r.quantity()));
            }
            table.rows.push(TRow {
                kind: Kind::Normal,
                cells: vec![
                    name,
                    format!("{}", r.page + 1),
                    match r.route {
                        Route::Laser => lang.t("route_laser").to_string(),
                        Route::LaserBend => lang.t("route_bend").to_string(),
                        Route::Folded => lang.t("folded_part").to_string(),
                    },
                ],
                link: Some(r.page),
            });
        }
    }
    let n = table.rows.len().min(room);
    table.draw(&mut c, doc, page, x0, y, 0..n);
    finish(
        doc,
        page,
        paper,
        lang,
        meta,
        c,
        lang.t("cover_title"),
        "-",
        pages,
        lang.t("cover_title"),
    );
}

/// Draw a table over as many reserved pages as it needs, starting at
/// `first_page`; `caption` is the sheet's title, `title` the title block's
/// description. Returns the number of pages used.
#[allow(clippy::too_many_arguments)]
pub fn table_pages(
    doc: &mut Doc,
    first_page: usize,
    pages: usize,
    meta: &Meta,
    lang: Lang,
    table: &Table,
    caption: &str,
    title: &str,
    notes: &[&str],
) -> usize {
    let paper = Paper::A4;
    let (_, area) = begin(paper, lang, caption, "", notes);
    let cap = Table::capacity(area[3] - area[1]);
    let ranges = table.split(cap, cap);
    for (k, r) in ranges.iter().enumerate() {
        let cap_text = if k == 0 {
            caption.to_string()
        } else {
            format!("{caption} ({})", lang.t("continued"))
        };
        let (mut c, area) = begin(paper, lang, &cap_text, "", notes);
        let page = first_page + k;
        let x = area[0] + (area[2] - area[0] - table.width()) / 2.0;
        table.draw(&mut c, doc, page, x.max(area[0]), area[3], r.clone());
        let label = if k == 0 {
            title.to_string()
        } else {
            format!("{title} {}", k + 1)
        };
        finish(doc, page, paper, lang, meta, c, title, "-", pages, &label);
    }
    ranges.len()
}

/// How many A4 pages `table` takes.
pub fn table_page_count(table: &Table, lang: Lang, notes: &[&str]) -> usize {
    let (_, area) = begin(Paper::A4, lang, "", "", notes);
    let cap = Table::capacity(area[3] - area[1]);
    table.split(cap, cap).len()
}
