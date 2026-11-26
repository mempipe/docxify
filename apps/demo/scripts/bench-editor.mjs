// Times the browser editor: opening a document, then a key, Enter,
// Backspace at a paragraph's start, Shift+Enter, a key after the line break,
// and undo, each in the middle of the body. It runs the single-user editor
// and the shared one (?collab) in headless Chrome, on its own collaboration
// server, so nobody's room is touched.
//
//   node prototypes/docx-engine/scripts/bench-editor.mjs [paragraphs ...]
//
// Add --sustained to time 820 individual keys in the same paragraph. Each
// reported median covers the next 20 keys after the indicated typing history.
//
// Each count names target/bench/large-<n>.docx, which
// `cargo run --release -p docx-engine --example bench -- <n>` writes;
// the default suite also includes complex-mixed. Needs Google Chrome and a built demo
// (node web/build.mjs). MODES=local or MODES=shared runs one editor only.
//
// For each action it prints the median time from the event to the next
// frame, and the main thread's busy time per action split into script,
// style, layout and the rest (paint and compositing), from Chrome's own
// counters. The script time is split again into the editor's stages.
// TRACE=<action>, such as TRACE=key, also traces that action and prints the
// main thread's self time per trace event, and TRACE_OUT=<file> saves the
// last trace for Chrome DevTools' Performance panel. PROFILE=<action> prints
// the functions with the most self time while that action runs, and
// PROFILE=open while each large document opens in the single-user editor.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { launchChrome, chromeEndpoint } from "./chrome.mjs";
const root = resolve(import.meta.dirname, "..");
const sustained = process.argv.includes("--sustained");
const counts = process.argv.slice(2).filter((arg) => arg !== "--sustained").map(Number);
const modes = (process.env.MODES ?? "local,shared").split(",");
for (const n of counts) {
  if (!existsSync(join(root, `target/bench/large-${n}.docx`))) {
    console.error(`No target/bench/large-${n}.docx: run cargo run --release -p docx-engine --example bench -- ${n}`);
    process.exit(1);
  }
}
if (!existsSync(join(root, "web/dist/browser-demo.js"))) {
  console.error("No built demo: run node web/build.mjs");
  process.exit(1);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const freePort = () =>
  new Promise((done) => {
    const server = createServer().listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => done(port));
    });
  });

const port = await freePort();
const server = spawn(process.execPath, ["web/collab-server.ts", "--port", String(port)], { cwd: root, stdio: ["ignore", "pipe", "inherit"] });
await new Promise((ready) => server.stdout.on("data", (chunk) => String(chunk).includes("Collaboration demo") && ready()));
const base = `http://127.0.0.1:${port}`;

const profile = mkdtempSync(join(tmpdir(), "bench-editor-"));
const debug = await freePort();
const chrome = launchChrome(profile, debug);
const quit = (code) => {
  chrome.kill();
  server.kill();
  setTimeout(() => rmSync(profile, { recursive: true, force: true }), 500);
  setTimeout(() => process.exit(code), 600);
};
process.on("uncaughtException", (e) => {
  console.error(e);
  quit(1);
});

const browser = (await chromeEndpoint(chrome, debug)).webSocketDebuggerUrl;

/** A CDP connection to one page. */
async function connect(url) {
  const ws = new WebSocket(url);
  await new Promise((r) => (ws.onopen = r));
  let id = 0;
  const pending = new Map();
  const errors = [];
  const trace = { events: [], done: null };
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id) {
      pending.get(msg.id)?.(msg);
      pending.delete(msg.id);
    }
    if (msg.method === "Runtime.exceptionThrown") errors.push(msg.params.exceptionDetails.exception?.description);
    if (msg.method === "Tracing.dataCollected") trace.events.push(...msg.params.value);
    if (msg.method === "Tracing.tracingComplete") trace.done?.();
  };
  const send = (method, params = {}) =>
    new Promise((r) => {
      const i = ++id;
      pending.set(i, r);
      ws.send(JSON.stringify({ id: i, method, params }));
    });
  const ev = async (expression) => {
    const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.error) throw new Error(r.error.message);
    if (r.result.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description);
    return r.result.result.value;
  };
  await send("Runtime.enable");
  await send("Page.enable");
  await send("DOM.enable");
  await send("Performance.enable");
  return { send, ev, errors, trace, close: () => ws.close() };
}

const ready = `!!window.docxDemo?.surface && document.querySelectorAll(".pages .line").length > 0`;
async function until(page, expression, limit = 60_000) {
  const start = performance.now();
  while (performance.now() - start < limit) {
    if (await page.ev(expression).catch(() => false)) return performance.now() - start;
    await sleep(20);
  }
  throw new Error(`timed out waiting for ${expression}`);
}

/** Opens `file` (relative to the engine) in a fresh page, and how long it took to show its pages. */
async function open(file, mode) {
  const { targetId } = (await (await connect(browser)).send("Target.createTarget", { url: "about:blank" })).result;
  const target = (await (await fetch(`http://127.0.0.1:${debug}/json/list`)).json()).find((t) => t.id === targetId);
  const page = await connect(target.webSocketDebuggerUrl);
  const fixture = file.startsWith("fixtures/") ? file.slice("fixtures/".length) : null;
  let url = `${base}/web/index.html?fixture=${encodeURIComponent(fixture ?? "generated/complex-mixed.docx")}`;
  if (mode === "shared") {
    let query = `fixture=${encodeURIComponent(fixture ?? "")}`;
    if (!fixture) {
      const response = await fetch(`${base}/upload`, { method: "POST", body: readFileSync(join(root, file)) });
      query = `upload=${(await response.json()).upload}`;
    }
    url = `${base}/web/index.html?collab&${query}`;
  }
  const start = performance.now();
  await page.send("Page.navigate", { url });
  await until(page, ready);
  let opened = performance.now() - start;
  if (mode === "local" && !fixture) {
    await page.ev(`window.previous = docxDemo.surface; true`);
    const { root: doc } = (await page.send("DOM.getDocument")).result;
    const { nodeId } = (await page.send("DOM.querySelector", { nodeId: doc.nodeId, selector: "#file" })).result;
    const opening = async () => {
      await page.send("DOM.setFileInputFiles", { nodeId, files: [join(root, file)] });
      opened = await until(page, `docxDemo.surface !== window.previous && ${ready}`);
    };
    if (process.env.PROFILE === "open") console.log(`profile of opening ${file}, ms:\n${await profiled(page, opening, 1)}`);
    else await opening();
  }
  return { page, opened };
}

/** Wraps the editor's stages so each action's script time can be split. */
const instrument = `(() => {
  const s = docxDemo.surface;
  const spent = (window.spent = {});
  const wrap = (obj, name, label) => {
    const f = obj[name];
    if (typeof f !== "function" || f.wrapped) return;
    obj[name] = function (...args) {
      const t = performance.now();
      try { return f.apply(this, args); } finally { spent[label] = (spent[label] ?? 0) + performance.now() - t; }
    };
    obj[name].wrapped = true;
  };
  wrap(s, "layout", "layout");
  wrap(s.blocks, "layout", "layout.blocks");
  wrap(s.paginator, "layout", "layout.paginate");
  wrap(s.painter, "render", "layout.paint");
  wrap(s, "decorate", "decorate");
  const engine = docxDemo.session.replica;
  for (const name of ["apply", "inspectNode", "inspectChildren", "inspect", "takeRedraw", "undo", "redo", "text"]) wrap(engine, name, "engine." + name);
})()`;

/** Puts the caret in the middle of the body's middle paragraph. */
const caret = `(() => {
  const view = docxDemo.surface.body;
  const target = view.state.doc.childCount >> 1;
  let pos = null;
  for (let i = target; pos === null && i < view.state.doc.childCount; i++) {
    view.state.doc.forEach((node, offset, index) => {
      if (index === i && node.type.name === "paragraph" && node.content.size > 4) pos = offset + 1 + (node.content.size >> 1);
    });
  }
  view.focus();
  view.dispatch(view.state.tr.setSelection(view.state.selection.constructor.near(view.state.doc.resolve(pos))));
})()`;

const KEYS = { key: "x", enter: "Enter", join: "Backspace", break: "Enter", after: "y", undo: "z" };
/** One action in the page, timed to the next frame; returns milliseconds. */
// It runs in a task of its own: Chrome's script counter leaves out script
// that a DevTools evaluation runs directly.
const action = (name) => `new Promise((done) => setTimeout(() => {
  const view = docxDemo.surface.body;
  const t = performance.now();
  if (${JSON.stringify(name)} === "key" || ${JSON.stringify(name)} === "after") {
    view.dispatch(view.state.tr.insertText(${JSON.stringify(KEYS[name])}));
  } else {
    view.dom.dispatchEvent(new KeyboardEvent("keydown", {
      key: ${JSON.stringify(KEYS[name])},
      shiftKey: ${name === "break"},
      metaKey: ${name === "undo"},
      bubbles: true,
      cancelable: true,
    }));
  }
  requestAnimationFrame(() => setTimeout(() => done(performance.now() - t)));
}))`;

const COUNTERS = ["ScriptDuration", "RecalcStyleDuration", "LayoutDuration", "TaskDuration"];
const counters = async (page) =>
  Object.fromEntries((await page.send("Performance.getMetrics")).result.metrics.filter((m) => COUNTERS.includes(m.name)).map((m) => [m.name, m.value * 1000]));
const median = (xs) => [...xs].sort((a, b) => a - b)[xs.length >> 1];
const ms = (x) => (x < 10 ? x.toFixed(1) : Math.round(x).toString());

/** Runs `name` `n` times in a row, `between` ms apart, so typing is not grouped for undo unless meant. */
async function measure(page, names, n) {
  const times = Object.fromEntries(names.map((name) => [name, []]));
  const busy = Object.fromEntries(names.map((name) => [name, {}]));
  const stages = Object.fromEntries(names.map((name) => [name, {}]));
  for (let i = 0; i < n; i++) {
    for (const name of names) {
      await page.ev(`window.spent = Object.keys(window.spent).length ? (Object.keys(window.spent).forEach((k) => delete window.spent[k]), window.spent) : window.spent`);
      const before = await counters(page);
      times[name].push(await page.ev(action(name)));
      const after = await counters(page);
      for (const k of COUNTERS) busy[name][k] = (busy[name][k] ?? 0) + (after[k] - before[k]) / n;
      for (const [k, v] of Object.entries(await page.ev("window.spent"))) stages[name][k] = (stages[name][k] ?? 0) + v / n;
      await sleep(30);
    }
  }
  return names.map((name) => {
    const b = busy[name];
    const rest = b.TaskDuration - b.ScriptDuration - b.RecalcStyleDuration - b.LayoutDuration;
    return {
      name,
      median: ms(median(times[name])),
      busy: ms(b.TaskDuration),
      script: ms(b.ScriptDuration),
      style: ms(b.RecalcStyleDuration),
      layout: ms(b.LayoutDuration),
      rest: ms(rest),
      stages: Object.entries(stages[name])
        .filter(([, v]) => v >= 0.05)
        .sort((a, b) => b[1] - a[1])
        .map(([k, v]) => `${k} ${ms(v)}`)
        .join(", "),
    };
  });
}

/** The renderer main thread's self time per trace event name while `name` runs `n` times. */
async function traced(page, name, n) {
  page.trace.events = [];
  await page.send("Tracing.start", { traceConfig: { includedCategories: ["devtools.timeline", "disabled-by-default-devtools.timeline", "v8.execute", "blink", "cc", "gpu"] } });
  for (let i = 0; i < n; i++) {
    await page.ev(action(name));
    await sleep(30);
  }
  await new Promise((r) => {
    page.trace.done = r;
    page.send("Tracing.end");
  });
  const events = page.trace.events;
  if (process.env.TRACE_OUT) writeFileSync(process.env.TRACE_OUT, JSON.stringify({ traceEvents: events }));
  const main = new Set(events.filter((e) => e.name === "thread_name" && e.args?.name === "CrRendererMain").map((e) => `${e.pid}:${e.tid}`));
  const spans = events.filter((e) => e.ph === "X" && e.dur && main.has(`${e.pid}:${e.tid}`)).sort((a, b) => a.ts - b.ts || b.dur - a.dur);
  const self = new Map();
  const stack = [];
  for (const e of spans) {
    while (stack.length && stack.at(-1).ts + stack.at(-1).dur <= e.ts) stack.pop();
    const parent = stack.at(-1);
    if (parent) parent.child = (parent.child ?? 0) + e.dur;
    stack.push(e);
  }
  for (const e of spans) self.set(e.name, (self.get(e.name) ?? 0) + e.dur - (e.child ?? 0));
  return [...self]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 16)
    .map(([k, v]) => `${k} ${ms(v / 1000 / n)}`)
    .join(", ");
}

/** The functions with the most self time while `run` runs `n` times, from V8's sampling profiler. */
async function profiled(page, run, n) {
  await page.send("Profiler.enable");
  await page.send("Profiler.setSamplingInterval", { interval: 50 });
  await page.send("Profiler.start");
  for (let i = 0; i < n; i++) {
    await run();
    await sleep(30);
  }
  const { profile } = (await page.send("Profiler.stop")).result;
  const total = new Map();
  const byId = new Map(profile.nodes.map((node) => [node.id, node]));
  const deltas = profile.timeDeltas;
  for (let i = 0; i < profile.samples.length; i++) {
    const { callFrame: f } = byId.get(profile.samples[i]);
    const label = `${f.functionName || "(anonymous)"} ${f.url.split("/").pop()}:${f.lineNumber + 1}`;
    total.set(label, (total.get(label) ?? 0) + (deltas[i + 1] ?? 0));
  }
  return [...total]
    .filter(([k]) => !k.startsWith("(idle)") && !k.startsWith("(program)"))
    .sort((a, b) => b[1] - a[1])
    .slice(0, 25)
    .map(([k, v]) => `  ${ms(v / 1000 / n)} ${k}`)
    .join("\n");
}

async function measureSustained(page) {
  const paragraph = "docxDemo.surface.body.state.selection.$from.parent.textContent";
  const [prefix, suffix] = await page.ev(`(() => {
    const { parent, parentOffset } = docxDemo.surface.body.state.selection.$from;
    return [parent.textBetween(0, parentOffset), parent.textBetween(parentOffset, parent.content.size)];
  })()`);
  let typed = 0;
  for (const start of [0, 100, 200, 400, 800]) {
    while (typed < start) {
      await page.ev(action("key"));
      typed++;
    }
    const [row] = await measure(page, ["key"], 20);
    typed += 20;
    if (await page.ev(paragraph) !== prefix + "x".repeat(typed) + suffix) {
      throw new Error("typing left the selected paragraph or lost a key");
    }
    console.log(`after ${start} typed characters: ${row.median} ms median to next frame (${row.stages})`);
  }
}

const files = counts.map((n) => `target/bench/large-${n}.docx`);
if (!sustained || !files.length) files.unshift("fixtures/generated/complex-mixed.docx");
console.log(`Chrome ${(await (await fetch(`http://127.0.0.1:${debug}/json/version`)).json()).Browser}, ${new Date().toISOString()}`);
for (const file of files) {
  for (const mode of modes) {
    const { page, opened } = await open(file, mode);
    await sleep(500);
    await page.ev(instrument);
    await page.ev(caret);
    await sleep(200);
    const info = await page.ev(`({ pages: docxDemo.surface.pages.length, blocks: docxDemo.surface.body.state.doc.childCount, dom: document.getElementsByTagName("*").length })`);
    console.log(`\n${file.split("/").pop()} ${mode}: opened in ${ms(opened)} ms; ${info.pages} pages, ${info.blocks} body blocks, ${info.dom} elements`);
    if (sustained) {
      await measureSustained(page);
      if (page.errors.length) throw new Error(page.errors.join("\n"));
      page.close();
      continue;
    }
    // Typing is measured apart from undo, then the structural keys in pairs
    // that leave the document as it was, then undo of what they did.
    const rows = [
      ...(await measure(page, ["key"], 20)),
      ...(await measure(page, ["enter", "join"], 10)),
      ...(await measure(page, ["break", "after"], 5)),
      ...(await measure(page, ["undo"], 10)),
    ];
    console.log("action | to next frame | busy | script | style | layout | rest | script stages");
    for (const r of rows) console.log(`${r.name} | ${r.median} | ${r.busy} | ${r.script} | ${r.style} | ${r.layout} | ${r.rest} | ${r.stages}`);
    const name = process.env.PROFILE;
    if (name && name !== "open") console.log(`profile of ${name}, ms each:\n${await profiled(page, () => page.ev(action(name)), 20)}`);
    if (process.env.TRACE) console.log(`trace of ${process.env.TRACE}, ms each:`, await traced(page, process.env.TRACE, 10));
    if (page.errors.length) console.log("page errors:", page.errors.join("\n"));
    page.close();
  }
}
quit(0);
