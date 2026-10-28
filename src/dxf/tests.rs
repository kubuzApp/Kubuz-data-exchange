use super::blank::{arc_point, flat_blank, Seg, P2};
use super::{write_dxf, write_dxf_json, DxfExport};
use crate::view::View;

fn view(name: &str) -> View {
    let json = std::fs::read_to_string(format!("fixtures/{name}.view.json")).unwrap();
    serde_json::from_str(&json).unwrap()
}

fn export(name: &str) -> DxfExport {
    write_dxf(&view(name)).unwrap()
}

/// The DXF's entities by layer: (type, layer) pairs.
fn entities(dxf: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let lines: Vec<&str> = dxf.lines().collect();
    let mut i = lines.iter().position(|l| *l == "ENTITIES").unwrap() + 1;
    while i + 1 < lines.len() {
        if lines[i] == "0" && lines[i + 1] == "ENDSEC" {
            break;
        }
        if lines[i] == "0" {
            let kind = lines[i + 1].to_string();
            let mut layer = String::new();
            let mut j = i + 2;
            while j + 1 < lines.len() && lines[j] != "0" {
                if lines[j] == "8" {
                    layer = lines[j + 1].to_string();
                }
                j += 2;
            }
            out.push((kind, layer));
            i = j;
        } else {
            i += 2;
        }
    }
    out
}

fn texts(dxf: &str) -> Vec<String> {
    let lines: Vec<&str> = dxf.lines().collect();
    (0..lines.len() - 1)
        .filter(|&i| i % 2 == 0 && lines[i] == "1" && lines[i + 1] != "AC1009")
        .map(|i| lines[i + 1].to_string())
        .collect()
}

fn close(a: P2, b: P2) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

fn ends(s: &Seg) -> Vec<P2> {
    match *s {
        Seg::Line { a, b } => vec![a, b],
        Seg::Arc { c, r, start, end } => vec![arc_point(c, r, start), arc_point(c, r, end)],
        Seg::Circle { c, r } => vec![arc_point(c, r, 0.0)],
    }
}

/// Every loop is closed: consecutive segments share an end, and the last
/// shares one with the first. A lone circle is a loop of its own.
fn assert_closed(name: &str) {
    let v = view(name);
    for s in &v.solids {
        let b = flat_blank(s, false, 1.0).unwrap();
        assert!(!b.loops.is_empty(), "{name} {}", s.id);
        for lp in &b.loops {
            if lp.len() == 1 {
                assert!(matches!(lp[0], Seg::Circle { .. }), "{name}: {:?}", lp[0]);
                continue;
            }
            for (i, seg) in lp.iter().enumerate() {
                let next = &lp[(i + 1) % lp.len()];
                let joined = ends(seg)
                    .iter()
                    .any(|p| ends(next).iter().any(|q| close(*p, *q)));
                assert!(joined, "{name} {}: {seg:?} then {next:?}", s.id);
            }
        }
        assert!(b.min == [0.0, 0.0], "{name}: {:?}", b.min);
        assert!(b.max[0] > 0.0 && b.max[1] > 0.0);
    }
}

#[test]
fn every_flat_fixture_chains_into_closed_loops() {
    for n in [
        "four_down_flat",
        "deck_splash_flat",
        "mixed_channel_flat",
        "hex_pan_flat",
        "curved_board_flat",
    ] {
        assert_closed(n);
    }
}

#[test]
fn four_down_is_one_outline_with_holes_and_eight_bends() {
    let v = view("four_down_flat");
    let b = flat_blank(&v.solids[0], false, 1.0).unwrap();
    // The blank spans the solid's own x and y, moved to the origin.
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in &v.solids[0].vertices {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    assert!(close(b.max, [hi[0] - lo[0], hi[1] - lo[1]]), "{:?}", b.max);
    assert!(b.max[0] > 1000.0 && b.max[1] > 650.0, "{:?}", b.max);
    // The outline and one slot (four lines, four arcs) in the top.
    assert_eq!(b.loops.len(), 2, "{:?}", b.loops);
    let arcs = b
        .loops
        .iter()
        .flatten()
        .filter(|s| matches!(s, Seg::Arc { .. }))
        .count();
    assert_eq!(arcs, 4);
    assert_eq!(b.bends.len(), 8);
    assert!(b.bends.iter().all(|x| x.direction.label() == "DOWN"));

    let out = export("four_down_flat");
    assert_eq!(out.files.len(), 1);
    assert!(out.skipped.is_empty());
    let f = &out.files[0];
    assert_eq!(f.name, "Top");
    assert_eq!(f.part_ids, vec!["Top"]);
    let es = entities(&f.dxf);
    let on = |layer: &str, kind: &str| es.iter().filter(|(k, l)| k == kind && l == layer).count();
    assert_eq!(on("BEND", "LINE"), 8);
    assert_eq!(on("BEND", "TEXT"), 8);
    assert_eq!(on("ENGRAVE", "TEXT"), 1);
    assert!(on("CUT", "LINE") > 8);
    assert_eq!(on("CUT", "ARC"), 4);
    let odd: Vec<_> = es
        .iter()
        .filter(|(_, l)| !["CUT", "BEND", "ENGRAVE"].contains(&l.as_str()))
        .collect();
    assert!(odd.is_empty(), "{odd:?}");
    let ts = texts(&f.dxf);
    assert!(ts.contains(&"Top".to_string()), "{ts:?}");
    assert_eq!(
        ts.iter().filter(|t| *t == "DOWN 90%%d R1.5").count(),
        8,
        "{ts:?}"
    );
    assert!(f.dxf.starts_with("0\nSECTION\n2\nHEADER\n"));
    assert!(f.dxf.ends_with("0\nEOF\n"));
    for layer in ["CUT", "BEND", "ENGRAVE"] {
        assert!(f.dxf.contains(&format!("0\nLAYER\n2\n{layer}\n")));
    }
}

#[test]
fn a_channel_notes_up_and_down_with_their_angles() {
    let out = export("mixed_channel_flat");
    let ts = texts(&out.files[0].dxf);
    assert!(ts.iter().any(|t| t.starts_with("DOWN 45%%d")), "{ts:?}");
    assert!(ts.iter().any(|t| t.starts_with("UP 60%%d")), "{ts:?}");
    // Its apron is cut round: arcs on the outline.
    let es = entities(&out.files[0].dxf);
    assert!(es.iter().any(|(k, l)| k == "ARC" && l == "CUT"));
}

#[test]
fn an_assembly_gives_one_file_per_distinct_blank() {
    let out = export("deck_splash_flat");
    assert_eq!(out.files.len(), 3, "{:?}", out.skipped);
    let names: Vec<&str> = out.files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Deck", "Splash", "Gusset"]);
    assert!(entities(&out.files[1].dxf).iter().all(|(_, l)| l != "BEND"));
}

#[test]
fn mirrored_scaled_and_turned_parts() {
    let out = export("four_down_flat_mirrored");
    assert!(out.skipped.is_empty(), "{:?}", out.skipped);
    // Top and Twin (turned 90°) share one blank; Mirror and Half each get
    // their own.
    assert_eq!(out.files.len(), 3);
    assert_eq!(out.files[0].part_ids, vec!["Top", "Twin"]);
    assert_eq!(out.files[1].part_ids, vec!["Mirror"]);
    assert_eq!(out.files[2].part_ids, vec!["Half"]);
    let v = view("four_down_flat_mirrored");
    let plain = flat_blank(&v.solids[0], false, 1.0).unwrap();
    let mirror = flat_blank(&v.solids[0], true, 1.0).unwrap();
    let half = flat_blank(&v.solids[0], false, 0.5).unwrap();
    assert!(close(plain.max, mirror.max));
    assert!(close(half.max, [plain.max[0] / 2.0, plain.max[1] / 2.0]));
    // The mirror image: every bend line of the mirror is a bend line of
    // the plain blank with x flipped, and folds the same way.
    for b in &mirror.bends {
        let flipped = [plain.max[0] - b.a[0], b.a[1]];
        let twin = plain
            .bends
            .iter()
            .find(|p| close(p.a, flipped) || close(p.b, flipped))
            .unwrap_or_else(|| panic!("no twin for {b:?}"));
        assert_eq!(twin.direction, b.direction);
    }
    // Arcs survive the mirror as counter-clockwise arcs: every loop is closed.
    for lp in &mirror.loops {
        for (i, seg) in lp.iter().enumerate() {
            let next = &lp[(i + 1) % lp.len()];
            if lp.len() == 1 {
                continue;
            }
            assert!(ends(seg)
                .iter()
                .any(|p| ends(next).iter().any(|q| close(*p, *q))));
        }
    }
    assert!((half.bends[0].inside_radius - 0.75).abs() < 1e-9);
}

#[test]
fn folded_views_bars_and_meshes_are_skipped_not_written() {
    let out = export("four_down");
    assert!(out.files.is_empty());
    assert_eq!(out.skipped.len(), 1);
    assert!(
        out.skipped[0].message.contains("folded"),
        "{}",
        out.skipped[0].message
    );
    let out = export("mesh_cube");
    assert!(out.files.is_empty());
    assert!(out.skipped[0].message.contains("mesh"));
    let out = export("roll_tube");
    assert!(out.files.is_empty(), "a roll's skins are cylinders");
}

#[test]
fn json_in_and_out() {
    assert!(write_dxf_json("{}").is_err());
    let json = std::fs::read_to_string("fixtures/four_down_flat.view.json").unwrap();
    let out = write_dxf_json(&json).unwrap().to_json();
    assert_eq!(out["ok"], true);
    assert_eq!(out["files"][0]["quantity"], 1);
    assert_eq!(out["files"][0]["partIds"][0], "Top");
    assert!(out["files"][0]["dxf"]
        .as_str()
        .unwrap()
        .contains("ENTITIES"));
}

#[test]
fn numbers_are_short_and_never_negative_zero() {
    use super::writer::num;
    assert_eq!(num(1.5), "1.5");
    assert_eq!(num(-0.0), "0");
    assert_eq!(num(-0.0000000001), "0");
    assert_eq!(num(1000.0), "1000");
    assert_eq!(num(1.6493361431346414), "1.649336143");
}
