// Types, presses Enter, Backspace and Delete, pastes and composes with
// Track Changes on in the browser demo, locally and in two shared tabs, and
// checks the view, the engine, the painted page, the other tab, the export
// and undo.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs track

import { check, same } from "./harness.mjs";

export const fixture = "generated/simple-paragraphs.docx";

export const helpers = `(() => {
  window.texts = () => { const out = []; pm().state.doc.forEach((n) => out.push(n.type.name === "paragraph" ? n.textContent : "[" + n.type.name + "]")); return out; };
  // Each paragraph's inserted text, as the view marks it: [text, author].
  window.inserted = () => { const out = []; pm().state.doc.descendants((n) => { const m = n.isText && n.marks.find((m) => m.type.name === "revision"); if (m) out.push([n.text, m.attrs.author]); }); return out; };
  // Each deletion atom of the view: [position, deleted text, author].
  window.deleted = () => { const out = []; pm().state.doc.descendants((n, pos) => { const i = n.type.name === "inline_atom" && n.attrs.node; if (i && i.type === "revision" && i.change === "del") out.push([pos, n.textContent || n.attrs.node.children.map(function t(c) { return c.text ?? (c.children ?? []).map(t).join(""); }).join(""), i.author]); }); return out; };
  // The engine's revisions in the body: change, author, date, text, editable.
  window.revisions = (inspection) => { const out = []; const text = (n) => n.text ?? (n.children ?? []).map(text).join("");
    const walk = (n) => { if (n.type === "revision") out.push([n.change, n.author, n.date, text(n), n.support]); n.children?.forEach(walk); };
    walk(inspection.stories[0]); return out; };
  window.engineRevisions = () => { const out = []; pm().state.doc.forEach((p) => { if (p.attrs.id) out.push(...revisions({ stories: [docxDemo.session.inspectNode(p.attrs.id)] })); }); return out; };
  window.marks = () => { const out = []; pm().state.doc.forEach((p) => out.push(p.attrs.info?.mark_revision ?? null)); return out; };
  window.paintedAuthors = () => [...document.querySelectorAll(".pages [data-author]")].filter((e) => !e.closest(".input")).map((e) => [e.textContent, e.dataset.author, e.title.split(" ")[0]]);
  window.cut = () => { const dt = new DataTransfer(); pm().dom.dispatchEvent(new ClipboardEvent("cut", { clipboardData: dt, bubbles: true, cancelable: true })); return dt.getData("text/plain"); };
  window.paste = (text) => { const dt = new DataTransfer(); dt.setData("text/plain", text); pm().dom.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true })); };
  window.exportBytes = () => docxDemo.session.replica.export();
  window.exported = () => { const bytes = exportBytes(); const valid = docxDemo.engine.validate(bytes).valid; const re = docxDemo.engine.open(bytes);
    const inspection = re.inspect(); const revs = revisions(inspection); const hash = [...bytes].reduce((h, b) => (h * 31 + b) >>> 0, 7); re.free(); return { valid, revisions: revs, hash, length: bytes.length }; };
  window.toggle = () => document.getElementById("track").click();
  window.author = (name) => { const input = document.getElementById("name"); input.value = name; input.dispatchEvent(new Event("change")); };
})()`;

export async function run(mode, a, b) {
  await a.front(b);
  const erase = async (key) => {
    for (const type of ["keyDown", "keyUp"]) {
      await a.send("Input.dispatchKeyEvent", { type, key, code: key, windowsVirtualKeyCode: key === "Delete" ? 46 : 8 });
    }
  };
  const settle = a.paint;
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  const clearNotice = () => a.ev(`document.getElementById("notice").textContent = ""`);
  const type = async (text) => { await a.ev(`pm().focus()`); await a.send("Input.insertText", { text }); await settle(); };
  const head = () => a.ev(`pm().state.selection.head`);
  const other = async (label) => {
    if (!b) return;
    await b.front(a);
    check(`${mode} ${label}: other tab shows the same text`, same(await b.ev(`texts()`), await a.ev(`texts()`)), [await b.ev(`texts()`), await a.ev(`texts()`)]);
    check(`${mode} ${label}: other tab shows the same changes`, same(await b.ev(`[inserted(), deleted().map(([, t, w]) => [t, w])]`), await a.ev(`[inserted(), deleted().map(([, t, w]) => [t, w])]`)), await b.ev(`[inserted(), deleted()]`));
    const [ea, eb] = [await a.ev(`exported()`), await b.ev(`exported()`)];
    check(`${mode} ${label}: both tabs export the same bytes`, same(ea, eb) && ea.valid, [ea, eb]);
    await a.front(b);
  };
  const original = await a.ev(`texts()`);
  const before = await a.ev(`exported()`);
  check(`${mode} opened with no revisions`, before.revisions.length === 0, before.revisions);

  await a.ev(`author("Alice")`);
  await a.ev(`toggle()`);
  check(`${mode} toggle pressed`, (await a.ev(`document.getElementById("track").getAttribute("aria-pressed")`)) === "true", null);
  check(`${mode} session tracks as Alice`, (await a.ev(`docxDemo.session.tracking`)) === "Alice", await a.ev(`docxDemo.session.tracking`));

  // Typing is an insertion by Alice, and typing on joins it.
  const p0 = await a.ev(`start(0)`);
  await a.ev(`select(${p0 + 5}, ${p0 + 5})`);
  await type("XY");
  // Typing on after a pause joins the same insertion, as in Word.
  await a.ev(`new Promise(resolve => { const second = Math.floor(Date.now() / 1000); const frame = () => Math.floor(Date.now() / 1000) > second ? resolve(true) : requestAnimationFrame(frame); frame(); })`);
  await type("Z");
  let texts = await a.ev(`texts()`);
  check(`${mode} typed: text in place`, texts[0] === original[0].slice(0, 5) + "XYZ" + original[0].slice(5), texts[0]);
  check(`${mode} typed: marked inserted by Alice in the view`, same(await a.ev(`inserted()`), [["XYZ", "Alice"]]), await a.ev(`inserted()`));
  let revs = await a.ev(`engineRevisions()`);
  check(`${mode} typed: one editable insertion in the engine`, revs.length === 1 && revs[0][0] === "ins" && revs[0][1] === "Alice" && revs[0][3] === "XYZ" && revs[0][4] === "editable" && /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/.test(revs[0][2]), revs);
  check(`${mode} typed: caret after it`, (await head()) === p0 + 8, await head());
  check(`${mode} typed: painted as Alice's insertion`, same(await a.ev(`paintedAuthors()`), [["XYZ", "Alice", "ins"]]), await a.ev(`paintedAuthors()`));
  let out = await a.ev(`exported()`);
  check(`${mode} typed: export validates and reopens with the insertion`, out.valid && same(out.revisions.map((r) => [r[0], r[1], r[3]]), [["ins", "Alice", "XYZ"]]), out);
  await other("typed");

  // Backspace takes back Alice's own typing.
  await erase("Backspace"); await settle();
  check(`${mode} backspace own typing: removed`, same(await a.ev(`inserted()`), [["XY", "Alice"]]) && (await head()) === p0 + 7, [await a.ev(`inserted()`), await head()]);

  // Backspace in the document's text marks it deleted, and passes over it.
  await a.ev(`select(${p0 + 3}, ${p0 + 3})`);
  await erase("Backspace"); await settle();
  await a.ev(`new Promise(resolve => { const second = Math.floor(Date.now() / 1000); const frame = () => Math.floor(Date.now() / 1000) > second ? resolve(true) : requestAnimationFrame(frame); frame(); })`);
  await erase("Backspace"); await settle();
  let gone = await a.ev(`deleted()`);
  check(`${mode} backspace: one deletion by Alice`, same(gone, [[p0 + 1, original[0].slice(1, 3), "Alice"]]), gone);
  check(`${mode} backspace: caret before it`, (await head()) === p0 + 1, await head());
  check(`${mode} backspace: text gone from the line`, (await a.ev(`texts()`))[0].startsWith(original[0][0] + original[0].slice(3, 5) + "XY"), (await a.ev(`texts()`))[0]);
  // Delete passes over the deletion and takes the next character.
  await erase("Delete"); await settle();
  gone = await a.ev(`deleted()`);
  check(`${mode} delete: joins the deletion`, same(gone, [[p0 + 1, original[0].slice(1, 4), "Alice"]]), gone);
  check(`${mode} delete: caret after it`, (await head()) === p0 + 2, await head());
  revs = await a.ev(`engineRevisions()`);
  check(`${mode} delete: engine has one deletion and one insertion`, same(revs.map((r) => [r[0], r[1], r[3]]), [["del", "Alice", original[0].slice(1, 4)], ["ins", "Alice", "XY"]]), revs);
  await other("deleted");

  // Typing over a selection deletes it and inserts after it.
  await a.ev(`select(${p0 + 2}, ${p0 + 3})`);
  await type("Q");
  revs = await a.ev(`engineRevisions()`);
  check(`${mode} typed over: deletion then insertion`, same(revs.map((r) => [r[0], r[3]]).slice(0, 2), [["del", original[0].slice(1, 5)], ["ins", "Q"]]), revs);

  // Enter starts a paragraph whose mark is inserted, and Backspace takes it back.
  const count = (await a.ev(`texts()`)).length;
  await a.ev(`key("Enter")`); await settle();
  check(`${mode} enter: a new paragraph`, (await a.ev(`texts()`)).length === count + 1 && (await a.ev(`marks()`))[0] === "ins", [await a.ev(`texts()`), await a.ev(`marks()`)]);
  check(`${mode} enter: caret at the new paragraph's start`, (await head()) === (await a.ev(`start(1)`)), [await head(), await a.ev(`start(1)`)]);
  await other("enter");
  await erase("Backspace"); await settle();
  check(`${mode} backspace own paragraph mark: joined`, (await a.ev(`texts()`)).length === count && (await a.ev(`marks()`))[0] === null, [await a.ev(`texts()`), await a.ev(`marks()`)]);

  // Backspace at the start of the document's paragraph marks the paragraph mark before it deleted.
  const p1 = await a.ev(`start(1)`);
  await a.ev(`select(${p1}, ${p1})`);
  await erase("Backspace"); await settle();
  check(`${mode} backspace at a paragraph start: mark deleted, paragraphs kept`, (await a.ev(`texts()`)).length === count && (await a.ev(`marks()`))[0] === "del", await a.ev(`marks()`));
  check(`${mode} backspace at a paragraph start: caret at the end of the one before`, (await head()) === p1 - 2, await head());
  // Delete there passes over the deleted mark and takes the next paragraph's first character.
  const second = (await a.ev(`texts()`))[1];
  await erase("Delete"); await settle();
  check(`${mode} delete over a deleted mark: next paragraph's first character`, same(await a.ev(`deleted()`).then((d) => d.map(([p, t]) => [p, t]).at(-1)), [p1, second[0]]) && (await head()) === p1 + 1, [await a.ev(`deleted()`), await head()]);
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} delete over a deleted mark: undone`, (await a.ev(`texts()`))[1] === second, await a.ev(`texts()`));
  await other("paragraph mark");

  // Pasted lines are inserted paragraphs.
  await a.ev(`select(${p1}, ${p1})`);
  await a.ev(`paste("one\\ntwo")`); await settle();
  texts = await a.ev(`texts()`);
  check(`${mode} paste: two paragraphs inserted`, texts.length === count + 1 && texts[1] === "one" && texts[2].startsWith("two"), texts);
  check(`${mode} paste: marked inserted`, (await a.ev(`inserted()`)).some(([t, w]) => t === "one" && w === "Alice") && (await a.ev(`inserted()`)).some(([t, w]) => t === "two" && w === "Alice"), await a.ev(`inserted()`));
  await other("paste");

  // An IME's composed text is tracked when the composition ends.
  const p2 = await a.ev(`start(2)`);
  await a.ev(`select(${p2}, ${p2})`);
  await a.ev(`pm().focus()`);
  await a.send("Input.imeSetComposition", { text: "に", selectionStart: 1, selectionEnd: 1 });
  await a.paint();
  await a.send("Input.imeSetComposition", { text: "日本", selectionStart: 2, selectionEnd: 2 });
  await a.paint();
  await a.send("Input.insertText", { text: "日本" });
  await settle();
  check(`${mode} composed: tracked`, (await a.ev(`inserted()`)).some(([t, w]) => t.includes("日本") && w === "Alice") && (await a.ev(`texts()`))[2].startsWith("日本two"), [await a.ev(`inserted()`), await a.ev(`texts()`)]);
  await other("composed");

  // Cutting across two paragraphs marks the text and the paragraph mark
  // between them deleted, and puts the text on the clipboard.
  const p3 = await a.ev(`start(3)`);
  const [secondNow, third] = (await a.ev(`texts()`)).slice(2, 4);
  const cutText = await a.ev(`select(${p3 - 5}, ${p3 + 3}); cut()`);
  await settle();
  check(`${mode} cut: clipboard has the text`, cutText === secondNow.slice(-3) + "\n" + third.slice(0, 3), [cutText, secondNow, third]);
  const cutGone = (await a.ev(`deleted()`)).map(([, t, w]) => [t, w]);
  texts = await a.ev(`texts()`);
  check(`${mode} cut: marked deleted by Alice, paragraphs kept`, cutGone.some(([t, w]) => t === secondNow.slice(-3) && w === "Alice") && cutGone.some(([t, w]) => t === third.slice(0, 3) && w === "Alice") && (await a.ev(`marks()`))[2] === "del" && texts[2] === secondNow.slice(0, -3) && texts[3] === third.slice(3), [cutGone, await a.ev(`marks()`), texts]);
  await other("cut");
  // Formatting is refused while tracking, and changes nothing.
  await clearNotice();
  const now = await a.ev(`exported()`);
  await a.ev(`select(${p0 + 2}, ${p0 + 4})`);
  await a.ev(`key("b", { metaKey: true })`); await settle();
  await a.ev(`key("Enter", { shiftKey: true })`); await settle();
  check(`${mode} formatting refused`, (await notice()).startsWith("Track Changes records typing and deleting") && same(await a.ev(`exported()`), now), await notice());
  await clearNotice();

  const errors = [...a.errors, ...(b?.errors ?? [])];
  check(`${mode}: no exceptions`, errors.length === 0, errors);
  check(`${mode}: no refusal left`, (await notice()) === "", await notice());

  // Undo takes all of it back, to the bytes the document opened with.
  for (let i = 0; i < 40 && (await a.ev(`exported()`)).hash !== before.hash; i++) { await a.ev(`key("z", { metaKey: true })`); await settle(); }
  out = await a.ev(`exported()`);
  check(`${mode} all undone: the bytes it opened with`, out.hash === before.hash && same(await a.ev(`texts()`), original), [out, await a.ev(`texts()`)]);
  check(`${mode} all undone: nothing painted as a change`, (await a.ev(`paintedAuthors()`)).length === 0, await a.ev(`paintedAuthors()`));
  await other("all undone");

  // With Track Changes off, typing is not tracked.
  await a.ev(`toggle()`);
  await a.ev(`select(${p0 + 1}, ${p0 + 1})`);
  await type("U");
  check(`${mode} off: typing untracked`, (await a.ev(`engineRevisions()`)).length === 0 && (await a.ev(`texts()`))[0].startsWith(original[0][0] + "U"), await a.ev(`engineRevisions()`));
  await a.ev(`key("z", { metaKey: true })`); await settle();
}
