//! Binary STL: every part placed and merged into one body, millimetres,
//! Z up, the facet normal computed from the outward triangle. No colour, no
//! names: a slicer needs neither.

use super::{facet_normal, Scene};

pub fn write_stl(scene: &Scene, name: &str) -> Vec<u8> {
    let triangles: Vec<_> = scene
        .parts
        .iter()
        .flat_map(|p| scene.world_triangles(p))
        .collect();
    let mut out = Vec::with_capacity(84 + triangles.len() * 50);
    // The header must not start with "solid": readers take that as ASCII.
    let mut header = format!("Kubuz data exchange, mm, {name}").into_bytes();
    header.resize(80, b' ');
    out.extend_from_slice(&header);
    out.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
    for t in &triangles {
        let n = facet_normal(t);
        for v in std::iter::once(&n).chain(t.iter()) {
            for x in v {
                out.extend_from_slice(&(*x as f32).to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}
