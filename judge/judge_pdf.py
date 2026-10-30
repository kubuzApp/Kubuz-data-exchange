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
* every bend line, hole centre, slot centre and blank extent is a station
  of a chain in its direction; a chain may write equal spaces as one run
  `6× 40 (=240)` whose count times pitch is the gap;
* every hole diameter and slot size has a callout counting the features
  of that size (`⌀26`, `8× ⌀18`, `2× 12 × 28 h.o.h.`);
* every hole pattern is rebuilt from the printed values alone and every
  centre so rebuilt is a feature of the DXF: a field (six or more equal
  features on a grid) from the centre of its first cell and the runs on
  the chains, holes on a pitch circle (three or more, equally spaced)
  from the circle's centre, `⌀PCD`, `n× ⌀d`, the spacing `n× step°` and
  the first hole's angle from +x when it is not on the axis;
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

The front matter (`lists` in the report) is judged too:

* the page count is the sheets plus the lists' pages, every list page is
  an A4 sheet (the assembly an A3), free of collisions;
* the parts list prints every sheet's position, quantity, flat size,
  folded size and weight, and every balloon on the assembly is its
  position number drawn where the report says;
* the laser list prints every blank's cut length as the judge sums it
  from the DXF (lines, arcs and circles) and its number of holes as the
  judge counts the closed CUT loops;
* the bend list prints every bend's angle, inside radius and the length
  of its DXF bend line;
* every link annotation is printable and points at a page; every sheet
  links back to the parts list, and every row of the parts list links to
  its sheet.

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


def dxf_name(part_id):
    """The DXF file name view2dxf gives a part: anything but letters,
    digits, `-` and `_` becomes `_`."""
    return "".join(c if c.isalnum() or c in "-_" else "_" for c in part_id) + ".dxf"


def mm_text(v):
    """A length as the drawing writes it: a tenth, decimal comma, no
    trailing zero."""
    r = round(v, 1)
    if abs(r - round(r)) < 1e-9:
        return str(int(round(r)))
    return f"{r:.1f}".replace(".", ",")


def fine_text(v):
    """A pitch as the drawing writes it: to a hundredth when a tenth would
    drift over the field."""
    r = round(v, 2)
    if abs(r - round(r, 1)) < 1e-9:
        return mm_text(r)
    return f"{r:.2f}".replace(".", ",")


def deg_text(v):
    """An angle as the drawing writes it: a tenth, no trailing zero, °."""
    return f"{mm_text(v)}°"


def run_text(k, pitch, total):
    """Equal spaces on a chain as ISO 129-1 writes them: `6× 40 (=240)`."""
    return f"{k}× {fine_text(pitch)} (={mm_text(total)})"


def parse_run(txt):
    """(count, pitch, total) of a run text, else None."""
    try:
        k, rest = txt.split("× ", 1)
        pitch, total = rest.split(" (=", 1)
        if not total.endswith(")"):
            return None
        return int(k), float(pitch.replace(",", ".")), float(total[:-1].replace(",", "."))
    except ValueError:
        return None


def parse_mm(txt):
    return float(txt.replace(",", "."))


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
    merge_arcs(out)
    return out


def merge_arcs(d):
    """Arcs about one centre with one radius that follow on from each
    other are one arc (the kernel writes a round end as its quarters), and
    a full turn of them is one round hole."""
    rings = {}
    for arc in d["arcs"]:
        c, r = arc[0], arc[1]
        rings.setdefault((round(c[0], 2), round(c[1], 2), round(r, 2)), []).append(arc)
    kept = []
    for arcs in rings.values():
        c = (sum(a[0][0] for a in arcs) / len(arcs), sum(a[0][1] for a in arcs) / len(arcs))
        r = sum(a[1] for a in arcs) / len(arcs)
        sweep = sum((a1 - a0) % 360.0 or 360.0 for _, _, a0, a1 in arcs)
        if len(arcs) > 1 and abs(sweep - 360.0) < 0.01:
            d["circles"].append((c, r))
            continue
        # Chain the arcs: each one starts where the previous ends.
        rest = sorted(arcs, key=lambda a: a[2] % 360.0)
        while rest:
            _, _, a0, a1 = rest.pop(0)
            grown = True
            while grown:
                grown = False
                for i, (_, _, b0, b1) in enumerate(rest):
                    if abs((b0 - a1 + 180.0) % 360.0 - 180.0) < 0.01:
                        a1 = b1
                        rest.pop(i)
                        grown = True
                        break
                    if abs((a0 - b1 + 180.0) % 360.0 - 180.0) < 0.01:
                        a0 = b0
                        rest.pop(i)
                        grown = True
                        break
            kept.append((c, r, a0, a1))
    d["arcs"] = kept


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


def cutouts(d, with_members=False):
    """The inner contours of the blank that are not circles, each as
    (lo, hi, segment count, points: ends, arc centres and extremes[, the
    entities]): the CUT lines and arcs joined end to end, the contour
    with the widest extent being the outline."""
    segs = [(a, b) for a, b in d["lines"]]
    segs += [(arc_point(c, r, a0), arc_point(c, r, a1)) for c, r, a0, a1 in d["arcs"]]
    ents = list(d["lines"]) + list(d["arcs"])
    parent = list(range(len(segs)))

    def find(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    ends = {}
    for i, (a, b) in enumerate(segs):
        for p in (a, b):
            k = (round(p[0], 2), round(p[1], 2))
            if k in ends:
                parent[find(i)] = find(ends[k])
            else:
                ends[k] = i
    groups = {}
    for i in range(len(segs)):
        groups.setdefault(find(i), []).append(i)
    boxes = []
    for members in groups.values():
        pts = [p for i in members for p in segs[i]]
        for c, r, a0, a1 in (d["arcs"][i - len(d["lines"])] for i in members if i >= len(d["lines"])):
            pts.append(c)
            sweep = (a1 - a0) % 360.0
            for q in (0.0, 90.0, 180.0, 270.0):
                if (q - a0) % 360.0 <= sweep:
                    pts.append(arc_point(c, r, q))
        lo = (min(p[0] for p in pts), min(p[1] for p in pts))
        hi = (max(p[0] for p in pts), max(p[1] for p in pts))
        row = (lo, hi, len(members), pts)
        if with_members:
            row += ([ents[i] for i in members],)
        boxes.append(row)
    if not boxes:
        return []
    outer = max(range(len(boxes)), key=lambda i: max(boxes[i][1][0] - boxes[i][0][0], boxes[i][1][1] - boxes[i][0][1]))
    return [b for i, b in enumerate(boxes) if i != outer]


def slots(d):
    """Every slot among the cut-outs: two parallel CUT lines closed by two
    semicircles of one radius, as (centre, end centres, width, length
    between the end centres, its points)."""
    out = []
    for lo, hi, n, pts, members in cutouts(d, with_members=True):
        if n != 4:
            continue
        arcs = [m for m in members if len(m) == 4]
        lines = [m for m in members if len(m) == 2]
        if len(arcs) != 2 or len(lines) != 2:
            continue
        (c0, r0, a0, a1), (c1, r1, b0, b1) = arcs
        if abs(r0 - r1) > 0.02 or any(abs((e - b) % 360.0 - 180.0) > 0.5 for b, e in ((a0, a1), (b0, b1))):
            continue
        if math.dist(c0, c1) < 0.02:
            continue
        a, b = sorted((c0, c1), key=lambda c: (round(c[1], 2), round(c[0], 2)))
        out.append((((a[0] + b[0]) / 2, (a[1] + b[1]) / 2), (a, b), 2 * r0, math.dist(a, b), pts))
    return out


def polar(centres):
    """(centre, radius, start angle, step) when the centres sit equally
    spaced on one circle, else None."""
    n = len(centres)
    if n < 3:
        return None
    c = (sum(p[0] for p in centres) / n, sum(p[1] for p in centres) / n)
    r = math.dist(c, centres[0])
    if r < 1.0 or any(abs(math.dist(c, p) - r) > 0.02 for p in centres):
        return None
    angles = sorted(math.degrees(math.atan2(p[1] - c[1], p[0] - c[0])) % 360.0 for p in centres)
    step = 360.0 / n
    start = angles[0]
    if any(abs(a - (start + step * k) % 360.0) > 0.05 for k, a in enumerate(angles)):
        return None
    if start < 0.05 or 360.0 - start < 0.05:
        start = 0.0
    return c, r, start, step


def grid(centres):
    """(nx, ny, pitch x, pitch y) when the centres fill an equally spaced
    grid, else None."""
    axes = []
    for k in (0, 1):
        vs = []
        for v in sorted(c[k] for c in centres):
            if not vs or v - vs[-1] > 0.02:
                vs.append(v)
        pitch = (vs[-1] - vs[0]) / (len(vs) - 1) if len(vs) > 1 else 0.0
        if any(abs(b - a - pitch) > 0.02 for a, b in zip(vs, vs[1:])):
            return None
        axes.append((len(vs), pitch))
    (nx, px), (ny, py) = axes
    if nx * ny != len(centres) or max(nx, ny) < 3:
        return None
    return nx, ny, px, py


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
    # A cut-out's middle: a field of cut-outs is located by the centres
    # of its first and last cells, a slot by its centre.
    for lo, hi, _, _ in cutouts(d):
        pool.add(round(proj(((lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2)), 3))
    for centre, _, _, _, _ in slots(d):
        pool.add(round(proj(centre), 3))
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
    runs = {}  # direction -> list of (t0, t1, count, pitch)
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
            run = parse_run(txt)
            if run is not None:
                # Equal spaces: `6× 40 (=240)`, the sum the gap and the
                # count times the pitch the gap too (ISO 129-1 7.8.1).
                k, pitch, total = run
                if txt != run_text(k, pitch, v) or abs(k * pitch - v) > 0.005 * k + 1e-6:
                    problems.append(f"{label}: run {txt!r} does not add up to {mm_text(v)}")
                runs.setdefault(key, []).append((ts[i], ts[i + 1], k, pitch))
            elif mm_text(v) != txt:
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
    # Holes, slots and cut-outs, ISO 129-1 7.8 and ISO 15786: a lone
    # feature is located by its centre (a cut-out by any point of it);
    # equal holes on a pitch circle by the circle's centre, its diameter,
    # the number and diameter of the holes, their angular spacing and the
    # first hole's angle from +x when it is not on the axis; six or more
    # equal features on a grid by the first and last cell's centres with
    # the spaces as a run `n× p (=sum)` on the chains. The judge rebuilds
    # every centre from the printed values and finds it in the DXF.
    slot_rows = slots(d)
    slot_pts = {id(row[4]) for row in slot_rows}
    groups = {}
    for c, r in d["circles"]:
        groups.setdefault(("circle", round(2 * r, 1)), []).append((c, [c]))
    for row in slot_rows:
        centre, ends, w, L, pts = row
        groups.setdefault(("slot", round(w, 1), round(L, 1)), []).append((centre, [centre]))
    for lo, hi, n, pts in cutouts(d):
        if n == 4 and any(math.dist(((lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2), row[0]) < 0.02 for row in slot_rows):
            continue
        key = ("cutout", round(hi[0] - lo[0], 2), round(hi[1] - lo[1], 2), n)
        groups.setdefault(key, []).append((((lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2), pts))
    dims = sheet.get("dimensions", [])
    cc_words = ("h.o.h.", "c/c", "entraxe")

    def callout_of(key, n):
        kind = key[0]
        if kind == "circle":
            one = f"⌀{mm_text(key[1])}"
        elif kind == "slot":
            one = None
        else:
            one = f"{mm_text(key[1])} × {mm_text(key[2])}"
        if kind == "slot":
            wants = [f"{mm_text(key[1])} × {mm_text(key[2])} {cc}" for cc in cc_words]
        else:
            wants = [one]
        if n > 1:
            wants = [f"{n}× {w}" for w in wants]
        return wants

    def reconstruct(label, want, have, tol):
        for p in have:
            if not any(math.dist(p, q) <= tol for q in want):
                problems.append(f"{label}: the printed values put no centre at ({mm_text(p[0])}, {mm_text(p[1])})")

    lone = {}  # (kind, size) -> count of features called out one by one
    for key, members in groups.items():
        centres = [m[0] for m in members]
        n = len(members)
        pattern = None
        if key[0] == "circle" and n >= 3:
            pattern = ("polar", polar(centres))
        if pattern is None or pattern[1] is None:
            pattern = ("grid", grid(centres)) if n >= 6 else None
        if pattern is None or pattern[1] is None:
            for c, pts in members:
                xs = [p[0] for p in pts]
                ys = [p[1] for p in pts]
                if not any(located((1.0, 0.0), x) for x in xs) or not any(located((0.0, 1.0), y) for y in ys):
                    problems.append(f"{key[0]} at ({mm_text(c[0])}, {mm_text(c[1])}) not located")
            lone[key] = lone.get(key, 0) + n
            continue
        kind, found = pattern
        label = f"{kind} of {n} {key[0]}s"
        wants = callout_of(key, n)
        if not any(w in words for w in wants):
            problems.append(f"{label}: no callout {wants[0]!r}")
        if kind == "polar":
            c, r, start, step = found
            if not located((1.0, 0.0), c[0]) or not located((0.0, 1.0), c[1]):
                problems.append(f"{label}: centre ({mm_text(c[0])}, {mm_text(c[1])}) not located")
            pcd = f"⌀{mm_text(2 * r)}"
            spacing = f"{n}× {deg_text(step)}"
            printed = {"pitch": pcd, "spacing": spacing}
            if start > 0.0:
                printed["angle"] = deg_text(start)
            for what, txt in printed.items():
                if txt not in words:
                    problems.append(f"{label}: {what} {txt!r} not printed")
                if not any(x["kind"] == what and x["text"] == txt for x in dims):
                    problems.append(f"{label}: {what} {txt!r} not in the report")
            if start == 0.0 and any(x["kind"] == "angle" for x in dims):
                problems.append(f"{label}: an angle is drawn for a hole on the axis")
            # Rebuild the holes from what is printed.
            r2 = parse_mm(pcd[1:]) / 2.0
            step2 = parse_mm(spacing.split("× ")[1].rstrip("°"))
            start2 = parse_mm(printed["angle"].rstrip("°")) if "angle" in printed else 0.0
            want = [arc_point(c, r2, start2 + step2 * k) for k in range(n)]
            reconstruct(label, want, centres, SAME + 0.05 + r * math.radians(0.05))
        else:
            nx, ny, px, py = found
            first = (min(p[0] for p in centres), min(p[1] for p in centres))
            last = (max(p[0] for p in centres), max(p[1] for p in centres))
            rebuilt = [first, last]
            for axis, cnt, pitch, u in ((0, nx, px, (1.0, 0.0)), (1, ny, py, (0.0, 1.0))):
                keyu = (round(u[0], 6), round(u[1], 6))
                for p in (first, last):
                    if not located(u, p[axis]):
                        problems.append(f"{label}: cell at ({mm_text(p[0])}, {mm_text(p[1])}) not located along ({u[0]:g}, {u[1]:g})")
                if cnt < 2:
                    continue
                run = next((x for x in runs.get(keyu, []) if abs(x[0] - first[axis]) <= SAME and abs(x[1] - last[axis]) <= SAME), None)
                if run is None:
                    problems.append(f"{label}: no run {run_text(cnt - 1, pitch, last[axis] - first[axis])!r} on the chain along ({u[0]:g}, {u[1]:g})")
                    continue
                if run[2] != cnt - 1:
                    problems.append(f"{label}: run counts {run[2]} spaces, the field has {cnt - 1}")
                rebuilt.append((cnt, run[3]))
            if len(rebuilt) == 2 + (nx > 1) + (ny > 1):
                steps = iter(rebuilt[2:])
                cx, pxx = next(steps) if nx > 1 else (1, 0.0)
                cy, pyy = next(steps) if ny > 1 else (1, 0.0)
                want = [(first[0] + pxx * i, first[1] + pyy * j) for j in range(cy) for i in range(cx)]
                reconstruct(label, want, centres, SAME + 0.005 * max(cx, cy))
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

    # Callouts of the features outside any pattern: one per size,
    # counting them (`⌀26`, `2× 12 × 28 h.o.h.`).
    for key, n in lone.items():
        if key[0] == "cutout":
            continue
        wants = callout_of(key, n)
        if not any(w in words for w in wants):
            problems.append(f"no callout {wants[0]!r}")

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


# --- Front matter -------------------------------------------------------------


def kg_text(w):
    """A weight as the lists print it."""
    v = round(w, 1) if w >= 10.0 else round(w, 2)
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return s.replace(".", ",")


def dxf_cut_length(d):
    total = 0.0
    for a, b in d["lines"]:
        total += math.hypot(b[0] - a[0], b[1] - a[1])
    for c, r, s, e in d["arcs"]:
        sweep = (e - s) % 360.0
        if sweep < 1e-9:
            sweep = 360.0
        total += r * math.radians(sweep)
    for c, r in d["circles"]:
        total += 2.0 * math.pi * r
    return total


def dxf_hole_count(d):
    """Closed CUT loops besides the outline: circles plus the chains of
    lines and arcs."""
    nodes = []

    def node(p):
        for i, q in enumerate(nodes):
            if abs(q[0] - p[0]) < 1e-3 and abs(q[1] - p[1]) < 1e-3:
                return i
        nodes.append(p)
        return len(nodes) - 1

    parent = {}

    def find(i):
        while parent.get(i, i) != i:
            i = parent[i]
        return i

    def union(i, j):
        parent[find(i)] = find(j)

    ends = []
    for a, b in d["lines"]:
        ends.append((node(a), node(b)))
    for c, r, s, e in d["arcs"]:
        ends.append((node(arc_point(c, r, s)), node(arc_point(c, r, e))))
    for i in range(len(nodes)):
        parent.setdefault(i, i)
    for i, j in ends:
        union(i, j)
    chains = len({find(i) for i in range(len(nodes))})
    return len(d["circles"]) + max(chains - 1, 0)


def segments_cross(a, b, c, d):
    """Whether segments ab and cd cross (touching ends do not count)."""
    def o(p, q, r):
        return (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    return o(a, b, c) * o(a, b, d) < -1e-9 and o(c, d, a) * o(c, d, b) < -1e-9


def link_problems(path, report):
    """Every Link annotation is printable and points at a page; every
    sheet links to the parts list and every parts list row to its sheet."""
    out = []
    lists = {l["kind"]: l for l in report.get("lists", [])}
    bom_pages = set()
    for kind in ("assembly", "bom"):
        if kind in lists:
            l = lists[kind]
            bom_pages.update(range(l["page"], l["page"] + l["pages"]))
    sheet_pages = {s["page"] for s in report["sheets"]}
    with pikepdf.open(path) as pdf:
        ids = [p.objgen for p in pdf.pages]
        targets = {i: set() for i in range(len(ids))}
        for i, page in enumerate(pdf.pages):
            for a in page.get("/Annots", []):
                if a.get("/Subtype") != "/Link":
                    out.append(f"page {i + 1}: {a.get('/Subtype')} annotation")
                    continue
                if int(a.get("/F", 0)) != 4:
                    out.append(f"page {i + 1}: link without the Print flag")
                dest = a.get("/Dest")
                if dest is None or dest[0].objgen not in ids:
                    out.append(f"page {i + 1}: link to nowhere")
                    continue
                targets[i].add(ids.index(dest[0].objgen))
    if bom_pages:
        for s in report["sheets"]:
            if not targets[s["page"]] & bom_pages:
                out.append(f"page {s['page'] + 1}: no link back to the parts list")
        linked = set()
        for p in bom_pages:
            linked |= targets[p]
        for s in report["sheets"]:
            if s["page"] not in linked:
                out.append(f"parts list: no link to page {s['page'] + 1} ({s['title']})")
    return out


def list_problems(doc, report, dxf_dir, prob):
    """The front matter against the sheets and the DXF files."""
    lists = report.get("lists", [])
    by_kind = {}
    for l in lists:
        by_kind.setdefault(l["kind"], []).append(l)
    texts_of = {}
    for l in lists:
        want = "A3" if l["kind"] in ("assembly", "subassembly", "arrangement") else "A4"
        for k in range(l["pages"]):
            idx = l["page"] + k
            if idx >= doc.page_count:
                prob(f"{l['kind']}: page {idx + 1} missing")
                continue
            page = doc[idx]
            tag = f"page {idx + 1} ({l['kind']})"
            w, h = page.rect.width * PT, page.rect.height * PT
            ww, hh = SHEETS[want]
            if abs(w - ww) > 0.5 or abs(h - hh) > 0.5:
                prob(f"{tag}: {w:.1f} × {h:.1f} mm is not {want}")
            texts = page_texts(page)
            texts_of[idx] = texts
            collision_problems(page, texts, prob, tag)

    def words_on(kinds):
        ws = set()
        for kind in kinds:
            for l in by_kind.get(kind, []):
                for k in range(l["pages"]):
                    for t, *_ in texts_of.get(l["page"] + k, []):
                        ws.add(t)
        return ws

    # The parts list.
    bom = words_on(("assembly", "bom"))
    if bom:
        for s in report["sheets"]:
            tag = f"parts list ({s['title']})"
            for want in (str(s["index"]), str(s["quantity"])):
                if want not in bom:
                    prob(f"{tag}: {want!r} not printed")
            if s.get("kind") == "flat":
                want = f"{mm_text(s['blank'][0])} × {mm_text(s['blank'][1])}"
                if want not in bom:
                    prob(f"{tag}: flat size {want!r} not printed")
            size = s.get("extents") or s.get("size")
            if size:
                want = " × ".join(mm_text(v) for v in size)
                if want not in bom:
                    prob(f"{tag}: folded size {want!r} not printed")
            if s.get("weight") is not None and kg_text(s["weight"]) not in bom:
                prob(f"{tag}: weight {kg_text(s['weight'])!r} not printed")
    # Balloons, on the assembly sheet and on every group sheet and detail
    # view: each drawn where the report says, no two leaders of one
    # picture crossing, and every position balloned somewhere.
    sheets_asm = report.get("assemblies") or ([report["assembly"]] if report.get("assembly") else [])
    if sheets_asm:
        seen = set()
        for asm in sheets_asm:
            texts = texts_of.get(asm["page"])
            if texts is None:
                texts = page_texts(doc[asm["page"]])
            tag = f"page {asm['page'] + 1} (assembly)"
            pictures = [asm["balloons"]]
            if asm.get("detail"):
                pictures.append(asm["detail"]["balloons"])
                letter = asm["detail"]["letter"]
                if sum(1 for t, *_ in texts if t == letter) < 1:
                    prob(f"{tag}: detail {letter} has no letter on the picture")
                if not any(letter in t and asm["detail"]["scale"] in t for t, *_ in texts):
                    prob(f"{tag}: detail {letter} has no caption with its scale")
            for balloons in pictures:
                for b in balloons:
                    at = b["at"]
                    hit = any(
                        t == str(b["pos"])
                        and abs((box[0] + box[2]) / 2 - at[0]) < 3.0
                        and abs((box[1] + box[3]) / 2 - at[1]) < 3.0
                        for t, box, *_ in texts
                    )
                    if not hit:
                        prob(f"{tag}: balloon {b['pos']} not drawn at ({at[0]:.1f}, {at[1]:.1f})")
                    seen.add(b["pos"])
                for i, a in enumerate(balloons):
                    for b in balloons[i + 1:]:
                        if segments_cross(a["at"], a["anchor"], b["at"], b["anchor"]):
                            prob(f"{tag}: the leaders of balloons {a['pos']} and {b['pos']} cross")
            for co in asm.get("callouts", []):
                if not any(t == co["text"] for t, *_ in texts):
                    prob(f"{tag}: callout {co['text']!r} not printed")
                if not any(l["page"] == co["page"] for l in lists if l["kind"] == "subassembly"):
                    prob(f"{tag}: callout {co['text']!r} points at page {co['page'] + 1}, not a group sheet")
        for s in report["sheets"]:
            if s["index"] not in seen:
                prob(f"assembly: no balloon for pos {s['index']} ({s['title']})")
    # The laser list.
    laser = words_on(("laser",))
    bend = words_on(("bend",))
    for s in report["sheets"]:
        if s.get("kind") != "flat":
            continue
        try:
            d = read_dxf(os.path.join(dxf_dir, dxf_name(s["partIds"][0])))
        except OSError:
            continue
        tag = f"laser list ({s['title']})"
        if laser:
            cut = mm_text(dxf_cut_length(d))
            if cut not in laser:
                prob(f"{tag}: cut length {cut!r} not printed")
            holes = str(dxf_hole_count(d))
            if holes not in laser:
                prob(f"{tag}: {holes} holes not printed")
            if s.get("cutLength") is not None and abs(s["cutLength"] - dxf_cut_length(d)) > 0.05:
                prob(f"{tag}: report cut length {s['cutLength']:.2f} is not the DXF's {dxf_cut_length(d):.2f}")
        if s.get("bends"):
            if not bend:
                prob(f"bend list missing for {s['title']}")
                continue
            tag = f"bend list ({s['title']})"
            for b in s["bends"]:
                for want in (f"{mm_text(b['angleDeg'])}°", mm_text(b["insideRadius"]), mm_text(b["allowance"])):
                    if want not in bend:
                        prob(f"{tag}: bend {b['nr']}: {want!r} not printed")
            for a, bb in d["bends"]:
                want = mm_text(math.hypot(bb[0] - a[0], bb[1] - a[1]))
                if want not in bend:
                    prob(f"{tag}: bend length {want!r} not printed")
    for s in report["sheets"]:
        if not bend and s.get("bends") and s.get("kind") == "flat":
            prob(f"bend list missing for {s['title']}")


# --- The general arrangement ------------------------------------------------


def part_rows(part):
    """The part's rotation as rows (R = Rx · Ry · Rz when there is no
    matrix), as the view reader does."""
    m = part.get("matrix")
    if m:
        return m
    x, y, z = [math.radians(v) for v in part.get("rotation", [0, 0, 0])]
    rx = [[1, 0, 0], [0, math.cos(x), -math.sin(x)], [0, math.sin(x), math.cos(x)]]
    ry = [[math.cos(y), 0, math.sin(y)], [0, 1, 0], [-math.sin(y), 0, math.cos(y)]]
    rz = [[math.cos(z), -math.sin(z), 0], [math.sin(z), math.cos(z), 0], [0, 0, 1]]

    def mul(a, b):
        return [[sum(a[i][k] * b[k][j] for k in range(3)) for j in range(3)] for i in range(3)]

    return mul(mul(rx, ry), rz)


def de_boor(degree, cp, knots, mults, s):
    """A B-spline point at fraction `s` of its range (de Boor)."""
    u = []
    for k, m in zip(knots, mults):
        u.extend([k] * m)
    p, n = degree, len(cp)
    if len(u) < n + p + 1:
        t = s * (n - 1)
        i = min(int(t), n - 1)
        j = min(i + 1, n - 1)
        f = t - i
        return tuple(cp[i][c] * (1 - f) + cp[j][c] * f for c in range(3))
    lo, hi = u[p], u[n]
    t = lo + (hi - lo) * min(max(s, 0.0), 1.0)
    k = p
    while k + 1 < n and t >= u[k + 1]:
        k += 1
    d = [tuple(cp[j + k - p]) for j in range(p + 1)]
    for r in range(1, p + 1):
        for j in range(p, r - 1, -1):
            i = j + k - p
            den = u[i + p - r + 1] - u[i]
            a = 0.0 if abs(den) < 1e-15 else (t - u[i]) / den
            d[j] = tuple(d[j - 1][c] * (1 - a) + d[j][c] * a for c in range(3))
    return d[p]


def solid_points(solid):
    """Points on the solid's edges: every vertex, every circle edge and
    every B-spline edge sampled finely (what its extents come from)."""
    pts = [tuple(v) for v in solid.get("vertices", [])]
    for e in solid.get("edges", []):
        c = solid["curves"][e["curve"]]
        a, b = solid["vertices"][e["start"]], solid["vertices"][e["end"]]
        if c["kind"] == "circle":
            ax = unit(c["axis"])
            o, r = c["center"], c["radius"]
            va = (a[0] - o[0], a[1] - o[1], a[2] - o[2])
            rad = tuple(va[i] - ax[i] * dot(va, ax) for i in range(3))
            if math.sqrt(dot(rad, rad)) < 1e-9:
                e1 = unit(cross(ax, (1, 0, 0) if abs(ax[0]) < 0.9 else (0, 1, 0)))
            else:
                e1 = unit(rad)
            e2 = cross(ax, e1)
            vb = (b[0] - o[0], b[1] - o[1], b[2] - o[2])
            sweep = math.atan2(dot(vb, e2), dot(vb, e1))
            if e["start"] == e["end"] or (abs(sweep) < 1e-9 and math.dist(a, b) < 1e-6):
                sweep = math.tau
            elif sweep <= 0:
                sweep += math.tau
            n = max(8, int(sweep / math.radians(1.0)))
            for i in range(n + 1):
                t = sweep * i / n
                pts.append(tuple(o[k] + r * (e1[k] * math.cos(t) + e2[k] * math.sin(t)) for k in range(3)))
        elif c["kind"] == "bspline":
            for i in range(0, 65):
                pts.append(de_boor(c["degree"], c["controlPoints"], c["knots"], c["multiplicities"], i / 64))
    if solid.get("mesh"):
        pts.extend(tuple(v) for v in solid["mesh"]["vertices"])
    return pts


def skin_shift(solid):
    """A sheet part (its outer skin one plane) is placed by that plane: a
    flat view has the neutral plane at the origin, the folded view the
    outer skin. The shift that puts the outer skin at the origin (zero
    for a folded part, a bar or a tube)."""
    normal = None
    d = 0.0
    for f in solid.get("faces", []):
        if f.get("role") != "outerSkin":
            continue
        surf = solid["surfaces"][f["surface"]]
        if surf["kind"] != "plane":
            return (0.0, 0.0, 0.0)
        n = unit(surf["normal"])
        if normal is None:
            normal, d = n, dot(surf["origin"], n)
        elif dot(normal, n) < 0.9999:
            return (0.0, 0.0, 0.0)
    if normal is None:
        return (0.0, 0.0, 0.0)
    return (-normal[0] * d, -normal[1] * d, -normal[2] * d)


def placed_points(view, part):
    """The part's edge points in the world, its outer skin at its origin."""
    solid = next(s for s in view["solids"] if s["id"] == part["solid"])
    m = part_rows(part)
    t = part.get("position", [0, 0, 0])
    sh = skin_shift(solid)
    return [
        tuple(m[i][0] * (p[0] + sh[0]) + m[i][1] * (p[1] + sh[1]) + m[i][2] * (p[2] + sh[2]) + t[i] for i in range(3))
        for p in solid_points(solid)
    ]


def bounds_of(pts):
    return [min(p[i] for p in pts) for i in range(3)] + [max(p[i] for p in pts) for i in range(3)]


def view_for_arrangement(name, view, views_root):
    """The view whose placed parts the arrangement shows: the folded twin
    of a flat view when it has the same parts, else the view itself when
    none of its solids is a flat pattern with bends."""
    if name.endswith("_flat") or "_flat_" in name:
        base = name.replace("_flat", "", 1)
        path = os.path.join(views_root or "", base + ".view.json")
        if os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                twin = json.load(f)
            if {p["id"] for p in twin.get("parts", [])} == {p["id"] for p in view.get("parts", [])}:
                return twin
        if any(s.get("bends") for s in view["solids"]):
            return None
    return view


def arrangement_problems(doc, report, view, prob):
    """The general arrangement sheet against the placed parts: the
    extents and the three views' sizes, every overall value, and every
    run of equally spaced parts rebuilt from its printed values."""
    arr = report.get("arrangement")
    sheets = report["sheets"]
    placed = sum(len(s["partIds"]) for s in sheets)
    if not arr:
        if report.get("assembly") and placed >= 2:
            prob("arrangement: no sheet although the assembly has a picture and two or more parts")
        return
    if arr["page"] >= doc.page_count:
        prob("arrangement: page missing")
        return
    tag = f"page {arr['page'] + 1} (arrangement)"
    lst = [l for l in report.get("lists", []) if l["kind"] == "arrangement"]
    if not lst or lst[0]["page"] != arr["page"]:
        prob(f"{tag}: not listed as the arrangement")
    page = doc[arr["page"]]
    texts = page_texts(page)
    words = {t for t, *_ in texts}
    for d in arr["dimensions"]:
        if d["text"] not in words:
            prob(f"{tag}: claimed {d['text']!r} is not printed")
    for names in (("Vooraanzicht", "Front view", "Vue de face"), ("Bovenaanzicht", "Top view", "Vue de dessus"), ("Zijaanzicht van links", "Left side view", "Vue de gauche")):
        if not any(n in words for n in names):
            prob(f"{tag}: view name {names[0]!r} missing")
    s = arr["scaleFactor"]
    views = {v["name"]: v for v in arr["views"]}
    for want in ("front", "top", "left"):
        if want not in views:
            prob(f"{tag}: no {want} view")
            return
    f, t, l = views["front"]["bbox"], views["top"]["bbox"], views["left"]["bbox"]
    # First angle: the top view under the front view, the left view right
    # of it, sharing the axes.
    if t[3] >= f[1] or abs(t[0] - f[0]) > 0.3 or abs(t[2] - f[2]) > 0.3:
        prob(f"{tag}: top view is not under the front view")
    if l[0] <= f[2] or abs(l[1] - f[1]) > 0.3 or abs(l[3] - f[3]) > 0.3:
        prob(f"{tag}: left view is not right of the front view")
    if view is None:
        # A flat view with bent parts and no folded twin of the same
        # parts: nothing to judge the sizes against.
        return
    parts = view.get("parts") or [{"id": so["id"], "solid": so["id"]} for so in view["solids"]]
    by_id = {}
    for p in parts:
        try:
            by_id[p["id"]] = bounds_of(placed_points(view, p))
        except (StopIteration, KeyError):
            prob(f"{tag}: part {p['id']} has no solid")
    drawn = [pid for sh in sheets for pid in sh["partIds"] if pid in by_id]
    if not drawn:
        return
    ext = [min(by_id[p][i] for p in drawn) for i in range(3)] + [max(by_id[p][i + 3] for p in drawn) for i in range(3)]
    for i in range(6):
        if abs(arr["extents"][i] - ext[i]) > 0.5:
            prob(f"{tag}: extents {arr['extents']} are not the parts' {[round(v, 2) for v in ext]}")
            break
    span = [ext[3] - ext[0], ext[4] - ext[1], ext[5] - ext[2]]
    for bx, w, h, name in ((f, span[0], span[2], "front"), (t, span[0], span[1], "top"), (l, span[1], span[2], "left")):
        if abs((bx[2] - bx[0]) - w * s) > 0.6 or abs((bx[3] - bx[1]) - h * s) > 0.6:
            prob(f"{tag}: {name} view is {bx[2] - bx[0]:.1f} × {bx[3] - bx[1]:.1f} mm, the parts {w * s:.1f} × {h * s:.1f}")
    # Overall values.
    overall = {d["axis"]: d for d in arr["dimensions"] if d["kind"] == "overall"}
    for i, axis in enumerate("xyz"):
        d = overall.get(axis)
        if not d:
            prob(f"{tag}: no overall {axis} dimension")
            continue
        if abs(d["value"] - span[i]) > 0.5 or d["text"] != mm_text(d["value"]):
            prob(f"{tag}: overall {axis} {d['text']!r} is not {mm_text(span[i])!r}")
    # Runs: every line of three or more equally spaced parts of one
    # position has its run, and every run rebuilds such a line.
    runs = [d for d in arr["dimensions"] if d["kind"] == "run"]
    for d in runs:
        if d["text"] != run_text(d["count"], d["pitch"], d["count"] * d["pitch"]) and d["text"] != run_text(d["count"], d["pitch"], d["value"]):
            prob(f"{tag}: run {d['text']!r} is not written as count× pitch (=total)")
        parsed = parse_run(d["text"])
        if not parsed or abs(parsed[0] * parsed[1] - parsed[2]) > 0.005 * parsed[0] + 1e-6:
            prob(f"{tag}: run {d['text']!r} does not add up")
    # Bays and levels: every bay runs from one full-height part's centre
    # to the next, and every level from the floor or a lower level to the
    # top face of a placed part. With levels, they carry the heights and
    # the height runs are not wanted.
    bays = [d for d in arr["dimensions"] if d["kind"] == "bay"]
    levels = [d for d in arr["dimensions"] if d["kind"] == "level"]
    tall_x = [(by_id[p][0] + by_id[p][3]) / 2 for p in drawn if by_id[p][5] - by_id[p][2] >= 0.8 * span[2]]
    tops = [by_id[p][5] for p in drawn]
    for d in bays:
        for x in (d["first"], d["first"] + d["value"]):
            if not any(abs(x - c) <= 0.6 for c in tall_x):
                prob(f"{tag}: bay {d['text']!r}: {x:.1f} is no upright's centre")
        if d["text"] != mm_text(d["value"]):
            prob(f"{tag}: bay {d['text']!r} is not {mm_text(d['value'])!r}")
    for a, b in zip(bays, bays[1:]):
        if abs(a["first"] + a["value"] - b["first"]) > 0.1:
            prob(f"{tag}: bays {a['text']!r} and {b['text']!r} do not chain")
    for d in levels:
        z = d["first"] + d["value"]
        if not any(abs(z - t) <= 0.6 for t in tops):
            prob(f"{tag}: level {d['text']!r} ends at {z:.1f}, no part's top")
        starts = [ext[2]] + [e["first"] + e["value"] for e in levels if e["pitch"] == d["pitch"]]
        if not any(abs(d["first"] - z0) <= 0.1 for z0 in starts):
            prob(f"{tag}: level {d['text']!r} starts at {d['first']:.1f}, not the floor or a level")
        if d["text"] != mm_text(d["value"]):
            prob(f"{tag}: level {d['text']!r} is not {mm_text(d['value'])!r}")
    used = set()
    for sh in sheets:
        ids = [pid for pid in sh["partIds"] if pid in by_id]
        if len(ids) < 3:
            continue
        centres = {pid: [(by_id[pid][i] + by_id[pid][i + 3]) / 2 for i in range(3)] for pid in ids}
        taken = set()
        for axis in range(3):
            others = [(axis + 1) % 3, (axis + 2) % 3]
            lines = []
            for pid in ids:
                if pid in taken:
                    continue
                c = centres[pid]
                for ln in lines:
                    c0 = centres[ln[0]]
                    if all(abs(c[o] - c0[o]) <= 0.6 for o in others):
                        ln.append(pid)
                        break
                else:
                    lines.append([pid])
            for ln in lines:
                if len(ln) < 3:
                    continue
                along = sorted(centres[pid][axis] for pid in ln)
                pitch = (along[-1] - along[0]) / (len(along) - 1)
                if pitch < 1.0 or any(abs(b - a - pitch) > 0.1 for a, b in zip(along, along[1:])):
                    continue
                taken.update(ln)
                if axis == 2 and levels:
                    continue
                k = len(along) - 1
                hit = None
                for j, d in enumerate(runs):
                    if d["axis"] == "xyz"[axis] and d["pos"] == sh["index"] and d["count"] == k and abs(d["first"] - along[0]) <= 0.6 and abs(d["pitch"] - pitch) <= 0.1:
                        hit = j
                        break
                if hit is None:
                    prob(f"{tag}: pos {sh['index']}: {len(ln)} parts at {mm_text(pitch)} along {'xyz'[axis]} from {mm_text(along[0])} have no run")
                    continue
                used.add(hit)
                d = runs[hit]
                parsed = parse_run(d["text"])
                # Rebuilt from the printed values alone.
                for i, c in enumerate(along):
                    want = d["first"] + i * parsed[1]
                    if abs(want - c) > 0.6 + 0.005 * i:
                        prob(f"{tag}: run {d['text']!r}: centre {i + 1} rebuilt at {want:.2f} is not {c:.2f}")
                        break
                # The chain to the edges.
                lo, hi = ext[axis], ext[axis + 3]
                for v in (along[0] - lo, hi - along[-1]):
                    if mm_text(v) not in words:
                        prob(f"{tag}: run {d['text']!r}: edge distance {mm_text(v)!r} not printed")
    for j, d in enumerate(runs):
        if j not in used:
            prob(f"{tag}: run {d['text']!r} (pos {d['pos']}) matches no line of parts")


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
    front = sum(l["pages"] for l in report.get("lists", []))
    if doc.page_count != len(report["sheets"]) + front:
        prob(f"{doc.page_count} pages for {len(report['sheets'])} sheets and {front} list pages")
    for p in structure_problems(path):
        prob(p)
    for p in link_problems(path, report):
        prob(p)
    list_problems(doc, report, dxf_dir, prob)
    own_view_path = os.path.join(views_root or "", name + ".view.json")
    try:
        with open(own_view_path, encoding="utf-8") as f:
            own_view = json.load(f)
    except OSError as e:
        own_view = None
        prob(f"arrangement: view: {e}")
    if own_view is not None:
        arrangement_problems(doc, report, view_for_arrangement(name, own_view, views_root), prob)
    if jar:
        v = verapdf(jar, path)
        if v:
            prob(f"veraPDF: {v}")
        else:
            out["pdfa"] = "2b"

    for sheet in report["sheets"]:
        idx = sheet.get("page", 0)
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
        dxf_path = os.path.join(dxf_dir, dxf_name(sheet["partIds"][0]))
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
