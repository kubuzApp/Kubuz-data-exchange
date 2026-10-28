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
            assert!(!s.dimensions.is_empty(), "{name} {}", s.title);
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
fn a_folded_view_is_refused_with_the_reason() {
    let err = write_drawings(&view("four_down"), &Meta::default()).unwrap_err();
    assert!(err.contains("no flat pattern"), "{err}");
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
