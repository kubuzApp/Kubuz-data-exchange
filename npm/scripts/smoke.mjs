// Load the built wasm in Node and write one fixture. Exit 1 on any miss.
import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const wasmDir = join(root, "wasm");
const { default: init, exportStep } = await import(
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
console.log(`ok: deck_splash, ${result.step.length} bytes, ${parts} parts`);
