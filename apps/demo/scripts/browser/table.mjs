// Adds and removes table rows and columns with the table buttons in the
// browser demo, locally and in two shared tabs, and checks the view, the
// engine, the painted page, the other tab, the export and undo.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs table

import { check, same, waitForSharedCaret } from "./harness.mjs";

export const fixture = "generated/table.docx";

export const helpers = `(() => {
  window.texts = () => { const out = []; pm().state.doc.forEach((n) => out.push(n.type.name === "paragraph" ? n.textContent : "[" + n.type.name + "]")); return out; };
  // Each link of the view, as its text and where it points, in document order.
  window.links = () => { const out = []; pm().state.doc.descendants((n) => { if (!n.isText) return true;
    const m = n.marks.find((m) => m.type.name === "link"); if (!m) { out.last = null; return; }
    if (out.last === m.attrs.id) out[out.length - 1][0] += n.text; else out.push([n.text, m.attrs.target]); out.last = m.attrs.id; }); return out.map((x) => x); };
  // The same from the engine's inspection.
  window.engineLinks = () => { const out = []; const text = (n) => (n.type === "text" ? n.text : "") + (n.children ?? []).map(text).join("");
    const walk = (n) => { if (n.type === "hyperlink") out.push([text(n), n.anchor !== undefined ? (n.target ?? "") + "#" + n.anchor : n.target]); n.children?.forEach(walk); };
    pm().state.doc.forEach((p) => p.attrs.id && walk(docxDemo.session.inspectNode(p.attrs.id))); return out; };
  // The painted spans holding a word, with their colour and whether they are underlined.
  window.look = (word) => [...document.querySelectorAll(".pages .line span")].filter((s) => s.children.length === 0 && s.textContent.includes(word))
    .map((s) => getComputedStyle(s).color + (getComputedStyle(s).textDecorationLine.includes("underline") ? " u" : ""));
  window.bar = () => { const $ = (id) => document.getElementById(id);
    return { shown: !$("linkbar").hidden, form: !$("linkform").hidden, view: !$("linkview").hidden, text: $("linkopen").textContent,
      href: $("linkopen").getAttribute("href"), value: $("linkurl").value, focus: document.activeElement?.id || document.activeElement?.tagName }; };
  window.below = () => { const c = docxDemo.surface.caretRect(); const b = document.getElementById("linkbar").getBoundingClientRect();
    return !!c && b.top >= c.bottom && b.left >= 0 && b.right <= innerWidth; };
  window.pressedLink = () => document.getElementById("link").getAttribute("aria-pressed");
  window.firstTable = () => { let t = null; pm().state.doc.descendants((n) => { if (!t && n.type.name === "table") { t = n; return false; } return !t; }); return t; };
  // Each row of the first table as its drawn cells' text; a cell a vertical merge continues is not drawn.
  window.grid = () => { const t = firstTable(); if (!t) return null; const rows = []; t.forEach((r) => { const cells = []; r.forEach((c) => cells.push(c.textContent)); rows.push(cells); }); return rows; };
  window.engineGrid = () => { const t = firstTable(); if (!t) return null; const node = docxDemo.session.inspectNode(t.attrs.id);
    const text = (n) => (n.type === "text" ? n.text : "") + (n.children ?? []).map(text).join("");
    return node.children.filter((r) => r.type === "row").map((r) => r.children.filter((c) => c.type === "cell" && c.vertical_merge !== "continue").map(text)); };
  // Inside the first paragraph of drawn cell c of row r of the first table.
  window.cellPos = (r, c) => { let pos = null; pm().state.doc.descendants((n, p) => { if (pos !== null) return false; if (n.type.name !== "table") return true;
    let rp = p + 1; for (let i = 0; i < r; i++) rp += n.child(i).nodeSize; const row = n.child(r); let cp = rp + 1; for (let j = 0; j < c; j++) cp += row.child(j).nodeSize; pos = cp + 2; return false; }); return pos; };
  window.paintedCaret = (selector) => {
    const caret = document.querySelector(selector);
    if (!caret) return null;
    return [[...document.querySelectorAll(".pages .page")].indexOf(caret.closest(".page")), caret.style.left, caret.style.top];
  };
  window.tablebar = () => !document.getElementById("tablebar").hidden;
  window.widths = () => { const t = firstTable(); return t ? docxDemo.session.inspectNode(t.attrs.id).grid : null; };
  window.opened = []; window.open = (url, name, features) => { opened.push([url, name, features]); return null; };
  window.exported = () => { return docxDemo.engine.validate(docxDemo.session.replica.export()).valid; };
})()`;

const ORIGINAL = [["Name", "Role", "Notes"], ["Ada", "Spans two columns"], ["Inner AInner B", "Last cell"]];
const ROW = [["Name", "Role", "Notes"], ["Ada", "Spans two columns"], [""], ["Inner AInner B", "Last cell"]];
const TYPED = [["Name", "Role", "Notes"], ["Ada", "Spans two columns"], ["Typed"], ["Inner AInner B", "Last cell"]];
const COLUMN = [["Name", "", "Role", "Notes"], ["Ada", "", "Spans two columns"], ["", "Typed"], ["", "Inner AInner B", "Last cell"]];
const ABOVE = [["", "", ""], ...TYPED];
// Right of "Spans two columns", then left of the cell that adds after it.
const SPAN_RIGHT = [["Name", "Role", "Notes", ""], ["Ada", "Spans two columns", ""], ["Typed", ""], ["Inner AInner B", "Last cell", ""]];
const SPAN_LEFT = [["Name", "Role", "Notes", "", ""], ["Ada", "Spans two columns", "", ""], ["Typed", "", ""], ["Inner AInner B", "Last cell", "", ""]];
// Left of "Last cell", whose row starts with a cell a merge continues: grid column 2, inside the span of the cell below Ada.
const LEFT = [["Name", "Role", "", "Notes"], ["Ada", "Spans two columns"], ["Typed"], ["Inner AInner B", "", "Last cell"]];

export async function run(mode, a, b) {
  await a.front(b);
  const settle = a.paint;
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  const clearNotice = () => a.ev(`document.getElementById("notice").textContent = ""`);
  const type = (text) => a.send("Input.insertText", { text });
  // A pointer press, which moves focus as a person's does.
  const press = async (id) => {
    const [x, y] = await a.ev(`(() => { const r = document.getElementById(${JSON.stringify(id)}).getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; })()`);
    for (const type of ["mousePressed", "mouseReleased"]) await a.send("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1 });
  };
  const click = (id) => a.ev(`(() => { const b = document.getElementById(${JSON.stringify(id)}); b.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })); b.click(); })()`);
  const caret = async (r, c) => { const at = await a.ev(`cellPos(${r}, ${c})`); await a.ev(`select(${at}, ${at})`); await settle(); };
  const expect = async (label, want) => {
    await settle();
    const view = await a.ev(`grid()`), engine = await a.ev(`engineGrid()`);
    check(`${mode} ${label}: view`, same(view, want), { view, want });
    check(`${mode} ${label}: engine`, same(engine, want), { engine, want });
    if (b) {
      await b.front(a);
      const other = await b.ev(`grid()`);
      check(`${mode} ${label}: other tab`, same(other, want), other);
      await a.front(b);
    }
  };
  const original = await a.ev(`texts()`);
  await expect("opened", ORIGINAL);

  // The table buttons show only with the caret in a table.
  const before = await a.ev(`start(0)`);
  await a.ev(`select(${before + 1}, ${before + 1})`); await settle();
  check(`${mode} caret before the table: no table buttons`, !(await a.ev(`tablebar()`)), null);
  await clearNotice();
  await a.ev(`docxDemo.surface.table("row_below")`);
  await expect("table edit outside a table", ORIGINAL);
  check(`${mode} table edit outside a table: notice`, (await notice()).includes("table cell first"), await notice());
  await clearNotice();
  await caret(1, 0);
  check(`${mode} caret in a cell: table buttons`, await a.ev(`tablebar()`), null);

  // A row below Ada continues Ada's vertical merge, as the row below it does.
  await press("row_below");
  await expect("row below", ROW);
  check(`${mode} row below: editor keeps focus`, await a.ev(`pm().hasFocus()`), await a.ev(`document.activeElement?.id`));
  await caret(2, 0);
  if (b) {
    const local = await a.ev(`paintedCaret(".pages .caret.local")`);
    if (!local) throw new Error("The caret in the new cell was not painted");
    await waitForSharedCaret(a);
    await b.front(a);
    const remote = 'paintedCaret(".pages .caret:not(.local)")';
    await b.wait("remote caret in the new cell", `JSON.stringify(${remote}) === ${JSON.stringify(JSON.stringify(local))}`);
    check(`${mode} empty new cell has a remote caret before typing`, same(local, await b.ev(remote)), local);
    await a.front(b);
  }
  await type("Typed");
  await expect("typed in the new row", TYPED);
  const painted = await a.ev(`[...document.querySelectorAll(".pages .line span")].some((s) => s.textContent.includes("Typed"))`);
  check(`${mode} typed in the new row: painted`, painted, null);

  // A column right of Ada's.
  await caret(1, 0); await click("column_right");
  await expect("column right", COLUMN);
  if (mode === "local") check(`${mode} column right: export validates`, (await a.ev(`exported()`)) === true, await a.ev(`exported()`));
  await caret(0, 1); await click("remove_column");
  await expect("column removed", TYPED);

  await caret(0, 0); await click("row_above");
  await expect("row above the header", ABOVE);
  await caret(0, 0); await click("remove_row");
  await expect("row removed", TYPED);
  await caret(3, 1); await click("column_left");
  await expect("column left of a cell after a merge", LEFT);
  await caret(3, 1); await click("remove_column");
  await expect("that column removed", TYPED);

  // Right of a cell spanning two columns is after both, and halves the second; a cell after it starts two columns on.
  const w = await a.ev(`widths()`), half = (x) => Math.floor(x / 2);
  const right = [w[0], w[1], w[2] - half(w[2]), half(w[2])];
  const left = [...right.slice(0, 3), right[3] - half(right[3]), half(right[3])];
  const widths = async (label, want) => {
    check(`${mode} ${label}: widths`, same(await a.ev(`widths()`), want), [await a.ev(`widths()`), want]);
    if (b) { await b.front(a); check(`${mode} ${label}: other tab's widths`, same(await b.ev(`widths()`), want), await b.ev(`widths()`)); await a.front(b); }
  };
  await caret(1, 1); await click("column_right");
  await expect("column right of a spanning cell", SPAN_RIGHT);
  await widths("column right of a spanning cell", right);
  await caret(1, 2); await click("column_left");
  await expect("column left of a cell after a spanning cell", SPAN_LEFT);
  await widths("column left of a cell after a spanning cell", left);
  for (let i = 0; i < 2; i++) { await a.ev(`key("z", { metaKey: true })`); await settle(); }
  await expect("both undone", TYPED);
  await widths("both undone", w);

  // A row whose merged cell continues below it stays, and the notice says why.
  await clearNotice();
  await caret(1, 0); await click("remove_row");
  await expect("merged row refused", TYPED);
  check(`${mode} merged row refused: notice`, (await notice()).includes("merged cell"), await notice());
  await clearNotice();

  await caret(1, 0); await click("remove_table");
  await settle();
  const removed = original.filter((t) => t !== "[table]");
  check(`${mode} table removed`, original.includes("[table]") && same(await a.ev(`texts()`), removed), [original, await a.ev(`texts()`)]);
  if (mode === "local") check(`${mode} table removed: export validates`, (await a.ev(`exported()`)) === true, await a.ev(`exported()`));

  for (let i = 0; i < 9; i++) { await a.ev(`key("z", { metaKey: true })`); await settle(); }
  await expect("all undone", ORIGINAL);
  check(`${mode} all undone: text`, same(await a.ev(`texts()`), original), await a.ev(`texts()`));
  check(`${mode}: no refusal left`, (await notice()) === "", await notice());
  const errors = [...a.errors, ...(b?.errors ?? [])];
  check(`${mode}: no exceptions`, errors.length === 0, errors);
}
