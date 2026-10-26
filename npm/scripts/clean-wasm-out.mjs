// Runs straight after wasm-pack. It writes a `.gitignore` containing `*`
// into its out-dir, and npm's pack list honours a nested .gitignore even for
// a directory named in `files`, so the package would ship without its wasm.
import { existsSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const wasmDir = resolve(dirname(fileURLToPath(import.meta.url)), "..", "wasm");
if (!existsSync(wasmDir)) {
  console.error(`no wasm output at ${wasmDir}: did wasm-pack run?`);
  process.exit(1);
}
const ignore = join(wasmDir, ".gitignore");
if (existsSync(ignore)) rmSync(ignore);
