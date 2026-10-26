//! Kubuz data exchange: file formats written from the Kubuz kernel's
//! **exact-geometry view**.
//!
//! The view is one JSON document: solids made of vertices, curves, edges,
//! surfaces and faces, plus the parts that place them. The kernel writes
//! it; this crate turns it into files. It knows nothing about sheet metal.
//!
//! Formats today: STEP AP214 ([`step`]). More come as the CPQ needs them.
//!
//! ```no_run
//! let view_json = std::fs::read_to_string("worktable.view.json").unwrap();
//! let file = kubuz_data_exchange::write_step_json(&view_json, "worktable").unwrap();
//! std::fs::write("worktable.step", file).unwrap();
//! ```

pub mod step;
pub mod view;

pub use step::{write_step, write_step_json};

#[cfg(feature = "wasm")]
mod wasm {
    use wasm_bindgen::prelude::*;

    /// STEP from the view's JSON text, named `name`. Returns JSON:
    /// `{ "ok": true, "step": "…" }` or `{ "ok": false, "error": "…" }`.
    #[wasm_bindgen(js_name = exportStep)]
    pub fn export_step(view_json: &str, name: &str) -> String {
        let v = match crate::step::write_step_json(view_json, name) {
            Ok(step) => serde_json::json!({ "ok": true, "step": step }),
            Err(e) => serde_json::json!({ "ok": false, "error": e }),
        };
        v.to_string()
    }
}
