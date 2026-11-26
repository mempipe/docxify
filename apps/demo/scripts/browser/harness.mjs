// What the browser checks share: a collaboration server and headless Chrome
// for one feature, tabs driven over the DevTools protocol, and the check
// counters. scripts/check-browser.mjs runs the features; each feature module
// exports its fixture (or fixtures, each run in turn), the helpers it adds to
// every tab, and run(mode, a, b), which runs once on a single-user tab
// ("local") and once on two tabs in one shared room ("shared").

import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { launchChrome, chromeEndpoint } from "../chrome.mjs";
const root = resolve(import.meta.dirname, "../..");

async function until(description, ready) {
  const deadline = Date.now() + 15_000;
  while (!await ready()) {
    if (Date.now() >= deadline) throw new Error(`Timed out waiting for ${description}`);
    await new Promise(resolve => setTimeout(resolve, 20));
  }
}
export const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

let checks = 0;
let failures = 0;
export const check = (label, ok, detail) => {
  checks++;
  if (!ok) failures++;
  console.log(`${ok ? "ok" : "FAIL"} ${label}${ok ? "" : ` ${JSON.stringify(detail)}`}`);
};

/** Clicks the middle of `selector` as a mouse would; the feature's helpers define `center`. */
export async function click(t, selector) {
  const [x, y] = await t.ev(`center(${JSON.stringify(selector)})`);
  for (const type of ["mousePressed", "mouseReleased"]) await t.send("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1 });
}

/** Keep the sender visible until its current caret is in awareness. */
export async function waitForSharedCaret(tab) {
  const head = await tab.ev(`(async () => {
    const { pointAt } = await import("./dist/editor.js");
    return docxDemo.session.sticky(pointAt(pm().state.selection.$head));
  })()`);
  if (!head) throw new Error("The selection has no shared caret");
  await tab.wait("the local caret to be shared", `docxDemo.session.awareness.getLocalState()?.cursor?.head === ${JSON.stringify(head)}`);
}

/** The helpers every feature uses, the same in all of them. */
const shared = `(() => {
  window.pm = () => docxDemo.surface.body;
  window.start = (i) => { let at = 0; pm().state.doc.forEach((n, offset, index) => { if (index === i) at = offset + 1; }); return at; };
  window.select = (from, to) => { const v = pm(); v.focus(); const S = v.state.selection.constructor; v.dispatch(v.state.tr.setSelection(S.create(v.state.doc, from, to))); };
  window.key = (key, mods = {}) => pm().dom.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...mods }));
})()`;

// Ports from the system, not a fixed range, so features run at once cannot
// collide and nobody's room on a well-known port is touched.
const freePort = () =>
  new Promise((done) => {
    const server = createServer().listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => done(port));
    });
  });

let running = null;
const stop = async () => {
  if (!running) return;
  const { server, chrome, profile, closed } = running;
  running = null;
  chrome.kill();
  server.kill();
  // Exit is only the main process. Close also waits for inherited pipes;
  // Chrome helpers can still write profile state after the main process exits.
  await closed;
  rmSync(profile, { recursive: true, force: true });
};
process.on("exit", () => {
  if (!running) return;
  running.chrome.kill();
  running.server.kill();
  // This synchronous exit hook cannot wait for helpers. Keep their profile
  // instead of racing their final writes with deletion.
  console.error(`Browser check interrupted; temporary profile retained at ${running.profile}`);
});
process.on("uncaughtException", (e) => {
  console.log(String(e.stack));
  process.exit(1);
});

/**
 * Runs `feature` on its own collaboration server and Chrome, then stops both.
 * Returns the number of checks and failures; a throw means it crashed.
 */
export async function runFeature(feature) {
  checks = 0;
  failures = 0;
  try {
    await session(feature);
  } finally {
    await stop();
  }
  return { checks, failures };
}

async function session(feature) {
  const port = await freePort();
  const server = spawn(process.execPath, ["web/collab-server.ts", "--port", String(port)], { cwd: root, stdio: ["ignore", "pipe", "inherit"] });
  const profile = mkdtempSync(join(tmpdir(), "docx-browser-check-"));
  const debug = await freePort();
  const chrome = launchChrome(profile, debug, feature.windowSize);
  // Register at spawn so an early exit cannot be missed by cleanup.
  const closed = Promise.all([once(chrome, "close"), once(server, "close")]);
  running = { server, chrome, profile, closed };
  await new Promise((ready, fail) => {
    server.stdout.on("data", (chunk) => String(chunk).includes("Collaboration demo") && ready());
    server.on("exit", (code) => fail(new Error(`collaboration server exited with ${code}`)));
  });
  await chromeEndpoint(chrome, debug);

  async function tab(url) {
    const target = await (await fetch(`http://127.0.0.1:${debug}/json/new?about:blank`, { method: "PUT" })).json();
    const ws = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((r) => (ws.onopen = r));
    let id = 0;
    const pending = new Map();
    const errors = [];
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      if (msg.id) {
        pending.get(msg.id)?.(msg);
        pending.delete(msg.id);
      }
      if (msg.method === "Runtime.exceptionThrown") errors.push(msg.params.exceptionDetails.exception?.description);
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
      if (r.result.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description ?? r.result.exceptionDetails.text);
      return r.result.result.value;
    };
    await send("Runtime.enable");
    await send("Page.enable");
    await send("Page.navigate", { url });
    await until("the document and its first painted line", () =>
      ev(`!!window.docxDemo?.surface && document.querySelectorAll(".pages .line").length > 0`).catch(() => false));
    const paint = () => ev(`new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve(true))))`);
    const digest = () => ev(`crypto.subtle.digest("SHA-256", docxDemo.session.replica.export()).then(hash => Array.from(new Uint8Array(hash)).join("."))`);
    const front = async (peer) => {
      const expected = peer && await peer.digest();
      await send("Page.bringToFront");
      if (expected) await until("the visible replica to export the peer's document", async () => await digest() === expected);
      await paint();
    };
    await ev(shared);
    await ev(feature.helpers);
    return { send, ev, errors, paint, front, digest, wait: (label, expression) => until(label, () => ev(expression)) };
  }

  const base = `http://127.0.0.1:${port}/web/index.html`;
  const fixtures = feature.fixtures ?? [feature.fixture];
  for (const fixture of fixtures) {
    if (fixtures.length > 1) console.log(fixture);
    // A feature may add fixture-specific parameters to the address.
    const more = feature.query ? `${feature.query}&` : "";
    await feature.run("local", await tab(`${base}?${more}fixture=${fixture}`), undefined, fixture);
    const room = `${base}?collab&${more}fixture=${fixture}`;
    const one = await tab(room);
    const two = await tab(room);
    await one.front(two);
    await feature.run("shared", one, two, fixture);
  }
}
