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
* every printed ordinate is a coordinate of the DXF (no invented numbers);
* every hole centre, bend-line end and blank extent in the DXF is
  dimensioned;
* every hole diameter has a callout counting the holes of that size;
* every bend line has a numbered tag;
* no two texts overlap, and no line runs through a text;
* the file stays small: 24 KB plus 4 KB a page.

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


def judge(path, dxf_root, jar):
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

        # Claims and must-haves against the DXF.
        dxf_path = os.path.join(dxf_dir, sheet["partIds"][0] + ".dxf")
        try:
            d = read_dxf(dxf_path)
        except OSError as e:
            prob(f"{tag}: dxf: {e}")
            continue
        xs, ys = dxf_coordinates(d)
        (x_lo, x_hi), (y_lo, y_hi) = dxf_extent(d)
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

        # Collisions.
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

    if out["problems"]:
        out["ok"] = False
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("pdfs", nargs="+")
    ap.add_argument("--dxf", required=True, help="folder holding <name>/<part>.dxf per dossier")
    ap.add_argument("--verapdf", help="veraPDF greenfield-apps jar; validates PDF/A-2b when given")
    args = ap.parse_args()
    failed = 0
    for path in args.pdfs:
        result = judge(path, args.dxf, args.verapdf)
        print(json.dumps(result, ensure_ascii=False))
        if not result["ok"]:
            failed += 1
    sys.exit(failed)


if __name__ == "__main__":
    main()
