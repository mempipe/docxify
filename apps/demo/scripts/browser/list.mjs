// Makes lists in the browser demo, locally and in two shared tabs, with the
// keys and the buttons, and checks the view, the engine, the painted
// markers, the buttons, the other tab, the export and undo.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs list

import { check } from "./harness.mjs";

export const fixture = "generated/simple-paragraphs.docx";

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
  // A paragraph's list as the view and the engine have it: "kind level", or "" for none.
  const shown = (list) => list?.definition ? (list.definition.format === "bullet" ? "bullet" : "number") + " " + list.level : "";
  window.listOf = (i) => shown(pm().state.doc.child(i).attrs.list);
  window.engineList = (i) => shown(docxDemo.session.inspectNode(pm().state.doc.child(i).attrs.id).list);
  window.markers = () => { const out = []; pm().state.doc.forEach((n) => out.push(n.attrs.marker?.text ?? n.attrs.marker ?? "")); return out; };
  window.lists = () => ["bullet", "number"].map((k) => document.getElementById(k).getAttribute("aria-pressed") === "true" ? k[0] : "").join("");
  window.exported = () => { return docxDemo.engine.validate(docxDemo.session.replica.export()).valid; };
  window.styleOf = (i) => pm().state.doc.child(i).attrs.info.style ?? "";
  window.engineStyle = (i) => docxDemo.session.inspectNode(pm().state.doc.child(i).attrs.id).style ?? "";
  window.picked = () => document.getElementById("style").value;
  window.pressed = () => ["bold", "italic", "underline"].map((p) => document.getElementById(p).getAttribute("aria-pressed") === "true" ? p[0] : "").join("");
})()`;

export async function run(mode, a, b) {
  await a.front(b);
  const settle = a.paint;
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  const startOf = (i) => a.ev(`start(${i})`);
  const endOf = async (i) => (await startOf(i)) + (await a.ev(`pm().state.doc.child(${i}).content.size`));
  // ⌘⇧8 and ⌘⇧7: Shift turns the digit into a symbol; the key code still names the digit.
  const shortcut = async (digit) => {
    for (const type of ["keyDown", "keyUp"]) {
      await a.send("Input.dispatchKeyEvent", { type, key: { 8: "*", 7: "&" }[digit], code: `Digit${digit}`, windowsVirtualKeyCode: 48 + digit, modifiers: 4 | 8 });
    }
  };
  const press = async (key, shift = false) => {
    const codes = { Tab: 9, Enter: 13, Backspace: 8 };
    for (const type of ["keyDown", "keyUp"]) {
      await a.send("Input.dispatchKeyEvent", { type, key, code: key, windowsVirtualKeyCode: codes[key], modifiers: shift ? 8 : 0, ...(type === "keyDown" && key === "Enter" ? { text: "\r" } : {}) });
    }
  };
  const expect = async (label, want, { pressed, texts } = {}) => {
    await settle();
    const view = await Promise.all(Object.keys(want).map((i) => a.ev(`listOf(${i})`)));
    const engine = await Promise.all(Object.keys(want).map((i) => a.ev(`engineList(${i})`)));
    const expected = Object.values(want);
    check(`${mode} ${label}: view`, JSON.stringify(view) === JSON.stringify(expected), { view, expected });
    check(`${mode} ${label}: engine`, JSON.stringify(engine) === JSON.stringify(expected), { engine, expected });
    if (pressed !== undefined) check(`${mode} ${label}: buttons`, (await a.ev(`lists()`)) === pressed, await a.ev(`lists()`));
    if (texts) { const got = await a.ev(`texts()`); check(`${mode} ${label}: text`, JSON.stringify(got) === JSON.stringify(texts), got); }
    const n = await notice(); check(`${mode} ${label}: no refusal`, n === "", n);
    if (b) {
      await b.front(a);
      const other = await Promise.all(Object.keys(want).map((i) => b.ev(`listOf(${i})`)));
      check(`${mode} ${label}: other tab`, JSON.stringify(other) === JSON.stringify(expected), other);
      await a.front(b);
    }
  };
  const undo = async (label, want) => { await a.ev(`key("z", { metaKey: true })`); await expect(`${label} undone`, want); };
  const original = await a.ev(`texts()`);
  // The new paragraph comes before the block holding the section's properties.
  const added = [...original.slice(0, 3), "", ...original.slice(3)];

  // ⌘⇧8 at the caret, then again to take the bullet off.
  const s1 = await startOf(1);
  await a.ev(`select(${s1 + 3}, ${s1 + 3})`); await settle();
  check(`${mode} plain paragraph: buttons`, (await a.ev(`lists()`)) === "", await a.ev(`lists()`));
  await shortcut(8);
  await expect("⌘⇧8 at the caret", { 0: "", 1: "bullet 0", 2: "" }, { pressed: "b" });
  check(`${mode} ⌘⇧8: marker`, (await a.ev(`markers()`))[1] === "•" || (await a.ev(`markers()`))[1] !== "", await a.ev(`markers()`));
  check(`${mode} ⌘⇧8: painted marker`, (await a.ev(`painted()`)).some((s) => s.startsWith("•") || s.startsWith("")), (await a.ev(`painted()`)).slice(0, 12));
  if (mode === "local") check(`${mode} ⌘⇧8: export validates`, (await a.ev(`exported()`)) === true, await a.ev(`exported()`));
  await shortcut(8);
  await expect("⌘⇧8 again", { 0: "", 1: "", 2: "" }, { pressed: "" });
  await undo("⌘⇧8 again", { 0: "", 1: "bullet 0", 2: "" });
  await undo("⌘⇧8", { 0: "", 1: "", 2: "" });

  // The numbering button over all three paragraphs keeps the selection and the focus.
  const e2 = await endOf(2);
  await a.ev(`select(1, ${e2})`);
  await a.ev(`(() => { const b = document.getElementById("number"); b.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })); b.click(); })()`);
  await expect("numbering button over three", { 0: "number 0", 1: "number 0", 2: "number 0" }, { pressed: "n" });
  check(`${mode} numbering button: focus kept`, await a.ev(`pm().hasFocus()`), await a.ev(`document.activeElement?.tagName`));
  const sel = await a.ev(`[pm().state.selection.from, pm().state.selection.to]`);
  check(`${mode} numbering button: selection kept`, sel[0] === 1 && sel[1] === e2, sel);
  const numbers = await a.ev(`markers()`);
  check(`${mode} numbering: markers count`, JSON.stringify(numbers.slice(0, 3)) === JSON.stringify(["1.", "2.", "3."]), numbers);

  // Tab at an item's start goes a level in, and Shift+Tab back out.
  const n1 = await startOf(1);
  await a.ev(`select(${n1}, ${n1})`);
  await press("Tab");
  await expect("Tab at an item's start", { 0: "number 0", 1: "number 1", 2: "number 0" }, { texts: original });
  const nested = await a.ev(`markers()`);
  check(`${mode} Tab: markers count again`, nested[0] === "1." && nested[2] === "2." && nested[1] !== "", nested);
  check(`${mode} Tab: caret kept`, (await a.ev(`pm().state.selection.from`)) === n1, await a.ev(`pm().state.selection.from`));
  await press("Tab", true);
  await expect("Shift+Tab", { 0: "number 0", 1: "number 0", 2: "number 0" });
  // Tab inside an item's text is not a level change.
  await a.ev(`select(${n1 + 3}, ${n1 + 3})`);
  await press("Tab");
  await expect("Tab inside the text", { 1: "number 0" }, { texts: original });

  // Enter at the end of the last item adds an item; Enter again on the empty one ends the list.
  await a.ev(`select(${await endOf(2)}, ${await endOf(2)})`);
  await press("Enter");
  await expect("Enter at the end of an item", { 2: "number 0", 3: "number 0" }, { texts: added });
  check(`${mode} Enter: fourth marker`, (await a.ev(`markers()`))[3] === "4.", await a.ev(`markers()`));
  await press("Enter");
  await expect("Enter on the empty item", { 2: "number 0", 3: "" }, { texts: added });

  // Backspace at an item's start takes it out of the list and joins nothing.
  const b1 = await startOf(1);
  await a.ev(`select(${b1}, ${b1})`);
  await press("Backspace");
  await expect("Backspace at an item's start", { 0: "number 0", 1: "", 2: "number 0" }, { texts: added });
  check(`${mode} Backspace: the rest renumber`, (await a.ev(`markers()`))[2] === "2.", await a.ev(`markers()`));
  if (mode === "local") check(`${mode} lists: export validates`, (await a.ev(`exported()`)) === true, await a.ev(`exported()`));

  for (const label of ["Backspace", "Enter on the empty item", "Enter", "Shift+Tab", "Tab"]) {
    await a.ev(`key("z", { metaKey: true })`); await settle();
  }
  await undo("numbering button", { 0: "", 1: "", 2: "" });
  const back = await a.ev(`texts()`);
  check(`${mode} all undone: text`, JSON.stringify(back) === JSON.stringify(original), back);

  const errors = [...a.errors, ...(b?.errors ?? [])];
  check(`${mode}: no exceptions`, errors.length === 0, errors);
}
