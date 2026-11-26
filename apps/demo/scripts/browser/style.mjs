// Gives paragraphs a style in the browser demo, locally and in two shared
// tabs, with the keys and the picker, and checks the view, the engine, the
// painted pages, the picker, the other tab and undo.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs style

import { fileURLToPath } from "node:url";
import { check } from "./harness.mjs";

export const fixture = "generated/headings-styles.docx";

export const helpers = `(() => {
  window.texts = () => { const out = []; pm().state.doc.forEach((n) => out.push(n.type.name === "paragraph" ? n.textContent : "[" + n.type.name + "]")); return out; };
  const flags = (e) => (e.bold ? "b" : "") + (e.italic ? "i" : "") + (e.underline && e.underline !== "none" ? "u" : "");
  // Each character of paragraph i with the formatting its run shows, in the view.
  window.fmt = (i) => { const p = pm().state.doc.child(i); const out = [];
    p.forEach((c) => { if (!c.isText) return; const run = c.marks.find((m) => m.type.name === "run"); for (const ch of c.text) out.push(ch + ":" + flags(run?.attrs.info.effective ?? {})); });
    return out.join("\\u0001"); };
  // The same from the engine's inspection.
  window.engineFmt = (i) => { const id = pm().state.doc.child(i).attrs.id; const out = [];
    const walk = (n, run) => { if (n.type === "run") run = n; if (n.type === "text" && n.support === "editable") for (const ch of n.text) out.push(ch + ":" + flags(run?.effective ?? {})); n.children?.forEach((c) => walk(c, run)); };
    walk(docxDemo.session.inspectNode(id)); return out.join("\\u0001"); };
  // The painted spans as text with their weight, style and decoration.
  window.painted = () => [...document.querySelectorAll(".pages .line span")].map((s) => s.textContent + ":" + (Number(s.style.fontWeight || (s.style.font.includes("bold") ? 700 : 400)) >= 700 ? "b" : "") + (s.style.fontStyle === "italic" || s.style.font.includes("italic") ? "i" : "") + (s.style.textDecoration.includes("underline") ? "u" : ""));
  window.styleOf = (i) => pm().state.doc.child(i).attrs.info.style ?? "";
  window.engineStyle = (i) => docxDemo.session.inspectNode(pm().state.doc.child(i).attrs.id).style ?? "";
  window.picked = () => document.getElementById("style").value;
  window.pressed = () => ["bold", "italic", "underline"].map((p) => document.getElementById(p).getAttribute("aria-pressed") === "true" ? p[0] : "").join("");
})()`;

export async function run(mode, a, b) {
  await a.front(b);
  const settle = a.paint;
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  const texts = await a.ev(`texts()`);
  const body = texts.findIndex((t) => t.startsWith("Body text"));
  const section = texts.findIndex((t) => t.startsWith("Section 1.1"));
  const title = texts.findIndex((t) => t.startsWith("Document title"));
  console.log(`${mode}: body ${body}, section ${section}, title ${title}`);
  const startOf = (i) => a.ev(`start(${i})`);
  // Option with a digit types a symbol on a Mac; the key code still names the digit.
  const level = async (n) => {
    for (const type of ["keyDown", "keyUp"]) {
      await a.send("Input.dispatchKeyEvent", { type, key: "¡™£¢"[n] ?? String(n), code: `Digit${n}`, windowsVirtualKeyCode: 48 + n, modifiers: 1 | 4 });
    }
  };
  const expect = async (label, want, { picked, painted } = {}) => {
    await settle();
    const view = await Promise.all(Object.keys(want).map((i) => a.ev(`styleOf(${i})`)));
    const engine = await Promise.all(Object.keys(want).map((i) => a.ev(`engineStyle(${i})`)));
    const expected = Object.values(want);
    check(`${mode} ${label}: view`, JSON.stringify(view) === JSON.stringify(expected), { view, expected });
    check(`${mode} ${label}: engine`, JSON.stringify(engine) === JSON.stringify(expected), { engine, expected });
    if (picked !== undefined) check(`${mode} ${label}: picker`, (await a.ev(`picked()`)) === picked, await a.ev(`picked()`));
    if (painted) {
      const spans = await a.ev(`painted()`);
      check(`${mode} ${label}: painted`, spans.some((s) => s === painted), { want: painted, spans: spans.slice(0, 16) });
    }
    const n = await notice(); check(`${mode} ${label}: no refusal`, n === "", n);
    if (b) {
      await b.front(a);
      const other = await Promise.all(Object.keys(want).map((i) => b.ev(`styleOf(${i})`)));
      check(`${mode} ${label}: other tab`, JSON.stringify(other) === JSON.stringify(expected), other);
      if (painted) { const spans = await b.ev(`painted()`); check(`${mode} ${label}: other tab painted`, spans.some((s) => s === painted), spans.slice(0, 16)); }
      await a.front(b);
    }
  };
  const undo = async (label, want) => { await a.ev(`key("z", { metaKey: true })`); await expect(`${label} undone`, want); };

  const b0 = await startOf(body);
  await a.ev(`select(${b0 + 3}, ${b0 + 3})`); await settle();
  check(`${mode} caret in body text: picker`, (await a.ev(`picked()`)) === "Normal", await a.ev(`picked()`));
  check(`${mode} body text painted plain`, (await a.ev(`painted()`)).includes("Body:"), (await a.ev(`painted()`)).slice(0, 16));

  // The caret's paragraph, with nothing selected.
  await level(1);
  await expect("⌘⌥1 at the caret", { [body]: "Heading1", [section]: "Heading2" }, { picked: "Heading1", painted: "Body:b" });
  await undo("⌘⌥1", { [body]: "", [section]: "Heading2" });

  // The picker, over a selection across two paragraphs.
  const s0 = await startOf(section);
  await a.ev(`select(${b0 + 3}, ${s0 + 4})`);
  await a.ev(`(() => { const s = document.getElementById("style"); s.focus(); s.value = "Title"; s.dispatchEvent(new Event("change")); })()`);
  await expect("picker over two paragraphs", { [body]: "Title", [section]: "Title" }, { picked: "Title" });
  check(`${mode} picker: focus back in the editor`, await a.ev(`pm().hasFocus()`), await a.ev(`document.activeElement?.tagName`));
  const sel = await a.ev(`[pm().state.selection.from, pm().state.selection.to]`);
  check(`${mode} picker: selection kept`, sel[0] === b0 + 3 && sel[1] === s0 + 4, sel);

  // ⌘⌥0 gives the default style, which is no style.
  await level(0);
  await expect("⌘⌥0 over two paragraphs", { [body]: "", [section]: "" }, { picked: "Normal" });
  await undo("⌘⌥0", { [body]: "Title", [section]: "Title" });
  await undo("picker", { [body]: "", [section]: "Heading2" });

  // Paragraphs in different styles leave the picker blank.
  const t0 = await startOf(title);
  await a.ev(`select(${t0 + 2}, ${b0 + 2})`); await settle();
  check(`${mode} mixed selection: picker blank`, (await a.ev(`picked()`)) === "", await a.ev(`picked()`));

  // Enter inside a styled paragraph gives both halves the style.
  await a.ev(`select(${b0 + 4}, ${b0 + 4})`);
  await level(2);
  await expect("⌘⌥2", { [body]: "Heading2" });
  await a.ev(`key("Enter")`);
  await expect("Enter inside it", { [body]: "Heading2", [body + 1]: "Heading2" });
  const split = await a.ev(`texts()`);
  check(`${mode} Enter inside it: text`, split[body] === "Body" && split[body + 1].startsWith(" text under"), split.slice(body, body + 2));
  await a.ev(`key("z", { metaKey: true })`); await settle();
  await undo("⌘⌥2", { [body]: "" });

  if (mode === "local") await checkStyleFont(a);
  const errors = [...a.errors, ...(b?.errors ?? [])];
  check(`${mode}: no exceptions`, errors.length === 0, errors);
}

// MacroText in the python-docx fixture uses Courier, a family absent from its initial text.
async function checkStyleFont(tab) {
  const open = async (bytes) => {
    await tab.ev(`(async () => {
      const files = new DataTransfer();
      files.items.add(new File([${bytes}], "styled.docx"));
      const picker = document.querySelector("#file");
      picker.files = files.files;
      picker.dispatchEvent(new Event("change"));
    })()`);
    await tab.wait("styled file ready", '!document.querySelector("#file").disabled');
  };
  const file = fileURLToPath(new URL("../../fixtures/real-world/python-docx.docx", import.meta.url));
  const { root } = (await tab.send("DOM.getDocument")).result;
  const { nodeId } = (await tab.send("DOM.querySelector", { nodeId: root.nodeId, selector: "#file" })).result;
  const chooseFile = async () => {
    await tab.send("DOM.setFileInputFiles", { nodeId, files: [file] });
    await tab.wait("file picker ready", '!document.querySelector("#file").disabled');
  };
  await tab.send("Network.enable");
  await tab.send("Network.setBlockedURLs", { urls: ["*cousine-greek-400-normal.woff*"] });
  await tab.ev(`{
    const p = Object.getPrototypeOf(docxDemo.surface.paginator);
    const layout = p.layout;
    window.pageLayouts = 0;
    p.layout = function (...args) { pageLayouts++; return layout.apply(this, args); };
    window.restoreLayout = () => p.layout = layout;
  } void 0`);
  // Download while the replacement is loading. The visible file still owns the toolbar.
  await tab.ev(`{
    const before = docxDemo.session.replica.export();
    window.downloadedVisibleFile = null;
    document.fonts.addEventListener("loading", () => {
      const click = HTMLAnchorElement.prototype.click;
      HTMLAnchorElement.prototype.click = function () {
        const filename = this.download;
        fetch(this.href).then(r => r.bytes()).then(bytes => {
          window.downloadedVisibleFile = filename === "headings-styles.edited.docx"
            && bytes.length === before.length && bytes.every((b, i) => b === before[i]);
        });
      };
      try { document.querySelector("#download").click(); } finally { HTMLAnchorElement.prototype.click = click; }
    }, { once: true });
  } void 0`);
  await chooseFile();
  await tab.ev("restoreLayout(); void 0");
  check("opening paginates once after font loading settles", (await tab.ev("pageLayouts")) === 1);
  await tab.wait("download while opening", "downloadedVisibleFile !== null");
  check("Download keeps the visible file during the switch", await tab.ev("downloadedVisibleFile"));
  check("a missing font subset opens with fallback and a notice", await tab.ev(
    'document.querySelector("#view").children.length === 1 && Array.from(document.querySelector("#style").options).some(o => o.value === "MacroText") && document.querySelector("#notice").textContent.includes("font")'));
  await tab.send("Network.setBlockedURLs", { urls: [] });
  await tab.ev("window.fallbackSession = docxDemo.session; void 0");
  await chooseFile();
  check("choosing the same file again opens a fresh session after fonts recover", await tab.ev(
    'docxDemo.session !== fallbackSession && document.querySelector("#view").children.length === 1 && docxDemo.surface.root.parentElement === document.querySelector("#view")'));
  await tab.ev("window.opened = docxDemo.session; void 0");
  await open('"invalid DOCX"');
  check("a refused open keeps the current editor and reports the error", await tab.ev(
    'window.opened === docxDemo.session && docxDemo.surface.root.isConnected && document.querySelector("#notice").textContent.includes("open")'));
  const sheets = () => tab.ev(`document.querySelectorAll('link[href*="/cousine/"]').length`);
  const count = await sheets();
  check("the selectable Courier style has its faces before first use",
    count > 0 && await tab.ev(`document.fonts.check('12pt "Cousine"', "iЖΩ")`));
  await tab.ev('select(start(0), start(0))');
  await tab.send("Input.insertText", { text: "iiiiЖΩ ".repeat(80) });
  await tab.ev('const picker = document.querySelector("#style"); picker.value = "MacroText"; picker.dispatchEvent(new Event("change"))');
  await tab.paint();
  const lines = () => tab.ev('Array.from(document.querySelectorAll(".pages .line"), n => n.textContent).filter(text => text.includes("Ж"))');
  const edited = await lines();
  await open("docxDemo.session.replica.export()");
  await tab.wait("reopened styled paragraph painted", 'Array.from(document.querySelectorAll(".pages .line")).some(n => n.textContent.includes("Ж"))');
  const reopened = await lines();
  check("the styled lines wrap the same after reopen", edited.length > 1 && JSON.stringify(edited) === JSON.stringify(reopened), { edited, reopened });
  check("reopening reuses the page’s font stylesheets", count > 0 && await sheets() === count);
}
