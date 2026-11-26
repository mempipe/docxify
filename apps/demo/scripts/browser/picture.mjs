// Inserts pictures with the Picture path and by pasting an image in the
// browser demo, locally and in two shared tabs, and checks the view, the
// engine, the painted page, the other tab, the export and undo.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs picture

import { check, same } from "./harness.mjs";

export const fixture = "generated/simple-paragraphs.docx";

export const helpers = `(() => {
  window.texts = () => { const out = []; pm().state.doc.forEach((n) => out.push(n.type.name === "paragraph" ? n.textContent : "[" + n.type.name + "]")); return out; };
  // Each drawing atom of the view: its position and the engine's drawing info.
  window.drawings = () => { const out = []; pm().state.doc.descendants((n, pos) => { if (n.type.name === "inline_atom" && n.attrs.node.type === "drawing") out.push([pos, n.attrs.id]); }); return out; };
  window.engineDrawings = () => { const out = []; const walk = (n) => { if (n.type === "drawing") out.push({ w: n.width_emu, h: n.height_emu, part: n.media?.[0]?.part, type: n.media?.[0]?.content_type, id: n.drawing_id }); n.children?.forEach(walk); };
    pm().state.doc.forEach((p) => p.attrs.id && walk(docxDemo.session.inspectNode(p.attrs.id))); return out; };
  // Painted pictures, once each has loaded, as their natural width.
  // ProseMirror puts a separator image between adjacent atoms in the hidden editor; it is not painted.
  window.painted = async () => { const imgs = [...document.querySelectorAll(".pages img")].filter((i) => !i.closest(".input")); await Promise.all(imgs.map((i) => i.decode().catch(() => null))); return imgs.map((i) => i.naturalWidth); };
  window.png = async (w, h, type = "image/png") => { const c = new OffscreenCanvas(w, h); const g = c.getContext("2d"); g.fillStyle = "#3a7"; g.fillRect(0, 0, w, h); g.fillStyle = "#c33"; g.fillRect(0, 0, w / 2, h / 2); return c.convertToBlob({ type }); };
  window.textWidth = () => { const p = docxDemo.surface.pages[0]; return (p.width - p.margins.left - p.margins.right) * 12700; };
  window.paste = async (w, h, text) => { const blob = await png(w, h); const dt = new DataTransfer(); dt.items.add(new File([blob], "pasted.png", { type: "image/png" })); if (text) dt.setData("text/plain", text);
    pm().dom.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true })); };
  // Hands an image over as the Picture button's file picker does.
  window.choose = async (w, h) => { const dt = new DataTransfer(); dt.items.add(new File([await png(w, h)], "chosen.png", { type: "image/png" }));
    const input = document.getElementById("pictures"); input.files = dt.files; input.dispatchEvent(new Event("change")); };
  // Keeps every image from being read until the function it returns is called.
  window.holdReads = () => { const read = Blob.prototype.arrayBuffer; let go; const held = new Promise((r) => (go = r)); Blob.prototype.arrayBuffer = function () { return held.then(() => read.call(this)); };
    return () => { Blob.prototype.arrayBuffer = read; go(); }; };
  window.exportBytes = () => docxDemo.session.replica.export();
  window.exported = () => { const bytes = exportBytes(); const valid = docxDemo.engine.validate(bytes).valid; const re = docxDemo.engine.open(bytes);
    const out = []; const walk = (n) => { if (n.type === "drawing") out.push([n.width_emu, n.height_emu, n.media?.[0]?.part]); n.children?.forEach(walk); };
    walk(re.inspect().stories[0]); const hash = [...bytes].reduce((h, b) => (h * 31 + b) >>> 0, 7); re.free(); return { valid, drawings: out, hash, length: bytes.length }; };
})()`;

export async function run(mode, a, b) {
  await a.front(b);
  const settle = a.paint;
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  const clearNotice = () => a.ev(`document.getElementById("notice").textContent = ""`);
  const other = async (label, count) => {
    if (!b) return;
    await b.front(a);
    const drawn = await b.ev(`drawings().length`), shown = await b.ev(`painted()`);
    check(`${mode} ${label}: other tab's view`, drawn === count, drawn);
    check(`${mode} ${label}: other tab paints them`, shown.length === count && shown.every((w) => w > 0), shown);
    const [ea, eb] = [await a.ev(`exported()`), await b.ev(`exported()`)];
    check(`${mode} ${label}: both tabs export the same bytes`, same(ea, eb) && ea.valid, [ea, eb]);
    await a.front(b);
  };
  const refused = await a.ev(`(async () => {
    const r = docxDemo.session.replica;
    const before = [...r.snapshot()];
    const bytes = new Uint8Array(await (await png(2, 2)).arrayBuffer());
    const media = r.mediaName(bytes);
    const p = r.inspect().stories[0].children.find(n => n.type === "paragraph").id;
    let failed = false;
    try { r.apply([{command:"insert_picture",at:{start:p},media,width_emu:0,height_emu:914400}], bytes); }
    catch { failed = true; }
    return {failed, unchanged: JSON.stringify([...r.snapshot()]) === JSON.stringify(before)};
  })()`);
  check(`${mode} refused image insertion retains no shared bytes`, refused.failed && refused.unchanged, refused);
  const original = await a.ev(`texts()`);
  check(`${mode} opened with no pictures`, (await a.ev(`drawings().length`)) === 0, await a.ev(`drawings()`));

  // Typing changes projected node addresses. Media preparation must keep
  // the current address space until it has resolved the picture's target.
  await a.ev(`select(start(0), start(0)); pm().focus()`);
  await a.send("Input.insertText", { text: "Typed " });
  await settle();
  const typed = await a.ev(`texts()`);
  const typedAt = (await a.ev(`start(0)`)) + 3;
  await a.ev(`select(${typedAt}, ${typedAt})`);
  await a.ev(`png(3, 3).then((b) => docxDemo.surface.picture(b))`);
  await settle();
  check(`${mode} picture after typing resolves the current text node`, same(await a.ev(`drawings().map(d => d[0])`), [typedAt]), await notice());
  check(`${mode} picture after typing keeps the typed text`, same(await a.ev(`texts()`), typed), await a.ev(`texts()`));
  await other("picture after typing", 1);
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} picture after typing undoes independently`, same(await a.ev(`texts()`), typed) && (await a.ev(`drawings().length`)) === 0);
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} typing undo restores the original`, same(await a.ev(`texts()`), original));

  // A picture wider than the page is fitted between its margins.
  const at = (await a.ev(`start(0)`)) + 5;
  await a.ev(`select(${at}, ${at})`);
  // A file picker takes focus from the editor.
  await a.ev(`pm().dom.blur()`);
  await a.ev(`choose(1600, 400)`);
  await a.wait("the chosen picture", `drawings().length === 1`);
  await settle();
  let view = await a.ev(`drawings()`), engine = await a.ev(`engineDrawings()`), width = await a.ev(`textWidth()`);
  check(`${mode} wide picture: in the view at the caret`, view.length === 1 && view[0][0] === at, view);
  check(`${mode} wide picture: fitted to the text width, same shape`, engine.length === 1 && Math.abs(engine[0].w - width) <= 1 && Math.abs(engine[0].w - 4 * engine[0].h) <= 4, [engine, width]);
  check(`${mode} wide picture: added media`, /media\/image-[0-9a-f]{16}\.png$/.test(engine[0]?.part) && engine[0]?.type === "image/png", engine);
  check(`${mode} wide picture: text unchanged`, same(await a.ev(`texts()`), original), await a.ev(`texts()`));
  check(`${mode} wide picture: caret after it`, (await a.ev(`pm().state.selection.head`)) === at + 1, await a.ev(`pm().state.selection.head`));
  check(`${mode} wide picture: painted`, same(await a.ev(`painted()`), [1600]), await a.ev(`painted()`));
  check(`${mode} wide picture: editor keeps focus`, await a.ev(`pm().hasFocus()`), null);
  let out = await a.ev(`exported()`);
  check(`${mode} wide picture: export validates and reopens with it`, out.valid && out.drawings.length === 1 && out.drawings[0][2] === engine[0].part, out);
  await other("wide picture", 1);

  // A pasted image at the caret, at 96 dpi.
  await a.ev(`paste(40, 30)`);
  await settle();
  engine = await a.ev(`engineDrawings()`);
  check(`${mode} pasted image: at its size`, engine.length === 2 && engine[1].w === 381000 && engine[1].h === 285750, engine);
  check(`${mode} pasted image: its own drawing ID`, engine.every((d) => Number.isInteger(d.id) && d.id > 0) && new Set(engine.map((d) => d.id)).size === 2, engine);
  check(`${mode} pasted image: painted`, same(await a.ev(`painted()`), [1600, 40]), await a.ev(`painted()`));
  await other("pasted image", 2);

  // Text pasted with a picture of it, as Word copies, is pasted as text.
  await a.ev(`paste(40, 30, "Pasted")`);
  await settle();
  check(`${mode} text with an image: pasted as text`, (await a.ev(`texts()`))[0].includes("Pasted") && (await a.ev(`drawings().length`)) === 2, [await a.ev(`texts()`), await a.ev(`drawings()`)]);
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} text with an image: undone`, same(await a.ev(`texts()`), original) && (await a.ev(`drawings().length`)) === 2, await a.ev(`texts()`));

  // In place of a selection, as one undo step.
  const second = await a.ev(`start(1)`);
  const word = (await a.ev(`texts()`))[1].slice(0, 3);
  await a.ev(`select(${second}, ${second + 3})`);
  await a.ev(`png(40, 30).then((b) => docxDemo.surface.picture(b))`);
  await settle();
  check(`${mode} over a selection: the text goes`, (await a.ev(`texts()`))[1] === original[1].slice(3), (await a.ev(`texts()`))[1]);
  check(`${mode} over a selection: the picture is there`, (await a.ev(`drawings()`)).some(([pos]) => pos === second), await a.ev(`drawings()`));
  check(`${mode} over a selection: same media, written once`, (await a.ev(`exported()`)).valid && new Set((await a.ev(`engineDrawings()`)).map((d) => d.part)).size === 2, await a.ev(`engineDrawings()`));
  await other("over a selection", 3);
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} over a selection undone: text back`, same(await a.ev(`texts()`), original) && (await a.ev(`texts()`))[1].startsWith(word), await a.ev(`texts()`));
  check(`${mode} over a selection undone: picture gone`, (await a.ev(`drawings().length`)) === 2, await a.ev(`drawings()`));

  // Keep the original destination while either input route decodes the image.
  const first = await a.ev(`start(0)`);
  const [moved] = original[1].match(/\S+/);
  const movedAt = second + original[1].indexOf(moved);
  for (const source of ["paste", "picker"]) {
    await a.ev(`select(${first}, ${first}); window.release = holdReads()`);
    if (source === "paste") await a.ev(`paste(40, 30)`);
    else await a.ev(`choose(40, 30)`);
    await a.ev(`select(${movedAt}, ${movedAt + moved.length}); release()`);
    await a.wait("the picture read before the selection moved", `drawings().length === 3`);
    await settle();
    view = await a.ev(`drawings()`);
    check(`${mode} ${source}: picture at its original destination`, view.length === 3 && view[0][0] === first, view);
    check(`${mode} ${source}: the newly selected word is intact`, same(await a.ev(`texts()`), original), await a.ev(`texts()`));
    const kept = await a.ev(`[pm().state.selection.from, pm().state.selection.to]`);
    check(`${mode} ${source}: the word stays selected`, same(kept, [movedAt + 1, movedAt + 1 + moved.length]), kept);
    await other(source, 3);
    await a.ev(`key("z", { metaKey: true })`); await settle();
    check(`${mode} ${source}: one undo removes the picture`, same(await a.ev(`texts()`), original) && (await a.ev(`drawings().length`)) === 2);
  }

  // A picture that finishes later leaves the keyboard where the user has moved it since.
  const press = async (key, code, windowsVirtualKeyCode, modifiers = 0) => {
    for (const type of ["keyDown", "keyUp"]) await a.send("Input.dispatchKeyEvent", { type, key, code, windowsVirtualKeyCode, modifiers });
  };
  await a.ev(`select(${first}, ${first}); window.release = holdReads()`);
  await a.ev(`paste(40, 30)`);
  await a.ev(`select(${movedAt}, ${movedAt + moved.length})`);
  await press("k", "KeyK", 75, 4);
  await a.wait("the address field", `document.activeElement?.id === "linkurl"`);
  await a.ev(`release()`);
  await a.wait("the picture read while the address is typed", `drawings().length === 3`);
  await settle();
  await a.send("Input.insertText", { text: "XYZ" });
  const address = await a.ev(`({ focus: document.activeElement?.id, value: document.getElementById("linkurl").value })`);
  check(`${mode} a finished picture leaves typing in the address field`, address.focus === "linkurl" && address.value === "XYZ" && same(await a.ev(`texts()`), original), [address, await a.ev(`texts()`)]);
  await press("Escape", "Escape", 27);
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} a finished picture: one undo removes it`, same(await a.ev(`texts()`), original) && (await a.ev(`drawings().length`)) === 2, [await a.ev(`texts()`), await a.ev(`drawings()`)]);

  // What the engine cannot hold is refused, and nothing changes.
  await clearNotice();
  await a.ev(`png(20, 20, "image/webp").then((b) => docxDemo.surface.picture(b))`); await settle();
  check(`${mode} WebP refused`, (await notice()).includes("PNG, JPEG or GIF") && (await a.ev(`drawings().length`)) === 2, await notice());
  await clearNotice();
  await a.ev(`docxDemo.surface.picture(new Blob(["not an image"], { type: "image/png" }))`); await settle();
  check(`${mode} unreadable image refused`, (await notice()).includes("could not be read") && (await a.ev(`drawings().length`)) === 2, await notice());
  await clearNotice();

  for (let i = 0; i < 3; i++) { await a.ev(`key("z", { metaKey: true })`); await settle(); }
  check(`${mode} all undone`, (await a.ev(`drawings().length`)) === 0 && same(await a.ev(`texts()`), original), [await a.ev(`drawings()`), await a.ev(`texts()`)]);
  check(`${mode} all undone: nothing painted`, (await a.ev(`painted()`)).length === 0, await a.ev(`painted()`));
  out = await a.ev(`exported()`);
  check(`${mode} all undone: export has no picture`, out.valid && out.drawings.length === 0, out);
  await other("all undone", 0);
  if (mode === "local") {
    await a.ev(`(async () => {
      const image = await png(40, 30);
      const bytes = await image.arrayBuffer();
      image.arrayBuffer = () => new Promise(resolve => window.finishRead = () => resolve(bytes));
      window.pendingPicture = docxDemo.surface.picture(image);
      const files = new DataTransfer();
      files.items.add(new File([exportBytes()], "replacement.docx"));
      const picker = document.querySelector("#file");
      picker.files = files.files;
      picker.dispatchEvent(new Event("change"));
    })()`);
    await a.wait("replacement document ready", '!document.querySelector("#file").disabled');
    await a.ev("finishRead(); pendingPicture");
    check("a picture read from the closed document leaves the replacement alone", (await a.ev("drawings().length")) === 0);
  }
  check(`${mode}: no refusal left`, (await notice()) === "", await notice());
  const errors = [...a.errors, ...(b?.errors ?? [])];
  check(`${mode}: no exceptions`, errors.length === 0, errors);
}
