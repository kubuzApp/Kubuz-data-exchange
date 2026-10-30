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
  /** `YYYY-MM-DD`; blank when not approved. */
  approvedDate?: string;
  /** The customer named in the title block. */
  customer?: string;
  /** The owner's address lines under its name in the title block. */
  address?: string[];
  /** Density in kg/m³ by material name, for the weight in the title block; a built-in table covers the usual steels, stainless steels, aluminium, copper, brass, zinc and titanium. */
  densities?: Record<string, number>;
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
  /** `flange`, `width`, `height` and `angle` belong to a section view. */
  kind: "ordinate" | "diameter" | "radius" | "angle" | "flange" | "width" | "height";
  /** `x` or `y` on a flat sheet; `<view>:x` or `<view>:y` on a folded one. */
  axis?: string;
  value: number;
  text: string;
  count?: number;
  sources: string[];
  /** The section view's letter when the dimension is on one. */
  section?: string;
};

export type DrawingBend = {
  nr: number;
  id: string;
  angleDeg: number;
  direction: "up" | "down";
  insideRadius: number;
  allowance: number;
  /** The bend table's note for this bend on a flat sheet. */
  note?: string;
};

/** One station of a chain dimension: a position along the chain's direction (model mm) and the features that put it there. */
export type DrawingStation = { t: number; sources: string[] };

/** One chain of running dimensions on a flat sheet: along `direction` from the first station to the last, each value the distance between consecutive stations. */
export type DrawingChain = {
  kind: "bend" | "outline" | "hole" | "overall";
  axis: "x" | "y" | "oblique";
  direction: [number, number];
  /** -1: below or left of the blank; 1: above or right. */
  side: -1 | 1;
  stations: DrawingStation[];
  values: number[];
  texts: string[];
};

/** One piece of a section's profile, entry to exit: a flat length or a bend. */
export type DrawingPiece = {
  kind: "flat" | "bend";
  /** Flat pieces: the outside length in model mm. */
  length?: number;
  /** The drawn length of an interrupted flange. */
  shown?: number;
  /** Bend pieces: the bend number. */
  bend?: number;
};

/** A section view of the folded part on a flat sheet: one cut across the bends it shows. */
export type DrawingSection = {
  letter: string;
  /** The cut on the blank (model mm), entry to exit. */
  cut: [[number, number], [number, number]];
  direction: [number, number];
  bends: number[];
  pieces: DrawingPiece[];
  scale: string;
  scaleFactor: number;
  /** Paper position of the walk's origin (the cut's entry, outer skin), in mm from the lower left. */
  origin: [number, number];
  width: number;
  height: number;
  dimensions: DrawingDimension[];
};

/** One view of a folded part on its sheet. */
export type DrawingView = {
  /** A principal view, the isometric, or an auxiliary view `aux A`, `aux B`, … */
  name: string;
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
  /** Auxiliary views: the zone (flange) shown on its own. */
  partial?: string;
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
  /** Flat sheets: the running dimensions around the flat pattern. */
  chains?: DrawingChain[];
  bends: DrawingBend[];
  /** Flat sheets: section views of the folded flanges. */
  sections?: DrawingSection[];
  /** Flat sheets with bends: the folded part's L × W × H along the base flange's axes. */
  extents?: [number, number, number];
  /** Flat sheets: the isometric pictorial of the folded part, when it found room. */
  isometric?: { scale: string; scaleFactor: number; origin: [number, number]; bbox: [number, number, number, number] };
  /** `flat`: the flat pattern; `folded`: orthographic views of the folded part. */
  kind: "flat" | "folded";
  /** Folded sheets: the part's L × W × H. */
  size?: [number, number, number];
  /** Folded sheets: the views drawn. */
  views?: DrawingView[];
  /** Folded sheets: edges shorter than this were treated as reliefs. */
  minorBound?: number;
  /** The sheet's page in the dossier, 0-based. */
  page: number;
  /** `laser` (cut only), `laser+bend` or `folded` (drawn from a folded view). */
  route: "laser" | "laser+bend" | "folded";
  /** The position this blank is the mirror image of. */
  mirrorOf?: number;
  /** Per piece, kg, when the material's density is known. */
  weight?: number;
  /** Outline plus holes, mm. */
  cutLength?: number;
  /** Net area of the blank, mm². */
  area?: number;
  holes?: number;
};

/** One section of the dossier's front matter and where it is. */
export type DrawingList = {
  kind: "cover" | "assembly" | "subassembly" | "arrangement" | "bom" | "laser" | "bend";
  title: string;
  /** First page, 0-based, and how many pages. */
  page: number;
  pages: number;
};

export type DrawingBalloon = { pos: number; at: [number, number]; anchor: [number, number] };

/** An assembly sheet: the isometric of the placed parts with item
 * balloons. A complex assembly (two or more groups and more positions
 * than one picture holds) gets an overview that names the groups
 * (`callouts`, each linking to its group's sheet) and a sheet per group
 * (`group`), which shows the assembly as far as it is built when that
 * group goes on. Parts too small at the sheet's scale get a detail view. */
export type DrawingAssembly = {
  page: number;
  paper: "A3";
  scale: string;
  scaleFactor: number;
  origin: [number, number];
  bbox: [number, number, number, number];
  balloons: DrawingBalloon[];
  /** The group key of a group sheet; `null` on the overview. */
  group: string | null;
  callouts: { group: string; text: string; at: [number, number]; anchor: [number, number]; page: number }[];
  detail: {
    letter: string;
    scale: string;
    scaleFactor: number;
    /** The circle on the main picture, sheet mm. */
    centre: [number, number];
    radius: number;
    origin: [number, number];
    /** The enlarged circle on the sheet. */
    at: [number, number];
    atRadius: number;
    balloons: DrawingBalloon[];
  } | null;
};

/** One dimension of the general arrangement: the extents along an axis
 * (`overall`), the equal spaces between one position's parts (`run`,
 * `count× pitch (=total)` from the first centre), from one upright centre
 * to the next (`bay`, `first` = the left centre) or a bay's floor or level
 * to its next level (`level`, `first` = where it starts, `pitch` = the
 * bay's left upright centre). */
export type DrawingArrangementDimension = {
  kind: "overall" | "run" | "bay" | "level";
  axis: "x" | "y" | "z";
  view: "front" | "top" | "left";
  text: string;
  value: number;
  pos?: number;
  count?: number;
  pitch?: number;
  first?: number;
};

/** The general arrangement sheet (ISO 128-30, first angle): front, top
 * and left side views with the overall sizes and the pitch runs. */
export type DrawingArrangement = {
  page: number;
  paper: "A3";
  scale: string;
  scaleFactor: number;
  /** `[x0, y0, z0, x1, y1, z1]` of the placed parts, mm. */
  extents: [number, number, number, number, number, number];
  views: { name: "front" | "top" | "left"; origin: [number, number]; bbox: [number, number, number, number] }[];
  dimensions: DrawingArrangementDimension[];
};

export type DrawingsResult =
  | {
      ok: true;
      /** The PDF/A-2b file, base64. */
      pdf: string;
      /** The sheets in dossier order: positions follow the production route. */
      sheets: DrawingSheet[];
      /** Cover, assembly and parts list, general arrangement, laser list, bend list. */
      lists: DrawingList[];
      /** The overview; `null` when no part folds or places. */
      assembly: DrawingAssembly | null;
      /** Every assembly sheet: the overview first, then one per group when split. */
      assemblies: DrawingAssembly[];
      /** `null` when fewer than two parts place. */
      arrangement: DrawingArrangement | null;
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
