// Build the WASM package and the browser bundle.
//
//   node web/build.mjs
//
// Needs the wasm32-unknown-unknown target and wasm-bindgen-cli at the
// version in Cargo.lock. Writes web/pkg/ (wasm-bindgen output) and web/dist/
// (the TypeScript modules with types stripped by Node, for the browser, and
// one esbuild bundle of the collaboration libraries).

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { build } from "esbuild";

const web = dirname(fileURLToPath(import.meta.url));
const root = join(web, "..");
const run = (cmd, args) => execFileSync(cmd, args, { cwd: root, stdio: "inherit" });

run("cargo", ["build", "-p", "docx-engine-wasm", "--target", "wasm32-unknown-unknown", "--profile", "wasm"]);
const wasm = join(root, "target/wasm32-unknown-unknown/wasm/docx_engine_wasm.wasm");
run("wasm-bindgen", ["--target", "web", "--out-dir", join(web, "pkg"), wasm]);

mkdirSync(join(web, "dist"), { recursive: true });
const unstripped = ["node-demo.ts", "collab-check.ts", "collab-server.ts", "collab-deps.ts"];
for (const file of readdirSync(web).filter((f) => f.endsWith(".ts") && !f.endsWith(".test.ts") && !unstripped.includes(f))) {
  const js = stripTypeScriptTypes(readFileSync(join(web, file), "utf8"))
    .replace(/from "(\.\/[^"]+)\.ts"/g, 'from "$1.js"')
    .replace(/from "\.\/pkg\//g, 'from "../pkg/');
  writeFileSync(join(web, "dist", file.replace(/\.ts$/, ".js")), js);
}

await build({
  entryPoints: [join(web, "collab-deps.ts")],
  outfile: join(web, "dist/collab-deps.js"),
  bundle: true,
  format: "esm",
  minify: true,
  logLevel: "warning",
});

const size = (path) => {
  const bytes = readFileSync(path);
  return `${(statSync(path).size / 1024).toFixed(0)} KiB raw, ${(gzipSync(bytes, { level: 9 }).length / 1024).toFixed(0)} KiB gzip`;
};
console.log(`docx_engine_wasm_bg.wasm: ${size(join(web, "pkg/docx_engine_wasm_bg.wasm"))}`);
console.log(`docx_engine_wasm.js: ${size(join(web, "pkg/docx_engine_wasm.js"))}`);
console.log(`collab-deps.js: ${size(join(web, "dist/collab-deps.js"))}`);
