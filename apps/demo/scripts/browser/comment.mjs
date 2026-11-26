// Adds comments and replies in the browser demo, locally and in two shared
// tabs, and checks the field, the balloons, the shading, the other tab, the
// export, undo and redo.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs comment

import { check, click, same } from "./harness.mjs";

export const fixture = "generated/simple-paragraphs.docx";
export const windowSize = "1500,1400";

export const helpers = `(() => {
  window.texts = () => { const out = []; pm().state.doc.forEach((n) => out.push(n.type.name === "paragraph" ? n.textContent : "[" + n.type.name + "]")); return out; };
  window.field = () => { const bar = document.getElementById("commentbar"); const t = document.getElementById("commenttext"); return { open: !bar.hidden, placeholder: t.placeholder, focused: document.activeElement === t, value: t.value }; };
  window.fieldKey = (key, mods = {}) => document.getElementById("commenttext").dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...mods }));
  // Each balloon's lines, as painted.
  window.balloons = () => [...document.querySelectorAll(".pages .balloon")].map((b) => [...b.querySelectorAll(".line")].map((l) => l.textContent.trim()));
  window.replies = () => [...document.querySelectorAll(".pages .reply")].map((r) => r.dataset.comment);
  // The painted text each comment shades.
  window.shaded = () => { const out = {}; for (const e of document.querySelectorAll(".pages [data-comments]")) { if (e.closest(".input")) continue; for (const id of e.dataset.comments.split(" ")) out[id] = (out[id] ?? "") + e.textContent; } return out; };
  // A document's comments: ID, author, parent, text, each paragraph a line.
  window.commentsOf = (inspection) => { const out = []; const text = (n) => n.text ?? (n.children ?? []).map(text).join("");
    for (const s of inspection.stories) if (s.story === "comments") for (const c of s.children ?? []) if (c.type === "comment") out.push([c.comment_id, c.author, c.parent ?? null, (c.children ?? []).map(text).join("\\n")]);
    return out; };
  window.markers = (inspection) => { const out = []; const walk = (n) => { if (n.type === "marker" && n.element.includes(":comment")) out.push([n.element.split(":")[1], n.marker_id]); n.children?.forEach(walk); }; walk(inspection.stories[0]); return out; };
  window.exportBytes = () => docxDemo.session.replica.export();
  window.exported = () => { const bytes = exportBytes(); const valid = docxDemo.engine.validate(bytes).valid; const re = docxDemo.engine.open(bytes);
    const inspection = re.inspect(); const out = { valid, comments: commentsOf(inspection), markers: markers(inspection), hash: [...bytes].reduce((h, b) => (h * 31 + b) >>> 0, 7) }; re.free(); return out; };
  window.toggle = () => document.getElementById("track").click();
  window.author = (name) => { const input = document.getElementById("name"); input.value = name; input.dispatchEvent(new Event("change")); };
  window.center = (selector) => { const e = document.querySelector(selector); e.scrollIntoView({ block: "center", inline: "center" }); const r = e.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; };
})()`;

export async function run(mode, a, b) {
  await a.front(b);
  const settle = a.paint;
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  const clearNotice = () => a.ev(`document.getElementById("notice").textContent = ""`);
  const write = async (t, text) => { await t.send("Input.insertText", { text }); };
  const other = async (label) => {
    if (!b) return;
    await b.front(a);
    check(`${mode} ${label}: other tab shows the same balloons`, same(await b.ev(`balloons()`), await a.ev(`balloons()`)), [await b.ev(`balloons()`), await a.ev(`balloons()`)]);
    check(`${mode} ${label}: other tab shades the same text`, same(await b.ev(`shaded()`), await a.ev(`shaded()`)), [await b.ev(`shaded()`), await a.ev(`shaded()`)]);
    const [ea, eb] = [await a.ev(`exported()`), await b.ev(`exported()`)];
    check(`${mode} ${label}: both tabs export the same bytes`, same(ea, eb) && ea.valid, [ea, eb]);
    await a.front(b);
  };
  const original = await a.ev(`texts()`);
  check(`${mode} opened with no balloons`, (await a.ev(`balloons()`)).length === 0, await a.ev(`balloons()`));
  check(`${mode} the first balloon's label font is ready`, await a.ev(
    'Array.from(document.fonts).some(face => face.family === "Tinos" && face.weight === "700" && face.status === "loaded")'));
  await a.ev(`author("Alice")`);

  // ⌘⌥M over a selection opens the field under it; Enter posts.
  const p0 = await a.ev(`start(0)`);
  await a.ev(`select(${p0}, ${p0 + 5})`);
  await a.ev(`key("m", { metaKey: true, altKey: true })`);
  await a.paint();
  let f = await a.ev(`field()`);
  check(`${mode} ⌘⌥M opens the comment field, focused`, f.open && f.focused && f.placeholder.startsWith("Add a comment"), f);
  await write(a, "Why first?");
  await a.ev(`fieldKey("Enter")`); await settle();
  f = await a.ev(`field()`);
  check(`${mode} Enter posts and closes the field`, !f.open, f);
  check(`${mode} comment: a balloon by Alice says it`, same(await a.ev(`balloons()`), [["Alice", "Why first?"]]), await a.ev(`balloons()`));
  check(`${mode} comment: its text is shaded`, same(await a.ev(`shaded()`), { 0: original[0].slice(0, 5) }), await a.ev(`shaded()`));
  check(`${mode} comment: the text is unchanged`, same(await a.ev(`texts()`), original), await a.ev(`texts()`));
  let out = await a.ev(`exported()`);
  check(`${mode} comment: export validates and reopens with it`, out.valid && same(out.comments, [["0", "Alice", null, "Why first?"]]) && same(out.markers, [["commentRangeStart", "0"], ["commentRangeEnd", "0"], ["commentReference", "0"]]), out);
  await other("comment");

  // Reply in the balloon opens the field for a reply; it joins the balloon.
  await a.ev(`author("Bob")`);
  await click(a, ".pages .reply");
  await a.paint();
  f = await a.ev(`field()`);
  check(`${mode} Reply opens the field for a reply`, f.open && f.focused && f.placeholder === "Reply", f);
  await write(a, "Because.");
  await click(a, "#commentpost"); await settle();
  check(`${mode} reply: shades the comment's text`, same(await a.ev(`shaded()`), { 0: original[0].slice(0, 5), 1: original[0].slice(0, 5) }), await a.ev(`shaded()`));
  check(`${mode} reply: drawn in the comment's balloon`, same(await a.ev(`balloons()`), [["Alice", "Why first?", "Bob", "Because."]]) && same(await a.ev(`replies()`), ["0"]), [await a.ev(`balloons()`), await a.ev(`replies()`)]);
  out = await a.ev(`exported()`);
  check(`${mode} reply: export reopens with the thread`, out.valid && same(out.comments, [["0", "Alice", null, "Why first?"], ["1", "Bob", "0", "Because."]]), out);
  await other("reply");

  // A draft is on what was selected when its field opened, wherever the selection goes meanwhile.
  const p2 = await a.ev(`start(2)`);
  await a.ev(`select(${p2}, ${p2 + 5})`);
  await a.ev(`key("m", { metaKey: true, altKey: true })`); await a.paint();
  await write(a, "Held");
  await a.ev(`select(${p0 + 6}, ${p0 + 9})`); await a.paint();
  await a.ev(`document.getElementById("commenttext").focus(); fieldKey("Enter")`); await settle();
  check(`${mode} a draft comments on what it was opened on`, same(await a.ev(`shaded()`), { 0: original[0].slice(0, 5), 1: original[0].slice(0, 5), 2: original[2].slice(0, 5) }) && !(await a.ev(`field()`)).open, await a.ev(`shaded()`));
  await other("a held draft");
  await a.ev(`pm().focus(); key("z", { metaKey: true })`); await settle();
  check(`${mode} the held comment is undone`, same(await a.ev(`balloons()`), [["Alice", "Why first?", "Bob", "Because."]]), await a.ev(`balloons()`));

  // A comment of two lines on the word at the caret, with Track Changes on.
  await a.ev(`toggle()`);
  const p1 = await a.ev(`start(1)`);
  await a.ev(`select(${p1 + 3}, ${p1 + 3})`);
  await click(a, "#comment");
  await a.paint();
  check(`${mode} the Comment button opens the field`, (await a.ev(`field()`)).open, await a.ev(`field()`));
  await write(a, "Line one\nLine two");
  await a.ev(`fieldKey("Enter")`); await settle();
  check(`${mode} at the caret while tracking: no refusal`, (await notice()) === "", await notice());
  check(`${mode} at the caret: a second balloon, level with its word, with both lines`, same(await a.ev(`balloons()`), [["Alice", "Why first?", "Bob", "Because."], ["Bob", "Line one", "Line two"]]), await a.ev(`balloons()`));
  check(`${mode} at the caret: the word is shaded`, same(await a.ev(`shaded()`), { 0: original[0].slice(0, 5), 1: original[0].slice(0, 5), 2: original[1].slice(0, 6) }), await a.ev(`shaded()`));
  out = await a.ev(`exported()`);
  check(`${mode} at the caret: the word's range, and no tracked change`, out.valid && same(out.comments.at(-1), ["2", "Bob", null, "Line one\nLine two"]) && same(out.markers.slice(6), [["commentRangeStart", "2"], ["commentRangeEnd", "2"], ["commentReference", "2"]]) && same(await a.ev(`texts()`), original), out);
  await a.ev(`toggle()`);
  await other("at the caret");
  if (process.env.SHOT && mode === "local") {
    const r = await a.ev(`(() => { const p = document.querySelector(".pages .page"); p.scrollIntoView(); const b = p.getBoundingClientRect(); return [b.left, b.top, b.width, 260]; })()`);
    await a.paint();
    const shot = await a.send("Page.captureScreenshot", { format: "png", clip: { x: r[0], y: r[1], width: r[2], height: r[3], scale: 1.5 } });
    (await import("node:fs")).writeFileSync(process.env.SHOT, Buffer.from(shot.result.data, "base64"));
  }

  // A caret in an empty paragraph has no word to comment on.
  const last = original.findLastIndex((t) => !t.startsWith("["));
  const end = (await a.ev(`start(${last})`)) + original[last].length;
  await a.ev(`select(${end}, ${end})`);
  await a.ev(`key("Enter")`); await settle();
  await a.ev(`key("m", { metaKey: true, altKey: true })`); await a.paint();
  check(`${mode} no word at the caret: refused before a draft opens, nothing added`, !(await a.ev(`field()`)).open && (await notice()) === "Put the caret in a word, or select the text to comment on." && (await a.ev(`balloons()`)).length === 2 && (await a.ev(`exported()`)).comments.length === 3, [await notice(), await a.ev(`balloons()`), await a.ev(`field()`)]);
  await clearNotice();
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} the empty paragraph is undone`, same(await a.ev(`texts()`), original), await a.ev(`texts()`));

  // An empty field posts nothing; Escape closes it.
  await clearNotice();
  await a.ev(`select(${p0}, ${p0 + 2})`);
  await a.ev(`key("m", { metaKey: true, altKey: true })`); await a.paint();
  await a.ev(`fieldKey("Enter")`); await a.paint();
  check(`${mode} an empty comment is not posted`, (await a.ev(`field()`)).open && (await notice()) === "Type the comment first." && (await a.ev(`balloons()`)).length === 2, [await a.ev(`field()`), await notice()]);
  await write(a, "Half typed");
  await a.ev(`pm().focus()`); await a.paint();
  check(`${mode} a half-typed comment stays when the field loses focus`, same(await a.ev(`field()`), { open: true, placeholder: "Add a comment (Shift+Enter for a new line)", focused: false, value: "Half typed" }), await a.ev(`field()`));
  await a.ev(`document.getElementById("commenttext").focus()`);
  await a.ev(`fieldKey("Escape")`); await a.paint();
  check(`${mode} Escape closes the field and returns to editing`, !(await a.ev(`field()`)).open && (await a.ev(`pm().hasFocus()`)), await a.ev(`field()`));
  await clearNotice();

  await a.ev(`select(start(0), start(0) + 2); key("m", { metaKey: true, altKey: true })`);
  await a.ev(`document.getElementById("commenttext").value = "Keep\\tthis"; fieldKey("Enter")`);
  check(`${mode} an engine refusal preserves the draft`, await a.ev(`field().open && field().focused && field().value === "Keep\\tthis" && document.getElementById("notice").textContent.length > 0`), await a.ev(`field()`));
  await a.ev(`fieldKey("Escape")`);
  await clearNotice();

  // Undo takes back each in turn, and redo brings them back.
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} undo: the caret comment goes`, same(await a.ev(`balloons()`), [["Alice", "Why first?", "Bob", "Because."]]), await a.ev(`balloons()`));
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} undo: the reply goes`, same(await a.ev(`balloons()`), [["Alice", "Why first?"]]), await a.ev(`balloons()`));
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} undo: the comment goes`, (await a.ev(`balloons()`)).length === 0 && same(await a.ev(`shaded()`), {}), [await a.ev(`balloons()`), await a.ev(`shaded()`)]);
  out = await a.ev(`exported()`);
  check(`${mode} undo: the export has no comments`, out.valid && out.comments.length === 0 && out.markers.length === 0, out);
  await other("undone");
  await a.ev(`key("z", { metaKey: true, shiftKey: true })`); await settle();
  await a.ev(`key("z", { metaKey: true, shiftKey: true })`); await settle();
  check(`${mode} redo: the comment and the reply return`, same(await a.ev(`balloons()`), [["Alice", "Why first?", "Bob", "Because."]]), await a.ev(`balloons()`));
  await other("redone");

  if (b) {
    // The other person replies from their tab.
    await b.front(a);
    await b.ev(`author("Carol")`);
    await click(b, ".pages .reply");
    await b.paint();
    await write(b, "Me too.");
    await b.ev(`fieldKey("Enter")`); await b.paint();
    await a.front(b);
    check(`${mode} a reply from the other tab shows here`, same(await a.ev(`balloons()`), [["Alice", "Why first?", "Bob", "Because.", "Carol", "Me too."]]), await a.ev(`balloons()`));
    await other("the other tab's reply");
    // A new root has no replies keeping it alive. Leave a draft open on it.
    await a.ev(`select(start(0), start(0) + 2); key("m", { metaKey: true, altKey: true })`);
    await write(a, "Temporary thread");
    await a.ev(`fieldKey("Enter")`);
    await b.front(a);
    await b.ev(`select(start(1), start(1) + 2); key("m", { metaKey: true, altKey: true })`);
    await write(b, "Remaining thread");
    await b.ev(`fieldKey("Enter")`);
    await b.paint();
    await b.ev(`Array.from(document.querySelectorAll(".pages .reply")).find(el => el.parentElement.textContent.includes("Temporary thread"))?.click()`);
    await write(b, "Reply to temporary thread");
    await a.front(b);
    await a.ev(`key("z", { metaKey: true })`);
    await b.front(a);
    await b.ev(`fieldKey("Enter")`);
    await b.paint();
    check("removed thread keeps its reply draft", await b.ev(`field().open && field().value === "Reply to temporary thread" && document.getElementById("notice").textContent.includes("removed")`), await b.ev(`field()`));
    check("no other thread receives the draft", !(await b.ev(`JSON.stringify(balloons())`)).includes("Reply to temporary thread"));

    await b.ev(`fieldKey("Escape"); select(start(2), start(2) + 3); key("m", { metaKey: true, altKey: true })`);
    await write(b, "Retargeted draft");
    await a.front(b);
    await a.ev(`select(start(2), start(2) + 3); key("Backspace")`);
    await b.front(a);
    await b.ev(`fieldKey("Enter")`);
    check("deleting the selected text keeps its comment draft", await b.ev(`field().open && field().value === "Retargeted draft" && document.getElementById("notice").textContent.includes("Select other text")`), await b.ev(`field()`));
    await b.ev(`select(start(1), start(1) + 3); key("m", { metaKey: true, altKey: true }); fieldKey("Enter")`);
    await b.paint();
    check("the kept draft posts on the new selection", await b.ev(`!field().open && exported().comments.some(c => c[3] === "Retargeted draft") && Object.values(shaded()).includes(texts()[1].slice(0, 3))`), await b.ev(`shaded()`));

  }

  const errors = [...a.errors, ...(b?.errors ?? [])];
  check(`${mode}: no exceptions`, errors.length === 0, errors);
  check(`${mode}: no refusal left`, (await notice()) === "", await notice());
}
