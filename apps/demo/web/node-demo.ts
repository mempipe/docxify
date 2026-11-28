// The engine through WASM, in Node: open, inspect, edit, export, validate,
// re-open, and lay out with a fixed measurer. Runs the illustrated flow on
// one document, then the same checks over every fixture, and exits non-zero
// on any failure.
//
//   node web/node-demo.ts [out-dir]

import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { Blocks } from "@mempipe/docxify-editor/blocks";
import { loadDocxEngine } from "@mempipe/docxify-editor/engine";
import { markup } from "@mempipe/docxify-editor/markup";
import { FixedMeasurer } from "@mempipe/docxify-editor/measure";
import { Paginator } from "@mempipe/docxify-editor/paginate";
import type { Inspection, InspectNode, NodeId } from "@mempipe/docxify-editor/types";

const root = fileURLToPath(new URL("..", import.meta.url));
const outDir = process.argv[2] ?? join(root, "target", "web-demo");
mkdirSync(outDir, { recursive: true });

const started = performance.now();
const engine = await loadDocxEngine(readFileSync(fileURLToPath(import.meta.resolve("@mempipe/docxify-editor/docx_engine_wasm_bg.wasm"))));
console.log(`engine loaded in ${(performance.now() - started).toFixed(1)} ms`);

function* walk(node: InspectNode): Generator<InspectNode> {
  yield node;
  for (const child of node.children ?? []) yield* walk(child);
}
const firstEditable = (stories: InspectNode[]): NodeId | undefined =>
  stories.flatMap((s) => [...walk(s)]).find((n) => n.type === "text" && n.support === "editable" && n.text)?.id;
const fail = (message: string): never => {
  console.error(`FAIL ${message}`);
  process.exit(1);
};
function pages(inspection: Inspection) {
  const measurer = new FixedMeasurer();
  const blocks = new Blocks((family) => family);
  const input = blocks.layout(inspection);
  const laid = new Paginator(measurer).layout(input);
  return { pages: laid.length, balloons: markup(laid, input.comments, measurer, (run) => blocks.style(run)).pages.flat().length };
}

// The illustrated flow.
const bytes = new Uint8Array(readFileSync(join(root, "fixtures/generated/complex-mixed.docx")));
const doc = engine.seed(1, bytes);
const tree = doc.inspect();
const target = [...walk(tree.stories[0])].find((n) => n.type === "text" && n.text === "First point")?.id;
if (!target) fail("no 'First point' text node");
console.log(`opened: ${tree.stories.length} stories, ${tree.unsupported.length} kinds of content preserved without editing`);
doc.apply([{ command: "replace_text", node: target!, text: "New text" }]);
console.log(`Replaced text at ${target}`);
try {
  doc.apply([{ command: "replace_text", node: [...walk(tree.stories[0])].find((n) => n.type === "text" && n.support === "read_only")!.id, text: "x" }]);
  fail("editing read-only text was accepted");
} catch (e) {
  console.log(`read-only edit refused: ${(e as Error).message}`);
}
const output = doc.export();
const report = engine.validate(output);
if (!report.valid) fail(`export invalid: ${JSON.stringify(report.issues)}`);
const reopened = engine.open(output);
if (reopened.text(target!) !== "New text") fail("edit not visible after re-open");
writeFileSync(join(outDir, "complex-mixed.edited.docx"), output);
console.log(`exported ${output.length} bytes, valid, re-opened text "${reopened.text(target!)}"; wrote ${outDir}`);

// The same checks over every fixture, with timings.
let count = 0;
for (const dir of ["fixtures/generated", "fixtures/real-world"]) {
  for (const name of readdirSync(join(root, dir)).filter((f) => f.endsWith(".docx")).sort()) {
    const input = new Uint8Array(readFileSync(join(root, dir, name)));
    const t0 = performance.now();
    const d = engine.seed(1, input);
    const t1 = performance.now();
    const inspection = d.inspect();
    const t2 = performance.now();
    const node = firstEditable(inspection.stories);
    if (!node) fail(`${name}: no editable text`);
    d.apply([{ command: "replace_text", node: node!, text: "Edited in WASM" }]);
    const t3 = performance.now();
    const out = d.export();
    const t4 = performance.now();
    const r = engine.validate(out);
    if (!r.valid) fail(`${name}: ${JSON.stringify(r.issues)}`);
    if (engine.open(out).text(node!) !== "Edited in WASM") fail(`${name}: edit lost`);
    const t5 = performance.now();
    const laid = pages(inspection);
    const t6 = performance.now();
    if (laid.pages < 1) fail(`${name}: no pages laid out`);
    d.free();
    count++;
    console.log(
      `ok ${dir}/${name}: ${(input.length / 1024).toFixed(0)} KiB, open ${(t1 - t0).toFixed(2)} ms, inspect ${(t2 - t1).toFixed(2)} ms, export ${(t4 - t3).toFixed(2)} ms, ${laid.pages} page${laid.pages === 1 ? "" : "s"} and ${laid.balloons} balloon${laid.balloons === 1 ? "" : "s"} laid out in ${(t6 - t5).toFixed(2)} ms`,
    );
  }
}
console.log(`${count} fixtures passed through WASM`);
