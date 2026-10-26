//! `cargo run --example view2step -- view.json out.step [name]`

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: view2step view.json out.step [name]");
        std::process::exit(2);
    }
    let json = std::fs::read_to_string(&args[0]).expect("read view");
    let name = args.get(2).cloned().unwrap_or_else(|| {
        std::path::Path::new(&args[1])
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string()
    });
    // The kernel's CLI wraps the view in { ok, view }; accept both.
    let v: serde_json::Value = serde_json::from_str(&json).expect("json");
    let view = v.get("view").cloned().unwrap_or(v);
    match kubuz_data_exchange::write_step_json(&view.to_string(), &name) {
        Ok(step) => std::fs::write(&args[1], step).expect("write"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
