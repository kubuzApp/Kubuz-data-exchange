//! A part whose matrix is not a rotation (a mirror image, or a uniform
//! scale) cannot be a STEP placement: STEP placements are rigid. Such a
//! part gets its own solid with the pose baked into the geometry.

use crate::view::{det3, Curve, Solid, Surface, P3};

/// `solid` under `p' = m · p + t`, where `m` is `scale` × an orthogonal
/// matrix. A mirror image reverses every loop, turns every circle round and
/// flips the sense of the ruled faces, so the result is still outward.
pub fn bake(solid: &Solid, id: &str, m: [P3; 3], t: P3, scale: f64) -> Solid {
    let mirrored = det3(m) < 0.0;
    // Vectors (axes, normals) transform by the inverse transpose, which for
    // scale × orthogonal is the orthogonal part.
    let vec = |v: P3| unit(apply(m, v));
    let pt = |p: P3| add(apply(m, p), t);
    let flip = if mirrored { -1.0 } else { 1.0 };

    let mut out = Solid {
        id: id.to_string(),
        vertices: solid.vertices.iter().map(|&p| pt(p)).collect(),
        ..Default::default()
    };
    out.curves = solid
        .curves
        .iter()
        .map(|c| match c {
            Curve::Line => Curve::Line,
            Curve::Circle {
                center,
                axis,
                radius,
            } => Curve::Circle {
                center: pt(*center),
                axis: scaled(vec(*axis), flip),
                radius: radius * scale,
            },
            Curve::Bspline {
                degree,
                control_points,
                knots,
                multiplicities,
            } => Curve::Bspline {
                degree: *degree,
                control_points: control_points.iter().map(|&p| pt(p)).collect(),
                knots: knots.clone(),
                multiplicities: multiplicities.clone(),
            },
        })
        .collect();
    out.edges = solid.edges.clone();
    out.surfaces = solid
        .surfaces
        .iter()
        .map(|s| match s {
            Surface::Plane { origin, normal } => Surface::Plane {
                origin: pt(*origin),
                normal: vec(*normal),
            },
            Surface::Cylinder {
                origin,
                axis,
                radius,
            } => Surface::Cylinder {
                origin: pt(*origin),
                axis: vec(*axis),
                radius: radius * scale,
            },
            Surface::RuledBspline { top, bottom } => Surface::RuledBspline {
                top: *top,
                bottom: *bottom,
            },
        })
        .collect();
    out.faces = solid
        .faces
        .iter()
        .map(|f| {
            let mut face = f.clone();
            if mirrored {
                for lp in &mut face.loops {
                    lp.reverse();
                    for le in lp.iter_mut() {
                        le.forward = !le.forward;
                    }
                }
                if matches!(solid.surfaces[f.surface], Surface::RuledBspline { .. }) {
                    face.same_sense = !face.same_sense;
                }
            }
            face
        })
        .collect();
    out
}

fn apply(m: [P3; 3], v: P3) -> P3 {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scaled(v: P3, s: f64) -> P3 {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn unit(v: P3) -> P3 {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n < 1e-300 {
        return v;
    }
    scaled(v, 1.0 / n)
}
