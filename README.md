# Kubuz data exchange

File formats written from the Kubuz kernel's **exact-geometry view**. Today
that is STEP (ISO 10303-21, AP214) of the folded parts and DXF (R12) flat
patterns of the sheet blanks. More formats come as the CPQ needs them.

The kernel knows sheet metal: it turns a programme into exact faces (planes,
cylinders, ruled spline strips) and edges (lines, circles, cubic splines),
with shared vertices and each part's pose. That is the view. This repo knows
file syntax: it maps the view one to one onto STEP entities and writes the
parts as an assembly tree. Neither side knows the other's job, and nothing
runs on a server. It is one small Rust crate, also built as WebAssembly for
the browser.

## Use it

In TypeScript, next to the kernel:

```ts
import { exactGeometry } from "@kubuzapp/kernel";
import { exportStep } from "@kubuzapp/data-exchange";

const view = await exactGeometry(programJson);
if (view.ok) {
  const file = await exportStep(view.view, "worktable");
  if (file.ok) saveAs("worktable.step", file.step);
}
```

In Rust:

```rust
let file = kubuz_data_exchange::write_step_json(&view_json, "worktable")?;
```

### DXF flat patterns

The flat view (`exactGeometry(json, false)`) gives each sheet as its blank,
with the kernel's bend lines. `exportDxf` writes one DXF per distinct blank:

```ts
const flat = await exactGeometry(programJson, undefined, false);
if (flat.ok) {
  const out = await exportDxf(flat.view);
  if (out.ok) {
    for (const file of out.files) saveAs(`${file.name}.dxf`, file.dxf);
    for (const part of out.skipped) console.warn(part.id, part.message);
  }
}
```

```
cargo run --release --example view2dxf -- flat.view.json out_dir
```

Each file is DXF R12 (`AC1009`), in millimetres, the blank seen from its
outer skin with its lower-left corner at the origin, on three layers and
with no text, so nothing in the file can be taken for a cut:

| Layer | Holds |
|---|---|
| `CUT` | The outline and every hole, as `LINE`, `ARC` and `CIRCLE` entities. |
| `BEND` | One dashed `LINE` down the middle of each bend zone. |
| `ENGRAVE` | Declared for markings; empty until the kernel has some. |

What the drawing does not say about a bend comes back beside it: each file's
`bends` lists, in the order of the lines, the bend's `id`, `angleDeg`,
`direction` (`up` or `down`, seen from the outer skin) and `insideRadius`,
for a job sheet. Parts that share a solid share one file; `partIds` lists
them and `quantity` counts them. A part placed by a mirror image gets a mirrored
blank of its own (the bend directions stay: the outer skin is the same
skin), a uniform scale a scaled one. A part with no flat pattern (a
folded solid, a bar, a mesh, a rolled shell) is listed under `skipped`
with the reason, never written wrong. Splines on an outline are refused;
the kernel fits every curve with arcs before it reaches the view.

From the command line, with a view the kernel wrote (`exact_view` example in
the kernel repo, or any `{ ok, view }` result):

```
cargo run --release --example view2step -- part.view.json part.step
```

One part at the origin becomes a plain part. Anything else becomes an
assembly named `name`, with one `NEXT_ASSEMBLY_USAGE_OCCURRENCE` per part,
carrying the part's id, placed by its pose. Identical parts share one
product, as they share one solid in the view.

Two things the kernel never writes, but an app may add to a view:

- a **mesh solid**, `{"id": "Cube", "mesh": {"vertices": [[x, y, z]],
  "triangles": [[a, b, c]]}}`, for geometry that only ever was a mesh (a
  glTF model, an OpenSCAD part). Coplanar neighbouring triangles merge into
  one planar face with its holes; vertices are welded at 1e-5 mm.
- a part `matrix` that is a **mirror image or a uniform scale**. STEP
  placements are rigid, so such a part gets its own solid with the pose
  baked in (loops reversed, circles turned round, ruled faces flipped).
  Shear and non-uniform scale are refused.

## The view

One JSON document. Units are millimetres.

```jsonc
{
  "units": "mm",
  "solids": [{
    "id": "solid-0",
    "vertices": [[x, y, z]],
    "curves": [
      {"kind": "line"},
      {"kind": "circle", "center": [x, y, z], "axis": [x, y, z], "radius": 3.0},
      {"kind": "bspline", "degree": 3, "controlPoints": [[x, y, z]],
       "knots": [0, 0.3, 1], "multiplicities": [4, 1, 4]}
    ],
    "edges": [{"start": 0, "end": 1, "curve": 0}],
    "surfaces": [
      {"kind": "plane", "origin": [x, y, z], "normal": [x, y, z]},
      {"kind": "cylinder", "origin": [x, y, z], "axis": [x, y, z], "radius": 2.1},
      {"kind": "ruledBspline", "top": 2, "bottom": 3}
    ],
    "faces": [{
      "surface": 0, "sameSense": true,
      "loops": [[{"edge": 0, "forward": true}]],
      "featureId": "Apron", "zoneId": "Apron", "role": "outerSkin"
    }]
  }],
  "bends": [{"id": "BendApron", "zoneId": "BendApron", "angleDeg": 90,
             "direction": "down", "insideRadius": 1.5, "allowance": 3.3,
             "line": [[x, y, 0], [x, y, 0]]}],
  "parts": [{"id": "Apron", "solid": "solid-0", "material": "304", "thickness": 1.5,
             "color": "#b0b0b8",
             "position": [x, y, z], "rotation": [rx, ry, rz],
             "matrix": [[r00, r01, r02], [r10, r11, r12], [r20, r21, r22]]}]
}
```

Rules the kernel keeps and the writer relies on:

- every edge is used exactly twice across a solid's faces, once each way;
- vertices are shared by index, never welded by tolerance;
- an edge runs along its curve's own direction from `start` to `end`: a
  circle turns counter-clockwise about its axis, a spline goes from its first
  control point to its last;
- a face's first loop is its outer bound, the rest are holes;
- a `ruledBspline` is degree 1 between two `bspline` curves with the same
  degree and knots, u along them and v from `top` to `bottom`, normal du × dv;
- identical parts share one solid;
- a flat view's solid lists its `bends`: the centre line of each bend zone
  on the sheet's zero, the unsigned angle, `up` or `down` seen from the
  outer skin, the inside radius and the allowance. A folded solid has none;
- world point = `matrix` · local point + `position`. `rotation` is the same
  pose in degrees, R = Rx · Ry · Rz, and is used only when `matrix` is absent.

A part's `material` becomes its product's description and a
`MATERIAL_DESIGNATION`. `color` (`#rrggbb`, which the kernel never writes;
an app adds it from its material library) becomes a `STYLED_ITEM` on the
solid, so Inventor and OpenCascade show the part in that colour. Parts that
share a solid share one product and one `MANIFOLD_SOLID_BREP` per colour,
so a cabinet of twelve equal plates is one model with twelve occurrences.

The Rust types are in [`src/view.rs`](src/view.rs). A field is renamed only
with a matching change in the kernel. The fixtures `mesh_*.view.json` and
`four_down_mirrored.view.json` cover the two additions above; the
`*_flat.view.json` fixtures are flat views with bends.

## The judge

The judge is not us. [`judge/judge.py`](judge/judge.py) reads a STEP file
back with OpenCascade 8.0 (the `cadquery-ocp` package) and checks: the file
reads, the shape passes OpenCascade's validity check, every shell is closed,
there is one shell per solid, and no edge or vertex needed its tolerance
widened past 1e-3 mm. With `--mesh-volume` it also compares the volume with
the kernel's viewer mesh. CI runs it over every fixture here, and the kernel's
CI runs it over every gallery part the view covers.

```
pip install cadquery-ocp==8.0.1
python -I judge/judge.py out/*.step
```

Readers that have opened these files: OpenCascade (FreeCAD's engine) and
Autodesk Inventor, where Convert to Sheet Metal and Unfold give the kernel's
blank back.

DXF files have a judge too: [`judge/judge_dxf.py`](judge/judge_dxf.py)
reads each one back with [ezdxf](https://ezdxf.mozman.at/), audits it,
checks that every entity sits on `CUT`, `BEND` or `ENGRAVE`, that there is
no text, and that the `CUT` entities close into loops.

```
pip install ezdxf==1.4.5
python -I judge/judge_dxf.py out/*.dxf
```

## Build

```
cargo test
cd npm && npm install && npm run build && npm run smoke
```

`npm run build` needs `wasm-pack` 0.13 and `wasm-opt` on the path. The
package is published to GitHub Packages as `@kubuzapp/data-exchange` on each
release.
