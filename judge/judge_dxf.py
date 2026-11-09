"""Judge DXF flat patterns with ezdxf.

    python judge_dxf.py out/Top.dxf [more.dxf ...]     one line of JSON per file

Each file must read as DXF R12, pass ezdxf's audit with no error, keep every
entity on CUT, BEND, ENGRAVE, MILL or MILL_BACK, carry no text, and have
its CUT entities, and each mill layer's, chain into closed loops (every end
point shared by exactly two segments, or a whole circle). The exit code is the number of files that failed.
"""

import json
import math
import sys
from collections import Counter

import ezdxf

LAYERS = {"CUT", "BEND", "ENGRAVE", "MILL", "MILL_BACK"}
CLOSED = ("CUT", "MILL", "MILL_BACK")
TOL = 1e-3


def loops_close(msp, layer):
    """Every line or arc end on `layer` is shared by exactly two segments,
    within TOL millimetres."""
    points = []
    for e in msp.query(f'LINE[layer=="{layer}"]'):
        points.append((e.dxf.start[0], e.dxf.start[1]))
        points.append((e.dxf.end[0], e.dxf.end[1]))
    for e in msp.query(f'ARC[layer=="{layer}"]'):
        c, r = e.dxf.center, e.dxf.radius
        for a in (e.dxf.start_angle, e.dxf.end_angle):
            t = math.radians(a)
            points.append((c[0] + r * math.cos(t), c[1] + r * math.sin(t)))
    clusters = []  # [x, y, count]
    for x, y in sorted(points):
        for cl in clusters:
            if abs(cl[0] - x) <= TOL and abs(cl[1] - y) <= TOL:
                cl[2] += 1
                break
        else:
            clusters.append([x, y, 1])
    return [(round(x, 4), round(y, 4), n) for x, y, n in clusters if n != 2]


def judge(path):
    out = {"file": path, "ok": True, "problems": []}
    try:
        doc = ezdxf.readfile(path)
    except Exception as e:  # noqa: BLE001
        return {"file": path, "ok": False, "problems": [f"read: {e}"]}
    out["version"] = doc.dxfversion
    auditor = doc.audit()
    if auditor.has_errors:
        out["problems"].append(f"audit: {len(auditor.errors)} error(s)")
    msp = doc.modelspace()
    kinds = Counter((e.dxftype(), e.dxf.layer) for e in msp)
    out["entities"] = {f"{k}@{layer}": n for (k, layer), n in sorted(kinds.items())}
    for (_, layer), _ in kinds.items():
        if layer not in LAYERS:
            out["problems"].append(f"entity on layer {layer}")
    if not any(layer == "CUT" for _, layer in kinds):
        out["problems"].append("nothing on CUT")
    for layer in LAYERS:
        if layer not in doc.layers:
            out["problems"].append(f"layer {layer} missing")
    for layer in CLOSED:
        loose = loops_close(msp, layer)
        if loose:
            out["problems"].append(f"{len(loose)} open end(s) on {layer}")
    texts = sum(n for (k, _), n in kinds.items() if k in ("TEXT", "MTEXT"))
    if texts:
        out["problems"].append(f"{texts} text entit(ies)")
    out["ok"] = not out["problems"]
    return out


def main(argv):
    failed = 0
    for path in argv:
        r = judge(path)
        print(json.dumps(r))
        if not r["ok"]:
            failed += 1
    return failed


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
