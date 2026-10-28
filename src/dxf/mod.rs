//! DXF flat patterns from the exact-geometry view: one file per distinct
//! sheet blank, with the outline and holes on the CUT layer and the
//! kernel's bend lines on the BEND layer. The ENGRAVE layer is declared
//! for markings and empty for now; no text is written, so nothing in the
//! file can be mistaken for a cut. Each bend's angle, direction and inside
//! radius come back in the result's `bends`, for a job sheet.
//!
//! The view must be flat (`exactGeometry(json, false)`): every zone a
//! plane on the blank, and each solid listing its `bends`. A folded solid,
//! a bar or a mesh has no flat pattern and is reported under `skipped`,
//! never written wrong.

use std::collections::HashMap;

use crate::view::{BendDirection, Pose, View};

pub mod blank;
pub mod writer;

/// One DXF file: the blank that `part_ids` share.
#[derive(Clone, Debug)]
pub struct DxfFile {
    /// The first part's id, for the file name.
    pub name: String,
    pub part_ids: Vec<String>,
    /// The blank's bends, in the order of their lines in the file.
    pub bends: Vec<BendInfo>,
    pub dxf: String,
}

/// What the drawing does not say about a bend line.
#[derive(Clone, Debug)]
pub struct BendInfo {
    pub id: String,
    pub angle_deg: f64,
    pub direction: BendDirection,
    pub inside_radius: f64,
}

#[derive(Clone, Debug)]
pub struct Skipped {
    pub id: String,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct DxfExport {
    pub files: Vec<DxfFile>,
    pub skipped: Vec<Skipped>,
}

impl DxfExport {
    /// `{ "ok": true, "files": [{ name, partIds, quantity, bends: [{ id,
    /// angleDeg, direction, insideRadius }], dxf }], "skipped": [{ id,
    /// message }] }`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ok": true,
            "files": self.files.iter().map(|f| serde_json::json!({
                "name": f.name,
                "partIds": f.part_ids,
                "quantity": f.part_ids.len(),
                "bends": f.bends.iter().map(|b| serde_json::json!({
                    "id": b.id,
                    "angleDeg": b.angle_deg,
                    "direction": b.direction.label().to_lowercase(),
                    "insideRadius": b.inside_radius,
                })).collect::<Vec<_>>(),
                "dxf": f.dxf,
            })).collect::<Vec<_>>(),
            "skipped": self.skipped.iter().map(|s| serde_json::json!({
                "id": s.id, "message": s.message,
            })).collect::<Vec<_>>(),
        })
    }
}

/// The flat patterns of every sheet part in `view`. Parts that share a
/// solid and a pose kind (mirrored or not, same scale) share one file;
/// its `part_ids` say how many to cut.
pub fn write_dxf(view: &View) -> Result<DxfExport, String> {
    if view.units != "mm" {
        return Err(format!("view units are {}, only mm is written", view.units));
    }
    if view.solids.is_empty() {
        return Err("view has no solids".into());
    }
    let parts: Vec<(String, String, bool, f64)> = if view.parts.is_empty() {
        view.solids
            .iter()
            .map(|s| (s.id.clone(), s.id.clone(), false, 1.0))
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
            v.push((p.id.clone(), p.solid.clone(), mirror, scale));
        }
        v
    };

    let mut out = DxfExport::default();
    let mut by_blank: HashMap<(String, bool, i64), usize> = HashMap::new();
    for (id, solid_id, mirror, scale) in parts {
        let key = (solid_id.clone(), mirror, (scale * 1e9).round() as i64);
        if let Some(&i) = by_blank.get(&key) {
            out.files[i].part_ids.push(id);
            continue;
        }
        let Some(solid) = view.solids.iter().find(|s| s.id == solid_id) else {
            return Err(format!("part {id} points at unknown solid {solid_id}"));
        };
        match blank::flat_blank(solid, mirror, scale) {
            Ok(blank) => {
                let dxf = writer::write(&blank);
                by_blank.insert(key, out.files.len());
                out.files.push(DxfFile {
                    name: id.clone(),
                    part_ids: vec![id],
                    bends: blank
                        .bends
                        .iter()
                        .map(|b| BendInfo {
                            id: b.id.clone(),
                            angle_deg: b.angle_deg,
                            direction: b.direction,
                            inside_radius: b.inside_radius,
                        })
                        .collect(),
                    dxf,
                });
            }
            Err(message) => out.skipped.push(Skipped {
                id,
                message: format!("no flat pattern: {message}"),
            }),
        }
    }
    Ok(out)
}

/// [`write_dxf`] from the view's JSON text.
pub fn write_dxf_json(view_json: &str) -> Result<DxfExport, String> {
    let view: View = serde_json::from_str(view_json).map_err(|e| format!("view: {e}"))?;
    write_dxf(&view)
}

#[cfg(test)]
mod tests;
