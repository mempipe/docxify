// Build the browser bundle.
//
//   node web/build.mjs
//
// Needs a built @mempipe/docxify-editor. Writes web/dist/: the page's script
// bundled with the editor, and the WebAssembly module beside it, where the
// engine looks for it.

import { copyFileSync, readFileSync, rmSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { build } from "esbuild";

const web = dirname(fileURLToPath(import.meta.url));
const dist = join(web, "dist");

rmSync(dist, { recursive: true, force: true });
await build({
  // The browser checks import ./dist/editor.js in the page to call one of the
  // editor's functions, so the editor is an entry too. Splitting gives the
  // two entries one copy of everything they share.
  entryPoints: [
    { in: join(web, "browser-demo.ts"), out: "browser-demo" },
    { in: "@mempipe/docxify-editor/editor", out: "editor" },
  ],
  absWorkingDir: web,
  outdir: dist,
  bundle: true,
  splitting: true,
  format: "esm",
  logLevel: "warning",
});
copyFileSync(
  fileURLToPath(import.meta.resolve("@mempipe/docxify-editor/docx_engine_wasm_bg.wasm")),
  join(dist, "docx_engine_wasm_bg.wasm"),
);

const size = (path) => {
  const bytes = readFileSync(path);
  return `${(statSync(path).size / 1024).toFixed(0)} KiB raw, ${(gzipSync(bytes, { level: 9 }).length / 1024).toFixed(0)} KiB gzip`;
};
console.log(`docx_engine_wasm_bg.wasm: ${size(join(dist, "docx_engine_wasm_bg.wasm"))}`);
console.log(`browser-demo.js: ${size(join(dist, "browser-demo.js"))}`);
