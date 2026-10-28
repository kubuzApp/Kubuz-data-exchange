//! The exact-geometry view, as the Kubuz kernel writes it. This crate has no
//! sheet-metal knowledge: it only reads this document.
//!
//! Rules the kernel keeps and the writer relies on:
//!
//! - every edge is used exactly twice across a solid's faces, once each way;
//! - vertices are shared by index, never welded by tolerance;
//! - an edge runs along its curve's own direction from `start` to `end`
//!   (a circle turns counter-clockwise about its axis, a spline goes from
//!   its first control point to its last);
//! - a face's first loop is its outer bound, the rest are holes;
//! - identical parts share one solid.

use serde::Deserialize;

pub type P3 = [f64; 3];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    #[serde(default = "mm")]
    pub units: String,
    pub solids: Vec<Solid>,
    #[serde(default)]
    pub parts: Vec<Part>,
}

fn mm() -> String {
    "mm".into()
}

/// A solid is either the kernel's B-rep (vertices, curves, edges, surfaces,
/// faces) or, for geometry that was only ever a mesh, a closed triangle
/// `mesh`. The writer turns a mesh into planar faces first
/// ([`crate::mesh::mesh_to_brep`]).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Solid {
    pub id: String,
    #[serde(default)]
    pub vertices: Vec<P3>,
    #[serde(default)]
    pub curves: Vec<Curve>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub surfaces: Vec<Surface>,
    #[serde(default)]
    pub faces: Vec<Face>,
    #[serde(default)]
    pub mesh: Option<Mesh>,
    /// The brake bends of a flat blank (the kernel writes them only when
    /// the view is flat). The DXF writer draws them on the BEND layer.
    #[serde(default)]
    pub bends: Vec<Bend>,
}

/// One brake bend on a flat blank: the centre line of its bend zone on the
/// sheet's zero (w = 0), the unsigned fold angle, which way the flange
/// folds seen from the outer skin, the inside radius and the allowance.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bend {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub zone_id: String,
    pub angle_deg: f64,
    pub direction: BendDirection,
    #[serde(default)]
    pub inside_radius: f64,
    #[serde(default)]
    pub allowance: f64,
    pub line: [P3; 2],
}

/// `down` folds the flange away from the outer skin's normal; `up` folds
/// it toward whoever looks at the outer skin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BendDirection {
    Up,
    Down,
}

impl BendDirection {
    pub fn label(self) -> &'static str {
        match self {
            BendDirection::Up => "UP",
            BendDirection::Down => "DOWN",
        }
    }
}

/// A closed triangle mesh in millimetres: `triangles` index `vertices`,
/// counter-clockwise seen from outside. Vertices may be duplicated; the
/// writer welds them.
#[derive(Clone, Debug, Deserialize)]
pub struct Mesh {
    pub vertices: Vec<P3>,
    pub triangles: Vec<[usize; 3]>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Curve {
    Line,
    Circle {
        center: P3,
        axis: P3,
        radius: f64,
    },
    #[serde(rename_all = "camelCase")]
    Bspline {
        degree: usize,
        control_points: Vec<P3>,
        knots: Vec<f64>,
        multiplicities: Vec<usize>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Edge {
    pub start: usize,
    pub end: usize,
    pub curve: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Surface {
    Plane {
        origin: P3,
        normal: P3,
    },
    Cylinder {
        origin: P3,
        axis: P3,
        radius: f64,
    },
    /// Degree 1 between two B-spline curves (indices into `curves`) with the
    /// same degree and knots. u follows the curves, v runs from `top` to
    /// `bottom`; the normal is du × dv.
    RuledBspline {
        top: usize,
        bottom: usize,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Face {
    pub surface: usize,
    pub same_sense: bool,
    pub loops: Vec<Vec<LoopEdge>>,
    #[serde(default)]
    pub feature_id: String,
    #[serde(default)]
    pub zone_id: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct LoopEdge {
    pub edge: usize,
    pub forward: bool,
}

/// One placed part. World point = `matrix` · local point + `position`.
///
/// `matrix` is normally a rotation. It may also be a reflection or carry a
/// uniform scale: such a part gets its own baked solid in the file instead of
/// a placement ([`crate::step::bake`]).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub id: String,
    pub solid: String,
    #[serde(default)]
    pub material: String,
    #[serde(default)]
    pub thickness: f64,
    /// Display colour as `#rrggbb` (the kernel never writes one; an app
    /// adds it from its material library). Written as a STYLED_ITEM.
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub position: P3,
    /// Degrees, R = Rx · Ry · Rz. Used only when `matrix` is missing.
    #[serde(default)]
    pub rotation: P3,
    /// R row by row.
    #[serde(default)]
    pub matrix: Option<[P3; 3]>,
}

/// What a part's matrix is, once the rotation part is taken out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pose {
    /// A proper rotation: the part is placed, not copied.
    Rotation,
    /// `scale` × an orthogonal matrix with determinant `det` (±1).
    Similar { scale: f64, det: f64 },
    /// Shear or non-uniform scale: not something a solid survives.
    Other,
}

impl Part {
    /// The colour as red, green and blue in 0..1, or `None` when there is
    /// none. A string that is not `#rrggbb` is an error, not a wrong colour.
    pub fn rgb(&self) -> Result<Option<[f64; 3]>, String> {
        let Some(c) = &self.color else {
            return Ok(None);
        };
        let hex = c.trim().trim_start_matches('#');
        let bad = || format!("part {}: colour {c:?} is not #rrggbb", self.id);
        if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let ch = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
        Ok(Some([
            f64::from(ch(0)?) / 255.0,
            f64::from(ch(2)?) / 255.0,
            f64::from(ch(4)?) / 255.0,
        ]))
    }

    /// Classify [`Part::rows`].
    pub fn pose(&self) -> Pose {
        let m = self.rows();
        let col = |j: usize| [m[0][j], m[1][j], m[2][j]];
        let dot = |a: P3, b: P3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let lens = [0, 1, 2].map(|j| dot(col(j), col(j)).sqrt());
        let scale = lens[0];
        if scale < 1e-12 {
            return Pose::Other;
        }
        let tol = 1e-9 * scale;
        if (lens[1] - scale).abs() > tol || (lens[2] - scale).abs() > tol {
            return Pose::Other;
        }
        for (i, j) in [(0, 1), (1, 2), (0, 2)] {
            if dot(col(i), col(j)).abs() > tol * scale {
                return Pose::Other;
            }
        }
        let det = det3(m) / (scale * scale * scale);
        if (scale - 1.0).abs() <= 1e-9 && det > 0.0 {
            Pose::Rotation
        } else {
            Pose::Similar {
                scale,
                det: det.signum(),
            }
        }
    }
}

pub fn det3(m: [P3; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

impl Part {
    /// The rotation as rows of a 3 × 3 matrix.
    pub fn rows(&self) -> [P3; 3] {
        if let Some(m) = self.matrix {
            return m;
        }
        let [x, y, z] = self.rotation.map(f64::to_radians);
        let rx = [
            [1.0, 0.0, 0.0],
            [0.0, x.cos(), -x.sin()],
            [0.0, x.sin(), x.cos()],
        ];
        let ry = [
            [y.cos(), 0.0, y.sin()],
            [0.0, 1.0, 0.0],
            [-y.sin(), 0.0, y.cos()],
        ];
        let rz = [
            [z.cos(), -z.sin(), 0.0],
            [z.sin(), z.cos(), 0.0],
            [0.0, 0.0, 1.0],
        ];
        mul(mul(rx, ry), rz)
    }
}

fn mul(a: [P3; 3], b: [P3; 3]) -> [P3; 3] {
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}
