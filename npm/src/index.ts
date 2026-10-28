/**
 * Kubuz data exchange: file formats from the Kubuz kernel's exact-geometry
 * view, in WebAssembly. Runs in a browser, a worker or Node.
 *
 * ```ts
 * import { exactGeometry } from "@kubuzapp/kernel";
 * import { exportDxf, exportStep } from "@kubuzapp/data-exchange";
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
 * }
 * ```
 */

export type StepResult = { ok: true; step: string } | { ok: false; error: string };

/** One DXF flat pattern: the blank that `partIds` share, `quantity` times. */
export type DxfFile = { name: string; partIds: string[]; quantity: number; dxf: string };

export type DxfResult =
  | { ok: true; files: DxfFile[]; skipped: { id: string; message: string }[] }
  | { ok: false; error: string };

type WasmModule = {
  default: (moduleOrPath?: unknown) => Promise<unknown>;
  exportStep: (viewJson: string, name: string) => string;
  exportDxf: (viewJson: string) => string;
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
 * and holes on the CUT layer, the bend lines and their notes (direction,
 * angle, inside radius) on the BEND layer and the part's id on the ENGRAVE
 * layer. Parts that share a blank share a file; `partIds` lists them. A
 * part with no flat pattern (a folded solid, a bar, a mesh) is listed under
 * `skipped` with the reason.
 */
export async function exportDxf(view: object | string, options?: InitOptions): Promise<DxfResult> {
  const text = typeof view === "string" ? view : JSON.stringify(view);
  const mod = await initDataExchange(options);
  return JSON.parse(mod.exportDxf(text)) as DxfResult;
}
