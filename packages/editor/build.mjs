// Build the package: the WebAssembly module with its JavaScript glue, then
// the TypeScript modules as JavaScript with their declarations.
//
//   node build.mjs
//
// Needs the wasm32-unknown-unknown target and wasm-bindgen-cli at the
// version in Cargo.lock. Writes src/pkg/ (wasm-bindgen output, which the
// sources import) and dist/.

import { execFileSync } from "node:child_process";
import { cpSync, readFileSync, rmSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "../..");
const run = (cmd, args, cwd) => execFileSync(cmd, args, { cwd, stdio: "inherit" });

run("cargo", ["build", "-p", "docx-engine-wasm", "--target", "wasm32-unknown-unknown", "--profile", "wasm"], root);
const wasm = join(root, "target/wasm32-unknown-unknown/wasm/docx_engine_wasm.wasm");
run("wasm-bindgen", ["--target", "web", "--out-dir", join(here, "src/pkg"), wasm], root);

// A module removed from src/ must not stay importable from an earlier build.
rmSync(join(here, "dist"), { recursive: true, force: true });
run(join(here, "node_modules/.bin/tsc"), ["--project", join(here, "tsconfig.json")], here);
// The compiled modules import ./pkg/ as the sources do.
cpSync(join(here, "src/pkg"), join(here, "dist/pkg"), { recursive: true });

const size = (path) => {
  const bytes = readFileSync(path);
  return `${(statSync(path).size / 1024).toFixed(0)} KiB raw, ${(gzipSync(bytes, { level: 9 }).length / 1024).toFixed(0)} KiB gzip`;
};
console.log(`docx_engine_wasm_bg.wasm: ${size(join(here, "dist/pkg/docx_engine_wasm_bg.wasm"))}`);
console.log(`docx_engine_wasm.js: ${size(join(here, "dist/pkg/docx_engine_wasm.js"))}`);
