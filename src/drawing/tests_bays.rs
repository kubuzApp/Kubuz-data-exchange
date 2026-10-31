//! The bays on the general arrangement: from the parts' `arrangement`
//! when the view has it, guessed (with a note) when it does not.

use super::write_drawings_json;
use serde_json::{json, Value};

fn cabinet() -> Value {
    let json = std::fs::read_to_string("fixtures/frame_cabinet.view.json").unwrap();
    serde_json::from_str(&json).unwrap()
}

/// Gives every part `tag(name, part)` returns as its `arrangement`.
fn tagged(mut v: Value, tag: impl Fn(&str, &Value) -> Option<Value>) -> Value {
    for p in v["parts"].as_array_mut().unwrap() {
        let name = p["name"]["nl"].as_str().unwrap_or("").to_string();
        if let Some(t) = tag(&name, p) {
            p["arrangement"] = t;
        }
    }
    v
}

fn x(p: &Value) -> f64 {
    p["position"][0].as_f64().unwrap()
}

/// The arrangement's bay and level dimensions as (kind, text, pitch),
/// where `bays` came from, and the notes' codes.
type Got = (
    Vec<(String, String, Option<f64>)>,
    Option<String>,
    Vec<String>,
);

fn bays_of(v: &Value) -> Got {
    let out = write_drawings_json(&v.to_string(), "{}").unwrap();
    // The JSON an app reads carries the same notes.
    assert_eq!(
        out.to_json()["notes"].as_array().unwrap().len(),
        out.notes.len()
    );
    let arr = out.arrangement.expect("arrangement");
    let dims = arr
        .dimensions
        .iter()
        .filter(|d| d.kind == "bay" || d.kind == "level")
        .map(|d| (d.kind.clone(), d.text.clone(), d.pitch))
        .collect();
    let codes = out.notes.iter().map(|n| n.code.clone()).collect();
    (dims, arr.bays, codes)
}

#[test]
fn a_view_without_roles_gets_guessed_bays_and_a_note() {
    let (dims, source, notes) = bays_of(&cabinet());
    assert!(dims.iter().any(|d| d.0 == "bay"));
    assert_eq!(source.as_deref(), Some("guessed"));
    assert_eq!(notes, ["bays-guessed"]);
}

#[test]
fn roles_from_the_view_give_the_same_cabinet_without_a_note() {
    let (guessed, _, _) = bays_of(&cabinet());
    let v = tagged(cabinet(), |name, _| match name {
        "Staander" => Some(json!({ "role": "upright" })),
        "Plank" | "Bovenblad" | "Plankkader dwarsbuis" => Some(json!({ "role": "level" })),
        _ => Some(json!({ "role": "other" })),
    });
    let (dims, source, notes) = bays_of(&v);
    assert_eq!(dims, guessed);
    assert_eq!(source.as_deref(), Some("view"));
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn the_view_wins_over_the_proportions() {
    // Only the outer uprights divide the front: one bay over the whole
    // width, whose levels are the left planks (named by bay index). A
    // plank naming bay 5 is left off, with a note.
    let v = tagged(cabinet(), |name, p| match name {
        "Staander" if x(p) < 100.0 || x(p) > 2200.0 => Some(json!({ "role": "upright" })),
        "Plank" if x(p) < 100.0 => Some(json!({ "role": "level", "bay": 0 })),
        "Plank" if x(p) > 1600.0 => Some(json!({ "role": "level", "bay": 5 })),
        _ => None,
    });
    let (dims, source, notes) = bays_of(&v);
    let bays: Vec<&str> = dims
        .iter()
        .filter(|d| d.0 == "bay")
        .map(|d| d.1.as_str())
        .collect();
    assert_eq!(bays, ["2260"]);
    assert_eq!(dims.iter().filter(|d| d.0 == "level").count(), 4);
    assert_eq!(source.as_deref(), Some("view"));
    assert_eq!(notes, ["bays-out-of-range"]);
}

#[test]
fn one_upright_line_draws_no_bays_and_says_why() {
    let v = tagged(cabinet(), |name, p| {
        (name == "Staander" && x(p) < 100.0).then(|| json!({ "role": "upright" }))
    });
    let (dims, source, notes) = bays_of(&v);
    assert!(dims.is_empty(), "{dims:?}");
    assert_eq!(source, None);
    assert_eq!(notes, ["bays-incomplete"]);
}

#[test]
fn a_balustrade_has_no_bays_and_no_note() {
    let json = std::fs::read_to_string("fixtures/balustrade.view.json").unwrap();
    let out = write_drawings_json(&json, "{}").unwrap();
    assert!(out.notes.is_empty());
    assert_eq!(out.arrangement.and_then(|a| a.bays), None);
}
