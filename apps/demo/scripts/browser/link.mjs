// Makes, edits, follows and removes links in the browser demo, locally and
// in two shared tabs, with ⌘K, the toolbar button and the link bar, and
// checks the view, the engine, the painted look, the bar, the other tab,
// the export and undo.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs link

import { check, same } from "./harness.mjs";

export const fixture = "generated/simple-paragraphs.docx";

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
  window.opened = []; window.open = (url, name, features) => { opened.push([url, name, features]); return null; };
  window.exported = () => { return docxDemo.engine.validate(docxDemo.session.replica.export()).valid; };
})()`;

const BLUE = "rgb(5, 99, 193)";

export async function run(mode, a, b) {
  await a.front(b);
  const settle = a.paint;
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  const clearNotice = () => a.ev(`document.getElementById("notice").textContent = ""`);
  const startOf = (i) => a.ev(`start(${i})`);
  const keys = async (key, code, vk, modifiers = 0, text) => {
    for (const type of ["keyDown", "keyUp"]) await a.send("Input.dispatchKeyEvent", { type, key, code, windowsVirtualKeyCode: vk, modifiers, ...(type === "keyDown" && text ? { text } : {}) });
  };
  const cmdK = () => keys("k", "KeyK", 75, 4);
  const enter = () => keys("Enter", "Enter", 13, 0, "\r");
  const escape = () => keys("Escape", "Escape", 27);
  const type = (text) => a.send("Input.insertText", { text });
  // A pointer click, which moves focus as a person's does.
  const press = async (id) => {
    const [x, y] = await a.ev(`(() => { const r = document.getElementById(${JSON.stringify(id)}).getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; })()`);
    for (const type of ["mousePressed", "mouseReleased"]) await a.send("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1 });
  };
  const click = (id) => a.ev(`(() => { const b = document.getElementById(${JSON.stringify(id)}); b.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })); b.click(); })()`);
  const expect = async (label, want, { look } = {}) => {
    await settle();
    const view = await a.ev(`links()`), engine = await a.ev(`engineLinks()`);
    check(`${mode} ${label}: view`, same(view, want), { view, want });
    check(`${mode} ${label}: engine`, same(engine, want), { engine, want });
    if (look) for (const [word, wanted] of Object.entries(look)) {
      const got = await a.ev(`look(${JSON.stringify(word)})`);
      check(`${mode} ${label}: ${word} painted`, got.length > 0 && got.every((g) => wanted ? g === `${BLUE} u` : g !== `${BLUE} u`), got);
    }
    if (b) {
      await b.front(a);
      const other = await b.ev(`links()`);
      check(`${mode} ${label}: other tab`, same(other, want), other);
      await a.front(b);
    }
  };
  const original = await a.ev(`texts()`);
  const s0 = await startOf(0), s1 = await startOf(1);

  // ⌘K over a selection opens the address field; Enter links it.
  await a.ev(`select(${s0}, ${s0 + 5})`); await settle();
  await cmdK(); await a.paint();
  let bar = await a.ev(`bar()`);
  check(`${mode} ⌘K: address field`, bar.shown && bar.form && !bar.view && bar.focus === "linkurl" && bar.value === "", bar);
  check(`${mode} ⌘K: bar below the caret`, await a.ev(`below()`), null);
  await type("example.com"); await enter();
  await expect("⌘K, example.com, Enter", [["First", "https://example.com"]], { look: { First: true, paragraph: false } });
  check(`${mode} linked: editor focused`, await a.ev(`pm().hasFocus()`), await a.ev(`document.activeElement?.id`));
  check(`${mode} linked: no refusal`, (await notice()) === "", await notice());
  if (mode === "local") check(`${mode} linked: export validates`, (await a.ev(`exported()`)) === true, await a.ev(`exported()`));

  // A caret in the link shows where it points; outside it, nothing.
  await a.ev(`select(${s0 + 2}, ${s0 + 2})`); await settle();
  bar = await a.ev(`bar()`);
  check(`${mode} caret in the link: bar`, bar.shown && bar.view && !bar.form && bar.text === "https://example.com" && bar.href === "https://example.com/", bar);
  check(`${mode} caret in the link: button pressed`, (await a.ev(`pressedLink()`)) === "true", await a.ev(`pressedLink()`));
  await a.ev(`select(${s1 + 2}, ${s1 + 2})`); await settle();
  bar = await a.ev(`bar()`);
  check(`${mode} caret outside: no bar`, !bar.shown && (await a.ev(`pressedLink()`)) === "false", bar);

  // Edit changes the whole link from a caret in it.
  await a.ev(`select(${s0 + 1}, ${s0 + 1})`); await settle();
  await press("linkedit"); await a.paint();
  bar = await a.ev(`bar()`);
  check(`${mode} Edit: address field filled`, bar.form && bar.value === "https://example.com" && bar.focus === "linkurl", bar);
  await a.ev(`document.getElementById("linkurl").select()`);
  await type("someone@example.com");
  await press("linkapply");
  await expect("Edit to an email address", [["First", "mailto:someone@example.com"]]);

  // ⌘K over part of a link changes the whole link, and undo puts it back.
  await a.ev(`select(${s0 + 1}, ${s0 + 3})`); await settle();
  await cmdK(); await a.paint();
  bar = await a.ev(`bar()`);
  check(`${mode} ⌘K inside a link: address filled`, bar.form && bar.value === "mailto:someone@example.com", bar);
  await type("example.org"); await enter();
  await expect("⌘K inside a link, example.org", [["First", "https://example.org"]]);
  await a.ev(`key("z", { metaKey: true })`);
  await expect("⌘K inside a link undone", [["First", "mailto:someone@example.com"]]);

  // ⌘-click opens it in a new tab, without moving the caret; a document's other links never open.
  await a.ev(`select(${s1 + 1}, ${s1 + 1})`); await settle();
  const at = await a.ev(`(() => { const s = [...document.querySelectorAll(".pages .line span")].find((s) => s.children.length === 0 && s.textContent.includes("First")); const r = s.getBoundingClientRect(); return [r.left + 4, r.top + r.height / 2]; })()`);
  for (const type of ["mousePressed", "mouseReleased"]) await a.send("Input.dispatchMouseEvent", { type, x: at[0], y: at[1], button: "left", clickCount: 1, modifiers: 4 });
  await a.paint();
  check(`${mode} ⌘-click: opened`, same(await a.ev(`opened`), [["mailto:someone@example.com", "_blank", "noopener,noreferrer"]]), await a.ev(`opened`));
  check(`${mode} ⌘-click: caret kept`, (await a.ev(`pm().state.selection.from`)) === s1 + 1, await a.ev(`pm().state.selection.from`));
  await a.ev(`docxDemo.surface.onFollow("javascript:alert(1)")`);
  check(`${mode} javascript: never opens`, (await a.ev(`opened.length`)) === 1 && (await notice()) !== "", [await a.ev(`opened`), await notice()]);
  await clearNotice();

  // An address that is no web or email address is refused, and Escape returns to the text.
  await a.ev(`select(${s1}, ${s1 + 6})`); await settle();
  await cmdK(); await a.paint();
  await type("javascript:alert(1)"); await enter(); await a.paint();
  bar = await a.ev(`bar()`);
  check(`${mode} javascript: refused`, (await notice()) !== "" && bar.form, [await notice(), bar]);
  await escape(); await a.paint();
  bar = await a.ev(`bar()`);
  check(`${mode} Escape: closed and back to the text`, !bar.form && (await a.ev(`pm().hasFocus()`)), bar);
  await expect("after the refusal", [["First", "mailto:someone@example.com"]]);
  await clearNotice();

  // The address is for the text the field was opened on, even after someone else removes that text.
  if (b) {
    await a.ev(`select(${s0 + 5}, ${s0 + 16})`); await settle();
    await cmdK(); await a.paint();
    await type("example.net");
    await b.ev(`(() => { select(${s0 + 5}, ${s0 + 16}); key("Backspace"); })()`);
    await a.wait("the other tab's deletion", `texts()[0] === "First"`);
    await enter(); await a.paint();
    bar = await a.ev(`bar()`);
    check(`${mode} linked text removed meanwhile: refused, address kept`, (await notice()) !== "" && bar.form && bar.value === "example.net", [await notice(), bar]);
    check(`${mode} linked text removed meanwhile: neighbouring link unchanged`, same(await a.ev(`links()`), [["First", "mailto:someone@example.com"]]), await a.ev(`links()`));
    await escape(); await a.paint();
    await b.ev(`key("z", { metaKey: true })`);
    await a.wait("the other tab's undo", `texts()[0] === ${JSON.stringify(original[0])}`);
    await clearNotice();
  }

  // ⌘K with nothing selected outside a link says what to do.
  await a.ev(`select(${s1 + 2}, ${s1 + 2})`); await settle();
  await cmdK(); await a.paint();
  check(`${mode} ⌘K at a bare caret: refused`, (await notice()) === "Select the text to link first." && !(await a.ev(`bar()`)).shown, await notice());
  await clearNotice();

  // The toolbar button over two paragraphs links each to a bookmark.
  await a.ev(`select(${s0 + 6}, ${s1 + 6})`); await settle();
  await click("link"); await a.paint();
  check(`${mode} button: address field`, (await a.ev(`bar()`)).form, await a.ev(`bar()`));
  await type("#_Toc1"); await enter();
  await expect("button over two paragraphs", [["First", "mailto:someone@example.com"], ["paragraph.", "#_Toc1"], ["Second", "#_Toc1"]]);

  // Remove takes out the whole link the caret is in.
  await a.ev(`select(${s0 + 2}, ${s0 + 2})`); await settle();
  await press("linkremove");
  await expect("Remove", [["paragraph.", "#_Toc1"], ["Second", "#_Toc1"]], { look: { First: false } });
  if (mode === "local") check(`${mode} links: export validates`, (await a.ev(`exported()`)) === true, await a.ev(`exported()`));

  for (let i = 0; i < 4; i++) { await a.ev(`key("z", { metaKey: true })`); await settle(); }
  await expect("all undone", []);
  const back = await a.ev(`texts()`);
  check(`${mode} all undone: text`, same(back, original), back);
  check(`${mode}: no refusal left`, (await notice()) === "", await notice());
  if (b) {
    // The address is for the three selected letters even if a peer links the enclosing word.
    await a.ev(`select(${s0 + 1}, ${s0 + 4})`);
    await cmdK(); await a.paint();
    await type("inner.example");
    await b.ev(`select(${s0}, ${s0 + 5}); docxDemo.surface.link("https://outer.example")`);
    await a.wait("the peer's enclosing link", `links()[0]?.[1] === "https://outer.example"`);
    await enter();
    await expect("submit over the original letters", [["F", "https://outer.example"], ["irs", "https://inner.example"], ["t", "https://outer.example"]]);
  }

  const errors = [...a.errors, ...(b?.errors ?? [])];
  check(`${mode}: no exceptions`, errors.length === 0, errors);
}
