"""Judge STEP files with OpenCascade (the cadquery-ocp package).

    python judge.py out.step [more.step ...]           one line of JSON per file
    python judge.py --mesh-volume 12345.6 out.step     also compare the volume

Each file must read, pass OpenCascade's validity check, be made of closed
solids, and need no edge tolerance above 1e-3 mm. The exit code is the number
of files that failed.
"""

import json
import sys

from OCP.BRepCheck import BRepCheck_Analyzer
from OCP.BRepGProp import BRepGProp
from OCP.BRep import BRep_Tool
from OCP.Bnd import Bnd_Box
from OCP.BRepBndLib import BRepBndLib
from OCP.GProp import GProp_GProps
from OCP.IFSelect import IFSelect_RetDone
from OCP.STEPControl import STEPControl_Reader
from OCP.TopAbs import TopAbs_EDGE, TopAbs_FACE, TopAbs_SHELL, TopAbs_SOLID, TopAbs_VERTEX
from OCP.TopExp import TopExp_Explorer
from OCP.TopoDS import TopoDS

MAX_TOL = 1e-3


def count(shape, kind):
    n = 0
    ex = TopExp_Explorer(shape, kind)
    while ex.More():
        n += 1
        ex.Next()
    return n


def max_tolerance(shape):
    worst = 0.0
    ex = TopExp_Explorer(shape, TopAbs_EDGE)
    while ex.More():
        worst = max(worst, BRep_Tool.Tolerance_s(TopoDS.Edge(ex.Current())))
        ex.Next()
    ex = TopExp_Explorer(shape, TopAbs_VERTEX)
    while ex.More():
        worst = max(worst, BRep_Tool.Tolerance_s(TopoDS.Vertex(ex.Current())))
        ex.Next()
    return worst


def judge(path, mesh_volume=None):
    r = STEPControl_Reader()
    if r.ReadFile(path) != IFSelect_RetDone:
        return {"file": path, "ok": False, "error": "did not read"}
    r.TransferRoots()
    shape = r.OneShape()
    out = {
        "file": path,
        "solids": count(shape, TopAbs_SOLID),
        "shells": count(shape, TopAbs_SHELL),
        "faces": count(shape, TopAbs_FACE),
        "valid": BRepCheck_Analyzer(shape).IsValid(),
        "maxTolerance": max_tolerance(shape),
    }
    closed = True
    ex = TopExp_Explorer(shape, TopAbs_SHELL)
    while ex.More():
        closed = closed and BRep_Tool.IsClosed_s(ex.Current())
        ex.Next()
    out["closed"] = closed
    props = GProp_GProps()
    BRepGProp.VolumeProperties_s(shape, props)
    out["volume"] = props.Mass()
    box = Bnd_Box()
    BRepBndLib.Add_s(shape, box, False)
    lo = box.CornerMin()
    hi = box.CornerMax()
    out["bounds"] = [[lo.X(), lo.Y(), lo.Z()], [hi.X(), hi.Y(), hi.Z()]]
    ok = (
        out["valid"]
        and closed
        and out["solids"] >= 1
        and out["shells"] == out["solids"]
        and out["maxTolerance"] <= MAX_TOL
        and out["volume"] > 0
    )
    if mesh_volume is not None and mesh_volume > 0:
        out["volumeDiffPct"] = abs(out["volume"] - mesh_volume) / mesh_volume * 100
        # The mesh cuts chords off every bend and leaves them in every hole,
        # so the exact solid is allowed to differ by a little.
        ok = ok and out["volumeDiffPct"] < 0.5
    out["ok"] = ok
    return out


def main(argv):
    mesh_volume = None
    files = []
    it = iter(argv)
    for a in it:
        if a == "--mesh-volume":
            mesh_volume = float(next(it))
        else:
            files.append(a)
    failed = 0
    for f in files:
        try:
            res = judge(f, mesh_volume)
        except Exception as e:  # noqa: BLE001 - report, do not crash the run
            res = {"file": f, "ok": False, "error": repr(e)}
        print(json.dumps(res))
        failed += 0 if res["ok"] else 1
    return failed


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
