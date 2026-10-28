/**
 * Kubuz data exchange: file formats from the Kubuz kernel's exact-geometry
 * view, in WebAssembly. Runs in a browser, a worker or Node.
 *
 * ```ts
 * import { exactGeometry } from "@kubuzapp/kernel";
 * import { exportDrawings, exportDxf, exportStep } from "@kubuzapp/data-exchange";
 *
 * const view = await exactGeometry(programJson);
 * if (view.ok) {
 *   const file = await exportStep(view.view, "worktable");
 *   if (file.ok) download("worktable.step", file.step);
 * }
 * const flat = await exactGeometry(programJson, undefined, false);
 * if (flat.ok) {
 *   const out = await exportDxf(flat.view);
 *   if (out.ok) for (const f of out.files) download(`${f.name}.dxf`, f.dxf);
 *   const dossier = await exportDrawings(flat.view, { date: "2026-10-28", title: "Worktable" });
 *   if (dossier.ok) download("worktable-drawings.pdf", dossier.pdfBytes());
 * }
 * ```
 */

export type StepResult = { ok: true; step: string } | { ok: false; error: string };

/** What the drawing does not say about a bend line, in the order of the lines. */
export type DxfBend = { id: string; angleDeg: number; direction: "up" | "down"; insideRadius: number };

/** One DXF flat pattern: the blank that `partIds` share, `quantity` times. */
export type DxfFile = { name: string; partIds: string[]; quantity: number; bends: DxfBend[]; dxf: string };

export type DxfResult =
  | { ok: true; files: DxfFile[]; skipped: { id: string; message: string }[] }
  | { ok: false; error: string };

/** What the CPQ knows and the drawings do not. Every field is optional. */
export type DrawingMeta = {
  /** Legal owner (ISO 7200): the title block's name when there is no logo. Default "Kubuz". */
  owner?: string;
  createdBy?: string;
  approvedBy?: string;
  /** `YYYY-MM-DD`. */
  date?: string;
  /** The dossier's title: the configuration or the order. */
  title?: string;
  /** Drawing number prefix; each sheet is `<number>-<position>`. */
  number?: string;
  revision?: string;
  status?: string;
  /** `nl` (default), `en` or `fr`. */
  language?: string;
  /** The general tolerance note. Default "ISO 2768-mK". */
  tolerances?: string;
  /** The owner's logo as SVG path data (M L H V C S Q T Z; no arcs), drawn in the title block. */
  logo?: { width: number; height: number; paths: { d: string; fill?: string }[] };
};

export type DrawingDimension = {
  kind: "ordinate" | "diameter" | "radius" | "angle";
  /** `x` or `y` on a flat sheet; `<view>:x` or `<view>:y` on a folded one. */
  axis?: string;
  value: number;
  text: string;
  count?: number;
  sources: string[];
};

export type DrawingBend = {
  nr: number;
  id: string;
  angleDeg: number;
  direction: "up" | "down";
  insideRadius: number;
  allowance: number;
};

/** One view of a folded part on its sheet. */
export type DrawingView = {
  name: "front" | "left" | "right" | "top" | "bottom" | "isometric";
  /** The view's frame in part coordinates: right, up and towards the viewer. */
  r: [number, number, number];
  u: [number, number, number];
  d: [number, number, number];
  /** The part point at the view's lower-left corner, where its ordinates start. */
  originWorld: [number, number, number];
  /** That corner on the sheet, in mm from the lower left. */
  origin: [number, number];
  scaleFactor: number;
  /** The view's extent in part millimetres. */
  size: [number, number];
  /** Bend angles written on this view (bends that are not 90°). */
  angles: { bend: number; value: number; text: string }[];
  /** Bend numbers tagged on this view. */
  tags: number[];
};

/** One sheet of the dossier: the blank that `partIds` share, `quantity` times. */
export type DrawingSheet = {
  index: number;
  label: string;
  title: string;
  partIds: string[];
  quantity: number;
  paper: "A4" | "A3";
  scale: string;
  scaleFactor: number;
  origin: [number, number];
  blank: [number, number];
  material: string;
  thickness: number;
  dimensions: DrawingDimension[];
  bends: DrawingBend[];
  /** `flat`: the flat pattern; `folded`: orthographic views of the folded part. */
  kind: "flat" | "folded";
  /** Folded sheets: the part's L × W × H. */
  size?: [number, number, number];
  /** Folded sheets: the views drawn. */
  views?: DrawingView[];
  /** Folded sheets: edges shorter than this were treated as reliefs. */
  minorBound?: number;
};

export type DrawingsResult =
  | {
      ok: true;
      /** The PDF/A-2b file, base64. */
      pdf: string;
      sheets: DrawingSheet[];
      skipped: { id: string; message: string }[];
      /** The PDF as bytes, for a Blob or a file. */
      pdfBytes: () => Uint8Array;
    }
  | { ok: false; error: string };

type WasmModule = {
  default: (moduleOrPath?: unknown) => Promise<unknown>;
  exportStep: (viewJson: string, name: string) => string;
  exportDxf: (viewJson: string) => string;
  exportDrawings: (viewJson: string, metaJson: string) => string;
};

export type InitOptions = {
  /**
   * Where the wasm bytes come from: a URL, a Response, bytes, or a compiled
   * WebAssembly.Module. Left out, the glue code fetches the file next to it.
   */
  wasmSource?: unknown;
};

let loading: Promise<WasmModule> | undefined;

/** Load the WebAssembly once. Later calls share the same module. */
export function initDataExchange(options?: InitOptions): Promise<WasmModule> {
  if (!loading) {
    loading = (async () => {
      const mod = (await import("../wasm/kubuz_data_exchange.js")) as unknown as WasmModule;
      if (options?.wasmSource !== undefined) {
        await mod.default({ module_or_path: options.wasmSource });
      } else {
        await mod.default();
      }
      return mod;
    })();
    loading.catch(() => {
      loading = undefined;
    });
  }
  return loading;
}

/**
 * Write a STEP AP214 file from the kernel's exact-geometry view.
 *
 * `view` is the `view` field of the kernel's `exactGeometry` result, as an
 * object or as its JSON text. `name` names the file and, for an assembly,
 * the root product; each part keeps its own id. One part at the origin is
 * written as a plain part, anything else as an assembly.
 */
export async function exportStep(
  view: object | string,
  name: string,
  options?: InitOptions,
): Promise<StepResult> {
  const text = typeof view === "string" ? view : JSON.stringify(view);
  const mod = await initDataExchange(options);
  return JSON.parse(mod.exportStep(text, name)) as StepResult;
}

/**
 * Write DXF flat patterns from a flat exact-geometry view (`exactGeometry`
 * with `folded` false): one file per distinct sheet blank, with the outline
 * and holes on the CUT layer and the bend lines on the BEND layer; the
 * ENGRAVE layer is declared and empty, and no text is written. Each bend's
 * direction, angle and inside radius come back in `bends`. Parts that share
 * a blank share a file; `partIds` lists them. A part with no flat pattern
 * (a folded solid, a bar, a mesh) is listed under `skipped` with the reason.
 */
export async function exportDxf(view: object | string, options?: InitOptions): Promise<DxfResult> {
  const text = typeof view === "string" ? view : JSON.stringify(view);
  const mod = await initDataExchange(options);
  return JSON.parse(mod.exportDxf(text)) as DxfResult;
}

/**
 * Write a dossier of ISO detail drawings, as one PDF/A-2b file, from a flat
 * exact-geometry view (`exactGeometry` with `folded` false): one sheet per
 * distinct sheet blank, with the flat pattern seen from the outer skin,
 * running dimensions from the lower-left corner, hole and radius callouts,
 * numbered bend lines with a bend table, and an ISO 7200 title block. The
 * sheet (A4 or A3) and the scale are the smallest and largest that fit.
 * Every sheet comes back described in `sheets`, dimension by dimension.
 * Parts with no flat pattern are listed under `skipped`.
 */
export async function exportDrawings(
  view: object | string,
  meta: DrawingMeta = {},
  options?: InitOptions,
): Promise<DrawingsResult> {
  const text = typeof view === "string" ? view : JSON.stringify(view);
  const mod = await initDataExchange(options);
  const result = JSON.parse(mod.exportDrawings(text, JSON.stringify(meta))) as DrawingsResult;
  if (result.ok) {
    result.pdfBytes = () => base64ToBytes(result.pdf);
  }
  return result;
}

function base64ToBytes(b64: string): Uint8Array {
  if (typeof atob === "function") {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }
  // No atob (an old runtime): decode by hand.
  const T = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const clean = b64.replace(/[^A-Za-z0-9+/]/g, "");
  const out = new Uint8Array(Math.floor((clean.length * 3) / 4));
  let o = 0;
  for (let i = 0; i < clean.length; i += 4) {
    const have = Math.min(4, clean.length - i);
    const v = (k: number) => (k < have ? T.indexOf(clean[i + k]) : 0);
    const n = (v(0) << 18) | (v(1) << 12) | (v(2) << 6) | v(3);
    out[o++] = (n >> 16) & 255;
    if (have > 2) out[o++] = (n >> 8) & 255;
    if (have > 3) out[o++] = n & 255;
  }
  return out.subarray(0, o);
}
