// Edits a selection across two paragraphs in the browser demo, locally and in
// two shared tabs, and checks the view, the engine, the caret and undo.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs range

import { check, same } from "./harness.mjs";

export const fixtures = [
  "generated/simple-paragraphs.docx",
  "generated/rich-text.docx",
  "generated/headings-styles.docx",
  "generated/lists.docx",
];

export const helpers = `(() => {
  window.texts = () => { const out = []; pm().state.doc.forEach((n) => out.push(n.type.name === "paragraph" ? n.textContent : "[" + n.type.name + "]")); return out; };
  window.engineText = (id) => docxDemo.session.inspectNode(id).paragraph_text;
  window.clip = (type, text) => { const dt = new DataTransfer(); if (text !== undefined) dt.setData("text/plain", text); pm().dom.dispatchEvent(new ClipboardEvent(type, { clipboardData: dt, bubbles: true, cancelable: true })); return dt.getData("text/plain"); };
})()`;

export async function run(mode, a, b, fixture) {
  // Two neighbouring paragraphs of plain text, long enough to cut into.
  const pick = await a.ev(`(() => { const kids = []; pm().state.doc.forEach((n) => kids.push(n));
    const plain = (n) => n && n.type.name === "paragraph" && n.childCount > 0 && n.content.size >= 8 && [...Array(n.childCount).keys()].every((k) => n.child(k).isText);
    // No command moves a section break yet, so neither may end a section.
    const open = (n) => plain(n) && !docxDemo.session.inspectNode(n.attrs.id).section;
    const i = kids.findIndex((n, k) => open(n) && open(kids[k + 1]));
    return { i, a: kids[i].textContent, b: kids[i + 1].textContent, ids: [kids[i].attrs.id, kids[i + 1].attrs.id] }; })()`);
  const { i } = pick;
  const original = await a.ev(`texts()`);
  const A = pick.a, B = pick.b;
  console.log(`${mode}: paragraphs ${i} and ${i + 1}: ${JSON.stringify(A.slice(0, 30))} / ${JSON.stringify(B.slice(0, 30))}`);
  const s0 = await a.ev(`start(${i})`), s1 = await a.ev(`start(${i + 1})`);
  const range = `select(${s0 + 3}, ${s1 + 4})`;
  const settle = a.paint;
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  const expect = async (label, want, caret) => {
    await settle();
    const got = await a.ev(`texts()`);
    check(`${mode} ${label}: view`, JSON.stringify(got) === JSON.stringify(want), { got: got.slice(i, i + 4), want: want.slice(i, i + 4) });
    const engine = await a.ev(`(() => { const out = []; pm().state.doc.forEach((n, _o, k) => { if (k >= ${i} && k < ${i + 4} && n.type.name === "paragraph") out.push(engineText(n.attrs.id)); }); return out; })()`);
    const shown = want.slice(i, i + 4).filter((t) => !t.startsWith("["));
    check(`${mode} ${label}: engine`, JSON.stringify(engine.map((t) => t.replace(/[\t\n]/g, ""))) === JSON.stringify(shown), { engine, shown });
    if (caret !== undefined) { const at = await a.ev(`pm().state.selection.head`); check(`${mode} ${label}: caret`, at === caret, { at, caret }); }
    const n = await notice(); check(`${mode} ${label}: no refusal`, n === "", n);
    if (b) { await b.front(a); const other = await b.ev(`texts()`); check(`${mode} ${label}: other tab`, JSON.stringify(other) === JSON.stringify(want), other.slice(i, i + 4)); await a.front(b); }
  };
  const undo = async (label) => { await a.ev(`key("z", { metaKey: true })`); await expect(`${label} undone`, original); };
  const at = (k, list) => [...original.slice(0, i), ...list, ...original.slice(i + k)];

  await a.ev(`${range}; key("Backspace")`);
  await expect("backspace", at(2, [A.slice(0, 3) + B.slice(4)]), s0 + 3);
  await undo("backspace");

  await a.ev(`${range}; key("Delete")`);
  await expect("delete", at(2, [A.slice(0, 3) + B.slice(4)]), s0 + 3);
  await undo("delete");

  await a.ev(range);
  for (const type of ["keyDown", "keyUp"]) await a.send("Input.dispatchKeyEvent", { type, key: "x", code: "KeyX", text: type === "keyDown" ? "x" : undefined, windowsVirtualKeyCode: 88 });
  await expect("type over", at(2, [A.slice(0, 3) + "x" + B.slice(4)]), s0 + 4);
  await undo("type over");

  await a.ev(`${range}; key("Enter")`);
  await expect("enter", at(2, [A.slice(0, 3), B.slice(4)]), s0 + 5);
  await undo("enter");

  await a.ev(`${range}; key("Enter", { shiftKey: true })`);
  await expect("shift+enter", at(2, [A.slice(0, 3) + B.slice(4)]), s0 + 4);
  const br = await a.ev(`pm().state.doc.child(${i}).childCount`);
  check(`${mode} shift+enter: break between`, br >= 3, br);
  await undo("shift+enter");

  await a.ev(`${range}; clip("paste", "one\\r\\ntwo\\tsix\\u0001\\nthree")`);
  await expect("paste lines over", at(2, [A.slice(0, 3) + "one", "two six", "three" + B.slice(4)]), (await a.ev(`start(${i + 2})`)) + 5);
  await undo("paste lines over");

  await a.ev(`select(${s0 + 3}, ${s0 + 3}); clip("paste", "p\\nq")`);
  await expect("paste lines at caret", at(1, [A.slice(0, 3) + "p", "q" + A.slice(3)]), s0 + 7);
  await undo("paste lines at caret");

  // A clipboard with only a file the document cannot hold changes nothing.
  await a.ev(`select(${s0 + 3}, ${s0 + 6}); (() => { const dt = new DataTransfer(); dt.items.add(new File(["%PDF-1.7"], "pasted.pdf", { type: "application/pdf" })); pm().dom.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true })); })()`);
  await settle();
  check(`${mode} unsupported paste: the selection stays`, same(await a.ev(`texts()`), original) && same(await a.ev(`[pm().state.selection.from, pm().state.selection.to]`), [s0 + 3, s0 + 6]), await a.ev(`texts()`));
  check(`${mode} unsupported paste: says why`, (await notice()).startsWith("Nothing on the clipboard can be pasted here"), await notice());
  await a.ev(`document.getElementById("notice").textContent = ""`);

  const copied = await a.ev(`${range}; clip("copy")`);
  check(`${mode} copy: lines`, copied === A.slice(3) + "\n" + B.slice(0, 4), copied);
  const cut = await a.ev(`${range}; clip("cut")`);
  check(`${mode} cut: lines`, cut === copied, cut);
  await expect("cut", at(2, [A.slice(0, 3) + B.slice(4)]), s0 + 3);
  await a.ev(`clip("paste", ${JSON.stringify(cut)})`);
  await expect("paste the cut back", original, s1 + 4);
  await a.ev(`key("z", { metaKey: true })`);
  await undo("cut");

  if (fixture === "generated/simple-paragraphs.docx") {
    await a.ev(`key("a", { metaKey: true }); clip("paste", "Replaced all")`);
    await expect("select all and paste", ["Replaced all", "[block_atom]"], 13);
    await undo("select all and paste");
    await a.ev(`key("a", { metaKey: true }); key("Delete")`);
    await expect("select all and delete", ["", "[block_atom]"], 1);
    await undo("select all and delete");
  }

  const errors = [...a.errors, ...(b?.errors ?? [])];
  check(`${mode}: no exceptions`, errors.length === 0, errors);
}
