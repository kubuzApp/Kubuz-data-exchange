// Load the built wasm in Node and write one fixture. Exit 1 on any miss.
import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const wasmDir = join(root, "wasm");
const { default: init, exportDrawings, exportDxf, exportStep } = await import(
  pathToFileURL(join(wasmDir, "kubuz_data_exchange.js")).href
);
await init({ module_or_path: await readFile(join(wasmDir, "kubuz_data_exchange_bg.wasm")) });

const view = await readFile(join(root, "..", "fixtures", "deck_splash.view.json"), "utf8");
const result = JSON.parse(exportStep(view, "deck_splash"));
if (!result.ok) {
  console.error(result.error);
  process.exit(1);
}
const edges = (result.step.match(/ORIENTED_EDGE\(/g) ?? []).length;
const parts = (result.step.match(/NEXT_ASSEMBLY_USAGE_OCCURRENCE\(/g) ?? []).length;
if (!result.step.startsWith("ISO-10303-21;") || edges === 0 || parts !== 3) {
  console.error(`unexpected file: ${edges} oriented edges, ${parts} parts`);
  process.exit(1);
}
const bad = JSON.parse(exportStep("{}", "x"));
if (bad.ok) {
  console.error("an empty view must be refused");
  process.exit(1);
}
const flat = await readFile(join(root, "..", "fixtures", "four_down_flat.view.json"), "utf8");
const dxf = JSON.parse(exportDxf(flat));
if (!dxf.ok || dxf.files.length !== 1 || dxf.skipped.length !== 0) {
  console.error(`unexpected dxf result: ${JSON.stringify(dxf).slice(0, 200)}`);
  process.exit(1);
}
const bends = (dxf.files[0].dxf.match(/\nBEND\n/g) ?? []).length;
if (!dxf.files[0].dxf.startsWith("0\nSECTION\n") || bends < 8 || dxf.files[0].quantity !== 1) {
  console.error(`unexpected dxf: ${bends} bend entities`);
  process.exit(1);
}
const drawings = JSON.parse(exportDrawings(flat, JSON.stringify({ date: "2026-10-28", title: "Smoke" })));
if (!drawings.ok || drawings.sheets.length !== 1 || !drawings.pdf.startsWith("JVBERi0xLjc")) {
  console.error(`unexpected drawings result: ${JSON.stringify(drawings).slice(0, 200)}`);
  process.exit(1);
}
const pdfBytes = Buffer.from(drawings.pdf, "base64");
if (pdfBytes.length > 40_000 || drawings.sheets[0].dimensions.length < 8 || drawings.sheets[0].bends.length !== 8) {
  console.error(`unexpected dossier: ${pdfBytes.length} bytes, ${drawings.sheets[0].dimensions.length} dimensions`);
  process.exit(1);
}
console.log(`ok: deck_splash, ${result.step.length} bytes, ${parts} parts; four_down_flat, ${dxf.files[0].dxf.length} bytes of DXF, ${pdfBytes.length} bytes of PDF`);
