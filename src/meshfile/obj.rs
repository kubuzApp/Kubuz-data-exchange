//! Wavefront OBJ and its MTL: one `o` per part with its name, placed in
//! millimetres and turned Y up, one `usemtl` per look. OBJ has no
//! instances, so equal parts are written out each time.

use std::fmt::Write;

use super::{Scene, SceneMaterial};
use crate::view::P3;

/// Z up to Y up: (x, y, z) → (x, z, −y).
fn y_up(p: P3) -> P3 {
    [p[0], p[2], -p[1]]
}

/// A name OBJ and MTL keep on one line, without spaces in material names.
fn line_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn material_name(i: usize, m: &SceneMaterial) -> String {
    let base: String = line_name(&m.name)
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if base.is_empty() {
        format!("m{}", i + 1)
    } else {
        format!("m{}_{base}", i + 1)
    }
}

fn num(x: f64) -> String {
    let s = format!("{x:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.to_string()
    }
}

pub fn write_obj(scene: &Scene, name: &str) -> (String, String) {
    let mut obj = String::new();
    let _ = writeln!(obj, "# Kubuz data exchange {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(obj, "# millimetres, Y up");
    let _ = writeln!(obj, "mtllib {name}.mtl");
    let (mut v_base, mut n_base) = (1usize, 1usize);
    for part in &scene.parts {
        let mesh = &scene.meshes[part.mesh];
        let _ = writeln!(obj, "o {}", line_name(&part.name));
        for &p in &mesh.positions {
            let [x, y, z] = y_up(part.place(p));
            let _ = writeln!(obj, "v {} {} {}", num(x), num(y), num(z));
        }
        let has_normals = !mesh.normals.is_empty();
        for &n in &mesh.normals {
            let [x, y, z] = y_up(part.turn(n));
            let _ = writeln!(obj, "vn {} {} {}", num(x), num(y), num(z));
        }
        let m = &scene.materials[part.material];
        let _ = writeln!(obj, "usemtl {}", material_name(part.material, m));
        let flip = part.mirrored();
        for t in &mesh.triangles {
            let t = if flip { [t[0], t[2], t[1]] } else { *t };
            let _ = write!(obj, "f");
            for i in t {
                let i = i as usize;
                if has_normals {
                    let _ = write!(obj, " {}//{}", v_base + i, n_base + i);
                } else {
                    let _ = write!(obj, " {}", v_base + i);
                }
            }
            obj.push('\n');
        }
        v_base += mesh.positions.len();
        if has_normals {
            n_base += mesh.normals.len();
        }
    }

    let mut mtl = String::new();
    let _ = writeln!(mtl, "# Kubuz data exchange {}", env!("CARGO_PKG_VERSION"));
    for (i, m) in scene.materials.iter().enumerate() {
        let [r, g, b] = m.rgb.map(num);
        let _ = writeln!(mtl, "\nnewmtl {}", material_name(i, m));
        let _ = writeln!(mtl, "Ka 0 0 0");
        let _ = writeln!(mtl, "Kd {r} {g} {b}");
        // A rough surface has a weak, wide highlight.
        let shine = (1.0 - m.roughness) * 200.0 + 4.0;
        let spec = num(0.2 + 0.6 * m.metalness);
        let _ = writeln!(mtl, "Ks {spec} {spec} {spec}");
        let _ = writeln!(mtl, "Ns {}", num(shine));
        let _ = writeln!(mtl, "d 1");
        let _ = writeln!(mtl, "illum 2");
        // PBR extension (Blender, three.js read it).
        let _ = writeln!(mtl, "Pm {}", num(m.metalness));
        let _ = writeln!(mtl, "Pr {}", num(m.roughness));
    }
    (obj, mtl)
}
