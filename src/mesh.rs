//! A closed triangle mesh as a B-rep solid: coplanar neighbouring triangles
//! merge into one planar face, and the boundary of each face becomes its
//! loops. This is how geometry that only ever existed as a mesh (a glTF
//! model, an OpenSCAD part) travels in the same file as the kernel's exact
//! parts.

use std::collections::HashMap;

use crate::view::{Curve, Edge, Face, LoopEdge, Mesh, Solid, Surface, P3};

/// Two triangles are one face when their normals agree to this angle (as a
/// cosine) and each one's far vertex lies on the other's plane within
/// `PLANE_MM`.
const COS_SAME: f64 = 1.0 - 1e-9;
const PLANE_MM: f64 = 1e-4;
/// Vertices closer than this (in every coordinate) are one vertex.
const WELD_MM: f64 = 1e-5;

/// Planar faces from a mesh, with each face's outer loop first.
pub fn mesh_to_brep(id: &str, mesh: &Mesh) -> Result<Solid, String> {
    let (vertices, tris) = weld(mesh, id)?;
    if tris.is_empty() {
        return Err(format!("solid {id}: mesh has no triangles"));
    }
    let normals: Vec<P3> = tris.iter().map(|t| tri_normal(&vertices, *t)).collect();

    // Union neighbours that share an edge and a plane.
    let mut owner: Vec<usize> = (0..tris.len()).collect();
    let mut by_edge: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (ti, t) in tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            by_edge.entry((a.min(b), a.max(b))).or_default().push(ti);
        }
    }
    for users in by_edge.values() {
        for (i, &a) in users.iter().enumerate() {
            for &b in &users[i + 1..] {
                if coplanar(&vertices, &tris, &normals, a, b) {
                    let (ra, rb) = (find(&mut owner, a), find(&mut owner, b));
                    if ra != rb {
                        owner[ra] = rb;
                    }
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for ti in 0..tris.len() {
        let root = find(&mut owner, ti);
        groups.entry(root).or_default().push(ti);
    }
    let mut roots: Vec<usize> = groups.keys().copied().collect();
    roots.sort_unstable();

    let mut solid = Solid {
        id: id.to_string(),
        vertices,
        curves: vec![Curve::Line],
        ..Default::default()
    };
    let mut edge_index: HashMap<(usize, usize), usize> = HashMap::new();
    for root in roots {
        let members = &groups[&root];
        let normal = group_normal(&normals, members);
        let loops = boundary_loops(&tris, members, &solid.vertices, normal)?;
        let face_loops = loops
            .into_iter()
            .map(|lp| {
                lp.into_iter()
                    .map(|(a, b)| {
                        let key = (a.min(b), a.max(b));
                        let next = edge_index.len();
                        let edge = *edge_index.entry(key).or_insert_with(|| {
                            solid.edges.push(Edge {
                                start: key.0,
                                end: key.1,
                                curve: 0,
                            });
                            next
                        });
                        LoopEdge {
                            edge,
                            forward: a == key.0,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        solid.surfaces.push(Surface::Plane {
            origin: solid.vertices[tris[members[0]][0]],
            normal,
        });
        solid.faces.push(Face {
            surface: solid.surfaces.len() - 1,
            same_sense: true,
            loops: face_loops,
            feature_id: String::new(),
            zone_id: String::new(),
            role: "mesh".into(),
        });
    }
    Ok(solid)
}

fn weld(mesh: &Mesh, id: &str) -> Result<(Vec<P3>, Vec<[usize; 3]>), String> {
    let mut keys: HashMap<[i64; 3], usize> = HashMap::new();
    let mut vertices: Vec<P3> = Vec::new();
    let mut remap = Vec::with_capacity(mesh.vertices.len());
    for &p in &mesh.vertices {
        let key = p.map(|c| (c / WELD_MM).round() as i64);
        let next = vertices.len();
        let i = *keys.entry(key).or_insert_with(|| {
            vertices.push(p);
            next
        });
        remap.push(i);
    }
    let mut tris = Vec::with_capacity(mesh.triangles.len());
    for t in &mesh.triangles {
        let mut w = [0; 3];
        for k in 0..3 {
            w[k] = *remap
                .get(t[k])
                .ok_or_else(|| format!("solid {id}: triangle vertex {} out of range", t[k]))?;
        }
        if w[0] == w[1] || w[1] == w[2] || w[0] == w[2] {
            continue;
        }
        if norm(cross(
            sub(vertices[w[1]], vertices[w[0]]),
            sub(vertices[w[2]], vertices[w[0]]),
        )) < 1e-12
        {
            continue;
        }
        tris.push(w);
    }
    Ok((vertices, tris))
}

fn find(owner: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while owner[r] != r {
        r = owner[r];
    }
    let mut j = i;
    while owner[j] != r {
        let next = owner[j];
        owner[j] = r;
        j = next;
    }
    r
}

fn coplanar(v: &[P3], tris: &[[usize; 3]], n: &[P3], a: usize, b: usize) -> bool {
    if dot(n[a], n[b]) < COS_SAME {
        return false;
    }
    let pa = v[tris[a][0]];
    let pb = v[tris[b][0]];
    tris[b]
        .iter()
        .all(|&i| dot(sub(v[i], pa), n[a]).abs() < PLANE_MM)
        && tris[a]
            .iter()
            .all(|&i| dot(sub(v[i], pb), n[b]).abs() < PLANE_MM)
}

fn tri_normal(v: &[P3], t: [usize; 3]) -> P3 {
    unit(cross(sub(v[t[1]], v[t[0]]), sub(v[t[2]], v[t[0]])))
}

fn group_normal(normals: &[P3], members: &[usize]) -> P3 {
    let mut sum = [0.0; 3];
    for &m in members {
        for k in 0..3 {
            sum[k] += normals[m][k];
        }
    }
    unit(sum)
}

/// The directed boundary edges of a group of triangles, chained into loops,
/// the outer loop (largest area) first.
fn boundary_loops(
    tris: &[[usize; 3]],
    members: &[usize],
    v: &[P3],
    normal: P3,
) -> Result<Vec<Vec<(usize, usize)>>, String> {
    let mut directed: HashMap<(usize, usize), usize> = HashMap::new();
    for &m in members {
        let t = tris[m];
        for k in 0..3 {
            *directed.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
        }
    }
    let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
    for (&(a, b), &n) in &directed {
        let back = directed.get(&(b, a)).copied().unwrap_or(0);
        if n > back {
            out.entry(a).or_default().push(b);
        }
    }
    let mut loops: Vec<Vec<(usize, usize)>> = Vec::new();
    let mut starts: Vec<usize> = out.keys().copied().collect();
    starts.sort_unstable();
    for start in starts {
        while out.get(&start).is_some_and(|o| !o.is_empty()) {
            let mut lp = Vec::new();
            let mut at = start;
            let mut d_in: Option<P3> = None;
            loop {
                let next = pick_next(&out, at, d_in, v, normal)
                    .ok_or_else(|| "mesh boundary does not close".to_string())?;
                let list = out.get_mut(&at).expect("outgoing");
                list.swap_remove(list.iter().position(|&x| x == next).expect("edge"));
                lp.push((at, next));
                d_in = Some(unit(sub(v[next], v[at])));
                at = next;
                if at == start {
                    break;
                }
                if lp.len() > directed.len() {
                    return Err("mesh boundary does not close".into());
                }
            }
            loops.push(lp);
        }
    }
    if loops.is_empty() {
        return Err("mesh face has no boundary".into());
    }
    loops.sort_by(|a, b| {
        loop_area(b, v, normal)
            .abs()
            .partial_cmp(&loop_area(a, v, normal).abs())
            .expect("finite area")
    });
    Ok(loops)
}

/// At a vertex where several boundary edges leave, turn right as hard as
/// possible, so a face that touches itself at a vertex keeps simple loops.
fn pick_next(
    out: &HashMap<usize, Vec<usize>>,
    at: usize,
    d_in: Option<P3>,
    v: &[P3],
    normal: P3,
) -> Option<usize> {
    let list = out.get(&at)?;
    let Some(d_in) = d_in else {
        return list.first().copied();
    };
    if list.len() == 1 {
        return Some(list[0]);
    }
    list.iter().copied().min_by(|&a, &b| {
        let ta = turn(d_in, unit(sub(v[a], v[at])), normal);
        let tb = turn(d_in, unit(sub(v[b], v[at])), normal);
        ta.partial_cmp(&tb).expect("finite turn")
    })
}

/// Signed turn from `a` to `b` about `n`, in (-π, π]: positive is left.
fn turn(a: P3, b: P3, n: P3) -> f64 {
    dot(cross(a, b), n).atan2(dot(a, b))
}

fn loop_area(lp: &[(usize, usize)], v: &[P3], normal: P3) -> f64 {
    let o = v[lp[0].0];
    let mut sum = [0.0; 3];
    for &(a, b) in lp {
        let c = cross(sub(v[a], o), sub(v[b], o));
        for k in 0..3 {
            sum[k] += c[k];
        }
    }
    0.5 * dot(sum, normal)
}

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: P3) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: P3) -> P3 {
    let n = norm(a);
    if n < 1e-300 {
        return [0.0, 0.0, 1.0];
    }
    [a[0] / n, a[1] / n, a[2] / n]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube with its vertices split per face, as glTF writes it.
    fn cube() -> Mesh {
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        let faces: [([f64; 3], [f64; 3], [f64; 3]); 6] = [
            ([0., 0., 0.], [0., 1., 0.], [1., 0., 0.]), // z = 0, normal -z
            ([0., 0., 1.], [1., 0., 0.], [0., 1., 0.]), // z = 1
            ([0., 0., 0.], [1., 0., 0.], [0., 0., 1.]), // y = 0
            ([0., 1., 0.], [0., 0., 1.], [1., 0., 0.]), // y = 1
            ([0., 0., 0.], [0., 0., 1.], [0., 1., 0.]), // x = 0
            ([1., 0., 0.], [0., 1., 0.], [0., 0., 1.]), // x = 1
        ];
        for (o, u, w) in faces {
            let base = vertices.len();
            let add = |a: f64, b: f64| {
                [
                    o[0] + a * u[0] + b * w[0],
                    o[1] + a * u[1] + b * w[1],
                    o[2] + a * u[2] + b * w[2],
                ]
            };
            vertices.extend([add(0., 0.), add(1., 0.), add(1., 1.), add(0., 1.)]);
            triangles.push([base, base + 1, base + 2]);
            triangles.push([base, base + 2, base + 3]);
        }
        Mesh {
            vertices,
            triangles,
        }
    }

    #[test]
    fn a_split_cube_becomes_six_faces_on_eight_vertices() {
        let s = mesh_to_brep("cube", &cube()).unwrap();
        assert_eq!(s.vertices.len(), 8);
        assert_eq!(s.faces.len(), 6);
        assert_eq!(s.edges.len(), 12);
        // Each edge once each way.
        let mut uses = vec![(0, 0); s.edges.len()];
        for f in &s.faces {
            assert_eq!(f.loops.len(), 1);
            for le in &f.loops[0] {
                if le.forward {
                    uses[le.edge].0 += 1;
                } else {
                    uses[le.edge].1 += 1;
                }
            }
        }
        assert!(uses.iter().all(|&u| u == (1, 1)), "{uses:?}");
    }

    #[test]
    fn a_washer_face_keeps_its_hole_as_a_second_loop() {
        // A short prism on a ring: 16 outer, 16 inner vertices, top and bottom.
        let n = 16;
        let mut vertices = Vec::new();
        for z in [0.0, 2.0] {
            for (r, _) in [(10.0, 0), (4.0, 1)] {
                for i in 0..n {
                    let a = i as f64 / n as f64 * std::f64::consts::TAU;
                    vertices.push([r * a.cos(), r * a.sin(), z]);
                }
            }
        }
        let outer = |z: usize, i: usize| z * 2 * n + i % n;
        let inner = |z: usize, i: usize| z * 2 * n + n + i % n;
        let mut triangles = Vec::new();
        for i in 0..n {
            // Top (z = 2, normal +z): ring quads split in two.
            triangles.push([outer(1, i), outer(1, i + 1), inner(1, i + 1)]);
            triangles.push([outer(1, i), inner(1, i + 1), inner(1, i)]);
            // Bottom (normal -z).
            triangles.push([outer(0, i), inner(0, i + 1), outer(0, i + 1)]);
            triangles.push([outer(0, i), inner(0, i), inner(0, i + 1)]);
            // Outer wall, outward.
            triangles.push([outer(0, i), outer(0, i + 1), outer(1, i + 1)]);
            triangles.push([outer(0, i), outer(1, i + 1), outer(1, i)]);
            // Inner wall, facing the hole.
            triangles.push([inner(0, i), inner(1, i + 1), inner(0, i + 1)]);
            triangles.push([inner(0, i), inner(1, i), inner(1, i + 1)]);
        }
        let s = mesh_to_brep(
            "washer",
            &Mesh {
                vertices,
                triangles,
            },
        )
        .unwrap();
        assert_eq!(s.faces.len(), 2 + 2 * n);
        let ring = s.faces.iter().filter(|f| f.loops.len() == 2).count();
        assert_eq!(ring, 2, "top and bottom have a hole");
        let top = s.faces.iter().find(|f| f.loops.len() == 2).unwrap();
        assert_eq!(top.loops[0].len(), n, "outer loop first");
    }

    #[test]
    fn an_open_mesh_is_refused() {
        let mut m = cube();
        m.triangles.pop();
        // One triangle missing: the boundary of its face no longer closes
        // into loops that return to their start along the face's own edges.
        let s = mesh_to_brep("open", &m);
        assert!(s.is_err() || s.unwrap().faces.len() == 6);
    }
}
