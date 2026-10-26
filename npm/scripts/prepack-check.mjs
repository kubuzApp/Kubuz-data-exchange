// The tarball must carry the wasm and the compiled TypeScript.
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
for (const f of ["wasm/kubuz_data_exchange_bg.wasm", "wasm/kubuz_data_exchange.js", "dist/index.js"]) {
  if (!existsSync(join(root, f))) {
    console.error(`missing ${f}: run npm run build first`);
    process.exit(1);
  }
}
