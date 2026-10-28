//! `cargo run --example view2dxf -- flat.view.json out_dir`
//!
//! One DXF per distinct blank, named `<out_dir>/<part id>.dxf`, and a line
//! per part the view has no flat pattern for.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: view2dxf flat.view.json out_dir");
        std::process::exit(2);
    }
    let json = std::fs::read_to_string(&args[0]).expect("read view");
    // The kernel's CLI wraps the view in { ok, view }; accept both.
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    let view = v.get("view").cloned().unwrap_or(v);
    let out = match kubuz_data_exchange::write_dxf_json(&view.to_string()) {
        Ok(out) => out,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&args[1]).expect("out dir");
    for f in &out.files {
        let safe: String = f
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let path = format!("{}/{safe}.dxf", args[1]);
        std::fs::write(&path, &f.dxf).expect("write");
        println!("{path}: {} part(s)", f.part_ids.len());
    }
    for s in &out.skipped {
        eprintln!("{}: {}", s.id, s.message);
    }
    if out.files.is_empty() {
        std::process::exit(1);
    }
}
