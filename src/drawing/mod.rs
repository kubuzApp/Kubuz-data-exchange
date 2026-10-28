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

use serde::Deserialize;

use crate::dxf::{blank, Skipped};
use crate::pdf::{Doc, DocInfo};
use crate::view::{Pose, View};

pub mod dims;
pub mod features;
pub mod flat;
pub mod folded;
pub mod iso;
pub mod logo;
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
}

impl DrawingsExport {
    /// `{ "ok": true, "pdf": "<base64>", "sheets": [...], "skipped": [...] }`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ok": true,
            "pdf": base64(&self.pdf),
            "sheets": self.sheets,
            "skipped": self.skipped.iter().map(|s| serde_json::json!({
                "id": s.id, "message": s.message,
            })).collect::<Vec<_>>(),
        })
    }
}

/// The dossier of every sheet part in `view`. Parts that share a solid
/// and a pose kind share one sheet; its `partIds` say how many to make.
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
            });
        }
        v
    };

    // Group the parts by blank, keeping the view's order.
    struct Group {
        part_ids: Vec<String>,
        blank: Option<blank::Blank>,
        solid: usize,
        material: String,
        thickness: f64,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut by_blank: HashMap<(String, bool, i64), usize> = HashMap::new();
    let mut skipped = Vec::new();
    for p in parts {
        let key = (p.solid.clone(), p.mirror, (p.scale * 1e9).round() as i64);
        if let Some(&i) = by_blank.get(&key) {
            groups[i].part_ids.push(p.id);
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
                material: p.material,
                thickness: p.thickness,
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
                    material: p.material,
                    thickness: p.thickness,
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

    let title = if meta.title.is_empty() {
        lang.t("dossier").to_string()
    } else {
        meta.title.clone()
    };
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
    let total = groups.len();
    let mut sheets = Vec::with_capacity(total);
    for (i, g) in groups.iter().enumerate() {
        let info = flat::SheetInfo {
            index: i + 1,
            total,
            title: &g.part_ids[0],
            part_ids: &g.part_ids,
            material: &g.material,
            thickness: g.thickness,
        };
        let report = match &g.blank {
            Some(b) => flat::sheet(&mut doc, b, &info, meta, lang),
            None => folded::sheet(&mut doc, &view.solids[g.solid], &info, meta, lang),
        }
        .map_err(|e| format!("part {}: {e}", g.part_ids[0]))?;
        let name = if g.part_ids.len() > 1 {
            format!(
                "{} · {} ({}×)",
                report.label,
                g.part_ids[0],
                g.part_ids.len()
            )
        } else {
            format!("{} · {}", report.label, g.part_ids[0])
        };
        doc.add_outline(&[lang.t("details")], &name, doc.page_count() - 1);
        sheets.push(report);
    }
    Ok(DrawingsExport {
        pdf: doc.finish(),
        sheets,
        skipped,
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
