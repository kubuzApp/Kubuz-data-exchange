//! glTF 2.0 binary (GLB).
//!
//! - One root node named after the file: scale 0.001 and Z up to Y up, so
//!   every mesh and part matrix below it stays in millimetres, Z up.
//! - Under it a node per group (the template part) of two or more parts,
//!   and under that a node per part with its name and `extras { partId,
//!   material?, thickness? }`.
//! - Parts with the same solid and look share one glTF mesh; a part that is
//!   a mirror image or a scale gets its own baked mesh, because not every
//!   viewer turns the faces of a negative-determinant node round.
//! - One `pbrMetallicRoughness` material per colour, metalness and
//!   roughness. No textures, no compression.

use std::collections::HashMap;

use serde_json::{json, Value};

use super::{unit, Scene, SceneMesh, ScenePart};
use crate::view::P3;

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;

/// The root node's matrix, column by column: x stays, y = z, z = −y, × 0.001.
const ROOT: [f64; 16] = [
    0.001, 0.0, 0.0, 0.0, //
    0.0, 0.0, -0.001, 0.0, //
    0.0, 0.001, 0.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

#[derive(Default)]
struct Builder {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    meshes: Vec<Value>,
}

/// The accessors of one set of triangles.
#[derive(Clone, Copy)]
struct Geometry {
    position: usize,
    normal: Option<usize>,
    indices: usize,
}

impl Builder {
    fn view(&mut self, bytes: &[u8], target: u32) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let offset = self.bin.len();
        self.bin.extend_from_slice(bytes);
        self.views.push(json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": bytes.len(),
            "target": target,
        }));
        self.views.len() - 1
    }

    fn vec3(&mut self, values: &[P3], bounds: bool) -> usize {
        let mut bytes = Vec::with_capacity(values.len() * 12);
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for v in values {
            for (k, x) in v.iter().enumerate() {
                let f = *x as f32;
                min[k] = min[k].min(f);
                max[k] = max[k].max(f);
                bytes.extend_from_slice(&f.to_le_bytes());
            }
        }
        let view = self.view(&bytes, ARRAY_BUFFER);
        let mut a = json!({
            "bufferView": view,
            "componentType": FLOAT,
            "count": values.len(),
            "type": "VEC3",
        });
        if bounds {
            a["min"] = json!(min);
            a["max"] = json!(max);
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    fn geometry(&mut self, positions: &[P3], normals: &[P3], triangles: &[[u32; 3]]) -> Geometry {
        let position = self.vec3(positions, true);
        let normal = (!normals.is_empty()).then(|| self.vec3(normals, false));
        let mut bytes = Vec::with_capacity(triangles.len() * 12);
        for t in triangles {
            for i in t {
                bytes.extend_from_slice(&i.to_le_bytes());
            }
        }
        let view = self.view(&bytes, ELEMENT_ARRAY_BUFFER);
        self.accessors.push(json!({
            "bufferView": view,
            "componentType": UNSIGNED_INT,
            "count": triangles.len() * 3,
            "type": "SCALAR",
        }));
        Geometry {
            position,
            normal,
            indices: self.accessors.len() - 1,
        }
    }

    fn mesh(&mut self, name: &str, g: Geometry, material: usize) -> usize {
        let mut attributes = json!({ "POSITION": g.position });
        if let Some(n) = g.normal {
            attributes["NORMAL"] = json!(n);
        }
        self.meshes.push(json!({
            "name": name,
            "primitives": [{
                "attributes": attributes,
                "indices": g.indices,
                "material": material,
                "mode": 4,
            }],
        }));
        self.meshes.len() - 1
    }
}

/// `mesh` under the part's matrix (no translation), outward again when the
/// matrix mirrors.
fn baked(mesh: &SceneMesh, part: &ScenePart) -> (Vec<P3>, Vec<P3>, Vec<[u32; 3]>) {
    let m = part.matrix;
    let lin = |p: P3| [0, 1, 2].map(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2]);
    let positions = mesh.positions.iter().map(|&p| lin(p)).collect();
    let normals = mesh.normals.iter().map(|&n| unit(lin(n))).collect();
    let triangles = if part.mirrored() {
        mesh.triangles.iter().map(|t| [t[0], t[2], t[1]]).collect()
    } else {
        mesh.triangles.clone()
    };
    (positions, normals, triangles)
}

/// A node's matrix, column by column, from a rotation and a translation.
fn node_matrix(m: [P3; 3], t: P3) -> Vec<f64> {
    vec![
        m[0][0], m[1][0], m[2][0], 0.0, //
        m[0][1], m[1][1], m[2][1], 0.0, //
        m[0][2], m[1][2], m[2][2], 0.0, //
        t[0], t[1], t[2], 1.0,
    ]
}

const IDENTITY: [P3; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn is_identity(m: [P3; 3], t: P3) -> bool {
    t.iter().all(|x| x.abs() < 1e-12)
        && (0..3).all(|i| (0..3).all(|j| (m[i][j] - IDENTITY[i][j]).abs() < 1e-12))
}

pub fn write_glb(scene: &Scene, name: &str) -> Result<Vec<u8>, String> {
    let mut b = Builder::default();
    let mut shared_geometry: HashMap<usize, Geometry> = HashMap::new();
    let mut shared_mesh: HashMap<(usize, usize), usize> = HashMap::new();

    let materials: Vec<Value> = scene
        .materials
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let label = if m.name.is_empty() {
                format!("material {}", i + 1)
            } else {
                m.name.clone()
            };
            json!({
                "name": label,
                "pbrMetallicRoughness": {
                    "baseColorFactor": [m.rgb[0], m.rgb[1], m.rgb[2], 1.0],
                    "metallicFactor": m.metalness,
                    "roughnessFactor": m.roughness,
                },
            })
        })
        .collect();

    let mut nodes: Vec<Value> = vec![json!({ "name": name, "matrix": ROOT, "children": [] })];
    let mut group_node: HashMap<&str, usize> = HashMap::new();
    let mut group_size: HashMap<&str, usize> = HashMap::new();
    for part in &scene.parts {
        if let Some((id, _)) = &part.group {
            *group_size.entry(id.as_str()).or_default() += 1;
        }
    }
    for part in &scene.parts {
        let (mesh, matrix, position) = if part.is_rotation() {
            let geometry = *shared_geometry.entry(part.mesh).or_insert_with(|| {
                let m = &scene.meshes[part.mesh];
                b.geometry(&m.positions, &m.normals, &m.triangles)
            });
            let key = (part.mesh, part.material);
            let mesh = match shared_mesh.get(&key) {
                Some(&i) => i,
                None => {
                    let i = b.mesh(&part.name, geometry, part.material);
                    shared_mesh.insert(key, i);
                    i
                }
            };
            (mesh, part.matrix, part.position)
        } else {
            let (p, n, t) = baked(&scene.meshes[part.mesh], part);
            let geometry = b.geometry(&p, &n, &t);
            (
                b.mesh(&part.name, geometry, part.material),
                IDENTITY,
                part.position,
            )
        };

        let mut extras = json!({ "partId": part.id });
        if !part.material_name.is_empty() {
            extras["material"] = json!(part.material_name);
        }
        if part.thickness > 0.0 {
            extras["thickness"] = json!(part.thickness);
        }
        let mut node = json!({ "name": part.name, "mesh": mesh, "extras": extras });
        if !is_identity(matrix, position) {
            node["matrix"] = json!(node_matrix(matrix, position));
        }
        nodes.push(node);
        let index = nodes.len() - 1;

        // A group of one part is that part: no node of its own.
        let group = part
            .group
            .as_ref()
            .filter(|(id, _)| group_size[id.as_str()] > 1);
        let parent = match group {
            None => 0,
            Some((id, label)) => *group_node.entry(id.as_str()).or_insert_with(|| {
                nodes.push(json!({ "name": label, "children": [], "extras": { "group": id } }));
                let g = nodes.len() - 1;
                push_child(&mut nodes[0], g);
                g
            }),
        };
        push_child(&mut nodes[parent], index);
    }

    while !b.bin.len().is_multiple_of(4) {
        b.bin.push(0);
    }
    let doc = json!({
        "asset": {
            "version": "2.0",
            "generator": concat!("Kubuz data exchange ", env!("CARGO_PKG_VERSION")),
        },
        "scene": 0,
        "scenes": [{ "name": name, "nodes": [0] }],
        "nodes": nodes,
        "meshes": b.meshes,
        "materials": materials,
        "accessors": b.accessors,
        "bufferViews": b.views,
        "buffers": [{ "byteLength": b.bin.len() }],
    });
    let mut text = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }

    let total = 12 + 8 + text.len() + 8 + b.bin.len();
    let total = u32::try_from(total).map_err(|_| "model is over 4 GB".to_string())?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&text);
    out.extend_from_slice(&(b.bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&b.bin);
    Ok(out)
}

fn push_child(node: &mut Value, child: usize) {
    if let Some(children) = node["children"].as_array_mut() {
        children.push(json!(child));
    }
}
