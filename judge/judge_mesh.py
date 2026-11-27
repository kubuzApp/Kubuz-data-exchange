"""Judge GLB, STL and OBJ files against the view they were written from.

    pip install trimesh==4.6.8 numpy networkx==3.4.2
    npm install gltf-validator@2.0.0-dev.3.10   # optional, --gltf-validator
    python -I judge/judge_mesh.py --views fixtures [--gltf-validator node_modules] out/mesh/*

Each file is named `<view stem>.<ext>` (OBJ next to its `.mtl`). For every
file the judge checks, with trimesh as the reader:

- it reads, in millimetres with Z up once the format's own axes are undone
  (GLB: metres, Y up; OBJ: millimetres, Y up; STL: millimetres, Z up);
- its bounding box is the view's placed parts' to 0.01 mm;
- every closed input body is still closed and outward (positive volume);
- GLB and OBJ have one node or object per part, named as the part;
- GLB and OBJ carry one material per colour, metalness and roughness;
- with `--gltf-validator`, the Khronos glTF validator reports no error.
"""

import argparse
import json
import pathlib
import subprocess
import sys

import numpy as np
import trimesh

Y_UP_TO_Z_UP = np.array([[1, 0, 0, 0], [0, 0, -1, 0], [0, 1, 0, 0], [0, 0, 0, 1]], dtype=float)


def part_matrix(part):
    m = np.eye(4)
    if part.get("matrix") is not None:
        m[:3, :3] = np.array(part["matrix"], dtype=float)
    else:
        rx, ry, rz = np.radians(part.get("rotation", [0, 0, 0]))
        cx, sx, cy, sy, cz, sz = np.cos(rx), np.sin(rx), np.cos(ry), np.sin(ry), np.cos(rz), np.sin(rz)
        mx = np.array([[1, 0, 0], [0, cx, -sx], [0, sx, cx]])
        my = np.array([[cy, 0, sy], [0, 1, 0], [-sy, 0, cy]])
        mz = np.array([[cz, -sz, 0], [sz, cz, 0], [0, 0, 1]])
        m[:3, :3] = mx @ my @ mz
    m[:3, 3] = part.get("position", [0, 0, 0])
    return m


def expected(view):
    """The placed mesh parts: (name, vertices in mm Z up, closed?)."""
    solids = {s["id"]: s for s in view["solids"] if "mesh" in s}
    parts = view.get("parts") or [{"id": s["id"], "solid": s["id"]} for s in view["solids"]]
    out = []
    for p in parts:
        s = solids.get(p["solid"])
        if s is None:
            continue
        mesh = trimesh.Trimesh(s["mesh"]["vertices"], s["mesh"]["triangles"], process=True)
        closed = mesh.is_watertight
        v = trimesh.transform_points(np.array(s["mesh"]["vertices"], dtype=float), part_matrix(p))
        name = (p.get("name") or {}).get("nl") or (p.get("name") or {}).get("en") or p["id"]
        out.append((name, v, closed))
    return out


def looks(view):
    keys = set()
    for p in view.get("parts") or []:
        keys.add((p.get("color"), p.get("metalness"), p.get("roughness"), p.get("material", "")))
    return max(len(keys), 1)


def bounds(points):
    return np.array([points.min(axis=0), points.max(axis=0)])


def check_bounds(errors, got, want):
    if np.abs(got - want).max() > 0.01:
        errors.append(f"bounds {got.round(3).tolist()} != view {want.round(3).tolist()}")


def check_body(errors, label, mesh, closed):
    if not closed:
        return
    if not mesh.is_watertight:
        errors.append(f"{label}: not closed")
    elif mesh.volume <= 0:
        errors.append(f"{label}: inside out (volume {mesh.volume:.3f})")


def judge_glb(path, view, want, errors, validator):
    scene = trimesh.load(path, force="scene", process=False)
    # Undo the root node's metres and Y up.
    to_mm = np.diag([1000.0, 1000.0, 1000.0, 1.0]) @ Y_UP_TO_Z_UP
    points, names = [], []
    for node in scene.graph.nodes_geometry:
        matrix, geometry = scene.graph[node]
        mesh = scene.geometry[geometry].copy()
        mesh.apply_transform(to_mm @ matrix)
        points.append(mesh.vertices)
        names.append(node)
        closed = next((c for n, _, c in want if node.startswith(n)), True)
        check_body(errors, node, trimesh.Trimesh(mesh.vertices, mesh.faces, process=True), closed)
    check_bounds(errors, bounds(np.vstack(points)), bounds(np.vstack([v for _, v, _ in want])))
    if len(names) != len(want):
        errors.append(f"{len(names)} mesh nodes, view has {len(want)} mesh parts")
    doc = load_glb_json(path)
    for name, _, _ in want:
        if not any(n.get("name") == name for n in doc["nodes"]):
            errors.append(f"no node named {name!r}")
    if len(doc.get("materials", [])) != looks(view):
        errors.append(f"{len(doc.get('materials', []))} materials, view has {looks(view)} looks")
    if validator:
        script = (
            "const v=require('gltf-validator');const fs=require('fs');"
            "v.validateBytes(new Uint8Array(fs.readFileSync(process.argv[1])))"
            ".then(r=>{console.log(JSON.stringify(r.issues));})"
            ".catch(e=>{console.log(JSON.stringify({numErrors:1,messages:[String(e)]}));});"
        )
        res = subprocess.run(
            ["node", "-e", script, str(path)], capture_output=True, text=True,
            env={"NODE_PATH": validator, "PATH": "/usr/bin:/bin:/usr/local/bin"},
        )
        issues = json.loads(res.stdout or '{"numErrors":1,"messages":["no output"]}')
        if issues.get("numErrors", 0):
            errors.append(f"glTF validator: {issues.get('messages')}")


def load_glb_json(path):
    data = pathlib.Path(path).read_bytes()
    length = int.from_bytes(data[12:16], "little")
    return json.loads(data[20:20 + length])


def judge_stl(path, view, want, errors):
    mesh = trimesh.load(path, force="mesh", process=False)
    check_bounds(errors, bounds(mesh.vertices), bounds(np.vstack([v for _, v, _ in want])))
    merged = trimesh.Trimesh(mesh.vertices, mesh.faces, process=True)
    if all(c for _, _, c in want):
        for i, body in enumerate(merged.split(only_watertight=False)):
            check_body(errors, f"body {i}", body, True)


def judge_obj(path, view, want, errors):
    text = pathlib.Path(path).read_text()
    objects = [l[2:].strip() for l in text.splitlines() if l.startswith("o ")]
    if objects != [n for n, _, _ in want]:
        errors.append(f"objects {objects} != parts {[n for n, _, _ in want]}")
    mtl = pathlib.Path(path).with_suffix(".mtl")
    if not mtl.exists():
        errors.append("no .mtl next to it")
    else:
        count = sum(1 for l in mtl.read_text().splitlines() if l.startswith("newmtl "))
        if count != looks(view):
            errors.append(f"{count} materials, view has {looks(view)} looks")
    scene = trimesh.load(path, force="scene", process=False, group_material=False, split_object=True)
    points = []
    for name, geometry in scene.geometry.items():
        mesh = geometry.copy()
        mesh.apply_transform(Y_UP_TO_Z_UP)
        points.append(mesh.vertices)
        closed = next((c for n, _, c in want if name.startswith(n)), True)
        check_body(errors, name, trimesh.Trimesh(mesh.vertices, mesh.faces, process=True), closed)
    check_bounds(errors, bounds(np.vstack(points)), bounds(np.vstack([v for _, v, _ in want])))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--views", required=True)
    ap.add_argument("--gltf-validator", default=None, help="node_modules holding gltf-validator")
    ap.add_argument("files", nargs="+")
    args = ap.parse_args()
    failed = judged = 0
    for f in args.files:
        path = pathlib.Path(f)
        if path.suffix not in (".glb", ".stl", ".obj"):
            continue
        judged += 1
        view_file = pathlib.Path(args.views) / f"{path.stem}.view.json"
        view = json.loads(view_file.read_text())
        view = view.get("view", view)
        want = expected(view)
        errors = []
        try:
            if path.suffix == ".glb":
                judge_glb(path, view, want, errors, args.gltf_validator)
            elif path.suffix == ".stl":
                judge_stl(path, view, want, errors)
            else:
                judge_obj(path, view, want, errors)
        except Exception as e:  # noqa: BLE001 - a crash is a failure
            errors.append(f"could not read: {e}")
        status = "ok" if not errors else "FAIL"
        print(f"{status:4} {path.name}")
        for e in errors:
            print(f"     {e}")
        failed += bool(errors)
    print(f"{judged - failed} passed, {failed} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
