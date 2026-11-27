//! Mesh files from a view whose solids are triangle meshes: **GLB**
//! (glTF 2.0, binary), **STL** (binary) and **OBJ** with its MTL.
//!
//! These are for looking at, rendering and printing a product, not for
//! making it: a cylinder is facets, so nothing here unfolds or cuts. STEP
//! and DXF stay the production files.
//!
//! Units and axes, per format's own habit:
//!
//! - GLB: metres, Y up. The meshes stay in millimetres, Z up; the root node
//!   scales by 0.001 and turns Z up into Y up.
//! - STL: millimetres, Z up, every part placed and merged.
//! - OBJ: millimetres, Y up, one object per part.
//!
//! A solid without a `mesh` (an exact B-rep) is skipped with a reason, and
//! so are the parts that use it: the app sends the kernel's triangles.

pub mod glb;
pub mod obj;
pub mod stl;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use serde::Deserialize;

use crate::view::{self, name_in, Part, Pose, View, P3};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    Glb,
    Stl,
    Obj,
}

/// `{ "format": "glb" | "stl" | "obj", "name": "…", "lang": "nl" }`.
#[derive(Clone, Debug, Deserialize)]
pub struct Options {
    pub format: Format,
    /// The file name without extension, and the GLB root node's name.
    #[serde(default = "default_name")]
    pub name: String,
    /// The language of the part and group names (`nl`, `en`, `fr`).
    #[serde(default = "default_lang")]
    pub lang: String,
}

fn default_name() -> String {
    "model".into()
}

fn default_lang() -> String {
    "nl".into()
}

/// One written file.
#[derive(Clone, Debug)]
pub struct MeshFile {
    pub name: String,
    pub mime: &'static str,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct MeshOut {
    pub files: Vec<MeshFile>,
    /// `(id, message)` of every solid and part left out.
    pub skipped: Vec<(String, String)>,
}

impl MeshOut {
    /// `{ ok, files: [{ name, mime, data (base64) }], skipped: [{ id, message }] }`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ok": true,
            "files": self.files.iter().map(|f| serde_json::json!({
                "name": f.name,
                "mime": f.mime,
                "data": crate::drawing::base64(&f.data),
            })).collect::<Vec<_>>(),
            "skipped": self.skipped.iter().map(|(id, message)| serde_json::json!({
                "id": id,
                "message": message,
            })).collect::<Vec<_>>(),
        })
    }
}

/// A solid's triangles in its own frame (millimetres).
#[derive(Clone, Debug)]
pub struct SceneMesh {
    pub positions: Vec<P3>,
    /// Unit normals per vertex, or empty.
    pub normals: Vec<P3>,
    pub triangles: Vec<[u32; 3]>,
}

/// A look: colour, metalness and roughness.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneMaterial {
    /// The material's name in the app (`RVS 304`), may be empty.
    pub name: String,
    pub rgb: [f64; 3],
    pub metalness: f64,
    pub roughness: f64,
}

/// A part ready to write: which mesh, which look, where.
#[derive(Clone, Debug)]
pub struct ScenePart {
    pub id: String,
    pub name: String,
    /// Group id and its name, when the part has one.
    pub group: Option<(String, String)>,
    pub mesh: usize,
    pub material: usize,
    /// Linear part of the pose: a rotation, or a mirror image or uniform
    /// scale of one.
    pub matrix: [P3; 3],
    pub position: P3,
    pub material_name: String,
    pub thickness: f64,
}

impl ScenePart {
    pub fn mirrored(&self) -> bool {
        view::det3(self.matrix) < 0.0
    }

    pub fn is_rotation(&self) -> bool {
        let p = Part {
            id: String::new(),
            solid: String::new(),
            material: String::new(),
            thickness: 0.0,
            color: None,
            metalness: None,
            roughness: None,
            position: [0.0; 3],
            rotation: [0.0; 3],
            matrix: Some(self.matrix),
            name: None,
            group: None,
            group_name: None,
            arrangement: None,
        };
        p.pose() == Pose::Rotation
    }

    /// A point of the part's mesh in the view's frame.
    pub fn place(&self, p: P3) -> P3 {
        let m = self.matrix;
        [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + self.position[i])
    }

    /// A normal of the part's mesh in the view's frame (unit length).
    pub fn turn(&self, n: P3) -> P3 {
        let m = self.matrix;
        unit([0, 1, 2].map(|i| m[i][0] * n[0] + m[i][1] * n[1] + m[i][2] * n[2]))
    }
}

/// Everything the three writers read, checked once.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub meshes: Vec<SceneMesh>,
    pub materials: Vec<SceneMaterial>,
    pub parts: Vec<ScenePart>,
    pub skipped: Vec<(String, String)>,
}

/// The look of a part without colour: brushed stainless steel.
const DEFAULT_RGB: [f64; 3] = [0.78, 0.78, 0.8];
const DEFAULT_METALNESS: f64 = 1.0;
const DEFAULT_ROUGHNESS: f64 = 0.45;

impl Scene {
    pub fn from_view(view: &View, lang: &str) -> Result<Scene, String> {
        if view.units != "mm" {
            return Err(format!("units must be mm, not {:?}", view.units));
        }
        let mut scene = Scene::default();
        let mut by_solid: HashMap<&str, Option<usize>> = HashMap::new();
        for solid in &view.solids {
            let slot = match solid_mesh(solid) {
                Ok(mesh) => {
                    scene.meshes.push(mesh);
                    Some(scene.meshes.len() - 1)
                }
                Err(message) => {
                    scene.skipped.push((solid.id.clone(), message));
                    None
                }
            };
            by_solid.insert(solid.id.as_str(), slot);
        }

        let implicit: Vec<Part>;
        let parts: &[Part] = if view.parts.is_empty() {
            implicit = view
                .solids
                .iter()
                .map(|s| Part {
                    id: s.id.clone(),
                    solid: s.id.clone(),
                    material: String::new(),
                    thickness: 0.0,
                    color: None,
                    metalness: None,
                    roughness: None,
                    position: [0.0; 3],
                    rotation: [0.0; 3],
                    matrix: None,
                    name: None,
                    group: None,
                    group_name: None,
                    arrangement: None,
                })
                .collect();
            &implicit
        } else {
            &view.parts
        };

        for part in parts {
            let Some(found) = by_solid.get(part.solid.as_str()) else {
                return Err(format!("part {}: no solid {:?}", part.id, part.solid));
            };
            let Some(mesh) = *found else {
                scene.skipped.push((
                    part.id.clone(),
                    format!("solid {} has no triangle mesh", part.solid),
                ));
                continue;
            };
            if part.pose() == Pose::Other {
                scene.skipped.push((
                    part.id.clone(),
                    "matrix must be a rotation, a mirror image or a uniform scale".into(),
                ));
                continue;
            }
            let material = scene.material_for(part)?;
            let group = part.group.as_ref().map(|g| {
                let label = name_in(&part.group_name, lang).unwrap_or_else(|| g.clone());
                (g.clone(), label)
            });
            scene.parts.push(ScenePart {
                id: part.id.clone(),
                name: name_in(&part.name, lang).unwrap_or_else(|| part.id.clone()),
                group,
                mesh,
                material,
                matrix: part.rows(),
                position: part.position,
                material_name: part.material.clone(),
                thickness: part.thickness,
            });
        }
        if scene.parts.is_empty() {
            let why = scene
                .skipped
                .first()
                .map(|(id, m)| format!(": {id}: {m}"))
                .unwrap_or_default();
            return Err(format!("no part to write{why}"));
        }
        Ok(scene)
    }

    fn material_for(&mut self, part: &Part) -> Result<usize, String> {
        let unit_range = |v: Option<f64>, default: f64, what: &str| -> Result<f64, String> {
            match v {
                None => Ok(default),
                Some(x) if (0.0..=1.0).contains(&x) => Ok(x),
                Some(x) => Err(format!("part {}: {what} {x} is not in 0..1", part.id)),
            }
        };
        let look = SceneMaterial {
            name: part.material.clone(),
            rgb: part.rgb()?.unwrap_or(DEFAULT_RGB),
            metalness: unit_range(part.metalness, DEFAULT_METALNESS, "metalness")?,
            roughness: unit_range(part.roughness, DEFAULT_ROUGHNESS, "roughness")?,
        };
        if let Some(i) = self.materials.iter().position(|m| *m == look) {
            return Ok(i);
        }
        self.materials.push(look);
        Ok(self.materials.len() - 1)
    }

    /// Every part's triangles in the view's frame, outward, with a facet
    /// normal: what STL and OBJ write.
    pub fn world_triangles(&self, part: &ScenePart) -> Vec<[P3; 3]> {
        let mesh = &self.meshes[part.mesh];
        let flip = part.mirrored();
        mesh.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| part.place(mesh.positions[i as usize]));
                if flip {
                    [a, c, b]
                } else {
                    [a, b, c]
                }
            })
            .collect()
    }
}

fn solid_mesh(solid: &view::Solid) -> Result<SceneMesh, String> {
    let Some(mesh) = &solid.mesh else {
        return Err("an exact solid has no triangles; send the kernel's mesh".into());
    };
    if mesh.triangles.is_empty() {
        return Err("mesh has no triangles".into());
    }
    let n = mesh.vertices.len();
    if n > u32::MAX as usize {
        return Err("mesh has too many vertices".into());
    }
    let mut triangles = Vec::with_capacity(mesh.triangles.len());
    for t in &mesh.triangles {
        if t.iter().any(|&i| i >= n) {
            return Err(format!("triangle {t:?} points past {n} vertices"));
        }
        triangles.push(t.map(|i| i as u32));
    }
    if mesh.vertices.iter().flatten().any(|v| !v.is_finite()) {
        return Err("mesh has a vertex that is not a number".into());
    }
    // Normals are a nicety: a wrong count or a zero normal drops them all
    // and the viewer shades the facets flat.
    let normals = if mesh.normals.len() == n {
        let unit: Vec<P3> = mesh.normals.iter().map(|&v| unit(v)).collect();
        if unit
            .iter()
            .all(|v| v.iter().all(|x| x.is_finite()) && len(*v) > 0.5)
        {
            unit
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    Ok(SceneMesh {
        positions: mesh.vertices.clone(),
        normals,
        triangles,
    })
}

pub fn len(v: P3) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

pub fn unit(v: P3) -> P3 {
    let l = len(v);
    if l > 0.0 {
        v.map(|x| x / l)
    } else {
        v
    }
}

/// The facet normal of a triangle, outward when it is counter-clockwise.
pub fn facet_normal(t: &[P3; 3]) -> P3 {
    let u = [0, 1, 2].map(|i| t[1][i] - t[0][i]);
    let w = [0, 1, 2].map(|i| t[2][i] - t[0][i]);
    unit([
        u[1] * w[2] - u[2] * w[1],
        u[2] * w[0] - u[0] * w[2],
        u[0] * w[1] - u[1] * w[0],
    ])
}

/// Write `view` in `options.format`.
pub fn write_mesh(view: &View, options: &Options) -> Result<MeshOut, String> {
    let scene = Scene::from_view(view, &options.lang)?;
    let files = match options.format {
        Format::Glb => vec![MeshFile {
            name: format!("{}.glb", options.name),
            mime: "model/gltf-binary",
            data: glb::write_glb(&scene, &options.name)?,
        }],
        Format::Stl => vec![MeshFile {
            name: format!("{}.stl", options.name),
            mime: "model/stl",
            data: stl::write_stl(&scene, &options.name),
        }],
        Format::Obj => {
            let (obj, mtl) = obj::write_obj(&scene, &options.name);
            vec![
                MeshFile {
                    name: format!("{}.obj", options.name),
                    mime: "model/obj",
                    data: obj.into_bytes(),
                },
                MeshFile {
                    name: format!("{}.mtl", options.name),
                    mime: "model/mtl",
                    data: mtl.into_bytes(),
                },
            ]
        }
    };
    Ok(MeshOut {
        files,
        skipped: scene.skipped,
    })
}

/// [`write_mesh`] from the view's JSON text and the options' JSON text.
pub fn write_mesh_json(view_json: &str, options_json: &str) -> Result<MeshOut, String> {
    let view: View = serde_json::from_str(view_json).map_err(|e| format!("view: {e}"))?;
    let options: Options =
        serde_json::from_str(options_json).map_err(|e| format!("options: {e}"))?;
    if options.name.is_empty() || options.name.contains(['/', '\\']) {
        return Err(format!(
            "options: name {:?} is not a file name",
            options.name
        ));
    }
    write_mesh(&view, &options)
}
