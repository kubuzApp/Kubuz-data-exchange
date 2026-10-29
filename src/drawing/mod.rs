//! ISO detail drawings from the exact-geometry view, as one PDF/A-2b
//! dossier: a sheet per distinct flat blank (ISO 5457 sheet, ISO 7200
//! title block, ISO 128-2 lines, ISO 129-1 running dimensions, ISO 3098
//! lettering), with bookmarks and page labels by position number.
//!
//! The view must be flat, as for [`crate::dxf`]; parts that have no flat
//! pattern are reported under `skipped`. Folded views and bars come in a
//! later phase.
//!
//! Besides the PDF the export describes every sheet: its scale, where the
//! blank sits, and every dimension with the features it came from, so an
//! outside judge can rebuild the part from the drawing alone.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::dxf::{blank, Skipped};
use crate::pdf::{Doc, DocInfo};
use crate::view::{Pose, View, P3};

pub mod arrangement;
pub mod assembly;
pub mod dims;
pub mod features;
pub mod flat;
pub mod folded;
pub mod iso;
pub mod lists;
pub mod logo;
pub mod pattern;
pub mod project;
pub mod running;
pub mod section;
pub mod sheet;
pub mod text;

pub use flat::SheetReport;

/// What the CPQ knows and the drawings do not: who owns them, who made
/// and approved them, the date, the order. Every field has a default so
/// `{}` is a valid meta.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Meta {
    /// Legal owner (ISO 7200): the name in the title block when there is
    /// no logo, and the document's author.
    pub owner: String,
    pub created_by: String,
    pub approved_by: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub approved_date: String,
    /// The customer named in the title block.
    pub customer: String,
    /// The owner's address lines under its name in the title block.
    pub address: Vec<String>,
    /// Density in kg/m³ by material name, for the weight in the title
    /// block.
    pub densities: HashMap<String, f64>,
    /// The dossier's title (the configuration or order).
    pub title: String,
    /// Drawing number prefix; each sheet is `<number>-<position>`.
    pub number: String,
    pub revision: String,
    pub status: String,
    /// `nl` (default), `en` or `fr`.
    pub language: String,
    /// The general tolerance note, e.g. `ISO 2768-mK`.
    pub tolerances: String,
    pub logo: Option<logo::Logo>,
}

impl Default for Meta {
    fn default() -> Self {
        Meta {
            owner: "Kubuz".into(),
            created_by: "Kubuz CPQ".into(),
            approved_by: String::new(),
            date: "2026-01-01".into(),
            approved_date: String::new(),
            customer: String::new(),
            address: Vec::new(),
            densities: HashMap::new(),
            title: String::new(),
            number: String::new(),
            revision: "-".into(),
            status: String::new(),
            language: "nl".into(),
            tolerances: "ISO 2768-mK".into(),
            logo: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DrawingsExport {
    pub pdf: Vec<u8>,
    pub sheets: Vec<SheetReport>,
    pub skipped: Vec<Skipped>,
    /// The front matter: cover, assembly, parts list, laser list, bend
    /// list, with their pages.
    pub lists: Vec<ListReport>,
    pub assembly: Option<assembly::AssemblyReport>,
    /// The general arrangement: three views with the overall sizes and
    /// the pitch runs.
    pub arrangement: Option<arrangement::ArrangementReport>,
}

impl DrawingsExport {
    /// `{ "ok": true, "pdf": "<base64>", "sheets": [...], "lists": [...],
    /// "assembly": {...}, "arrangement": {...}, "skipped": [...] }`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ok": true,
            "pdf": base64(&self.pdf),
            "sheets": self.sheets,
            "lists": self.lists,
            "assembly": self.assembly,
            "arrangement": self.arrangement,
            "skipped": self.skipped.iter().map(|s| serde_json::json!({
                "id": s.id, "message": s.message,
            })).collect::<Vec<_>>(),
        })
    }
}

/// One section of the dossier's front matter and where it is.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListReport {
    /// `cover`, `assembly`, `arrangement`, `bom`, `laser` or `bend`.
    pub kind: String,
    pub title: String,
    /// First page, 0-based, and how many pages.
    pub page: usize,
    pub pages: usize,
}

/// The dossier of every sheet part in `view`: a cover, the assembly with
/// its parts list, the laser list, the bend list, then a sheet per
/// distinct blank in production order (laser only, then laser and
/// bending, by material and thickness). Parts that share a solid and a
/// pose kind share one sheet; its `partIds` say how many to make.
pub fn write_drawings(view: &View, meta: &Meta) -> Result<DrawingsExport, String> {
    if view.units != "mm" {
        return Err(format!("view units are {}, only mm is drawn", view.units));
    }
    if view.solids.is_empty() {
        return Err("view has no solids".into());
    }
    let lang = text::Lang::parse(&meta.language)?;
    if meta.date.len() != 10
        || !meta.date.chars().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
    {
        return Err(format!("meta.date {:?} is not YYYY-MM-DD", meta.date));
    }
    struct PartRef {
        id: String,
        solid: String,
        mirror: bool,
        scale: f64,
        material: String,
        thickness: f64,
        matrix: [P3; 3],
        position: P3,
    }
    let parts: Vec<PartRef> = if view.parts.is_empty() {
        view.solids
            .iter()
            .map(|s| PartRef {
                id: s.id.clone(),
                solid: s.id.clone(),
                mirror: false,
                scale: 1.0,
                material: String::new(),
                thickness: 0.0,
                matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                position: [0.0; 3],
            })
            .collect()
    } else {
        let mut v = Vec::with_capacity(view.parts.len());
        for p in &view.parts {
            let (mirror, scale) = match p.pose() {
                Pose::Rotation => (false, 1.0),
                Pose::Similar { scale, det } => (det < 0.0, scale),
                Pose::Other => {
                    return Err(format!(
                        "part {}: matrix must be a rotation, a mirror image or a uniform scale",
                        p.id
                    ))
                }
            };
            v.push(PartRef {
                id: p.id.clone(),
                solid: p.solid.clone(),
                mirror,
                scale,
                material: p.material.clone(),
                thickness: p.thickness * scale,
                matrix: p.rows(),
                position: p.position,
            });
        }
        v
    };

    // Group the parts by blank, keeping the view's order.
    struct Group {
        part_ids: Vec<String>,
        blank: Option<blank::Blank>,
        solid: usize,
        mirror: bool,
        scale: f64,
        material: String,
        thickness: f64,
        /// Instances: placement of each part.
        placed: Vec<([P3; 3], P3)>,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut by_blank: HashMap<(String, bool, i64), usize> = HashMap::new();
    let mut skipped = Vec::new();
    for p in parts {
        let key = (p.solid.clone(), p.mirror, (p.scale * 1e9).round() as i64);
        if let Some(&i) = by_blank.get(&key) {
            groups[i].part_ids.push(p.id);
            groups[i].placed.push((p.matrix, p.position));
            continue;
        }
        let Some(si) = view.solids.iter().position(|s| s.id == p.solid) else {
            return Err(format!("part {} points at unknown solid {}", p.id, p.solid));
        };
        let solid = &view.solids[si];
        if solid.mesh.is_none() && folded::is_folded(solid) {
            if p.mirror || (p.scale - 1.0).abs() > 1e-9 {
                skipped.push(Skipped {
                    id: p.id,
                    message: "a mirrored or scaled folded part is not drawn yet".into(),
                });
                continue;
            }
            by_blank.insert(key, groups.len());
            groups.push(Group {
                part_ids: vec![p.id],
                blank: None,
                solid: si,
                mirror: false,
                scale: 1.0,
                material: p.material,
                thickness: p.thickness,
                placed: vec![(p.matrix, p.position)],
            });
            continue;
        }
        match blank::flat_blank(solid, p.mirror, p.scale) {
            Ok(b) => {
                by_blank.insert(key, groups.len());
                groups.push(Group {
                    part_ids: vec![p.id],
                    blank: Some(b),
                    solid: si,
                    mirror: p.mirror,
                    scale: p.scale,
                    material: p.material,
                    thickness: p.thickness,
                    placed: vec![(p.matrix, p.position)],
                });
            }
            Err(message) => skipped.push(Skipped {
                id: p.id,
                message: format!("no flat pattern: {message}"),
            }),
        }
    }
    if groups.is_empty() {
        return Err(format!(
            "no part can be drawn ({})",
            skipped
                .iter()
                .map(|s| format!("{}: {}", s.id, s.message))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }

    // Production order: laser only, then laser and bending, then parts
    // from a folded view; within that by material, thickness and the
    // bend tool, then as the view lists them.
    let route_of = |g: &Group| match &g.blank {
        None => lists::Route::Folded,
        Some(b) if b.bends.is_empty() => lists::Route::Laser,
        Some(_) => lists::Route::LaserBend,
    };
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|&a, &b| {
        let (ga, gb) = (&groups[a], &groups[b]);
        let tool = |g: &Group| {
            g.blank
                .as_ref()
                .map(|bl| {
                    bl.bends
                        .iter()
                        .map(|x| x.inside_radius)
                        .fold(f64::INFINITY, f64::min)
                })
                .unwrap_or(f64::INFINITY)
        };
        route_of(ga)
            .cmp(&route_of(gb))
            .then(ga.material.to_lowercase().cmp(&gb.material.to_lowercase()))
            .then(ga.thickness.total_cmp(&gb.thickness))
            .then(tool(ga).total_cmp(&tool(gb)))
            .then(a.cmp(&b))
    });
    let mut pos_of = vec![0usize; groups.len()];
    for (k, &g) in order.iter().enumerate() {
        pos_of[g] = k + 1;
    }
    let mirror_of = |g: &Group| -> Option<usize> {
        if !g.mirror {
            return None;
        }
        groups
            .iter()
            .position(|o| o.solid == g.solid && !o.mirror && (o.scale - g.scale).abs() < 1e-9)
            .map(|i| pos_of[i])
    };

    // The rows of the lists, in position order, with what the lists say.
    let mut rows: Vec<lists::Row> = Vec::new();
    for &gi in &order {
        let g = &groups[gi];
        let mut row = lists::Row {
            pos: pos_of[gi],
            id: g.part_ids[0].clone(),
            part_ids: g.part_ids.clone(),
            material: g.material.clone(),
            thickness: g.thickness,
            blank: None,
            folded: None,
            weight: None,
            cut_length: None,
            area: None,
            holes: None,
            bends: Vec::new(),
            route: route_of(g),
            mirror_of: mirror_of(g),
            page: 0,
        };
        if let Some(b) = &g.blank {
            let f = features::features(b);
            row.blank = Some(b.max);
            row.folded = section::folded_extents(b, g.thickness);
            row.weight = flat::weight(b, &f, g.thickness, &g.material, meta);
            row.cut_length = Some(b.loops.iter().flatten().map(features::seg_len).sum::<f64>());
            row.area = Some(flat::net_area(&f));
            row.holes = Some(b.loops.len().saturating_sub(1));
            row.bends = b
                .bends
                .iter()
                .enumerate()
                .map(|(i, x)| lists::BendRow {
                    nr: i + 1,
                    angle_deg: x.angle_deg,
                    direction: x.direction,
                    inside_radius: x.inside_radius,
                    allowance: x.allowance,
                    length: (x.b[0] - x.a[0]).hypot(x.b[1] - x.a[1]),
                })
                .collect();
        }
        rows.push(row);
    }

    // The assembly picture: every part placed as the view places it, a
    // sheet part folded from its blank, anything else as its solid.
    let mut instances = Vec::new();
    for (k, &gi) in order.iter().enumerate() {
        let g = &groups[gi];
        for (m, p) in &g.placed {
            instances.push(assembly::Instance {
                row: k,
                // A plate without bends is its solid, thickness and
                // holes included; a bent part is its blank folded.
                shape: match &g.blank {
                    Some(b) if !b.bends.is_empty() => assembly::Shape::Blank {
                        blank: b,
                        thickness: g.thickness,
                    },
                    Some(b) => assembly::Shape::Plate {
                        solid: &view.solids[g.solid],
                        blank: b,
                    },
                    None => assembly::Shape::Solid(&view.solids[g.solid]),
                },
                matrix: *m,
                position: *p,
            });
        }
    }
    let scene = assembly::scene(&instances);
    let iso = scene
        .as_ref()
        .and_then(|(sc, owner)| sc.render(iso::Dir::Iso).map(|iso| (iso, owner)));
    let arrangement_plan = scene
        .as_ref()
        .filter(|_| iso.is_some())
        .and_then(|(sc, owner)| arrangement::plan(sc, owner, &rows));

    // Page plan: cover, assembly (+ parts list overflow), laser list,
    // bend list, then the sheets.
    let bom = lists::bom_table(&rows, lang);
    let laser = lists::laser_table(&rows, lang);
    let bend = lists::bend_table(&rows, lang);
    let laser_notes = [lang.t("laser_note")];
    let bend_notes = [lang.t("bend_list_note")];
    let bom_notes = [lang.t("balloon_note"), lang.t("order_note")];
    let a3_cap = {
        let (_, area) = lists::begin(sheet::Paper::A3, lang, "", "", &bom_notes);
        lists::Table::capacity(area[3] - area[1])
    };
    let a4_cap = {
        let (_, area) = lists::begin(sheet::Paper::A4, lang, "", "", &bom_notes);
        lists::Table::capacity(area[3] - area[1])
    };
    let bom_ranges = if iso.is_some() {
        bom.split(a3_cap, a4_cap)
    } else {
        bom.split(a4_cap, a4_cap)
    };
    let has_cut = rows.iter().any(|r| r.blank.is_some());
    let has_bends = rows.iter().any(|r| !r.bends.is_empty());
    let laser_pages = if has_cut {
        lists::table_page_count(&laser, lang, &laser_notes)
    } else {
        0
    };
    let bend_pages = if has_bends {
        lists::table_page_count(&bend, lang, &bend_notes)
    } else {
        0
    };
    let cover_page = 0;
    let assembly_page = 1;
    let arrangement_page = assembly_page + bom_ranges.len();
    let arrangement_pages = usize::from(arrangement_plan.is_some());
    let laser_page = arrangement_page + arrangement_pages;
    let bend_page = laser_page + laser_pages;
    let first_sheet = bend_page + bend_pages;
    let pages = first_sheet + groups.len();
    for (k, row) in rows.iter_mut().enumerate() {
        row.page = first_sheet + k;
    }

    let title = lists::dossier_title(meta, lang);
    let mut doc = Doc::new(DocInfo {
        title: title.clone(),
        author: meta.owner.clone(),
        creator: format!("kubuz-data-exchange {}", env!("CARGO_PKG_VERSION")),
        date: meta.date.clone(),
        lang: lang.code().into(),
    });
    for paper in sheet::Paper::ALL {
        let (w, h) = paper.size();
        let form = sheet::frame_form(paper, lang, meta.logo.as_ref(), &meta.owner, &meta.address)?;
        doc.add_form(paper.form_name(), w, h, form);
    }
    for _ in 0..first_sheet {
        doc.reserve_page();
    }

    // The sheets first: they know their folded sizes.
    let mut sheets = Vec::with_capacity(groups.len());
    for (k, &gi) in order.iter().enumerate() {
        let g = &groups[gi];
        let info = flat::SheetInfo {
            index: pos_of[gi],
            total: groups.len(),
            page: first_sheet + k,
            pages,
            title: &g.part_ids[0],
            part_ids: &g.part_ids,
            material: &g.material,
            thickness: g.thickness,
            mirror_of: rows[k].mirror_of,
            bom_page: Some(assembly_page),
        };
        let mut report = match &g.blank {
            Some(b) => flat::sheet(&mut doc, b, &info, meta, lang),
            None => folded::sheet(&mut doc, &view.solids[g.solid], &info, meta, lang),
        }
        .map_err(|e| format!("part {}: {e}", g.part_ids[0]))?;
        report.page = first_sheet + k;
        report.route = rows[k].route.name().into();
        report.mirror_of = rows[k].mirror_of;
        report.weight = rows[k].weight;
        report.cut_length = rows[k].cut_length;
        report.area = rows[k].area;
        report.holes = rows[k].holes;
        if rows[k].folded.is_none() {
            rows[k].folded = report.size;
        }
        sheets.push(report);
    }
    debug_assert_eq!(doc.page_count(), pages);

    // Front matter into the reserved pages.
    let bom = lists::bom_table(&rows, lang);
    let mut lists_report = Vec::new();
    lists_report.push(ListReport {
        kind: "cover".into(),
        title: lang.t("cover_title").into(),
        page: cover_page,
        pages: 1,
    });
    let mut assembly_report = None;
    match &iso {
        Some((iso, owner_row)) => {
            let r = assembly::sheet(
                &mut doc,
                assembly_page,
                pages,
                meta,
                lang,
                &rows,
                iso,
                owner_row,
                &bom,
                bom_ranges[0].clone(),
            );
            assembly_report = Some(r);
            lists_report.push(ListReport {
                kind: "assembly".into(),
                title: lang.t("assembly_title").into(),
                page: assembly_page,
                pages: 1,
            });
            // Overflow of the parts list on A4.
            for (k, r) in bom_ranges.iter().enumerate().skip(1) {
                let page = assembly_page + k;
                let caption = format!("{} ({})", lang.t("bom"), lang.t("continued"));
                let (mut c, area) = lists::begin(sheet::Paper::A4, lang, &caption, "", &bom_notes);
                let x = area[0] + (area[2] - area[0] - bom.width()) / 2.0;
                bom.draw(&mut c, &mut doc, page, x.max(area[0]), area[3], r.clone());
                lists::finish(
                    &mut doc,
                    page,
                    sheet::Paper::A4,
                    lang,
                    meta,
                    c,
                    lang.t("bom_title"),
                    "-",
                    pages,
                    &format!("{} {}", lang.t("bom_title"), k + 1),
                );
            }
            if bom_ranges.len() > 1 {
                lists_report.push(ListReport {
                    kind: "bom".into(),
                    title: lang.t("bom_title").into(),
                    page: assembly_page + 1,
                    pages: bom_ranges.len() - 1,
                });
            }
        }
        None => {
            let n = assembly::bom_only(&mut doc, assembly_page, pages, meta, lang, &bom);
            debug_assert_eq!(n, bom_ranges.len());
            lists_report.push(ListReport {
                kind: "bom".into(),
                title: lang.t("bom_title").into(),
                page: assembly_page,
                pages: n,
            });
        }
    }
    let arrangement_report = arrangement_plan.as_ref().map(|plan| {
        let r = arrangement::sheet(&mut doc, arrangement_page, pages, meta, lang, plan);
        lists_report.push(ListReport {
            kind: "arrangement".into(),
            title: lang.t("arrangement_title").into(),
            page: arrangement_page,
            pages: 1,
        });
        r
    });
    if has_cut {
        lists::table_pages(
            &mut doc,
            laser_page,
            pages,
            meta,
            lang,
            &laser,
            lang.t("laser_list"),
            lang.t("laser_title"),
            &laser_notes,
        );
        lists_report.push(ListReport {
            kind: "laser".into(),
            title: lang.t("laser_title").into(),
            page: laser_page,
            pages: laser_pages,
        });
    }
    if has_bends {
        lists::table_pages(
            &mut doc,
            bend_page,
            pages,
            meta,
            lang,
            &bend,
            lang.t("bend_list"),
            lang.t("bend_title"),
            &bend_notes,
        );
        lists_report.push(ListReport {
            kind: "bend".into(),
            title: lang.t("bend_title").into(),
            page: bend_page,
            pages: bend_pages,
        });
    }
    let mut sections: Vec<(String, usize, usize)> = lists_report
        .iter()
        .filter(|l| l.kind != "cover")
        .map(|l| (l.title.clone(), l.page, l.page + l.pages - 1))
        .collect();
    sections.push((lang.t("details").into(), first_sheet, pages - 1));
    lists::cover(
        &mut doc,
        cover_page,
        pages,
        meta,
        lang,
        &lists::CoverInfo {
            rows: &rows,
            sections,
        },
    );

    // Bookmarks: the front matter, then every sheet under "details".
    for l in &lists_report {
        doc.add_outline(&[], &l.title, l.page);
    }
    for (k, s) in sheets.iter().enumerate() {
        let g = &groups[order[k]];
        let name = if g.part_ids.len() > 1 {
            format!("{} · {} ({}×)", s.label, g.part_ids[0], g.part_ids.len())
        } else {
            format!("{} · {}", s.label, g.part_ids[0])
        };
        doc.add_outline(&[lang.t("details")], &name, s.page);
    }
    Ok(DrawingsExport {
        pdf: doc.finish(),
        sheets,
        skipped,
        lists: lists_report,
        assembly: assembly_report,
        arrangement: arrangement_report,
    })
}

/// [`write_drawings`] from the view's JSON text and the meta's (`{}` for
/// the defaults).
pub fn write_drawings_json(view_json: &str, meta_json: &str) -> Result<DrawingsExport, String> {
    let view: View = serde_json::from_str(view_json).map_err(|e| format!("view: {e}"))?;
    let meta_json = if meta_json.trim().is_empty() {
        "{}"
    } else {
        meta_json
    };
    let meta: Meta = serde_json::from_str(meta_json).map_err(|e| format!("meta: {e}"))?;
    write_drawings(&view, &meta)
}

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

#[cfg(test)]
mod tests;
