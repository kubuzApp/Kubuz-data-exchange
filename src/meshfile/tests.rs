use serde_json::{json, Value};

use super::write_mesh_json;

fn fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(format!("{}/fixtures/{name}", env!("CARGO_MANIFEST_DIR")))
        .expect("fixture");
    serde_json::from_str(&text).expect("json")
}

/// A cube of 20 mm twice: once in place, once 100 mm along x, the second
/// mirrored, plus a part on an exact solid.
fn three_cubes() -> Value {
    let mut v = fixture("mesh_cube.view.json");
    v["parts"] = json!([
        { "id": "a", "solid": "cube", "material": "RVS 304", "color": "#c0c0c8",
          "metalness": 1.0, "roughness": 0.4, "group": "frame",
          "groupName": { "nl": "Frame", "en": "Frame" },
          "name": { "nl": "Kubus links", "en": "Left cube" } },
        { "id": "b", "solid": "cube", "material": "RVS 304", "color": "#c0c0c8",
          "metalness": 1.0, "roughness": 0.4, "position": [100, 0, 0], "group": "frame" },
        { "id": "c", "solid": "cube", "material": "Staal", "color": "#202020", "group": "alone",
          "position": [0, 100, 0], "matrix": [[-1, 0, 0], [0, 1, 0], [0, 0, 1]] },
    ]);
    v
}

fn run(view: &Value, format: &str) -> super::MeshOut {
    let options = json!({ "format": format, "name": "cubes", "lang": "nl" });
    write_mesh_json(&view.to_string(), &options.to_string()).expect("written")
}

fn glb_json(data: &[u8]) -> (Value, usize) {
    assert_eq!(&data[0..4], b"glTF");
    assert_eq!(u32::from_le_bytes(data[4..8].try_into().unwrap()), 2);
    assert_eq!(
        u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize,
        data.len()
    );
    let len = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    assert_eq!(&data[16..20], b"JSON");
    assert_eq!(len % 4, 0);
    let doc: Value = serde_json::from_slice(&data[20..20 + len]).unwrap();
    let bin_at = 20 + len;
    let bin_len = u32::from_le_bytes(data[bin_at..bin_at + 4].try_into().unwrap()) as usize;
    assert_eq!(&data[bin_at + 4..bin_at + 8], b"BIN\0");
    assert_eq!(bin_at + 8 + bin_len, data.len());
    assert_eq!(
        doc["buffers"][0]["byteLength"].as_u64().unwrap() as usize,
        bin_len
    );
    (doc, bin_len)
}

#[test]
fn glb_is_a_valid_container_with_a_metre_y_up_root() {
    let out = run(&three_cubes(), "glb");
    assert_eq!(out.files.len(), 1);
    assert_eq!(out.files[0].name, "cubes.glb");
    let (doc, _) = glb_json(&out.files[0].data);
    assert_eq!(doc["asset"]["version"], "2.0");
    let root = &doc["nodes"][0];
    assert_eq!(root["name"], "cubes");
    let m: Vec<f64> = serde_json::from_value(root["matrix"].clone()).unwrap();
    // Column 1 (y) goes to −z, column 2 (z) goes to +y.
    assert_eq!(m[6], -0.001);
    assert_eq!(m[9], 0.001);
}

#[test]
fn glb_groups_names_and_gives_every_part_its_own_mesh() {
    let out = run(&three_cubes(), "glb");
    let (doc, _) = glb_json(&out.files[0].data);
    let nodes = doc["nodes"].as_array().unwrap();
    let named = |n: &str| nodes.iter().find(|x| x["name"] == n).unwrap();
    let frame = named("Frame");
    assert_eq!(frame["children"].as_array().unwrap().len(), 2);
    let left = named("Kubus links");
    assert_eq!(left["extras"]["partId"], "a");
    assert_eq!(left["extras"]["material"], "RVS 304");
    assert!(left.get("matrix").is_none(), "identity is left out");
    let b = nodes.iter().find(|x| x["extras"]["partId"] == "b").unwrap();
    assert_ne!(
        b["mesh"], left["mesh"],
        "equal parts keep their own mesh, to animate each one"
    );
    let c = nodes.iter().find(|x| x["extras"]["partId"] == "c").unwrap();
    assert!(
        c["extras"].get("thickness").is_none(),
        "unknown thickness is left out"
    );
    // Parts a and b share a group; c is alone in its own, so it has no node.
    assert_eq!(nodes.len(), 5, "root, group, three parts");
    assert_ne!(c["mesh"], left["mesh"], "mirrored: baked");
    // Two looks; three parts, three meshes, each with its own positions and
    // indices (no normals).
    assert_eq!(doc["materials"].as_array().unwrap().len(), 2);
    assert_eq!(doc["meshes"].as_array().unwrap().len(), 3);
    assert_eq!(doc["accessors"].as_array().unwrap().len(), 6);
    let positions: Vec<&Value> = doc["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| &m["primitives"][0]["attributes"]["POSITION"])
        .collect();
    assert_ne!(positions[0], positions[1], "no shared buffers either");
    let pbr = &doc["materials"][0]["pbrMetallicRoughness"];
    assert_eq!(pbr["metallicFactor"], 1.0);
    assert_eq!(pbr["roughnessFactor"], 0.4);
}

#[test]
fn glb_mirrored_mesh_stays_outward() {
    let out = run(&three_cubes(), "glb");
    let data = &out.files[0].data;
    let (doc, _) = glb_json(data);
    // The baked cube: its first triangle's normal must point away from its
    // centre, as the original's does.
    let bin_at = 20 + u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize + 8;
    let read = |accessor: usize| -> Vec<u8> {
        let a = &doc["accessors"][accessor];
        let v = &doc["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
        let off = bin_at + v["byteOffset"].as_u64().unwrap() as usize;
        data[off..off + v["byteLength"].as_u64().unwrap() as usize].to_vec()
    };
    for mesh in doc["meshes"].as_array().unwrap() {
        let prim = &mesh["primitives"][0];
        let pos = read(prim["attributes"]["POSITION"].as_u64().unwrap() as usize);
        let idx = read(prim["indices"].as_u64().unwrap() as usize);
        let p = |i: usize| -> [f64; 3] {
            [0, 1, 2].map(|k| {
                let o = i * 12 + k * 4;
                f64::from(f32::from_le_bytes(pos[o..o + 4].try_into().unwrap()))
            })
        };
        let n = pos.len() / 12;
        let c = (0..n).fold([0.0; 3], |s, i| {
            [0, 1, 2].map(|k| s[k] + p(i)[k] / n as f64)
        });
        let mut outward = 0;
        for t in idx.chunks(12) {
            let [a, b, cc] = [0, 1, 2]
                .map(|k| p(u32::from_le_bytes(t[k * 4..k * 4 + 4].try_into().unwrap()) as usize));
            let nrm = super::facet_normal(&[a, b, cc]);
            let to = [0, 1, 2].map(|k| a[k] - c[k]);
            if nrm[0] * to[0] + nrm[1] * to[1] + nrm[2] * to[2] > 0.0 {
                outward += 1;
            }
        }
        assert_eq!(outward, idx.len() / 12, "{}", mesh["name"]);
    }
}

#[test]
fn stl_merges_every_part_in_millimetres() {
    let out = run(&three_cubes(), "stl");
    let data = &out.files[0].data;
    assert!(!data.starts_with(b"solid"));
    let n = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
    assert_eq!(n, 36);
    assert_eq!(data.len(), 84 + 50 * n);
    let mut max_x = f32::MIN;
    let mut min_x = f32::MAX;
    for t in 0..n {
        for v in 1..4 {
            let o = 84 + t * 50 + v * 12;
            let x = f32::from_le_bytes(data[o..o + 4].try_into().unwrap());
            max_x = max_x.max(x);
            min_x = min_x.min(x);
        }
    }
    assert_eq!(max_x, 120.0);
    assert_eq!(min_x, -20.0, "the mirrored cube");
}

#[test]
fn obj_has_an_object_per_part_y_up_and_its_mtl() {
    let out = run(&three_cubes(), "obj");
    assert_eq!(out.files.len(), 2);
    let obj = String::from_utf8(out.files[0].data.clone()).unwrap();
    let mtl = String::from_utf8(out.files[1].data.clone()).unwrap();
    assert!(obj.contains("mtllib cubes.mtl"));
    assert_eq!(obj.lines().filter(|l| l.starts_with("o ")).count(), 3);
    assert!(obj.contains("o Kubus links"));
    assert_eq!(obj.lines().filter(|l| l.starts_with("v ")).count(), 72);
    assert_eq!(obj.lines().filter(|l| l.starts_with("f ")).count(), 36);
    // The cube 100 mm along y sits 100 mm along −z once Y is up.
    assert!(obj
        .lines()
        .any(|l| l.starts_with("v ") && l.ends_with(" -120")));
    assert!(mtl.contains("newmtl m1_RVS_304"));
    assert!(mtl.contains("newmtl m2_Staal"));
    assert!(mtl.contains("Pm 1"));
    for l in obj.lines().filter(|l| l.starts_with("f ")) {
        for i in l.split_whitespace().skip(1) {
            let i: usize = i.parse().unwrap();
            assert!((1..=72).contains(&i));
        }
    }
}

#[test]
fn normals_are_written_when_the_mesh_has_them() {
    let mut v = three_cubes();
    let count = v["solids"][0]["mesh"]["vertices"].as_array().unwrap().len();
    v["solids"][0]["mesh"]["normals"] = json!(vec![[0.0, 0.0, 1.0]; count]);
    let (doc, _) = glb_json(&run(&v, "glb").files[0].data);
    assert!(doc["meshes"][0]["primitives"][0]["attributes"]["NORMAL"].is_u64());
    let obj = String::from_utf8(run(&v, "obj").files[0].data.clone()).unwrap();
    assert!(obj.contains("vn 0 1 0"), "z up becomes y up");
    assert!(obj.lines().any(|l| l.starts_with("f ") && l.contains("//")));
}

#[test]
fn exact_solids_are_skipped_with_a_reason() {
    let mut v = three_cubes();
    let exact = fixture("short_flange.view.json");
    v["solids"]
        .as_array_mut()
        .unwrap()
        .push(exact["solids"][0].clone());
    let id = exact["solids"][0]["id"].clone();
    v["parts"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "flange", "solid": id }));
    let out = run(&v, "glb");
    assert!(out.skipped.iter().any(|(i, _)| i == "flange"));
    assert!(out
        .skipped
        .iter()
        .any(|(i, m)| *i == id && m.contains("no triangles")));
}

#[test]
fn nothing_to_write_or_bad_input_is_an_error() {
    let exact = fixture("short_flange.view.json");
    let opts = json!({ "format": "stl", "name": "x" }).to_string();
    assert!(write_mesh_json(&exact.to_string(), &opts).is_err());
    let mut v = three_cubes();
    v["parts"][0]["color"] = json!("red");
    assert!(write_mesh_json(&v.to_string(), &opts).is_err());
    let bad = json!({ "format": "fbx", "name": "x" }).to_string();
    assert!(write_mesh_json(&three_cubes().to_string(), &bad).is_err());
    let path = json!({ "format": "glb", "name": "../x" }).to_string();
    assert!(write_mesh_json(&three_cubes().to_string(), &path).is_err());
    let mut v = three_cubes();
    v["parts"][0]["roughness"] = json!(2.0);
    assert!(write_mesh_json(&v.to_string(), &opts).is_err());
}

#[test]
fn json_result_carries_base64_files() {
    let out = run(&fixture("mesh_washers.view.json"), "glb");
    let v = out.to_json();
    assert_eq!(v["ok"], true);
    assert_eq!(v["files"][0]["mime"], "model/gltf-binary");
    assert!(v["files"][0]["data"]
        .as_str()
        .unwrap()
        .starts_with("Z2xURg"));
}
