//! `cargo run --example view2mesh -- view.json glb|stl|obj out-dir [name] [lang]`
//!
//! Writes `<name>.glb`, `<name>.stl`, or `<name>.obj` + `<name>.mtl` into
//! `out-dir`. The view's solids must be triangle meshes.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: view2mesh view.json glb|stl|obj out-dir [name] [lang]");
        std::process::exit(2);
    }
    let json = std::fs::read_to_string(&args[0]).expect("read view");
    let name = args.get(3).cloned().unwrap_or_else(|| {
        let stem = std::path::Path::new(&args[0])
            .file_stem()
            .unwrap()
            .to_string_lossy();
        stem.trim_end_matches(".view").to_string()
    });
    let lang = args.get(4).cloned().unwrap_or_else(|| "nl".into());
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    let view = v.get("view").cloned().unwrap_or(v);
    let options = serde_json::json!({ "format": args[1], "name": name, "lang": lang });
    match kubuz_data_exchange::write_mesh_json(&view.to_string(), &options.to_string()) {
        Ok(out) => {
            std::fs::create_dir_all(&args[2]).expect("out dir");
            for f in &out.files {
                std::fs::write(std::path::Path::new(&args[2]).join(&f.name), &f.data)
                    .expect("write");
            }
            for (id, message) in &out.skipped {
                eprintln!("skipped {id}: {message}");
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
