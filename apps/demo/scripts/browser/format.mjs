// Formats a selection across two paragraphs in the browser demo, locally and
// in two shared tabs, and checks the view, the engine, the painted pages, the
// selection, the toolbar and undo.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs format

import { check } from "./harness.mjs";

export const fixtures = ["generated/simple-paragraphs.docx", "generated/headings-styles.docx"];

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
  window.pressed = () => ["bold", "italic", "underline"].map((p) => document.getElementById(p).getAttribute("aria-pressed") === "true" ? p[0] : "").join("");
})()`;

export async function run(mode, a, b, fixture) {
  if (fixture === fixtures[0]) await selection(mode, a, b);
  else await heading(mode, a, b);
}

/** Formatting a selection across two paragraphs. */
async function selection(mode, a, b) {
  // Two neighbouring paragraphs of unformatted text, long enough to cut into.
  const pick = await a.ev(`(() => { const kids = []; pm().state.doc.forEach((n) => kids.push(n));
    const plain = (n, k) => n && n.type.name === "paragraph" && n.content.size >= 8 && [...Array(n.childCount).keys()].every((c) => n.child(c).isText) && !/:[biu]/.test(fmt(k));
    const i = kids.findIndex((n, k) => plain(n, k) && plain(kids[k + 1], k + 1));
    return { i, a: kids[i]?.textContent, b: kids[i + 1]?.textContent }; })()`);
  const { i } = pick;
  // A background tab gets no animation frames, so it lays nothing out.
  await a.front(b);
  if (i < 0) throw new Error("The selection fixture needs two plain paragraphs");
  const A = pick.a, B = pick.b;
  console.log(`${mode}: paragraphs ${i} and ${i + 1}: ${JSON.stringify(A.slice(0, 30))} / ${JSON.stringify(B.slice(0, 30))}`);
  const s0 = await a.ev(`start(${i})`), s1 = await a.ev(`start(${i + 1})`);
  const sel = [s0 + 3, s1 + 4];
  const range = `select(${sel[0]}, ${sel[1]})`;
  const settle = a.paint;
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  const want = (flag, textA = A, textB = B, fromA = 3, toB = 4) => [
    [...textA].map((ch, k) => ch + ":" + (k >= fromA ? flag : "")).join("\u0001"),
    [...textB].map((ch, k) => ch + ":" + (k < toB ? flag : "")).join("\u0001"),
  ];
  const original = want("");
  const expect = async (label, [wa, wb], { selection, pressed, painted } = {}) => {
    await settle();
    const view = [await a.ev(`fmt(${i})`), await a.ev(`fmt(${i + 1})`)];
    check(`${mode} ${label}: view`, view[0] === wa && view[1] === wb, { view, want: [wa, wb] });
    const engine = [await a.ev(`engineFmt(${i})`), await a.ev(`engineFmt(${i + 1})`)];
    check(`${mode} ${label}: engine`, engine[0] === wa && engine[1] === wb, { engine });
    if (selection) { const s = await a.ev(`[pm().state.selection.from, pm().state.selection.to]`); check(`${mode} ${label}: selection kept`, s[0] === selection[0] && s[1] === selection[1], { s, selection }); }
    if (pressed !== undefined) { const p = await a.ev(`pressed()`); check(`${mode} ${label}: toolbar`, p === pressed, { p, pressed }); }
    if (painted) { const spans = await a.ev(`painted()`); const ok = spans.some((s) => s.startsWith(painted[0]) && s.endsWith(":" + painted[1])); check(`${mode} ${label}: painted`, ok, { want: painted, spans: spans.slice(0, 14), lines: await a.ev(`document.querySelectorAll(".pages .line").length`) }); }
    const n = await notice(); check(`${mode} ${label}: no refusal`, n === "", n);
    if (b) {
      // A hidden page applies others' updates when it is shown.
      await b.front(a);
      const other = [await b.ev(`fmt(${i})`), await b.ev(`fmt(${i + 1})`)];
      check(`${mode} ${label}: other tab`, other[0] === wa && other[1] === wb, other);
      if (painted) { const spans = await b.ev(`painted()`); check(`${mode} ${label}: other tab painted`, spans.some((s) => s.startsWith(painted[0]) && s.endsWith(":" + painted[1])), spans.slice(0, 14)); }
      await a.front(b);
    }
  };
  const undo = async (label, state = original) => { await a.ev(`key("z", { metaKey: true })`); await expect(`${label} undone`, state); };
  const mod = (k) => a.ev(`key("${k}", { metaKey: true })`);

  await a.ev(range); await mod("b");
  await expect("bold", want("b"), { selection: sel, pressed: "b", painted: [A.slice(3, 5), "b"] });
  await mod("b");
  await expect("bold again turns it off", original, { selection: sel, pressed: "" });

  await a.ev(range); await mod("i");
  await expect("italic", want("i"), { selection: sel, pressed: "i", painted: [A.slice(3, 5), "i"] });
  await undo("italic");

  // Lines are painted a word at a time, so the check looks at the rest of the first word.
  // The button, clicked with the mouse, keeps the selection in the editor.
  await a.ev(range);
  const box = await a.ev(`(() => { const r = document.getElementById("underline").getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()`);
  for (const type of ["mousePressed", "mouseReleased"]) await a.send("Input.dispatchMouseEvent", { type, x: box[0], y: box[1], button: "left", clickCount: 1 });
  await expect("underline button", want("u"), { selection: sel, pressed: "u", painted: [A.slice(3, 5), "u"] });
  await undo("underline button");

  // Text typed inside bold text is bold.
  await a.ev(range); await mod("b"); await settle();
  await a.ev(`select(${s0 + 5}, ${s0 + 5})`);
  await settle();
  check(`${mode} caret in bold: toolbar`, (await a.ev(`pressed()`)) === "b", await a.ev(`pressed()`));
  for (const type of ["keyDown", "keyUp"]) await a.send("Input.dispatchKeyEvent", { type, key: "x", code: "KeyX", text: type === "keyDown" ? "x" : undefined, windowsVirtualKeyCode: 88 });
  const typed = A.slice(0, 5) + "x" + A.slice(5);
  await expect("type in bold", want("b", typed));
  await a.ev(`key("z", { metaKey: true })`); await settle();
  await undo("bold then type");

  // Nothing selected: refused with a reason, and nothing changes.
  await a.ev(`select(${s0 + 5}, ${s0 + 5})`); await mod("b"); await settle();
  check(`${mode} empty selection: refused`, (await notice()).startsWith("Select the text"), await notice());
  check(`${mode} empty selection: unchanged`, (await a.ev(`fmt(${i})`)) === original[0], await a.ev(`fmt(${i})`));
  await a.ev(`document.getElementById("notice").textContent = ""`);

  const errors = [...a.errors, ...(b?.errors ?? [])];
  check(`${mode}: no exceptions`, errors.length === 0, errors);
}

/** Unbolding part of a heading whose style makes it bold. */
async function heading(mode, a, b) {
  await a.front(b);
  const i = await a.ev(`(() => { let found = -1; pm().state.doc.forEach((n, _o, k) => { if (found < 0 && n.type.name === "paragraph" && n.content.size >= 8 && fmt(k).split("\\u0001").every((t) => t.slice(t.indexOf(":", 1) + 1).includes("b"))) found = k; }); return found; })()`);
  if (i < 0) throw new Error("The heading fixture needs a bold heading");
  const before = await a.ev(`fmt(${i})`);
  const s = await a.ev(`start(${i})`);
  await a.ev(`select(${s + 2}, ${s + 5})`);
  await a.paint();
  check(`${mode} heading: toolbar`, (await a.ev(`pressed()`)).includes("b"), await a.ev(`pressed()`));
  await a.ev(`key("b", { metaKey: true })`);
  await a.paint();
  const after = (await a.ev(`fmt(${i})`)).split("\u0001");
  const ok = after.every((c, k) => (k >= 2 && k < 5) === !c.slice(c.indexOf(":", 1) + 1).includes("b"));
  check(`${mode} heading: unbolded only the selection`, ok, after.slice(0, 7));
  check(`${mode} heading: engine`, (await a.ev(`engineFmt(${i})`)) === after.join("\u0001"), await a.ev(`engineFmt(${i})`));
  if (b) {
    await b.front(a);
    check(`${mode} heading: other tab`, (await b.ev(`fmt(${i})`)) === after.join("\u0001"), await b.ev(`fmt(${i})`));
    await a.front(b);
  }
  await a.ev(`key("z", { metaKey: true })`);
  await a.paint();
  check(`${mode} heading: undone`, (await a.ev(`fmt(${i})`)) === before, await a.ev(`fmt(${i})`));
}
