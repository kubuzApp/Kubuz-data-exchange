"""Judge PDF drawing dossiers against the DXF flat patterns they describe.

    python judge_pdf.py --dxf out/dxf out/*.pdf [--verapdf greenfield-apps.jar]

Each `<name>.pdf` comes with the report the exporter wrote beside it,
`<name>.report.json`, and the flat patterns it drew in `<dxf>/<name>/`.
The judge reads the PDF with PyMuPDF and pikepdf, and the DXF files with
its own reader, and never trusts the report for a fact it can check:

* every page is an ISO 5457 sheet (A4 portrait or A3 landscape);
* every font is embedded, the file carries an output intent and a
  language, and veraPDF passes it as PDF/A-2b when a jar is given;
* every dimension the report claims is printed on the page as it says;
* every printed ordinate is a coordinate of the DXF, or the corner two
  long cut lines make past a relief (no invented numbers);
* every hole centre, bend-line end and blank extent in the DXF is
  dimensioned;
* every hole diameter has a callout counting the holes of that size;
* every bend line has a numbered tag;
* no two texts overlap, and no line runs through a text;
* the file stays small: 24 KB plus 4 KB a page.

A folded sheet (kind "folded" in the report) is judged against the view
JSON the dossier was drawn from, `<views>/<name>.view.json`, instead:

* every printed ordinate is, in its view's frame, a coordinate of a
  vertex, a hole centre, a circle's extreme point or the corner where two
  edge-on faces meet (no invented numbers);
* every view's chain runs from 0 to the far side of the part;
* every round hole in a face a view looks straight at is located on both
  axes somewhere on the sheet and has a diameter callout;
* every bend is in the table, numbered on a view, and a bend that is not
  90° has its angle written at the bend;
* the views sit in first-angle arrangement: the left view right of the
  front view, the top view below it; an auxiliary view (`partial`) is
  judged on its zone's faces alone;
* the sheet's size line and the collisions and PDF/A checks above.

One line of JSON per file; the exit code is the number of files that
failed.
"""

import argparse
import json
import math
import os
import subprocess
import sys

import pikepdf
import pymupdf

PT = 25.4 / 72.0
SHEETS = {"A4": (210.0, 297.0), "A3": (420.0, 297.0)}
SAME = 0.06
SIZE_BASE = 24_000
SIZE_PER_PAGE = 4_000


def mm_text(v):
    """A length as the drawing writes it: a tenth, decimal comma, no
    trailing zero."""
    r = round(v, 1)
    if abs(r - round(r)) < 1e-9:
        return str(int(round(r)))
    return f"{r:.1f}".replace(".", ",")


# --- DXF --------------------------------------------------------------------


def read_dxf(path):
    """CUT lines, arcs and circles and BEND lines from an R12 file."""
    with open(path, encoding="utf-8") as f:
        lines = [l.rstrip("\r\n") for l in f]
    pairs = [(int(lines[i].strip()), lines[i + 1]) for i in range(0, len(lines) - 1, 2)]
    ents = []
    i = 0
    in_entities = False
    while i < len(pairs):
        code, val = pairs[i]
        if code == 2 and val == "ENTITIES":
            in_entities = True
        elif in_entities and code == 0:
            if val == "ENDSEC":
                break
            e = {"type": val}
            j = i + 1
            while j < len(pairs) and pairs[j][0] != 0:
                e[pairs[j][0]] = pairs[j][1]
                j += 1
            ents.append(e)
            i = j
            continue
        i += 1
    out = {"lines": [], "arcs": [], "circles": [], "bends": []}
    for e in ents:
        layer = e.get(8, "")
        if e["type"] == "LINE":
            seg = ((float(e[10]), float(e[20])), (float(e[11]), float(e[21])))
            if layer == "CUT":
                out["lines"].append(seg)
            elif layer == "BEND":
                out["bends"].append(seg)
        elif e["type"] == "ARC" and layer == "CUT":
            out["arcs"].append(((float(e[10]), float(e[20])), float(e[40]), float(e[50]), float(e[51])))
        elif e["type"] == "CIRCLE" and layer == "CUT":
            out["circles"].append(((float(e[10]), float(e[20])), float(e[40])))
    return out


def arc_point(c, r, deg):
    t = math.radians(deg)
    return (c[0] + r * math.cos(t), c[1] + r * math.sin(t))


def dxf_coordinates(d, centres=True):
    """Every x and every y a dimension could legitimately point at; without
    `centres`, only points on the metal."""
    xs, ys = set(), set()

    def take(p):
        xs.add(round(p[0], 3))
        ys.add(round(p[1], 3))

    for a, b in d["lines"]:
        take(a)
        take(b)
    for c, r, s, e in d["arcs"]:
        if centres:
            take(c)
        take(arc_point(c, r, s))
        take(arc_point(c, r, e))
        sweep = (e - s) % 360.0
        for q in (0.0, 90.0, 180.0, 270.0):
            if (q - s) % 360.0 <= sweep:
                take(arc_point(c, r, q))
    for c, r in d["circles"]:
        if centres:
            take(c)
        take((c[0] - r, c[1] - r))
        take((c[0] + r, c[1] + r))
    for a, b in d["bends"]:
        take(a)
        take(b)
    return xs, ys


def virtual_corners(d, bound):
    """Where two long CUT lines would meet if extended a little past a
    relief or notch between them: the theoretical corner a drawing
    dimensions instead of the relief's own ends."""
    spans = []
    for a, b in d["lines"]:
        l = math.dist(a, b)
        if l < bound:
            continue
        spans.append((a, ((b[0] - a[0]) / l, (b[1] - a[1]) / l), l))
    out = []
    for i in range(len(spans)):
        for j in range(i + 1, len(spans)):
            (p, u, lp), (q, v, lq) = spans[i], spans[j]
            den = u[0] * v[1] - u[1] * v[0]
            if abs(den) < 1e-6:
                continue
            t = ((q[0] - p[0]) * v[1] - (q[1] - p[1]) * v[0]) / den
            s2 = ((q[0] - p[0]) * u[1] - (q[1] - p[1]) * u[0]) / den
            if -2 * bound <= t <= lp + 2 * bound and -2 * bound <= s2 <= lq + 2 * bound:
                out.append((p[0] + u[0] * t, p[1] + u[1] * t))
    return out


def dxf_extent(d):
    """The box around the cut metal (bend lines and arc centres aside)."""
    cut_x, cut_y = dxf_coordinates({**d, "bends": []}, centres=False)
    return (min(cut_x), max(cut_x)), (min(cut_y), max(cut_y))


def intersections_of_lines(lines, tol=1e-6):
    """Corners where two CUT lines meet end to end, with each line's
    length: (x, y, shorter adjacent length)."""
    ends = {}
    for a, b in lines:
        length = math.dist(a, b)
        for p in (a, b):
            key = (round(p[0], 3), round(p[1], 3))
            ends.setdefault(key, []).append(length)
    return [(k[0], k[1], min(v)) for k, v in ends.items() if len(v) >= 2]


# --- PDF --------------------------------------------------------------------


CAP = 0.74
DESC = 0.22
DESCENDERS = set("gjpqy,;()[]{}")


def page_texts(page):
    """(text, box in mm from the lower left, rotated?) for every span. The
    box is the glyphs' own: cap height above the baseline and, when the text
    has descenders, a fifth of the size below it, not the font's line box."""
    h = page.rect.height
    out = []
    for block in page.get_text("dict", flags=pymupdf.TEXT_PRESERVE_WHITESPACE)["blocks"]:
        for line in block.get("lines", []):
            dx, dy = line["dir"]
            rotated = abs(dy) > 0.5
            for span in line["spans"]:
                t = span["text"].strip()
                if not t:
                    continue
                size = span["size"]
                asc = span["ascender"] * size
                desc = -span["descender"] * size
                want_desc = DESC * size if any(ch in DESCENDERS for ch in t) else 0.0
                x0, y0, x1, y1 = span["bbox"]
                # Which side of the box the glyphs' top and bottom lie on.
                if abs(dx) > 0.5:
                    if dx > 0:
                        y0 += asc - CAP * size
                        y1 -= desc - want_desc
                    else:
                        y1 -= asc - CAP * size
                        y0 += desc - want_desc
                elif dy < 0:
                    x0 += asc - CAP * size
                    x1 -= desc - want_desc
                else:
                    x1 -= asc - CAP * size
                    x0 += desc - want_desc
                out.append((t, (x0 * PT, (h - y1) * PT, x1 * PT, (h - y0) * PT), rotated))
    return out


def page_segments(page):
    """Straight pieces of every stroked path, in mm from the lower left."""
    h = page.rect.height
    segs = []

    def pt(p):
        return (p.x * PT, (h - p.y) * PT)

    for d in page.get_drawings():
        if d.get("type") not in ("s", "fs"):
            continue
        for item in d["items"]:
            kind = item[0]
            if kind == "l":
                segs.append((pt(item[1]), pt(item[2])))
            elif kind == "c":
                p0, p1, p2, p3 = (pt(item[k]) for k in (1, 2, 3, 4))
                prev = p0
                for k in range(1, 5):
                    t = k / 4.0
                    x = ((1 - t) ** 3 * p0[0] + 3 * (1 - t) ** 2 * t * p1[0]
                         + 3 * (1 - t) * t**2 * p2[0] + t**3 * p3[0])
                    y = ((1 - t) ** 3 * p0[1] + 3 * (1 - t) ** 2 * t * p1[1]
                         + 3 * (1 - t) * t**2 * p2[1] + t**3 * p3[1])
                    segs.append((prev, (x, y)))
                    prev = (x, y)
            elif kind == "re":
                r = item[1]
                a, b, c, e = pt(r.top_left), pt(r.top_right), pt(r.bottom_right), pt(r.bottom_left)
                segs += [(a, b), (b, c), (c, e), (e, a)]
    return segs


def seg_hits_box(seg, box):
    (x0, y0), (x1, y1) = seg
    bx0, by0, bx1, by1 = box
    # Liang–Barsky clipping of the segment against the box.
    dx, dy = x1 - x0, y1 - y0
    t0, t1 = 0.0, 1.0
    for p, q in ((-dx, x0 - bx0), (dx, bx1 - x0), (-dy, y0 - by0), (dy, by1 - y0)):
        if abs(p) < 1e-12:
            if q < 0:
                return False
            continue
        t = q / p
        if p < 0:
            t0 = max(t0, t)
        else:
            t1 = min(t1, t)
        if t0 > t1:
            return False
    return True


def boxes_overlap(a, b):
    return a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3]


def shrink(box, by):
    return (box[0] + by, box[1] + by, box[2] - by, box[3] - by)


def structure_problems(path):
    out = []
    with pikepdf.open(path) as pdf:
        root = pdf.Root
        if "/OutputIntents" not in root:
            out.append("no output intent")
        if "/Lang" not in root:
            out.append("no /Lang")
        if "/Outlines" not in root:
            out.append("no outlines")
        if "/PageLabels" not in root:
            out.append("no page labels")
        for page in pdf.pages:
            fonts = page.Resources.get("/Font", {})
            for name, font in fonts.items():
                f = font
                if f.get("/Subtype") == "/Type0":
                    f = f.DescendantFonts[0]
                desc = f.get("/FontDescriptor")
                if desc is None or "/FontFile2" not in desc:
                    out.append(f"font {name} not embedded")
    return out


def verapdf(jar, path):
    cmd = ["java", "-cp", jar, "org.verapdf.apps.GreenfieldCliWrapper", "-f", "2b", "--format", "text", path]
    r = subprocess.run(cmd, capture_output=True, text=True, check=False)
    text = r.stdout + r.stderr
    if "PASS " in text:
        return None
    lines = [l for l in text.splitlines() if l.strip() and "JAVA_TOOL_OPTIONS" not in l]
    return "; ".join(lines[-4:])[:400] or f"exit {r.returncode}"


# --- The judge --------------------------------------------------------------


def collision_problems(page, texts, prob, tag):
    """No two texts overlap, and no line runs through a text."""
    boxes = [(t, box) for t, box, _ in texts]
    for i in range(len(boxes)):
        for j in range(i + 1, len(boxes)):
            if boxes_overlap(shrink(boxes[i][1], 0.1), shrink(boxes[j][1], 0.1)):
                prob(f"{tag}: texts {boxes[i][0]!r} and {boxes[j][0]!r} overlap")
    segs = page_segments(page)
    for t, box in boxes:
        inner = shrink(box, 0.25)
        if inner[0] >= inner[2] or inner[1] >= inner[3]:
            continue
        n = sum(1 for s in segs if seg_hits_box(s, inner))
        if n:
            prob(f"{tag}: {n} line(s) run through {t!r} at ({box[0]:.1f}, {box[1]:.1f})")


# --- Folded sheets -----------------------------------------------------------


def dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def unit(v):
    n = math.sqrt(dot(v, v)) or 1.0
    return (v[0] / n, v[1] / n, v[2] / n)


def canon(axis):
    """A world axis and its sign, so that +x and -x share one entry."""
    k = max(range(3), key=lambda i: abs(axis[i]))
    if axis[k] < 0:
        return (-axis[0], -axis[1], -axis[2]), -1.0
    return tuple(axis), 1.0


def solid_of(view, sheet):
    part = next(p for p in view["parts"] if p["id"] == sheet["partIds"][0])
    return next(s for s in view["solids"] if s["id"] == part["solid"])


def round_holes(solid):
    """(centre, radius, outward normal) of every round hole or tube end
    in a planar skin face: an inner loop whose edges are all circles of one
    centre and radius."""
    out = []
    for f in solid["faces"]:
        if f["role"] not in ("outerSkin", "innerSkin"):
            continue
        surf = solid["surfaces"][f["surface"]]
        if surf["kind"] != "plane":
            continue
        n = unit(surf["normal"])
        if not f.get("sameSense", True):
            n = (-n[0], -n[1], -n[2])
        for lp in f["loops"][1:]:
            circles = []
            for le in lp:
                e = solid["edges"][le["edge"]]
                c = solid["curves"][e["curve"]]
                if c["kind"] != "circle":
                    circles = None
                    break
                circles.append((tuple(c["center"]), c["radius"]))
            if not circles:
                continue
            c0, r0 = circles[0]
            if all(math.dist(c, c0) < 1e-3 and abs(r - r0) < 1e-3 for c, r in circles):
                out.append((c0, r0, n))
    return out


def zone_solid(solid, zone):
    """The faces of one zone, with only the edges, vertices and
    surfaces they use: what a partial view shows."""
    faces = [f for f in solid["faces"] if f.get("zoneId") == zone]
    edge_ids = sorted({le["edge"] for f in faces for lp in f["loops"] for le in lp})
    emap = {e: i for i, e in enumerate(edge_ids)}
    edges = [solid["edges"][i] for i in edge_ids]
    vertex_ids = sorted({e["start"] for e in edges} | {e["end"] for e in edges})
    vmap = {v: i for i, v in enumerate(vertex_ids)}
    edges = [{**e, "start": vmap[e["start"]], "end": vmap[e["end"]]} for e in edges]
    surfaces = [solid["surfaces"][f["surface"]] for f in faces]
    faces = [
        {
            **f,
            "surface": i,
            "loops": [[{**le, "edge": emap[le["edge"]]} for le in lp] for lp in f["loops"]],
        }
        for i, f in enumerate(faces)
    ]
    return {
        "vertices": [solid["vertices"][i] for i in vertex_ids],
        "curves": solid["curves"],
        "edges": edges,
        "surfaces": surfaces,
        "faces": faces,
    }


def view_candidates(solid, r, u, d, origin):
    """Every value an ordinate of this view may show, per axis: vertices,
    circle centres and extremes, and the corners where two edge-on planes
    meet, as local coordinates from the view's origin."""
    pts = [tuple(p) for p in solid["vertices"]]
    # The part's extent: vertices and the extremes of full circles (an
    # arc's circle may reach past the metal).
    on_metal = len(pts)
    extremes = []
    for e in solid["edges"]:
        c = solid["curves"][e["curve"]]
        if c["kind"] != "circle":
            continue
        cc, cr, ca = tuple(c["center"]), c["radius"], unit(c["axis"])
        full = e["start"] == e["end"]
        for a in (r, u):
            s = math.sqrt(max(0.0, 1.0 - dot(ca, a) ** 2))
            for sg in (1.0, -1.0):
                q = (cc[0] + a[0] * cr * s * sg, cc[1] + a[1] * cr * s * sg, cc[2] + a[2] * cr * s * sg)
                if full:
                    pts.insert(on_metal, q)
                    on_metal += 1
                else:
                    extremes.append(q)
        extremes.append(cc)
    pts += extremes
    local = [(dot(p, r) - dot(origin, r), dot(p, u) - dot(origin, u)) for p in pts]
    metal = local[:on_metal]
    # Edge-on planes and the long straight edges as lines in the view,
    # and their pairwise corners (near the edges' own span).
    lines = []
    spans = []
    # A cylinder seen along a line across its axis shows its two
    # silhouette generators, a radius out from the axis on either side.
    for surf in solid["surfaces"]:
        if surf["kind"] != "cylinder":
            continue
        ax = unit(surf["axis"])
        if abs(dot(ax, d)) > 1e-6:
            continue
        side = unit(cross(ax, d))
        o = surf["origin"]
        for sg in (1.0, -1.0):
            q = (o[0] + side[0] * surf["radius"] * sg, o[1] + side[1] * surf["radius"] * sg, o[2] + side[2] * surf["radius"] * sg)
            lines.append(((dot(q, r) - dot(origin, r), dot(q, u) - dot(origin, u)), (dot(ax, r), dot(ax, u))))
    ext = max(max(v[0] for v in local) - min(v[0] for v in local), max(v[1] for v in local) - min(v[1] for v in local))
    bound = min(max(0.02 * ext, 6.0), 25.0)
    for e in solid["edges"]:
        c = solid["curves"][e["curve"]]
        if c["kind"] == "bspline":
            # A straight B-spline (a ruled face's edge) counts as a line.
            cp = c["controlPoints"]
            a, b = cp[0], cp[-1]
            ab = unit((b[0] - a[0], b[1] - a[1], b[2] - a[2]))
            if any(math.dist(q, (a[0] + ab[0] * dot((q[0] - a[0], q[1] - a[1], q[2] - a[2]), ab),
                                a[1] + ab[1] * dot((q[0] - a[0], q[1] - a[1], q[2] - a[2]), ab),
                                a[2] + ab[2] * dot((q[0] - a[0], q[1] - a[1], q[2] - a[2]), ab))) > 1e-3 for q in cp):
                continue
        elif c["kind"] != "line":
            continue
        a, b = solid["vertices"][e["start"]], solid["vertices"][e["end"]]
        pa = (dot(a, r) - dot(origin, r), dot(a, u) - dot(origin, u))
        pb = (dot(b, r) - dot(origin, r), dot(b, u) - dot(origin, u))
        l = math.dist(pa, pb)
        if l < bound:
            continue
        spans.append((pa, ((pb[0] - pa[0]) / l, (pb[1] - pa[1]) / l), (-2 * bound, l + 2 * bound)))
    for surf in solid["surfaces"]:
        if surf["kind"] != "plane":
            continue
        n = unit(surf["normal"])
        if abs(dot(n, d)) > 1e-6:
            continue
        dir3 = unit(cross(d, n))
        o = surf["origin"]
        lines.append(((dot(o, r) - dot(origin, r), dot(o, u) - dot(origin, u)), (dot(dir3, r), dot(dir3, u))))
    corners = []
    allv = [(p, a, None) for p, a in lines] + spans
    for i in range(len(allv)):
        for j in range(i + 1, len(allv)):
            (p, a, sa), (q, b, sb) = allv[i], allv[j]
            den = a[0] * b[1] - a[1] * b[0]
            if abs(den) < 1e-6:
                continue
            t = ((q[0] - p[0]) * b[1] - (q[1] - p[1]) * b[0]) / den
            s2 = ((q[0] - p[0]) * a[1] - (q[1] - p[1]) * a[0]) / den
            if sa and not (sa[0] <= t <= sa[1]):
                continue
            if sb and not (sb[0] <= s2 <= sb[1]):
                continue
            corners.append((p[0] + a[0] * t, p[1] + a[1] * t))
    xs = {v[0] for v in local}
    ys = {v[1] for v in local}
    extent = (
        min(v[0] for v in metal),
        max(v[0] for v in metal),
        min(v[1] for v in metal),
        max(v[1] for v in metal),
    )
    xs |= {c[0] for c in corners}
    ys |= {c[1] for c in corners}
    return xs, ys, extent


def bend_axis(solid, zone):
    """The axis of the outer-skin cylinder of a bend zone, if any."""
    for f in solid["faces"]:
        if f.get("zoneId") == zone and f["role"] == "outerSkin":
            surf = solid["surfaces"][f["surface"]]
            if surf["kind"] == "cylinder":
                return unit(surf["axis"])
    return None


def folded_problems(sheet, view, words):
    """What is wrong with one folded sheet, given the words on its page."""
    problems = []
    solid = solid_of(view, sheet)
    views = {v["name"]: v for v in sheet["views"]}
    claimed = {}  # canonical world axis -> set of world values

    def claim(axis, world):
        c, sign = canon(axis)
        claimed.setdefault(c, []).append(world * sign)

    def located(axis, world):
        c, sign = canon(axis)
        return any(abs(v - world * sign) <= SAME for v in claimed.get(c, []))

    extents = []
    for v in sheet["views"]:
        if v["name"] == "isometric":
            continue
        r, u, d, o = tuple(v["r"]), tuple(v["u"]), tuple(v["d"]), tuple(v["originWorld"])
        rh = cross(r, u)
        if abs(dot(rh, d) - 1.0) > 1e-6:
            problems.append(f"view {v['name']}: frame is not right-handed")
        shown = zone_solid(solid, v["partial"]) if v.get("partial") else solid
        if not shown["faces"]:
            problems.append(f"view {v['name']}: zone {v['partial']!r} has no faces")
            continue
        xs, ys, extent = view_candidates(shown, r, u, d, o)
        for dim in sheet["dimensions"]:
            if dim["kind"] != "ordinate" or not dim["axis"].startswith(v["name"] + ":"):
                continue
            axis = dim["axis"].split(":")[1]
            val = dim["value"]
            pool = xs if axis == "x" else ys
            if not any(abs(val - c) <= SAME for c in pool):
                problems.append(f"view {v['name']}: ordinate {axis} = {dim['text']} is no coordinate of the part")
            a = r if axis == "x" else u
            claim(a, val + dot(o, a))
        extents.append((v["name"], "x", r, dot(o, r), extent[0], extent[1]))
        extents.append((v["name"], "y", u, dot(o, u), extent[2], extent[3]))
        for ang in v.get("angles", []):
            if ang["text"] not in words:
                problems.append(f"view {v['name']}: angle {ang['text']!r} not printed")

    # Each view's chains run from 0 to the far side, on this view or on
    # the one sharing the world axis (a value is dimensioned once).
    for name, axis, a, o_a, lo, hi in extents:
        c, sign = canon(a)
        local = [w * sign - o_a for w in claimed.get(c, [])]
        if not local:
            problems.append(f"view {name}: nothing dimensioned along {axis}")
            continue
        if min(local) > SAME:
            problems.append(f"view {name}: {axis} chain starts at {mm_text(min(local))}, not 0")
        if max(local) < hi - lo - SAME:
            problems.append(f"view {name}: {axis} chain ends at {mm_text(max(local))} before the far side {mm_text(hi - lo)}")

    # Holes the sheet looks straight at.
    seen = {}
    for c, rad, n in round_holes(solid):
        for v in sheet["views"]:
            if v["name"] == "isometric" or dot(n, v["d"]) < 0.9999:
                continue
            key = (round(c[0], 3), round(c[1], 3), round(c[2], 3), round(rad, 3))
            if key in seen:
                continue
            seen[key] = True
            for a in (v["r"], v["u"]):
                if not located(a, dot(c, a)):
                    problems.append(
                        f"hole ⌀{mm_text(2 * rad)} at ({mm_text(c[0])}, {mm_text(c[1])}, {mm_text(c[2])}) not located along ({a[0]:g}, {a[1]:g}, {a[2]:g})"
                    )
            dia = mm_text(2 * rad)
            if not any(w == f"⌀{dia}" or (w.endswith(f"× ⌀{dia}")) for w in words):
                problems.append(f"no callout for ⌀{dia}")

    # Bends: table rows, tags and non-90° angles.
    tagged = {t for v in sheet["views"] for t in v.get("tags", [])}
    for b in sheet["bends"]:
        if b["nr"] not in tagged:
            problems.append(f"bend {b['nr']} ({b['id']}) tagged on no view")
        if str(b["nr"]) not in words:
            problems.append(f"bend {b['nr']}: tag not printed")
        ang = f"{mm_text(b['angleDeg'])}°"
        if ang not in words:
            problems.append(f"bend {b['nr']}: angle {ang!r} not in the table")
        drawn = any(a["bend"] == b["nr"] for v in sheet["views"] for a in v.get("angles", []))
        axis = bend_axis(solid, b["id"])
        along = axis is not None and any(
            v["name"] != "isometric" and abs(dot(axis, v["d"])) > 0.9999 for v in sheet["views"]
        )
        if abs(b["angleDeg"] - 90.0) > 0.05 and along and not drawn:
            problems.append(f"bend {b['nr']}: {ang} is not 90° yet drawn on no view")

    # First-angle arrangement and the size line.
    front = views.get("front")
    if front:
        if "left" in views and views["left"]["origin"][0] <= front["origin"][0]:
            problems.append("left view is not right of the front view")
        if "right" in views and views["right"]["origin"][0] >= front["origin"][0]:
            problems.append("right view is not left of the front view")
        if "top" in views and views["top"]["origin"][1] >= front["origin"][1]:
            problems.append("top view is not below the front view")
        if "bottom" in views and views["bottom"]["origin"][1] <= front["origin"][1]:
            problems.append("bottom view is not above the front view")
    size = sheet.get("size")
    if size:
        want = " × ".join(mm_text(x) for x in size)
        if not any(want in w for w in words):
            problems.append(f"size {want!r} not printed")
    return problems


def judge(path, dxf_root, jar, views_root=None):
    out = {"file": path, "ok": True, "problems": []}
    prob = out["problems"].append
    report_path = path[:-4] + ".report.json"
    try:
        with open(report_path, encoding="utf-8") as f:
            report = json.load(f)
    except OSError as e:
        return {"file": path, "ok": False, "problems": [f"report: {e}"]}
    name = os.path.basename(path)[:-4]
    dxf_dir = os.path.join(dxf_root, name)

    size = os.path.getsize(path)
    doc = pymupdf.open(path)
    out["pages"] = doc.page_count
    out["bytes"] = size
    if size > SIZE_BASE + SIZE_PER_PAGE * doc.page_count:
        prob(f"{size} bytes for {doc.page_count} page(s)")
    if doc.page_count != len(report["sheets"]):
        prob(f"{doc.page_count} pages for {len(report['sheets'])} sheets")
    for p in structure_problems(path):
        prob(p)
    if jar:
        v = verapdf(jar, path)
        if v:
            prob(f"veraPDF: {v}")
        else:
            out["pdfa"] = "2b"

    for idx, sheet in enumerate(report["sheets"]):
        if idx >= doc.page_count:
            break
        page = doc[idx]
        tag = f"page {idx + 1} ({sheet['title']})"
        w, h = page.rect.width * PT, page.rect.height * PT
        want = SHEETS.get(sheet["paper"])
        if not want or abs(w - want[0]) > 0.5 or abs(h - want[1]) > 0.5:
            prob(f"{tag}: {w:.1f} × {h:.1f} mm is not {sheet['paper']}")
        texts = page_texts(page)
        words = {}
        for t, box, _ in texts:
            words.setdefault(t, []).append(box)

        # Claims against the page.
        for d in sheet["dimensions"]:
            if d["text"] not in words:
                prob(f"{tag}: claimed {d['text']!r} is not printed")

        if sheet.get("kind") == "folded":
            view_path = os.path.join(views_root or "", name + ".view.json")
            try:
                with open(view_path, encoding="utf-8") as f:
                    view = json.load(f)
            except OSError as e:
                prob(f"{tag}: view: {e}")
                continue
            for p in folded_problems(sheet, view, words):
                prob(f"{tag}: {p}")
            collision_problems(page, texts, prob, tag)
            continue

        # Claims and must-haves against the DXF.
        dxf_path = os.path.join(dxf_dir, sheet["partIds"][0] + ".dxf")
        try:
            d = read_dxf(dxf_path)
        except OSError as e:
            prob(f"{tag}: dxf: {e}")
            continue
        xs, ys = dxf_coordinates(d)
        (x_lo, x_hi), (y_lo, y_hi) = dxf_extent(d)
        bound = min(max(0.02 * max(x_hi - x_lo, y_hi - y_lo), 6.0), 25.0)
        for cx, cy in virtual_corners(d, bound):
            xs.add(round(cx, 3))
            ys.add(round(cy, 3))
        claimed = {"x": [], "y": []}
        for dim in sheet["dimensions"]:
            if dim["kind"] != "ordinate":
                continue
            axis = dim["axis"]
            v = dim["value"]
            claimed[axis].append(v)
            pool = xs if axis == "x" else ys
            if not any(abs(v - c) <= SAME for c in pool):
                prob(f"{tag}: ordinate {axis} = {dim['text']} is no coordinate of the blank")

        def has(axis, v):
            return any(abs(v - c) <= SAME for c in claimed[axis])

        for v in (x_lo, x_hi):
            if not has("x", v):
                prob(f"{tag}: blank extent x = {mm_text(v)} not dimensioned")
        for v in (y_lo, y_hi):
            if not has("y", v):
                prob(f"{tag}: blank extent y = {mm_text(v)} not dimensioned")
        for c, r in d["circles"]:
            if not has("x", c[0]) or not has("y", c[1]):
                prob(f"{tag}: hole at ({mm_text(c[0])}, {mm_text(c[1])}) not located")
        for a, b in d["bends"]:
            if abs(a[0] - b[0]) < 1e-6:
                need = [("x", a[0])]
            elif abs(a[1] - b[1]) < 1e-6:
                need = [("y", a[1])]
            else:
                need = [("x", a[0]), ("y", a[1]), ("x", b[0]), ("y", b[1])]
            for axis, v in need:
                if not has(axis, v):
                    prob(f"{tag}: bend line {axis} = {mm_text(v)} not dimensioned")

        # Hole callouts.
        by_dia = {}
        for c, r in d["circles"]:
            by_dia.setdefault(round(2 * r, 1), 0)
            by_dia[round(2 * r, 1)] += 1
        for dia, n in by_dia.items():
            want_text = f"⌀{mm_text(dia)}" if n == 1 else f"{n}× ⌀{mm_text(dia)}"
            if want_text not in words:
                prob(f"{tag}: no callout {want_text!r}")

        # Bend tags.
        for nr in range(1, len(d["bends"]) + 1):
            if str(nr) not in words:
                prob(f"{tag}: no tag for bend {nr}")
        if len(sheet["bends"]) != len(d["bends"]):
            prob(f"{tag}: {len(sheet['bends'])} bends in the table, {len(d['bends'])} lines in the DXF")

        collision_problems(page, texts, prob, tag)

    if out["problems"]:
        out["ok"] = False
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("pdfs", nargs="+")
    ap.add_argument("--dxf", required=True, help="folder holding <name>/<part>.dxf per dossier")
    ap.add_argument("--verapdf", help="veraPDF greenfield-apps jar; validates PDF/A-2b when given")
    ap.add_argument("--views", help="folder holding <name>.view.json for folded dossiers")
    args = ap.parse_args()
    failed = 0
    for path in args.pdfs:
        result = judge(path, args.dxf, args.verapdf, args.views)
        print(json.dumps(result, ensure_ascii=False))
        if not result["ok"]:
            failed += 1
    sys.exit(failed)


if __name__ == "__main__":
    main()
