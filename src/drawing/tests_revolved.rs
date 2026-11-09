//! Cones and tori in the drawings: a transition (cone frustum shell) and
//! a round elbow (quarter torus tube).

use super::iso::{Dir, Scene};
use super::project::{project, topology, Basis};
use super::{write_drawings, Meta};
use crate::dxf::blank::Seg;
use crate::view::View;

fn view(name: &str) -> View {
    let json = std::fs::read_to_string(format!("fixtures/{name}.view.json")).unwrap();
    serde_json::from_str(&json).unwrap()
}

#[test]
fn a_cone_and_a_torus_get_a_folded_sheet_with_views() {
    for (name, size) in [
        ("cone_frustum", [100.0, 100.0, 40.0]),
        ("torus_elbow", [60.0, 60.0, 20.0]),
    ] {
        let out = write_drawings(&view(name), &Meta::default()).unwrap();
        let s = &out.sheets[0];
        assert_eq!(s.kind, "folded", "{name}");
        assert!(s.views.len() >= 3, "{name}: {} views", s.views.len());
        let mut got = s.size.expect("size");
        let mut want = size;
        got.sort_by(f64::total_cmp);
        want.sort_by(f64::total_cmp);
        for k in 0..3 {
            assert!((got[k] - want[k]).abs() < 0.5, "{name}: {got:?}");
        }
    }
}

/// Seen from the side, the frustum is a trapezoid: both rims are
/// straight lines across their full diameter and the cone's silhouette
/// runs from rim to rim.
#[test]
fn the_frustum_seen_from_the_side_is_a_trapezoid() {
    let v = view("cone_frustum");
    let s = &v.solids[0];
    let topo = topology(s);
    let b = Basis::new([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]);
    let p = project(s, &topo, b, 1.5);
    let lines: Vec<([f64; 2], [f64; 2])> = p
        .visible
        .iter()
        .filter_map(|g| match *g {
            Seg::Line { a, b } => Some((a, b)),
            _ => None,
        })
        .collect();
    let rim = |x: f64, half: f64| {
        lines.iter().any(|(a, b)| {
            (a[0] - x).abs() < 1e-6
                && (b[0] - x).abs() < 1e-6
                && ((a[1] - b[1]).abs() - 2.0 * half).abs() < 0.1
        })
    };
    assert!(rim(0.0, 50.0), "bottom rim");
    assert!(rim(40.0, 30.0), "top rim");
    // The silhouette: visible along the whole slant, from r 50 to r 30.
    for side in [1.0, -1.0] {
        let len: f64 = lines
            .iter()
            .filter(|(a, b)| {
                let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                m[0] > 0.5 && m[0] < 39.5 && (m[1] * side - (50.0 - m[0] / 2.0)).abs() < 0.2
            })
            .map(|(a, b)| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt())
            .sum();
        assert!((len - 44.72).abs() < 1.0, "side {side}: {len}");
    }
}

#[test]
fn a_torus_shows_in_the_pictorial() {
    let v = view("torus_elbow");
    let mut sc = Scene::new();
    sc.add_solid(&v.solids[0], &|p| p).unwrap();
    let iso = sc.render(Dir::Iso).unwrap();
    let len: f64 = iso
        .polylines
        .iter()
        .flat_map(|l| l.windows(2))
        .map(|w| ((w[0][0] - w[1][0]).powi(2) + (w[0][1] - w[1][1]).powi(2)).sqrt())
        .sum();
    // Both end rings and two silhouettes along the bend, at the least.
    assert!(len > 2.0 * 2.0 * std::f64::consts::PI * 10.0, "{len}");
}
#[test]
fn dbg_left() {
    let v = view("torus_elbow");
    let s = &v.solids[0];
    let topo = topology(s);
    let p = project(s, &topo, Basis::new([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]), 1.5);
    for g in &p.visible {
        eprintln!("vis {g:?}");
    }
    for g in &p.outline {
        eprintln!("out {g:?}");
    }
}
