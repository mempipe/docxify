// Compares the browser layout with Word's: page counts, and how many lines
// hold the same text. Word exports each fixture to PDF and pdftotext reads
// its lines; headless Chrome lays the same fixture out in the demo. Lines are
// compared per page as text with whitespace removed, so tab widths and list
// marker spacing do not count, and a line broken at a different word does.
// A line only one side has counts against the layout, whichever side it is.
//
//   node prototypes/docx-engine/scripts/compare-with-word.mjs [generated/notes.docx ...]
//
// Needs Microsoft Word, Google Chrome, pdftotext and pdfinfo (poppler), and a
// built demo (node web/build.mjs). Word's sandbox asks once for access to
// target/word-compare: choose that folder in its "Grant File Access" dialog,
// then run the script again.

import { execFileSync, spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { launchChrome, chromeEndpoint } from "./chrome.mjs";
import { LINES, score } from "./word-score.mjs";

const root = resolve(import.meta.dirname, "..");
const out = join(root, "target/word-compare");
mkdirSync(out, { recursive: true });
const fixtures = process.argv.slice(2).length
  ? process.argv.slice(2)
  : ["generated", "real-world"].flatMap((dir) =>
      readdirSync(join(root, "fixtures", dir))
        .filter((f) => f.endsWith(".docx"))
        .sort()
        .map((f) => `${dir}/${f}`),
    );
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const squash = (lines) => lines.map((l) => l.replace(/\s+/g, "")).filter(Boolean);

/** Word's lines on each page of a fixture, from its PDF export. */
function word(fixture) {
  const source = join(root, "fixtures", fixture);
  const docx = join(out, fixture.replace("/", "--"));
  const pdf = docx.replace(/\.docx$/, ".pdf");
  if (!existsSync(pdf) || statSync(pdf).mtimeMs < statSync(source).mtimeMs) {
    copyFileSync(source, docx);
    execFileSync("osascript", [
      "-e", "on run argv",
      "-e", 'tell application "Microsoft Word"',
      "-e", "open (POSIX file (item 1 of argv))",
      "-e", "set d to active document",
      "-e", "save as d file name (item 2 of argv) file format format PDF",
      "-e", "close d saving no",
      "-e", "end tell",
      "-e", "end run",
      docx,
      pdf,
    ]);
  }
  const count = Number(/Pages:\s+(\d+)/.exec(execFileSync("pdfinfo", [pdf], { encoding: "utf8" }))[1]);
  return Array.from({ length: count }, (_, i) =>
    squash(execFileSync("pdftotext", ["-f", `${i + 1}`, "-l", `${i + 1}`, pdf, "-"], { encoding: "utf8" }).split("\n")),
  );
}

/** A CDP session on a headless Chrome. */
async function browser() {
  const port = 9400 + Math.floor(Math.random() * 400);
  const profile = mkdtempSync(join(tmpdir(), "word-compare-"));
  const chrome = launchChrome(profile, port);
  chrome.once("close", () => rmSync(profile, { recursive: true, force: true }));
  try {
    await chromeEndpoint(chrome, port);
  } catch (error) {
    chrome.kill();
    throw error;
  }
  const target = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: "PUT" })).json();
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r) => (ws.onopen = r));
  let id = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    pending.get(msg.id)?.(msg);
    pending.delete(msg.id);
  };
  const send = (method, params = {}) =>
    new Promise((r) => {
      pending.set(++id, r);
      ws.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = async (expression) => {
    const { result } = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description);
    return result.result.value;
  };
  return { send, evaluate, close: () => chrome.kill() };
}

const LINES_OF_PAGES = `docxDemo.surface.pages.map((page) =>
  page.fragments.filter((f) => f.kind === "line").map((f) => f.line.pieces.map((p) => p.text).join("")))`;

const port = 8800 + Math.floor(Math.random() * 100);
const server = spawn("python3", ["-m", "http.server", `${port}`, "--bind", "127.0.0.1"], { cwd: root, stdio: "ignore" });
const chrome = await browser();
let passed = 0;
try {
  await sleep(500);
  for (const fixture of fixtures) {
    const theirs = word(fixture);
    await chrome.send("Page.navigate", { url: `http://127.0.0.1:${port}/web/index.html?fixture=${fixture}` });
    let ours = null;
    for (let i = 0; i < 100 && !ours; i++) {
      await sleep(100);
      ours = await chrome.evaluate(`window.docxDemo?.surface ? ${LINES_OF_PAGES} : null`).catch(() => null);
    }
    ours = (ours ?? []).map(squash);
    const { ok, same, total, inOrder, missing, extra } = score(theirs, ours);
    if (ok) passed++;
    console.log(`${ok ? "ok" : "differs"} ${fixture}: pages Word ${theirs.length}, ours ${ours.length}; lines the same ${same} of ${total}, ${inOrder} of them in Word's order`);
    for (const line of ok ? [] : missing.slice(0, 3)) console.log(`  Word's line not found: ${line}`);
    for (const line of ok ? [] : extra.slice(0, 3)) console.log(`  Our line Word does not have: ${line}`);
  }
} finally {
  chrome.close();
  server.kill();
}
console.log(`${passed} of ${fixtures.length} fixtures within the tolerance (same page count, ${LINES * 100}% of lines)`);
process.exit(passed === fixtures.length ? 0 : 1);
