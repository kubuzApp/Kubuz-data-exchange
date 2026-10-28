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
* every chain of dimensions runs from one edge of the metal to the other
  (a chain across an oblique bend from one edge to the bend line), every
  station of it is a coordinate of the DXF (a cut, a bend line, a hole
  centre, or the corner two long cut lines make past a relief), and
  every value is the gap between two stations (no invented numbers);
* every bend line, hole centre and blank extent is a station of a chain
  in its direction;
* every hole diameter has a callout counting the holes of that size;
* every bend line has a numbered tag, and its note when the report
  claims one;
* every section cuts the blank where the report says: its flat pieces are
  the DXF's distances between the bend zones along the cut, its flange
  lengths are those pieces plus the tangent lengths to the virtual
  corners, its width and height are those of the folded band the judge
  walks itself, and every bend that is not 90° has the angle between its
  flanges on it;
* the folded extents in the title block are the box around the folded
  part's own vertices, when the folded view `<views>/<name>.view.json`
  (the name without `_flat`) exists;
* no two texts overlap, and no line runs through a text;
* the file stays small: 25 KB plus 6 KB a page.

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
SIZE_BASE = 25_000
SIZE_PER_PAGE = 6_000


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
    """(text, box in mm from the lower left, rotated?, oriented box) for
    every span. The box is the glyphs' own: cap height above the baseline
    and, when the text has descenders, a fifth of the size below it, not
    the font's line box. The oriented box (centre, along, up, half-length,
    half-height) follows the baseline, so a value written along a slanted
    dimension line is judged in its own frame."""
    h = page.rect.height
    out = []
    # The embedded fonts, for the exact advance of a span, whatever its
    # angle (the bounding box of slanted text is too wide).
    fonts = {}
    for xref, _, _, basefont, *_ in page.get_fonts(full=True):
        name = basefont.split("+")[-1]
        if name not in fonts:
            try:
                fonts[name] = pymupdf.Font(fontbuffer=page.parent.extract_font(xref)[3])
            except (RuntimeError, ValueError):
                pass
    # Two values squeezed side by side on a chain come back as one span:
    # a span is split where a glyph starts past the previous glyph's
    # advance by more than 0,15 of the size (no spaces are invented).
    for block in page.get_text("rawdict", flags=pymupdf.TEXT_PRESERVE_WHITESPACE | pymupdf.TEXT_INHIBIT_SPACES)["blocks"]:
        for line in block.get("lines", []):
            dx, dy = line["dir"]
            rotated = abs(dy) > 0.5
            for span in line["spans"]:
                size = span["size"]
                font = fonts.get(span["font"].split("+")[-1])

                def advance(ch):
                    if font is not None:
                        return font.text_length(ch["c"], fontsize=size)
                    bx = ch["bbox"]
                    return (bx[3] - bx[1]) if rotated else (bx[2] - bx[0])

                pieces, cur, prev_end = [], [], None
                for ch in span["chars"]:
                    along = ch["origin"][0] * dx + ch["origin"][1] * dy
                    if cur and prev_end is not None and along - prev_end > 0.15 * size:
                        pieces.append(cur)
                        cur = []
                    cur.append(ch)
                    prev_end = along + advance(ch)
                if cur:
                    pieces.append(cur)
                for piece in pieces:
                    while piece and piece[0]["c"].isspace():
                        piece.pop(0)
                    while piece and piece[-1]["c"].isspace():
                        piece.pop()
                    if not piece:
                        continue
                    t = "".join(ch["c"] for ch in piece)
                    want_desc = DESC * size if any(ch in DESCENDERS for ch in t) else 0.0
                    ox, oy = piece[0]["origin"]
                    up = (dy, -dx)
                    if font is not None:
                        a0 = 0.0
                        a1 = font.text_length(t, fontsize=size)
                    else:
                        # Along the baseline: the glyph boxes' corners
                        # projected on it (exact for horizontal and
                        # vertical text).
                        along = []
                        for ch in piece:
                            x0, y0, x1, y1 = ch["bbox"]
                            along += [(cx - ox) * dx + (cy - oy) * dy for cx in (x0, x1) for cy in (y0, y1)]
                        a0, a1 = min(along), max(along)
                    p0, p1 = -want_desc, CAP * size
                    out.append(text_entry(t, ox, oy, dx, dy, up, a0, a1, p0, p1, rotated, h))
    return out


def text_entry(t, ox, oy, dx, dy, up, a0, a1, p0, p1, rotated, h):
    """One text's record: text, box (mm from the lower left), rotated?,
    oriented box (centre, along, up, half-length, half-height)."""
    cx = ox + dx * (a0 + a1) / 2 + up[0] * (p0 + p1) / 2
    cy = oy + dy * (a0 + a1) / 2 + up[1] * (p0 + p1) / 2
    obox = (
        (cx * PT, (h - cy) * PT),
        (dx, -dy),
        (up[0], -up[1]),
        (a1 - a0) / 2 * PT,
        (p1 - p0) / 2 * PT,
    )
    corners = []
    for sa in (-1, 1):
        for sp in (-1, 1):
            corners.append((
                obox[0][0] + obox[1][0] * sa * obox[3] + obox[2][0] * sp * obox[4],
                obox[0][1] + obox[1][1] * sa * obox[3] + obox[2][1] * sp * obox[4],
            ))
    box = (
        min(c[0] for c in corners),
        min(c[1] for c in corners),
        max(c[0] for c in corners),
        max(c[1] for c in corners),
    )
    return (t, box, rotated, obox)


def seg_hits_obox(seg, obox, by):
    """Whether the segment crosses the oriented box shrunk by `by`."""
    (c, a, u, ha, hp) = obox
    pts = [((p[0] - c[0]) * a[0] + (p[1] - c[1]) * a[1], (p[0] - c[0]) * u[0] + (p[1] - c[1]) * u[1]) for p in seg]
    return seg_hits_box(pts, (-ha + by, -hp + by, ha - by, hp - by))


def oboxes_overlap(p, q, by):
    """Separating-axis test of two oriented boxes shrunk by `by`."""
    for axis in (p[1], p[2], q[1], q[2]):
        spans = []
        for (c, a, u, ha, hp) in (p, q):
            m = c[0] * axis[0] + c[1] * axis[1]
            r = (ha - by) * abs(a[0] * axis[0] + a[1] * axis[1]) + (hp - by) * abs(u[0] * axis[0] + u[1] * axis[1])
            spans.append((m - r, m + r))
        if spans[0][1] <= spans[1][0] or spans[1][1] <= spans[0][0]:
            return False
    return True


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


# --- Flat sheets --------------------------------------------------------------


def projections(d, direction, bound):
    """Every value a station of a chain along `direction` may sit at (the
    DXF's coordinates and the virtual corners projected on it), and the
    metal's extent along it."""
    dx, dy = direction
    ang = math.degrees(math.atan2(dy, dx))

    def proj(p):
        return p[0] * dx + p[1] * dy

    pool = set()
    metal = []

    def take(p, on_metal=True):
        pool.add(round(proj(p), 3))
        if on_metal:
            metal.append(proj(p))

    for a, b in d["lines"]:
        take(a)
        take(b)
    for c, r, st, e in d["arcs"]:
        take(c, False)
        take(arc_point(c, r, st))
        take(arc_point(c, r, e))
        sweep = (e - st) % 360.0
        for q in (0.0, 90.0, 180.0, 270.0, ang, ang + 180.0):
            if (q - st) % 360.0 <= sweep:
                take(arc_point(c, r, q))
    for c, r in d["circles"]:
        take(c, False)
        for q in (0.0, 90.0, 180.0, 270.0, ang, ang + 180.0):
            take(arc_point(c, r, q))
    for a, b in d["bends"]:
        take(a)
        take(b)
    for q in virtual_corners(d, bound):
        pool.add(round(proj(q), 3))
    return pool, min(metal), max(metal)


def line_params(a, u, lines, arcs, circles):
    """Parameters t along the ray a + t·u where it crosses the given DXF
    lines, arcs and circles."""
    out = []
    for p, q in lines:
        v = (q[0] - p[0], q[1] - p[1])
        den = u[0] * v[1] - u[1] * v[0]
        if abs(den) < 1e-9:
            continue
        t = ((p[0] - a[0]) * v[1] - (p[1] - a[1]) * v[0]) / den
        s2 = ((p[0] - a[0]) * u[1] - (p[1] - a[1]) * u[0]) / den
        if -1e-6 <= s2 <= 1.0 + 1e-6:
            out.append(t)
    for c, r, st, e in [(c, r, st, e) for c, r, st, e in arcs] + [(c, r, 0.0, 360.0) for c, r in circles]:
        # |a + t u - c|² = r²
        w = (a[0] - c[0], a[1] - c[1])
        b = 2 * (w[0] * u[0] + w[1] * u[1])
        cc = w[0] * w[0] + w[1] * w[1] - r * r
        disc = b * b - 4 * cc
        if disc < 0:
            continue
        for t in ((-b - math.sqrt(disc)) / 2, (-b + math.sqrt(disc)) / 2):
            p = (a[0] + t * u[0], a[1] + t * u[1])
            deg = math.degrees(math.atan2(p[1] - c[1], p[0] - c[0]))
            if (deg - st) % 360.0 <= (e - st) % 360.0 + 1e-6 or (e - st) % 360.0 == 0.0:
                out.append(t)
    return out


def band_box(pieces, bends, t):
    """Walk the section the report describes (outer skin up, a down bend
    turning clockwise) and return the box of the folded band after turning
    its longest flange horizontal: (width, height)."""
    pts = []
    p = (0.0, 0.0)
    h = (1.0, 0.0)
    headings = []
    for piece in pieces:
        n = (-h[1], h[0])
        if piece["kind"] == "flat":
            ln = piece["length"]
            q = (p[0] + h[0] * ln, p[1] + h[1] * ln)
            pts += [p, q, (p[0] - n[0] * t, p[1] - n[1] * t), (q[0] - n[0] * t, q[1] - n[1] * t)]
            headings.append((ln, h))
            p = q
            continue
        b = bends[piece["bend"]]
        theta = math.radians(b["angleDeg"])
        down = b["direction"] == "down"
        r_in = b["insideRadius"]
        if down:
            c = (p[0] - n[0] * (r_in + t), p[1] - n[1] * (r_in + t))
            rot = -theta
        else:
            c = (p[0] + n[0] * r_in, p[1] + n[1] * r_in)
            rot = theta
        for k in range(0, 13):
            a = rot * k / 12.0
            ca, sa = math.cos(a), math.sin(a)
            for q in (p, (p[0] - n[0] * t, p[1] - n[1] * t)):
                w = (q[0] - c[0], q[1] - c[1])
                pts.append((c[0] + w[0] * ca - w[1] * sa, c[1] + w[0] * sa + w[1] * ca))
        ca, sa = math.cos(rot), math.sin(rot)
        w = (p[0] - c[0], p[1] - c[1])
        p = (c[0] + w[0] * ca - w[1] * sa, c[1] + w[0] * sa + w[1] * ca)
        h = (h[0] * ca - h[1] * sa, h[0] * sa + h[1] * ca)
    _, hl = max(headings, key=lambda x: x[0])
    ca, sa = hl[0], -hl[1]
    rp = [(x * ca - y * sa, x * sa + y * ca) for x, y in pts]
    return (
        max(q[0] for q in rp) - min(q[0] for q in rp),
        max(q[1] for q in rp) - min(q[1] for q in rp),
    )


def flat_problems(sheet, d, words, folded_solid, scale=1.0):
    """What is wrong with one flat sheet, given its DXF, the words on its
    page and, when known, the folded part's solid."""
    problems = []
    bends = {b["nr"]: b for b in sheet["bends"]}
    t = sheet["thickness"]
    (x_lo, x_hi), (y_lo, y_hi) = dxf_extent(d)
    bound = min(max(0.02 * max(x_hi - x_lo, y_hi - y_lo), 6.0), 25.0)

    # Chains: edge to edge, on the blank's coordinates, values printed.
    stations = {}  # direction -> list of t
    for c in sheet.get("chains", []):
        u = tuple(c["direction"])
        key = (round(u[0], 6), round(u[1], 6))
        label = f"{c['kind']} chain along ({u[0]:.3g}, {u[1]:.3g})"
        pool, lo, hi = projections(d, u, bound)
        ts = [s["t"] for s in c["stations"]]
        if len(ts) < 2:
            problems.append(f"{label}: fewer than two stations")
            continue
        if any(b - a <= 1e-6 for a, b in zip(ts, ts[1:])):
            problems.append(f"{label}: stations out of order")
        axis_chain = abs(u[0]) < 1e-9 or abs(u[1]) < 1e-9
        if axis_chain and (abs(ts[0] - lo) > SAME or abs(ts[-1] - hi) > SAME):
            problems.append(
                f"{label}: runs from {mm_text(ts[0])} to {mm_text(ts[-1])}, the metal from {mm_text(lo)} to {mm_text(hi)}"
            )
        if not axis_chain and abs(ts[0] - lo) > SAME and abs(ts[-1] - hi) > SAME:
            problems.append(f"{label}: neither end is at the edge of the metal")
        for v in ts:
            if not any(abs(v - q) <= SAME for q in pool):
                problems.append(f"{label}: station {mm_text(v)} is no coordinate of the blank")
        if len(c["values"]) != len(ts) - 1 or len(c["texts"]) != len(ts) - 1:
            problems.append(f"{label}: {len(c['values'])} values for {len(ts)} stations")
            continue
        for i, (v, txt) in enumerate(zip(c["values"], c["texts"])):
            if abs(v - (ts[i + 1] - ts[i])) > 1e-3:
                problems.append(f"{label}: value {txt} is not the gap between its stations")
            if mm_text(v) != txt:
                problems.append(f"{label}: text {txt!r} is not {mm_text(v)!r}")
            if txt not in words:
                problems.append(f"{label}: {txt!r} not printed")
        stations.setdefault(key, []).extend(ts)

    def located(u, v):
        key = (round(u[0], 6), round(u[1], 6))
        return any(abs(v - q) <= SAME for q in stations.get(key, []))

    if (1.0, 0.0) not in stations:
        problems.append("no chain along x")
    if (0.0, 1.0) not in stations:
        problems.append("no chain along y")
    for c, r in d["circles"]:
        if not located((1.0, 0.0), c[0]) or not located((0.0, 1.0), c[1]):
            problems.append(f"hole at ({mm_text(c[0])}, {mm_text(c[1])}) not located")
    for nr, (a, b) in enumerate(d["bends"], 1):
        ln = math.dist(a, b)
        if ln < 1e-9:
            continue
        n = ((b[1] - a[1]) / ln, -(b[0] - a[0]) / ln)
        hit = False
        for sg in (1.0, -1.0):
            u = (sg * n[0], sg * n[1])
            if abs(u[0]) < 1e-9:
                u = (0.0, 1.0)
            elif abs(u[1]) < 1e-9:
                u = (1.0, 0.0)
            if located(u, a[0] * u[0] + a[1] * u[1]):
                hit = True
        if not hit:
            problems.append(f"bend line {nr} not dimensioned")

    # Hole callouts.
    by_dia = {}
    for c, r in d["circles"]:
        by_dia[round(2 * r, 1)] = by_dia.get(round(2 * r, 1), 0) + 1
    for dia, n in by_dia.items():
        want_text = f"⌀{mm_text(dia)}" if n == 1 else f"{n}× ⌀{mm_text(dia)}"
        if want_text not in words:
            problems.append(f"no callout {want_text!r}")

    # Bend tags, notes and table.
    for nr in range(1, len(d["bends"]) + 1):
        if str(nr) not in words:
            problems.append(f"no tag for bend {nr}")
    if len(sheet["bends"]) != len(d["bends"]):
        problems.append(f"{len(sheet['bends'])} bends in the table, {len(d['bends'])} lines in the DXF")
    for b in sheet["bends"]:
        if b.get("note") and b["note"] not in words:
            problems.append(f"bend {b['nr']}: note {b['note']!r} not printed")

    # Sections.
    for sec in sheet.get("sections", []):
        L = sec["letter"]
        label = f"section {L}-{L}"
        if not any(w.startswith(f"{word} {L}-{L}") for w in words for word in ("DOORSNEDE", "SECTION", "COUPE")):
            problems.append(f"{label}: title not printed")
        if len(words.get(L, [])) < 3:
            problems.append(f"{label}: cut marker letter {L!r} not printed")
        a, b = sec["cut"]
        ln = math.dist(a, b)
        u = ((b[0] - a[0]) / ln, (b[1] - a[1]) / ln)
        edges = line_params(a, u, d["lines"], d["arcs"], d["circles"])
        if not edges:
            problems.append(f"{label}: the cut misses the blank")
            continue
        t_in, t_out = min(edges), max(edges)
        pieces = sec["pieces"]
        nrs = [p["bend"] for p in pieces if p["kind"] == "bend"]
        if nrs != sec["bends"] or len(pieces) != 2 * len(nrs) + 1 or any(
            (p["kind"] == "flat") != (i % 2 == 0) for i, p in enumerate(pieces)
        ):
            problems.append(f"{label}: pieces do not alternate flat and bend as the bends say")
            continue
        zones = []
        for nr in nrs:
            if nr not in bends or nr > len(d["bends"]):
                problems.append(f"{label}: bend {nr} is not in the table")
                break
            p, q = d["bends"][nr - 1]
            tt = line_params(a, u, [(p, q)], [], [])
            if not tt:
                problems.append(f"{label}: the cut misses bend line {nr}")
                break
            bl = math.dist(p, q)
            n = ((q[1] - p[1]) / bl, -(q[0] - p[0]) / bl)
            cosang = abs(u[0] * n[0] + u[1] * n[1])
            zones.append((tt[0], bends[nr]["allowance"] / 2.0 / max(cosang, 1e-9)))
        if len(zones) != len(nrs):
            continue
        edges_t = [t_in] + [x for z, hw in zones for x in (z - hw, z + hw)] + [t_out]
        for i, piece in enumerate(p for p in pieces if p["kind"] == "flat"):
            want = edges_t[2 * i + 1] - edges_t[2 * i]
            if abs(piece["length"] - want) > SAME:
                problems.append(
                    f"{label}: flat piece {i} is {mm_text(piece['length'])}, the DXF says {mm_text(want)}"
                )
        # Flange lengths to the virtual corners, the band's box, angles.
        dims = sec["dimensions"]
        for i, piece in enumerate(pieces):
            if piece["kind"] != "flat":
                continue
            want = piece["length"]
            for j in (i - 1, i + 1):
                if 0 <= j < len(pieces):
                    bb = bends[pieces[j]["bend"]]
                    want += (bb["insideRadius"] + t) * math.tan(math.radians(bb["angleDeg"]) / 2.0)
            dim = next((x for x in dims if x["kind"] == "flange" and x["sources"] == [f"piece:{i}"]), None)
            if dim is None:
                problems.append(f"{label}: flange of piece {i} ({mm_text(want)}) not dimensioned")
            elif abs(dim["value"] - want) > SAME or dim["text"] != mm_text(want):
                problems.append(f"{label}: flange of piece {i} is {dim['text']}, not {mm_text(want)}")
        w, h = band_box(pieces, bends, t)
        if abs(sec["width"] - w) > SAME or abs(sec["height"] - h) > SAME:
            problems.append(
                f"{label}: band is {mm_text(sec['width'])} × {mm_text(sec['height'])}, the judge walks {mm_text(w)} × {mm_text(h)}"
            )
        for kind, want in (("width", w), ("height", h)):
            for dim in (x for x in dims if x["kind"] == kind):
                if abs(dim["value"] - want) > SAME or dim["text"] != mm_text(want):
                    problems.append(f"{label}: {kind} {dim['text']} is not {mm_text(want)}")
        # The angle written between the flanges is the one the operator
        # measures: 180° less the bend's deviation from flat.
        for nr in nrs:
            ang = bends[nr]["angleDeg"]
            between = 180.0 - ang
            dim = next((x for x in dims if x["kind"] == "angle" and x["sources"] == [f"bend:{nr}"]), None)
            if abs(ang - 90.0) > 0.05 and dim is None:
                problems.append(f"{label}: bend {nr} is {mm_text(ang)}°, not drawn")
            if dim is not None and (abs(dim["value"] - between) > 0.05 or dim["text"] != f"{mm_text(between)}°"):
                problems.append(f"{label}: angle of bend {nr} is {dim['text']}, not {mm_text(between)}°")
        for dim in dims:
            if dim["text"] not in words:
                problems.append(f"{label}: {dim['text']!r} not printed")

    # Folded extents against the folded part.
    ext = sheet.get("extents")
    if folded_solid is not None:
        vs = folded_solid["vertices"]
        box = sorted(scale * (max(p[i] for p in vs) - min(p[i] for p in vs)) for i in range(3))
        if ext is None:
            problems.append("no folded extents, the folded part is " + " × ".join(mm_text(x) for x in box))
        elif any(abs(a - b) > SAME for a, b in zip(sorted(ext), box)):
            problems.append(
                "folded extents " + " × ".join(mm_text(x) for x in ext) + " are not the folded part's " + " × ".join(mm_text(x) for x in box)
            )
    if ext is not None:
        want = " × ".join(mm_text(x) for x in ext)
        if not any(want in w for w in words):
            problems.append(f"extents {want!r} not printed")
    return problems


# --- The judge --------------------------------------------------------------


def collision_problems(page, texts, prob, tag):
    """No two texts overlap, and no line runs through a text."""
    boxes = [(t, box, obox) for t, box, _, obox in texts]
    for i in range(len(boxes)):
        for j in range(i + 1, len(boxes)):
            if oboxes_overlap(boxes[i][2], boxes[j][2], 0.1):
                prob(f"{tag}: texts {boxes[i][0]!r} and {boxes[j][0]!r} overlap")
    segs = page_segments(page)
    for t, box, obox in boxes:
        if obox[3] <= 0.25 or obox[4] <= 0.25:
            continue
        n = sum(1 for s in segs if seg_hits_box(s, shrink(box, 0.25)) and seg_hits_obox(s, obox, 0.25))
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


def part_scale(view, sheet):
    """The uniform scale of the part's matrix (1 for a rotation)."""
    part = next(p for p in view["parts"] if p["id"] == sheet["partIds"][0])
    m = part.get("matrix")
    if not m:
        return 1.0
    return math.sqrt(m[0][0] ** 2 + m[1][0] ** 2 + m[2][0] ** 2)


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
        for t, box, _, _ in texts:
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

        # Claims and must-haves against the DXF and the folded part.
        dxf_path = os.path.join(dxf_dir, sheet["partIds"][0] + ".dxf")
        try:
            d = read_dxf(dxf_path)
        except OSError as e:
            prob(f"{tag}: dxf: {e}")
            continue
        folded_solid = None
        scale = 1.0
        if name.endswith("_flat") or "_flat_" in name:
            base = name.replace("_flat", "", 1)
            view_path = os.path.join(views_root or "", base + ".view.json")
            if os.path.exists(view_path):
                with open(view_path, encoding="utf-8") as f:
                    fv = json.load(f)
                try:
                    folded_solid = solid_of(fv, sheet)
                    scale = part_scale(fv, sheet)
                except StopIteration:
                    prob(f"{tag}: part {sheet['partIds'][0]} is not in {base}.view.json")
        for p in flat_problems(sheet, d, words, folded_solid, scale):
            prob(f"{tag}: {p}")
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
