// Drives the browser editor in headless Chrome, a feature at a time, each on
// its own collaboration server and Chrome: once in a single-user tab and once
// in two tabs sharing a room. Prints every check, then a summary, and exits
// non-zero if any check failed or any feature crashed.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs [feature ...]
//
// The features are the modules in scripts/browser/ other than harness.mjs,
// such as range, table or track; with none named, all run. Needs Google
// Chrome and a built demo (node web/build.mjs).

import { existsSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { runFeature } from "./browser/harness.mjs";

const root = resolve(import.meta.dirname, "..");
const all = readdirSync(join(import.meta.dirname, "browser"))
  .filter((f) => f.endsWith(".mjs") && f !== "harness.mjs")
  .map((f) => f.slice(0, -".mjs".length))
  .sort();
const names = process.argv.length > 2 ? process.argv.slice(2) : all;
const unknown = names.filter((n) => !all.includes(n));
if (unknown.length) {
  console.error(`No such feature: ${unknown.join(", ")}. Features: ${all.join(", ")}`);
  process.exit(1);
}
if (!existsSync(join(root, "web/dist/browser-demo.js"))) {
  console.error("No built demo: run node web/build.mjs");
  process.exit(1);
}

const seconds = (ms) => `${(ms / 1000).toFixed(1)} s`;
const results = [];
const started = performance.now();
for (const name of names) {
  console.log(`\n${name}`);
  const feature = await import(`./browser/${name}.mjs`);
  const t = performance.now();
  try {
    const { checks, failures } = await runFeature(feature);
    results.push({ name, line: `${checks} checks, ${failures} failed`, bad: failures > 0, ms: performance.now() - t });
  } catch (e) {
    console.log(String(e.stack));
    results.push({ name, line: "crashed", bad: true, ms: performance.now() - t });
  }
}

console.log("\nfeature | result | time");
for (const r of results) console.log(`${r.name} | ${r.line} | ${seconds(r.ms)}`);
const bad = results.filter((r) => r.bad).map((r) => r.name);
console.log(bad.length ? `\nFailed: ${bad.join(", ")} (${seconds(performance.now() - started)})` : `\nAll passed (${seconds(performance.now() - started)})`);
process.exit(bad.length ? 1 : 0);
