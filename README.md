# Kubuz data exchange

File formats written from the Kubuz kernel's **exact-geometry view**. Today
that is STEP (ISO 10303-21, AP214) of the folded parts, DXF (R12) flat
patterns of the sheet blanks, and a PDF/A dossier of ISO detail drawings,
one sheet per blank. More formats come as the CPQ needs them.

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

### PDF drawing dossier

A workshop still wants paper: a detail drawing per part, with the flat
pattern, its dimensions, the bend table and a title block. `exportDrawings`
draws them from a view, as one PDF/A-2b file: from the flat view it draws
the flat patterns, from the folded view the folded parts in orthographic
projection:

```ts
const flat = await exactGeometry(programJson, undefined, false);
if (flat.ok) {
  const dossier = await exportDrawings(flat.view, {
    owner: "Kubuz",
    address: ["Bosbeek 56 - 9120 Beveren-Waas", "www.kubuz.net"],
    createdBy: "Kubuz CPQ",
    date: "2026-10-28",
    customer: "Ateliers Dupont",
    title: "Worktable",
    number: "KB-2026-0412",
    language: "nl",
    densities: { S235: 7850, "AISI 304": 8000 },
    logo: { width: 120, height: 40, paths: [{ d: "M0 0H120V40H0Z", fill: "#003366" }] },
  });
  if (dossier.ok) saveAs("worktable-drawings.pdf", dossier.pdfBytes());
}
```

Every distinct blank gets a sheet (parts that share a blank share a sheet;
`quantity` says how many to make), on A4 portrait or A3 landscape, at the
largest ISO 5455 scale that fits; the title block is the same size on both.
The sheet follows ISO 5457 (frame, centring and trimming marks, the grid
reference `1 2 3 4 …` and `A B C …` around the frame), the title block the
Kubuz template on ISO 7200 (designer, design date, approved, approved date,
scale, size, projection symbol, material, thickness, weight, the flat
pattern's size, the part's number and description, the customer, the
owner's name and address, the ownership line; the folded L × W × H is in
the notes). The weight comes from the blank's area, the thickness and the
material's density: `densities` in the meta, else a built-in table by the
usual names (S235, DC01, DX51D, 304, 1.4301, AlMg3, CuZn, …), the lines ISO 128-2 (0,5 outline, 0,25 the
rest), the lettering ISO 3098 (osifont, embedded as a subset) and the
dimension style of the Inventor template (text 3 mm, arrows 3 × 0,75,
extension lines 1 mm past the dimension line, decimal comma, one decimal).

The flat pattern is the main drawing, as a press-brake operator reads it,
and the exporter puts on it what the drafter does:

- **chain dimensions** below and left of the blank, edge to bend line to
  bend line to edge, which are the back-gauge settings of the press brake,
  then the outline's corners (the sharp corner a fillet replaced), then the
  overall size underneath; the holes get their own chain from the opposite
  edge, above or right. An oblique bend gets one short dimension across its
  flange, from the bend line to the parallel edge. Every value stands over
  its gap when it fits; values that do not fit are squeezed in along the
  line in their order, pushing their neighbours aside and the first and
  last past the ends, and the extension lines are interrupted where a
  value stands on them. A value that still finds no room goes above a
  neighbouring gap with a leader, or outside the line: never on a line or
  another text;
- **section views** of the folded part for every bend, at the flat
  pattern's scale: a cut across the flanges, drawn as the sheet really is
  (both skins with the inside and outside radii), the flanges dimensioned
  parallel to the flange (outside lengths, interrupted when long), the
  angle between the flanges when the bend is not 90°, and the width and
  height. The section is marked on the flat pattern by one arrow with its
  letter, no cutting line; the view is titled `DOORSNEDE A-A`;
- an **isometric pictorial** of the folded part when the sheet has room
  for it (at the flat pattern's scale or up to two steps smaller, never on
  a bigger sheet for its sake): the outer skin folded from the blank's
  zones and bends, visible lines only;
- **callouts** `⌀d` and `R` next to the feature when the paper around it
  is free, else in a column at the right; equal sizes are counted, `4× ⌀9`;
- **numbered bend lines** with a table at the bottom of the sheet, aligned
  with the notes: angle (deviation from flat), direction (`OP` towards the
  viewer, `NEER` away: the view is the outer skin), inside radius and bend
  allowance; the V-opening is the press-brake operator's choice and is not
  written;
- **reliefs, notches and small rounds** on the outline are not dimensioned
  (the laser cuts them from the DXF) and the notes say so; a cutout is a
  feature at any size;
- a **free-form contour** (a run of tangent arcs, or a polyline standing in
  for a curve) is dimensioned by its ends and extreme points only, with a
  note that the shape is in the DXF.

A folded view gives each sheet-metal part a sheet of **orthographic views**
in first-angle projection (ISO 5456-2): the front view looks at the outer
skin of the largest flange, a side view and a top view are added when they
show a bend axis, a hole or a dimension the front view cannot, and an
isometric view at a smaller scale sits in a free corner. Hidden lines are
dashed, bend radii are drawn as the two tangent lines plus the arc, and the
views are dimensioned together: every value once, in the view that shows it
best, with running dimensions from each view's lower-left corner. Round
holes get `⌀` callouts in the view that looks at them, a bend angle other
than 90° is drawn between the flanges, every bend carries its number in a
circle on a view where it is seen (the tags chain when several sit on one
line), and the same bend table lists angle, direction and inside radius.
A rolled tube shows both diameters. A flange with a hole that no principal
view looks straight at gets an **auxiliary view** (ISO 128-30) of its own:
the flange true to shape, titled `AANZICHT A` (with its scale when it is
smaller), and an arrow with the letter on the front view saying where it
looks from. Slits between flanges and the relief corners of a bend are
"small" like reliefs in the flat pattern and are left to the flat pattern.

Numbers are written to a tenth with a decimal comma and no trailing zero
(`45,5`, `36`). The notes are in Dutch, English or French (`language`).
The dossier follows the material through the workshop. It opens with a
**cover** (title, number, revision, customer, date, the totals per
material and thickness, and a table of contents whose rows are links),
then the **assembly sheet** on A3: every part folded and placed as the view
places it, as one isometric with hidden lines removed and an item balloon
(ISO 6433) per position, with the **parts list** (ISO 7573) beside it:
position, quantity, description, material, thickness, flat size, folded
size and weight. Then the **laser list** per material and thickness (flat
size, cut length, holes, net area, weight per piece and in total) and the
**bend list** per tool (thickness and inside radius: every bend's angle,
direction, radius, allowance and length). The part sheets come last, in
production order: laser only first, then laser and bending, by material,
thickness and tool; the position numbers follow that order. A mirrored
part gets a sheet of its own that says which position it mirrors. Every
parts list row links to its sheet and every sheet's title block links
back; the dossier has bookmarks and page labels (`Voorblad`, `Pos 1`,
`Pos 2`, …), and it is small: about 20 KB for the font and 1 to 3 KB a
sheet, fully vector.

Besides the file the export describes every sheet (`sheets`): its kind
(`flat` or `folded`), paper, scale, where the blank sits, material, every
chain with its stations and the features that put them there, every section
with its cut and the pieces it shows, the folded extents, where the
isometric sits, and, for a folded sheet, each view's frame and origin in
part coordinates, so an outside judge can rebuild the part from the drawing
alone, plus its page, route, weight, cut length, net area and hole count.
`lists` says where the cover, the assembly, the parts list, the laser list
and the bend list are, and `assembly` where the isometric sits and where
every balloon points. Parts that cannot be drawn (a mesh, a mirrored
folded part) are listed under `skipped`.

From the command line:

```
cargo run --example view2pdf -- four_down_flat.view.json out.pdf meta.json
```

In Rust: `kubuz_data_exchange::write_drawings_json(&view_json, &meta_json)`.

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

The drawings have the strictest judge: [`judge/judge_pdf.py`](judge/judge_pdf.py)
reads each dossier back with PyMuPDF and pikepdf, and the flat patterns it
drew with its own DXF reader, and checks that every page is an ISO sheet,
every font is embedded, every chain the export claims is printed with its
values equal to the gaps between its stations, every station is a corner
of the blank (or the theoretical corner past a relief), a hole centre or a
bend line, every chain runs from edge to edge, every hole centre, bend line
and blank extent is dimensioned, every hole size has its callout, every bend
its numbered tag and its row in the table, every section's flange lengths
and angles match the DXF cut (outside length = flat length plus the bend
allowance of the thickness band), the folded extents match the folded
fixture, no two texts overlap (measured with the embedded font's advances),
no line runs through a text, the file stays under 25 KB plus 6 KB a page,
and veraPDF passes it as PDF/A-2b. The front matter is judged the same
way: the parts list prints every sheet's position, quantity, sizes and
weight, every balloon is drawn where the report says, the laser list's cut
lengths and hole counts are what the judge sums and counts from the DXF,
the bend list carries every bend's angle, radius and DXF line length, and
every link annotation points at a page, every sheet back at the parts list
and every parts list row at its sheet. A folded sheet
is judged against the view it was drawn from (`--views`): every ordinate
is, in its view's frame, a vertex, a hole centre, a circle's extreme, a
cylinder's silhouette or the corner of two edge-on faces; every chain runs
from 0 to the far side; every hole a view looks at is located and called
out; every bend is tabled, tagged, and drawn with its angle when it is not
90° and a view looks along it; and the views sit in first-angle
arrangement. CI runs it over every fixture, flat and folded.

```
pip install pymupdf pikepdf
curl -sSfLO https://repo1.maven.org/maven2/org/verapdf/apps/greenfield-apps/1.28.2/greenfield-apps-1.28.2.jar
python -I judge/judge_pdf.py --dxf out/dxf --views fixtures --verapdf greenfield-apps-1.28.2.jar out/pdf/*.pdf
```

## Build

```
cargo test
cd npm && npm install && npm run build && npm run smoke
```

`npm run build` needs `wasm-pack` 0.13 and `wasm-opt` on the path. The
package is published to GitHub Packages as `@kubuzapp/data-exchange` on each
release.
