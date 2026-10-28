use super::{write_drawings, write_drawings_json, Meta};
use crate::view::View;

fn view(name: &str) -> View {
    let json = std::fs::read_to_string(format!("fixtures/{name}.view.json")).unwrap();
    serde_json::from_str(&json).unwrap()
}

#[test]
fn every_flat_fixture_becomes_a_sheet() {
    for name in [
        "four_down_flat",
        "four_down_flat_mirrored",
        "hex_pan_flat",
        "mixed_channel_flat",
        "deck_splash_flat",
        "curved_board_flat",
    ] {
        let out = write_drawings(&view(name), &Meta::default()).unwrap();
        assert!(!out.sheets.is_empty(), "{name}");
        assert!(out.pdf.starts_with(b"%PDF-1.7"), "{name}");
        let per_sheet = out.pdf.len() / out.sheets.len();
        assert!(per_sheet < 40_000, "{name}: {per_sheet} bytes per sheet");
        for s in &out.sheets {
            assert!(
                !s.dimensions.is_empty() || !s.chains.is_empty(),
                "{name} {}",
                s.title
            );
            assert!(s.scale_factor <= 5.0);
        }
    }
}

#[test]
fn parts_sharing_a_blank_share_a_sheet() {
    let out = write_drawings(&view("deck_splash_flat"), &Meta::default()).unwrap();
    let total: usize = out.sheets.iter().map(|s| s.quantity).sum();
    assert_eq!(total, view("deck_splash_flat").parts.len());
}

#[test]
fn every_folded_fixture_becomes_a_sheet_with_views_and_bends() {
    for (name, bends, views) in [
        ("four_down", 8, 3),
        ("hex_pan", 6, 2),
        ("mixed_channel", 2, 4),
        ("deck_splash", 3, 3),
        ("short_flange", 1, 3),
        ("roll_tube", 0, 3),
    ] {
        let out = write_drawings(&view(name), &Meta::default()).unwrap();
        let first = view(name).parts[0].id.clone();
        let s = out.sheets.iter().find(|s| s.title == first).unwrap();
        assert_eq!(s.kind, "folded", "{name}");
        assert_eq!(s.bends.len(), bends, "{name}");
        assert_eq!(
            s.views.len(),
            views,
            "{name}: {:?}",
            s.views.iter().map(|v| &v.name).collect::<Vec<_>>()
        );
        assert!(s.size.is_some(), "{name}");
        assert!(out.pdf.len() / out.sheets.len() < 45_000, "{name}");
        // Every view's frame is right-handed and the extents are dimensioned.
        for v in &s.views {
            let [r, u, d] = [v.r, v.u, v.d];
            let c = [
                r[1] * u[2] - r[2] * u[1],
                r[2] * u[0] - r[0] * u[2],
                r[0] * u[1] - r[1] * u[0],
            ];
            assert!(
                (c[0] * d[0] + c[1] * d[1] + c[2] * d[2] - 1.0).abs() < 1e-6,
                "{name} {}",
                v.name
            );
        }
        let ords = s.dimensions.iter().filter(|d| d.kind == "ordinate").count();
        assert!(ords >= 6, "{name}: {ords} ordinates");
    }
    // The channel's 45° apron, with its hole, gets an auxiliary view of
    // its own zone and a diameter callout there.
    let out = write_drawings(&view("mixed_channel"), &Meta::default()).unwrap();
    let s = &out.sheets[0];
    let aux = s
        .views
        .iter()
        .find(|v| v.name == "aux A")
        .expect("aux view");
    assert_eq!(aux.partial.as_deref(), Some("Apron"));
    assert!(
        (aux.d[1] + aux.d[2]).abs() < 1e-6 && aux.d[1] < 0.0,
        "{:?}",
        aux.d
    );
    assert!(s
        .dimensions
        .iter()
        .any(|d| d.kind == "diameter" && d.text == "⌀12" && d.axis.as_deref() == Some("aux A")));
}

#[test]
fn a_mesh_view_is_refused_with_the_reason() {
    let err = write_drawings(&view("mesh_cube"), &Meta::default()).unwrap_err();
    assert!(err.contains("no part can be drawn"), "{err}");
    // A mirrored folded part is skipped with a message, not drawn.
    let out = write_drawings(&view("four_down_mirrored"), &Meta::default());
    match out {
        Ok(o) => assert!(
            o.skipped.iter().any(|m| m.message.contains("mirrored")),
            "{:?}",
            o.skipped.iter().map(|m| &m.message).collect::<Vec<_>>()
        ),
        Err(e) => assert!(e.contains("no part can be drawn"), "{e}"),
    }
    let bad = write_drawings_json("{}", "{}").unwrap_err();
    assert!(bad.starts_with("view:"), "{bad}");
    let bad_meta = write_drawings_json(
        &std::fs::read_to_string("fixtures/four_down_flat.view.json").unwrap(),
        r#"{ "date": "28/10/2026" }"#,
    )
    .unwrap_err();
    assert!(bad_meta.contains("YYYY-MM-DD"), "{bad_meta}");
}

#[test]
fn the_json_result_carries_the_pdf_in_base64() {
    assert_eq!(super::base64(b"PDF"), "UERG");
    assert_eq!(super::base64(b"PD"), "UEQ=");
    assert_eq!(super::base64(b"P"), "UA==");
    let out = write_drawings_json(
        &std::fs::read_to_string("fixtures/mixed_channel_flat.view.json").unwrap(),
        r#"{ "date": "2026-10-28", "language": "fr", "number": "KB-1" }"#,
    )
    .unwrap();
    let v = out.to_json();
    assert_eq!(v["ok"], true);
    assert!(v["pdf"].as_str().unwrap().starts_with("JVBERi0xLjc"));
    assert_eq!(v["sheets"][0]["label"], "Pos 1");
}

#[test]
fn the_dossier_opens_with_cover_assembly_and_lists_in_production_order() {
    let out = write_drawings(&view("deck_splash_flat"), &Meta::default()).unwrap();
    let kinds: Vec<&str> = out.lists.iter().map(|l| l.kind.as_str()).collect();
    assert_eq!(kinds, ["cover", "assembly", "laser", "bend"]);
    let first_sheet = out.lists.iter().map(|l| l.page + l.pages).max().unwrap();
    for (k, s) in out.sheets.iter().enumerate() {
        assert_eq!(s.page, first_sheet + k, "{}", s.title);
        assert_eq!(s.index, k + 1);
    }
    // Laser-only parts first, then the bent deck.
    let routes: Vec<&str> = out.sheets.iter().map(|s| s.route.as_str()).collect();
    assert_eq!(routes, ["laser", "laser", "laser+bend"]);
    assert!(out
        .sheets
        .iter()
        .all(|s| s.weight.is_some() && s.cut_length.is_some()));
    // The assembly: a balloon per position, and the picture is the folded
    // parts placed as the view places them: its extents on the picture
    // plane match the folded fixture's vertices projected the same way.
    let asm = out.assembly.expect("assembly");
    let mut pos: Vec<usize> = asm.balloons.iter().map(|b| b.pos).collect();
    pos.sort_unstable();
    assert_eq!(pos, [1, 2, 3]);
    let folded = view("deck_splash");
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in &folded.parts {
        let m = p.rows();
        let solid = folded.solids.iter().find(|s| s.id == p.solid).unwrap();
        for v in &solid.vertices {
            let w = [
                m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2] + p.position[0],
                m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2] + p.position[1],
                m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2] + p.position[2],
            ];
            let (q, _) = super::iso::project(w);
            for i in 0..2 {
                lo[i] = lo[i].min(q[i]);
                hi[i] = hi[i].max(q[i]);
            }
        }
    }
    let s = asm.scale_factor;
    let drawn = [
        (asm.bbox[2] - asm.bbox[0]) / s,
        (asm.bbox[3] - asm.bbox[1]) / s,
    ];
    for i in 0..2 {
        assert!(
            (drawn[i] - (hi[i] - lo[i])).abs() < 4.0,
            "axis {i}: drawn {} vs folded {}",
            drawn[i],
            hi[i] - lo[i]
        );
    }
    // A mirrored part says which position it mirrors.
    let out = write_drawings(&view("four_down_flat_mirrored"), &Meta::default()).unwrap();
    let mirrored = out
        .sheets
        .iter()
        .find(|s| s.mirror_of.is_some())
        .expect("mirror");
    let twin = out
        .sheets
        .iter()
        .find(|s| s.index == mirrored.mirror_of.unwrap())
        .unwrap();
    assert_eq!(twin.blank, mirrored.blank);
    assert!(out.pdf.len() < 25_000 + 6_000 * (out.sheets.len() + 4));
}
