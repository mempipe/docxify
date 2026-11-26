// Updates the fields of the generated fixture in the browser demo, locally
// and in two shared tabs, by clicking Update fields as a person would, and
// checks the painted text, the export, refusals, undo, redo and the other tab.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs fields

import { check, click, same } from "./harness.mjs";

export const fixtures = ["generated/fields.docx", "generated/sections.docx"];
export const windowSize = "1500,1400";

export const helpers = `(() => {
  window.startOf = (prefix) => { let at = null; pm().state.doc.descendants((n, pos) => { if (at === null && n.isTextblock && n.textContent.startsWith(prefix)) at = pos + 1; return at === null; }); return at; };
  window.focused = () => document.activeElement === pm().dom;
  window.textsOf = (inspection) => { const out = [];
    const text = (n) => n.type === "text" ? n.text : n.type === "tab" ? "\\t" : n.type === "field_code" ? "" : (n.children ?? []).map(text).join("");
    const walk = (n) => { if (n.type === "paragraph") out.push(text(n)); else n.children?.forEach(walk); };
    walk(inspection.stories[0]); return out; };
  window.exportBytes = () => docxDemo.session.replica.export();
  window.exported = () => { const bytes = exportBytes(); const valid = docxDemo.engine.validate(bytes).valid; const re = docxDemo.engine.open(bytes);
    const out = { valid, texts: textsOf(re.inspect()), hash: [...bytes].reduce((h, b) => (h * 31 + b) >>> 0, 7) }; re.free(); return out; };
  window.lines = () => [...document.querySelectorAll(".pages .line")].map((l) => l.textContent);
  window.line = (prefix) => lines().find((l) => l.startsWith(prefix)) ?? null;
  // Pages out of view are not painted, so tabs compare the lines that show fields, on the first two pages.
  window.watched = () => ["Introduction.", "Method.", "Results.", "Printed on", "This is page", "The method", "XThe method"].map(line);
  window.pageCount = () => document.querySelectorAll(".pages .page").length;
  window.toggle = () => document.getElementById("track").click();
  // Copies the whole footer that page \`shown\` paints, with the caret's page set to \`caret\`.
  window.footerCopy = (shown, caret) => { const s = docxDemo.surface;
    const painted = (page, key) => s.laid[page].fragments.filter((f) => f.kind === "line" && f.story === key).map((f) => f.line.pieces.map((p) => p.text).join("")).join("");
    const story = s.stories.find((x) => x.kind === "footer" && painted(shown, x.key)); const v = story.view;
    v.dispatch(v.state.tr.setSelection(v.state.selection.constructor.create(v.state.doc, 1, v.state.doc.content.size - 1)));
    s.page = caret; const clipboard = new DataTransfer();
    v.dom.dispatchEvent(new ClipboardEvent("copy", { clipboardData: clipboard, bubbles: true, cancelable: true }));
    return { here: painted(shown, story.key), first: painted(s.laid.findIndex((_, page) => painted(page, story.key)), story.key),
      copied: [clipboard.getData("text/plain"), new DOMParser().parseFromString(clipboard.getData("text/html"), "text/html").body.textContent] }; };
  window.center = (selector) => { const e = document.querySelector(selector); e.scrollIntoView({ block: "center", inline: "center" }); const r = e.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; };
})()`;

const today = new Date().toLocaleDateString("en-US", { weekday: "long", month: "long", day: "numeric", year: "numeric" });
const stale = ["Fields", "Contents", "Introduction\t9", "Method\t9", "Results\t9", "Printed on Monday, January 1, 2024", "This is page 9 of 9.", "Introduction", "The method is on page 9.", "Method", "How it was done.", "Results", "What was found."];
const fresh = ["Fields", "Contents", "Introduction\t1", "Method\t2", "Results\t3", `Printed on ${today}`, "This is page i of 4.", "Introduction", "The method is on page 2.", "Method", "How it was done.", "Results", "What was found."];

/** A footer repeats on several pages, each with its own page number. */
async function footers(a) {
  const there = await a.ev(`footerCopy(3, 3)`);
  check(`a footer copies the page number of the caret's page`, there.here === "Page 4 of 4" && same(there.copied, [there.here, there.here]), there);
  const gone = await a.ev(`footerCopy(3, 0)`);
  check(`a footer the caret's page does not show copies its first page's number`, gone.first === "Page ii of 4" && same(gone.copied, [gone.first, gone.first]), gone);
}

export async function run(mode, a, b, fixture) {
  if (fixture.endsWith("sections.docx")) return mode === "local" && footers(a);
  await a.front(b);
  const settle = a.paint;
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  const clearNotice = () => a.ev(`document.getElementById("notice").textContent = ""`);
  const texts = async () => (await a.ev(`exported()`)).texts;
  const other = async (label) => {
    if (!b) return;
    await b.front(a);
    check(`${mode} ${label}: the other tab paints the same fields`, same(await b.ev(`watched()`), await a.ev(`watched()`)), [await b.ev(`watched()`), await a.ev(`watched()`)]);
    const [ea, eb] = [await a.ev(`exported()`), await b.ev(`exported()`)];
    check(`${mode} ${label}: both tabs export the same bytes`, same(ea, eb) && ea.valid, [ea.hash, eb.hash, ea.valid]);
    await a.front(b);
  };
  const original = await a.ev(`exported()`);
  check(`${mode} opened: four pages, with the stale results`, (await a.ev(`pageCount()`)) === 4 && same(original.texts, stale), [await a.ev(`pageCount()`), original.texts]);
  check(`${mode} opened: the date paints as the file has it`, (await a.ev(`line("Printed on")`)) === "Printed on Monday, January 1, 2024", await a.ev(`line("Printed on")`));

  const copied = await a.ev(`(() => {
    const at = startOf("This is page");
    const paragraph = pm().state.doc.resolve(at).parent;
    select(at, at + paragraph.content.size);
    const clipboard = new DataTransfer();
    pm().dom.dispatchEvent(new ClipboardEvent("copy", { clipboardData: clipboard, bubbles: true, cancelable: true }));
    return [clipboard.getData("text/plain"), new DOMParser().parseFromString(clipboard.getData("text/html"), "text/html").body.textContent];
  })()`);
  const displayed = await a.ev(`line("This is page")`);
  check(`${mode} both clipboard formats use the displayed PAGE and NUMPAGES`, displayed === "This is page i of 4." && same(copied, [displayed, displayed]), { displayed, copied });

  // The DATE ends in an invisible field delimiter. Holding its visible edge
  // while a picture loads must put the image after the field.
  const dateEnd = await a.ev(`(() => {
    const start = startOf("Printed on");
    return start + pm().state.doc.resolve(start).parent.content.size;
  })()`);
  await a.ev(`select(${dateEnd}, ${dateEnd})`);
  const picture = await a.ev(`(async () => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 2;
    await docxDemo.surface.picture(await new Promise(resolve => canvas.toBlob(resolve)));
    return pm().state.doc.nodeAt(${dateEnd})?.attrs.node?.type;
  })()`);
  check(`${mode} picture inserted at the visible end of the date field`, picture === "drawing", [picture, await notice()]);
  if (picture === "drawing") await a.ev(`key("z", { metaKey: true })`);
  await settle();

  await a.ev(`select(startOf("Printed on"), ${dateEnd})`);
  await click(a, "#link");
  await a.send("Input.insertText", { text: "example.org" });
  await click(a, "#linkapply"); await settle();
  const draft = await a.ev(`[document.activeElement.id, document.getElementById("linkurl").value, document.getElementById("linkform").hidden]`);
  check(`${mode} link across the field is refused and keeps the address`, (await notice()).includes("part of a field") && same(draft, ["linkurl", "example.org", false]), [await notice(), draft]);
  await a.ev(`document.activeElement.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }))`);
  await clearNotice();

  // Update fields, with the caret in the body.
  const p0 = await a.ev(`start(0)`);
  await a.ev(`select(${p0}, ${p0})`);
  await click(a, "#refreshfields"); await settle();
  const updated = await a.ev(`exported()`);
  check(`${mode} Update fields gives each field its result`, updated.valid && same(updated.texts, fresh), updated.texts);
  check(`${mode} the painted date is today`, (await a.ev(`line("Printed on")`)) === `Printed on ${today}`, await a.ev(`line("Printed on")`));
  check(`${mode} the painted page reference is the Method page`, (await a.ev(`line("The method")`)) === "The method is on page 2.", await a.ev(`line("The method")`));
  const toc = await a.ev(`lines().filter((l) => /^(Introduction|Method|Results)\\D*\\d$/.test(l))`);
  check(`${mode} the table of contents paints the page numbers`, same(toc.map((l) => l.at(-1)), ["1", "2", "3"]), toc);
  check(`${mode} the editor keeps the focus, with no refusal`, (await a.ev(`focused()`)) && (await notice()) === "", [await a.ev(`focused()`), await notice()]);
  await other("updated");

  // Nothing newer: refused with a reason, and nothing changes.
  await click(a, "#refreshfields"); await settle();
  check(`${mode} a second update says there is nothing newer`, (await notice()) === "Fields have no newer results." && (await a.ev(`exported()`)).hash === updated.hash, [await notice()]);
  await clearNotice();

  // Track Changes does not record a field update.
  await a.ev(`toggle()`);
  await click(a, "#refreshfields"); await settle();
  check(`${mode} Track Changes refuses an update, with the reason`, (await notice()).includes("to update fields") && (await a.ev(`exported()`)).hash === updated.hash, await notice());
  await a.ev(`toggle()`);
  await clearNotice();

  // Typing beside a field keeps its result.
  const at = await a.ev(`startOf("The method")`);
  await a.ev(`select(${at}, ${at})`);
  await a.send("Input.insertText", { text: "X" }); await settle();
  const typed = await texts();
  check(`${mode} typing beside a field keeps its result`, typed[8] === "XThe method is on page 2." && same(typed.filter((_, i) => i !== 8), fresh.filter((_, i) => i !== 8)), typed);
  check(`${mode} the painted result stays`, (await a.ev(`line("XThe method")`)) === "XThe method is on page 2.", await a.ev(`line("XThe method")`));
  await other("typed beside");

  // The update undoes on its own, after the typing, and redoes.
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} undo: the typing goes first`, same(await texts(), fresh), await texts());
  await a.ev(`key("z", { metaKey: true })`); await settle();
  const undone = await a.ev(`exported()`);
  check(`${mode} undo: the update is gone`, same(undone.texts, stale) && undone.hash === original.hash, undone.texts);
  check(`${mode} undo: the date paints as the file has it`, (await a.ev(`line("Printed on")`)) === "Printed on Monday, January 1, 2024", await a.ev(`line("Printed on")`));
  await other("undone");
  await a.ev(`key("z", { metaKey: true, shiftKey: true })`); await settle();
  check(`${mode} redo: the results are back`, same(await texts(), fresh), await texts());
  await other("redone");

  if (b) {
    // The other person undoes nothing of ours: their own update, from the stale state, is theirs.
    await a.ev(`key("z", { metaKey: true })`); await settle();
    await b.front(a);
    await click(b, "#refreshfields"); await b.paint();
    check(`${mode} the other tab updates the fields`, same((await b.ev(`exported()`)).texts, fresh), (await b.ev(`exported()`)).texts);
    await a.front(b);
    check(`${mode} this tab shows their update`, same(await texts(), fresh) && (await a.ev(`line("Printed on")`)) === `Printed on ${today}`, await texts());
    await other("the other tab's update");
    await b.front(a);
    await b.ev(`key("z", { metaKey: true })`); await b.paint();
    await a.front(b);
    check(`${mode} their undo takes their update back here`, same(await texts(), stale), await texts());
    await other("the other tab's undo");
  }

  const errors = [...a.errors, ...(b?.errors ?? [])];
  check(`${mode}: no exceptions`, errors.length === 0, errors);
  check(`${mode}: no refusal left`, (await notice()) === "", await notice());
}
