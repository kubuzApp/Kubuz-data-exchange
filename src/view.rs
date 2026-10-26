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

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Solid {
    pub id: String,
    pub vertices: Vec<P3>,
    pub curves: Vec<Curve>,
    pub edges: Vec<Edge>,
    pub surfaces: Vec<Surface>,
    pub faces: Vec<Face>,
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
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub id: String,
    pub solid: String,
    #[serde(default)]
    pub material: String,
    #[serde(default)]
    pub thickness: f64,
    #[serde(default)]
    pub position: P3,
    /// Degrees, R = Rx · Ry · Rz. Used only when `matrix` is missing.
    #[serde(default)]
    pub rotation: P3,
    /// R row by row.
    #[serde(default)]
    pub matrix: Option<[P3; 3]>,
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
