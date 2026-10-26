//! ISO 10303-21 (AP214) from the exact-geometry view: each solid one to one
//! onto STEP B-rep entities, and the parts as an assembly tree.

use std::collections::HashMap;

use crate::mesh::mesh_to_brep;
use crate::view::{self, Curve, Pose, Solid, Surface, View};
use writer::{any_perp, join, quote, sub, unit, StepWriter};

pub mod bake;
pub mod writer;

/// Write `view` as one STEP file named `name`. One part at the origin
/// becomes a plain part; anything else becomes an assembly whose children
/// carry the part ids. A part's `material` goes into its product's
/// description and a MATERIAL_DESIGNATION; its `color` becomes a
/// STYLED_ITEM on the solid. Parts that share a solid but not a colour get
/// the solid written once per colour.
pub fn write_step(view: &View, name: &str) -> Result<String, String> {
    if view.units != "mm" {
        return Err(format!("view units are {}, only mm is written", view.units));
    }
    if view.solids.is_empty() {
        return Err("view has no solids".into());
    }
    let mut w = StepWriter::default();
    let ctx = Context::new(&mut w);

    // Mesh solids become planar B-reps; a part whose pose is a mirror image
    // or a scale gets its own solid with the pose baked in.
    let mut solids: Vec<Solid> = Vec::with_capacity(view.solids.len());
    for s in &view.solids {
        solids.push(match &s.mesh {
            Some(mesh) => mesh_to_brep(&s.id, mesh)?,
            None => s.clone(),
        });
    }
    let mut parts: Vec<view::Part> = view.parts.clone();
    for p in &mut parts {
        match p.pose() {
            Pose::Rotation => {}
            Pose::Similar { scale, det: _ } => {
                let Some(src) = solids.iter().find(|s| s.id == p.solid) else {
                    return Err(format!("part {} points at unknown solid {}", p.id, p.solid));
                };
                let id = format!("{}@{}", src.id, p.id);
                let baked = bake::bake(src, &id, p.rows(), p.position, scale);
                solids.push(baked);
                p.solid = id;
                p.position = [0.0; 3];
                p.rotation = [0.0; 3];
                p.matrix = None;
            }
            Pose::Other => {
                return Err(format!(
                    "part {}: matrix must be a rotation, a mirror image or a uniform scale",
                    p.id
                ));
            }
        }
    }

    let parts: Vec<view::Part> = if parts.is_empty() {
        view.solids
            .iter()
            .map(|s| view::Part {
                id: s.id.clone(),
                solid: s.id.clone(),
                material: String::new(),
                thickness: 0.0,
                color: None,
                position: [0.0; 3],
                rotation: [0.0; 3],
                matrix: None,
            })
            .collect()
    } else {
        parts
    };

    // The B-rep of every solid, once per colour it is shown in: a style
    // hangs on the solid, so two colours need two solids.
    let mut breps: HashMap<(&str, String), usize> = HashMap::new();
    for p in &parts {
        let Some(s) = solids.iter().find(|s| s.id == p.solid) else {
            return Err(format!("part {} points at unknown solid {}", p.id, p.solid));
        };
        let rgb = p.rgb()?;
        let key = (p.solid.as_str(), colour_key(rgb));
        if let std::collections::hash_map::Entry::Vacant(e) = breps.entry(key) {
            let brep = solid_brep(&mut w, s)?;
            if let Some(rgb) = rgb {
                style(&mut w, &ctx, brep, rgb);
            }
            e.insert(brep);
        }
    }
    let brep_of = |p: &view::Part| breps[&(p.solid.as_str(), colour_key(p.rgb().ok().flatten()))];

    let single = parts.len() == 1 && is_identity(&parts[0]);
    if single {
        let p = &parts[0];
        let product = Product::new(&mut w, &ctx, &p.id, &p.material, Some(brep_of(p)));
        let _ = product;
        return Ok(w.finish(name));
    }

    // One product per solid, material and colour, named by the first part
    // that uses it.
    let mut products: HashMap<(&str, &str, String), Product> = HashMap::new();
    for p in &parts {
        let key = (p.solid.as_str(), p.material.as_str(), colour_key(p.rgb()?));
        if let std::collections::hash_map::Entry::Vacant(e) = products.entry(key) {
            let prod = Product::new(&mut w, &ctx, &p.id, &p.material, Some(brep_of(p)));
            e.insert(prod);
        }
    }

    // The root assembly: a shape representation holding the placement of
    // every child, and one usage occurrence per part.
    let origin = w.axis([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
    let mut placements = Vec::with_capacity(parts.len());
    for p in &parts {
        let m = p.rows();
        let z = [m[0][2], m[1][2], m[2][2]];
        let x = [m[0][0], m[1][0], m[2][0]];
        placements.push(w.axis(p.position, z, x));
    }
    let mut items = vec![origin];
    items.extend(&placements);
    let root_rep = w.add(format!(
        "SHAPE_REPRESENTATION('{}',({}),#{})",
        quote(name),
        join(&items),
        ctx.geom
    ));
    let root = Product::with_rep(&mut w, &ctx, name, "", root_rep);
    for (i, p) in parts.iter().enumerate() {
        let child = &products[&(p.solid.as_str(), p.material.as_str(), colour_key(p.rgb()?))];
        let nauo = w.add(format!(
            "NEXT_ASSEMBLY_USAGE_OCCURRENCE('{id}','{id}','',#{},#{},$)",
            root.definition,
            child.definition,
            id = quote(&p.id)
        ));
        let pds = w.add(format!("PRODUCT_DEFINITION_SHAPE('','',#{nauo})"));
        let idt = w.add(format!(
            "ITEM_DEFINED_TRANSFORMATION('','',#{origin},#{})",
            placements[i]
        ));
        let rr = w.add(format!(
            "( REPRESENTATION_RELATIONSHIP('','',#{},#{root_rep}) \
             REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#{idt}) \
             SHAPE_REPRESENTATION_RELATIONSHIP() )",
            child.rep
        ));
        w.add(format!(
            "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#{rr},#{pds})"
        ));
    }
    Ok(w.finish(name))
}

/// [`write_step`] from the view's JSON text.
pub fn write_step_json(view_json: &str, name: &str) -> Result<String, String> {
    let view: View = serde_json::from_str(view_json).map_err(|e| format!("view: {e}"))?;
    write_step(&view, name)
}

/// A colour as a map key: the hex the view gave, or empty for none.
fn colour_key(rgb: Option<[f64; 3]>) -> String {
    match rgb {
        Some(c) => format!("{:.4},{:.4},{:.4}", c[0], c[1], c[2]),
        None => String::new(),
    }
}

/// Colour `brep` on both sides: the STYLED_ITEM chain OpenCascade and
/// Inventor read, in a presentation representation on the same context.
fn style(w: &mut StepWriter, ctx: &Context, brep: usize, rgb: [f64; 3]) {
    let colour = w.add(format!(
        "COLOUR_RGB('',{},{},{})",
        writer::f(rgb[0]),
        writer::f(rgb[1]),
        writer::f(rgb[2])
    ));
    let fill_colour = w.add(format!("FILL_AREA_STYLE_COLOUR('',#{colour})"));
    let fill = w.add(format!("FILL_AREA_STYLE('',(#{fill_colour}))"));
    let surface_fill = w.add(format!("SURFACE_STYLE_FILL_AREA(#{fill})"));
    let side = w.add(format!("SURFACE_SIDE_STYLE('',(#{surface_fill}))"));
    let usage = w.add(format!("SURFACE_STYLE_USAGE(.BOTH.,#{side})"));
    let psa = w.add(format!("PRESENTATION_STYLE_ASSIGNMENT((#{usage}))"));
    let item = w.add(format!("STYLED_ITEM('',(#{psa}),#{brep})"));
    w.add(format!(
        "MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION('',(#{item}),#{})",
        ctx.geom
    ));
}

fn is_identity(p: &view::Part) -> bool {
    let m = p.rows();
    p.position.iter().all(|v| v.abs() < 1e-12)
        && (0..3).all(|i| (0..3).all(|j| (m[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 1e-12))
}

/// Units, uncertainty and the application protocol, shared by every product.
struct Context {
    geom: usize,
    app: usize,
    product_ctx: usize,
    definition_ctx: usize,
}

impl Context {
    fn new(w: &mut StepWriter) -> Self {
        let mm = w.add("( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) )".into());
        let rad = w.add("( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) )".into());
        let sr = w.add("( NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT() )".into());
        let unc = w.add(format!(
            "UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-05),#{mm},'distance_accuracy_value','')"
        ));
        let geom = w.add(format!(
            "( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{unc})) \
             GLOBAL_UNIT_ASSIGNED_CONTEXT((#{mm},#{rad},#{sr})) REPRESENTATION_CONTEXT('','') )"
        ));
        let app = w.add("APPLICATION_CONTEXT('automotive_design')".into());
        w.add(format!(
            "APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,#{app})"
        ));
        let product_ctx = w.add(format!("PRODUCT_CONTEXT('',#{app},'mechanical')"));
        let definition_ctx = w.add(format!(
            "PRODUCT_DEFINITION_CONTEXT('part definition',#{app},'design')"
        ));
        Self {
            geom,
            app,
            product_ctx,
            definition_ctx,
        }
    }
}

/// A product with one shape representation.
struct Product {
    definition: usize,
    rep: usize,
}

impl Product {
    fn new(
        w: &mut StepWriter,
        ctx: &Context,
        name: &str,
        material: &str,
        brep: Option<usize>,
    ) -> Self {
        let origin = w.axis([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        let rep = match brep {
            Some(b) => w.add(format!(
                "ADVANCED_BREP_SHAPE_REPRESENTATION('{}',(#{origin},#{b}),#{})",
                quote(name),
                ctx.geom
            )),
            None => w.add(format!(
                "SHAPE_REPRESENTATION('{}',(#{origin}),#{})",
                quote(name),
                ctx.geom
            )),
        };
        Self::with_rep(w, ctx, name, material, rep)
    }

    /// `material` is the product's description and, when given, a
    /// MATERIAL_DESIGNATION on its definition.
    fn with_rep(w: &mut StepWriter, ctx: &Context, name: &str, material: &str, rep: usize) -> Self {
        let name = quote(name);
        let prod = w.add(format!(
            "PRODUCT('{name}','{name}','{}',(#{}))",
            quote(material),
            ctx.product_ctx
        ));
        w.add(format!(
            "PRODUCT_RELATED_PRODUCT_CATEGORY('part','',(#{prod}))"
        ));
        let pdf = w.add(format!("PRODUCT_DEFINITION_FORMATION('','',#{prod})"));
        let definition = w.add(format!(
            "PRODUCT_DEFINITION('design','',#{pdf},#{})",
            ctx.definition_ctx
        ));
        let pds = w.add(format!("PRODUCT_DEFINITION_SHAPE('','',#{definition})"));
        w.add(format!("SHAPE_DEFINITION_REPRESENTATION(#{pds},#{rep})"));
        if !material.is_empty() {
            w.add(format!(
                "MATERIAL_DESIGNATION('{}',(#{definition}))",
                quote(material)
            ));
        }
        let _ = ctx.app;
        Self { definition, rep }
    }
}

/// The MANIFOLD_SOLID_BREP of one solid.
fn solid_brep(w: &mut StepWriter, s: &Solid) -> Result<usize, String> {
    let bad = |what: &str, i: usize| format!("solid {}: {what} {i} out of range", s.id);
    let vertex_ids: Vec<usize> = s
        .vertices
        .iter()
        .map(|&p| {
            let pt = w.point(p);
            w.add(format!("VERTEX_POINT('',#{pt})"))
        })
        .collect();

    let mut edge_ids = Vec::with_capacity(s.edges.len());
    for (i, e) in s.edges.iter().enumerate() {
        let a = *s
            .vertices
            .get(e.start)
            .ok_or_else(|| bad("vertex", e.start))?;
        let b = *s.vertices.get(e.end).ok_or_else(|| bad("vertex", e.end))?;
        let curve = match s.curves.get(e.curve).ok_or_else(|| bad("curve", e.curve))? {
            Curve::Line => w.line(a, b),
            Curve::Circle {
                center,
                axis,
                radius,
            } => w.circle(*center, *axis, sub(a, *center), *radius),
            Curve::Bspline {
                degree,
                control_points,
                knots,
                multiplicities,
            } => w.bspline_curve(*degree, control_points, knots, multiplicities),
        };
        let _ = i;
        edge_ids.push(w.add(format!(
            "EDGE_CURVE('',#{},#{},#{curve},.T.)",
            vertex_ids[e.start], vertex_ids[e.end]
        )));
    }

    let mut faces = Vec::with_capacity(s.faces.len());
    for face in &s.faces {
        let surf = match s
            .surfaces
            .get(face.surface)
            .ok_or_else(|| bad("surface", face.surface))?
        {
            Surface::Plane { origin, normal } => w.plane(*origin, *normal),
            Surface::Cylinder {
                origin,
                axis,
                radius,
            } => {
                // The reference direction only sets the parameter origin.
                let axis = unit(*axis);
                w.cylinder(*origin, axis, any_perp(axis), *radius)
            }
            Surface::RuledBspline { top, bottom } => {
                let t = s.curves.get(*top).ok_or_else(|| bad("curve", *top))?;
                let b = s.curves.get(*bottom).ok_or_else(|| bad("curve", *bottom))?;
                match (t, b) {
                    (
                        Curve::Bspline {
                            degree,
                            control_points: ct,
                            knots,
                            multiplicities,
                        },
                        Curve::Bspline {
                            control_points: cb, ..
                        },
                    ) => w.ruled_bspline(*degree, ct, cb, knots, multiplicities),
                    _ => return Err(format!("solid {}: ruled surface needs two splines", s.id)),
                }
            }
        };
        let mut bounds = Vec::with_capacity(face.loops.len());
        for (li, lp) in face.loops.iter().enumerate() {
            let mut oes = Vec::with_capacity(lp.len());
            for le in lp {
                let e = *edge_ids.get(le.edge).ok_or_else(|| bad("edge", le.edge))?;
                oes.push(w.oriented(e, le.forward));
            }
            let el = w.add(format!("EDGE_LOOP('',({}))", join(&oes)));
            let kind = if li == 0 {
                "FACE_OUTER_BOUND"
            } else {
                "FACE_BOUND"
            };
            bounds.push(w.add(format!("{kind}('',#{el},.T.)")));
        }
        faces.push(w.face(&bounds, surf, face.same_sense));
    }
    let shell = w.add(format!("CLOSED_SHELL('',({}))", join(&faces)));
    Ok(w.add(format!("MANIFOLD_SOLID_BREP('{}',#{shell})", quote(&s.id))))
}

#[cfg(test)]
mod tests {
    use super::write_step_json;

    const FIXTURES: &[(&str, &str)] = &[
        (
            "short_flange",
            include_str!("../../fixtures/short_flange.view.json"),
        ),
        (
            "four_down",
            include_str!("../../fixtures/four_down.view.json"),
        ),
        (
            "mixed_channel",
            include_str!("../../fixtures/mixed_channel.view.json"),
        ),
        ("hex_pan", include_str!("../../fixtures/hex_pan.view.json")),
        (
            "roll_tube",
            include_str!("../../fixtures/roll_tube.view.json"),
        ),
        (
            "deck_splash",
            include_str!("../../fixtures/deck_splash.view.json"),
        ),
    ];

    /// A closed solid uses every edge twice, once each way.
    fn each_edge_twice_each_way(text: &str) -> Result<(), String> {
        let mut uses: std::collections::HashMap<&str, (u32, u32)> = Default::default();
        for line in text.lines().filter(|l| l.contains("ORIENTED_EDGE(")) {
            let inner = &line[line.find("*,*,").unwrap() + 4..];
            let (edge, sense) = inner.split_once(',').unwrap();
            let u = uses.entry(edge).or_default();
            if sense.starts_with(".T.") {
                u.0 += 1;
            } else {
                u.1 += 1;
            }
        }
        match uses.iter().find(|(_, &u)| u != (1, 1)) {
            Some((e, u)) => Err(format!("edge {e} used {u:?}")),
            None => Ok(()),
        }
    }

    #[test]
    fn fixtures_write_closed_shells() {
        for (name, json) in FIXTURES {
            let text = write_step_json(json, name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(text.starts_with("ISO-10303-21;"), "{name}");
            assert!(text.contains("AUTOMOTIVE_DESIGN"), "{name}");
            assert!(text.contains("CLOSED_SHELL("), "{name}");
            each_edge_twice_each_way(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn one_part_at_the_origin_is_a_plain_part() {
        let text = write_step_json(FIXTURES[0].1, "flange").unwrap();
        assert!(!text.contains("NEXT_ASSEMBLY_USAGE_OCCURRENCE"));
        // The product carries the part's own id; the file name is the file's.
        assert!(text.contains("PRODUCT('Top'"), "{text}");
        assert!(text.contains("FILE_NAME('flange.step'"));
    }

    #[test]
    fn placed_parts_become_an_assembly_named_by_part_id() {
        let text = write_step_json(FIXTURES[5].1, "deck").unwrap();
        let nauo = text
            .lines()
            .filter(|l| l.contains("NEXT_ASSEMBLY_USAGE_OCCURRENCE"))
            .count();
        assert_eq!(nauo, 3, "one occurrence per part");
        assert!(text.contains("ITEM_DEFINED_TRANSFORMATION"));
        assert!(text.contains("PRODUCT('deck'"));
    }

    #[test]
    fn a_bad_view_is_an_error_not_a_file() {
        assert!(write_step_json("{}", "x").is_err());
        assert!(write_step_json(r#"{"units":"in","solids":[],"parts":[]}"#, "x").is_err());
        let dangling = r#"{"units":"mm","solids":[{"id":"s","vertices":[],"curves":[],"edges":[],"surfaces":[],"faces":[{"surface":0,"sameSense":true,"loops":[[{"edge":0,"forward":true}]]}]}],"parts":[]}"#;
        assert!(write_step_json(dangling, "x").is_err());
    }

    #[test]
    fn a_mirrored_part_is_baked_not_placed() {
        let mut v: serde_json::Value = serde_json::from_str(FIXTURES[1].1).unwrap();
        v["parts"][0]["matrix"] = serde_json::json!([[-1, 0, 0], [0, 1, 0], [0, 0, 1]]);
        v["parts"][0]["position"] = serde_json::json!([5, 0, 0]);
        let text = write_step_json(&v.to_string(), "mirror").unwrap();
        assert!(
            !text.contains("NEXT_ASSEMBLY_USAGE_OCCURRENCE"),
            "one part at the origin"
        );
        each_edge_twice_each_way(&text).unwrap();
        let shear = serde_json::json!([[1, 0.5, 0], [0, 1, 0], [0, 0, 1]]);
        v["parts"][0]["matrix"] = shear;
        assert!(write_step_json(&v.to_string(), "shear").is_err());
    }

    #[test]
    fn a_mesh_solid_is_written_as_planar_faces() {
        let json = include_str!("../../fixtures/mesh_cube.view.json");
        let text = write_step_json(json, "cube").unwrap();
        assert_eq!(text.matches("ADVANCED_FACE(").count(), 6);
        each_edge_twice_each_way(&text).unwrap();
    }

    /// A part's material is its product's description and a material
    /// designation; its colour a style on the solid. Two colours on one
    /// solid are two solids in the file; a bad colour is an error.
    #[test]
    fn materials_and_colours_are_written() {
        let mut v: serde_json::Value = serde_json::from_str(FIXTURES[5].1).unwrap();
        v["parts"][0]["color"] = "#1e90ff".into();
        v["parts"][0]["material"] = "304".into();
        // The second part shows the first part's solid in another colour.
        v["parts"][1]["solid"] = "solid-0".into();
        v["parts"][1]["color"] = "#FFD700".into();
        v["parts"][1]["material"] = "316".into();
        v["parts"][2]["material"] = "304".into();
        let text = write_step_json(&v.to_string(), "deck").unwrap();
        assert!(text.contains("MATERIAL_DESIGNATION('304'"), "{text}");
        assert!(text.contains("MATERIAL_DESIGNATION('316'"));
        assert!(text.contains("PRODUCT('Deck','Deck','304'"), "{text}");
        assert!(
            text.contains("COLOUR_RGB('',0.117647059,0.564705882,1.0)"),
            "{text}"
        );
        assert_eq!(text.matches("MANIFOLD_SOLID_BREP(").count(), 3);
        assert_eq!(text.matches("STYLED_ITEM(").count(), 2);
        assert_eq!(text.matches("NEXT_ASSEMBLY_USAGE_OCCURRENCE").count(), 3);
        each_edge_twice_each_way(&text).unwrap();
        v["parts"][0]["color"] = "blue".into();
        assert!(write_step_json(&v.to_string(), "deck").is_err());
    }

    #[test]
    fn quotes_in_names_are_doubled() {
        let text = write_step_json(FIXTURES[5].1, "Dennis' deck").unwrap();
        assert!(text.contains("PRODUCT('Dennis'' deck'"));
    }
}
