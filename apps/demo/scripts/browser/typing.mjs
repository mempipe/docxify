// Types and deletes among equal letters in the browser demo, alone and in two
// shared tabs, and checks that the editor names the place of each change:
// the letters before and after cannot tell which one was typed.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs typing

import { check, same } from "./harness.mjs";

export const windowSize = "1100,650";
export const fixture = "generated/simple-paragraphs.docx";

export const helpers = `(() => {
  window.texts = () => { const out = []; pm().state.doc.forEach((n) => out.push(n.type.name === "paragraph" ? n.textContent : "[" + n.type.name + "]")); return out; };
  // Every text change the editor sends, without the node it names.
  window.sent = [];
  const replica = docxDemo.session.replica, apply = replica.apply;
  replica.apply = (commands) => { for (const c of commands) if (c.command === "replace_range") sent.push({ range: { start: c.from.offset, end: c.to.offset }, text: c.text }); return apply(commands); };
  window.paste = (text) => { const dt = new DataTransfer(); dt.setData("text/plain", text); pm().dom.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true })); };
})()`;

export async function run(mode, a, b) {
  await a.front(b);
  const settle = a.paint;
  const first = (await a.ev(`texts()`))[0];
  // Between the "p" and the "a" of "paragraph": a typed "a" joins an "a".
  const at = first.indexOf("pa") + 1;
  check(`${mode} the fixture has a "pa"`, at > 0, first);
  const press = async (key, code, vk, text) => {
    for (const type of ["keyDown", "keyUp"]) await a.send("Input.dispatchKeyEvent", { type, key, code, text: type === "keyDown" ? text : undefined, windowsVirtualKeyCode: vk });
  };
  const last = () => a.ev(`sent.at(-1)`);

  if (b) {
    // Start before either author has any other change to undo.
    const row = 2;
    for (const [tracking, text] of [[false, "abcdX"], [true, "abcdX"], [false, "abcdef"]]) {
      await a.front(b);
      const prior = (await a.ev("texts()"))[row];
      await a.ev(`select(start(${row}), start(${row}))`);
      await a.send("Input.insertText", { text: "abcdef" });
      await b.front(a);
      if (tracking) await b.ev('document.querySelector("#track").click()');
      await b.ev(`select(start(${row}), start(${row}) + 6)`);
      await b.send("Input.insertText", { text });
      const sent = await b.ev("sent.at(-1)");
      check(`whole replacement ${text} (tracking ${tracking})`, sent.text === text && sent.range.end - sent.range.start === 6, sent);
      await a.front(b);
      await a.ev('key("z", { metaKey: true })');
      await b.front(a);
      const kept = [(await a.ev("texts()"))[row], (await b.ev("texts()"))[row]];
      check(`replacement ${text} survives the first author's undo (tracking ${tracking})`, kept.every(t => t === text + prior), kept);
      if (tracking) await b.ev('document.querySelector("#track").click()');
    }
  }

  await a.front(b);
  await a.ev(`select(start(0) + ${at}, start(0) + ${at})`);
  await press("a", "KeyA", 65, "a");
  await settle();
  const typed = first.slice(0, at) + "a" + first.slice(at);
  check(`${mode} typed: view`, (await a.ev(`texts()`))[0] === typed, await a.ev(`texts()`));
  check(`${mode} typed: sent at the caret`, same(await last(), { range: { start: at, end: at }, text: "a" }), await last());

  await press("Backspace", "Backspace", 8);
  await settle();
  check(`${mode} Backspace: view`, (await a.ev(`texts()`))[0] === first, await a.ev(`texts()`));
  check(`${mode} Backspace: sent at the caret`, same(await last(), { range: { start: at, end: at + 1 }, text: "" }), await last());

  // Delete takes the "a" after the caret, and leaves the caret where it was.
  await press("Delete", "Delete", 46);
  await settle();
  const deleted = first.slice(0, at) + first.slice(at + 1);
  check(`${mode} Delete: view`, (await a.ev(`texts()`))[0] === deleted, await a.ev(`texts()`));
  check(`${mode} Delete: sent at the caret`, same(await last(), { range: { start: at, end: at + 1 }, text: "" }), await last());

  if (b) {
    await b.front(a);
    check(`${mode} the other tab shows the same text`, (await b.ev(`texts()`))[0] === deleted, await b.ev(`texts()`));
  }
  await a.front(b);
  await a.ev(`select(start(0) + 2, start(0) + 5); key("b", { metaKey: true }); select(start(0), start(0) + 8)`);
  await a.send("Input.insertText", { text: "X" });
  await a.paint();
  check(`${mode} typing replaces a selection spanning styled runs`, (await a.ev(`texts()`))[0] === "X" + deleted.slice(8));

  if (b) {
    await a.ev(`select(start(0), start(0) + pm().state.doc.child(0).content.size)`);
    await a.send("Input.insertText", { text: "aaaa" });
    await b.front(a);
    await b.ev(`select(start(0) + 2, start(0) + 2)`);
    await a.front(b);
    await a.ev(`select(start(0), start(0))`);
    await a.send("Input.insertText", { text: "a" });
    await b.front(a);
    check("a peer's caret stays beside its original letter", await b.ev(`pm().state.selection.head === start(0) + 3`));
    await b.send("Input.insertText", { text: "X" });
    await b.paint();
    check("typing at the restored caret", (await b.ev(`texts()`))[0] === "aaaXaa", await b.ev(`texts()`));
    // Keep the caret above the viewport while the other author splits a paragraph.
    await b.ev(`select(start(0), start(0)); window.scrollTo(0, document.documentElement.scrollHeight)`);
    await b.paint();
    const scroll = await b.ev(`window.scrollY`);
    await a.front(b);
    await a.ev(`select(start(1) + 2, start(1) + 2); key("Enter")`);
    await b.front(a);
    check("a remote split preserves this tab's scroll position", scroll > 0 && await b.ev(`window.scrollY`) === scroll, [scroll, await b.ev(`window.scrollY`)]);

  }
  // Text that replaces a selection is sent whole, though it shares letters with what it replaces.
  await a.front(b);
  const row = (await a.ev(`texts()`)).findIndex((text, index) => index > 0 && text.length > 5);
  const second = (await a.ev(`texts()`))[row];
  const whole = async () => { const { range, text } = await last(); return { length: range.end - range.start, text }; };
  await a.ev(`select(start(${row}), start(${row}) + 3); paste(${JSON.stringify(second.slice(0, 2) + "X")})`);
  await settle();
  check(`${mode} pasted over a selection: view`, (await a.ev(`texts()`))[row] === second.slice(0, 2) + "X" + second.slice(3), await a.ev(`texts()`));
  check(`${mode} pasted over a selection: sent whole`, same(await whole(), { length: 3, text: second.slice(0, 2) + "X" }), await last());
  await a.ev(`select(start(${row}), start(${row}) + 3)`);
  await a.send("Input.insertText", { text: second[0] });
  await settle();
  check(`${mode} typed over a selection: view`, (await a.ev(`texts()`))[row] === second[0] + second.slice(3), await a.ev(`texts()`));
  check(`${mode} typed over a selection: sent whole`, same(await whole(), { length: 3, text: second[0] }), await last());
  if (b) {
    // One person types letters, another pastes over them, and the first takes the typing back.
    await a.ev(`select(start(${row}), start(${row}))`);
    await a.send("Input.insertText", { text: "abc" });
    await settle();
    await b.front(a);
    await b.wait("the typed letters", `texts()[${row}].startsWith("abc")`);
    await b.ev(`select(start(${row}), start(${row}) + 3); paste("abX")`);
    await b.paint();
    await a.front(b);
    await a.wait("the paste", `texts()[${row}].startsWith("abX")`);
    await a.ev(`key("z", { metaKey: true })`);
    await settle();
    await b.front(a);
    await b.paint();
    const kept = [(await a.ev(`texts()`))[row].slice(0, 4), (await b.ev(`texts()`))[row].slice(0, 4)];
    check("a paste over someone's letters outlasts their undo", same(kept, ["abX" + second[0], "abX" + second[0]]), kept);
  }

  // Undo shows what it took back, though the window was scrolled away from it.
  await a.front(b);
  await a.ev(`select(start(0), start(0))`);
  await a.send("Input.insertText", { text: "Q" });
  await a.paint();
  await a.ev(`select(start(2), start(2)); window.scrollTo(0, document.documentElement.scrollHeight)`);
  await a.paint();
  const away = await a.ev(`window.scrollY`);
  await a.ev(`key("z", { metaKey: true })`);
  await a.paint();
  const back = await a.ev(`[window.scrollY, pm().state.selection.head, start(0)]`);
  check(`${mode} undo scrolls to what it took back`, away > 0 && back[0] < away && back[1] === back[2], [away, back]);
  // Backspace takes what the browser's own does, with Track Changes off or on:
  // an emoji with its skin tone, and an accent without its letter.
  await a.front(b);
  for (const [name, text, left] of [["an emoji with a skin tone", "\u{1F44D}\u{1F3FD}", ""], ["an accent", "e\u0301", "e"]]) {
    const kept = [];
    for (const tracking of [false, true]) {
      const line = (await a.ev(`texts()`))[0];
      await a.ev(`select(start(0), start(0))`);
      await a.send("Input.insertText", { text });
      await settle();
      if (tracking) await a.ev(`document.getElementById("track").click()`);
      await press("Backspace", "Backspace", 8);
      await settle();
      const after = (await a.ev(`texts()`))[0];
      kept.push(after.slice(0, after.length - line.length));
      if (tracking) await a.ev(`document.getElementById("track").click()`);
    }
    check(`${mode} Backspace after ${name} takes the same, tracked or not`, same(kept, [left, left]), kept);
  }
  for (const [key, vk, left] of [["Backspace", 8, "👍"], ["Delete", 46, "🏽"]]) {
    for (const tracking of [false, true]) {
      await a.ev('select(start(0), start(0)); paste("👍🏽"); select(start(0) + 2, start(0) + 4); key("b", { metaKey: true })');
      const pos = key === "Backspace" ? 4 : 0;
      await a.ev(`select(start(0) + ${pos}, start(0) + ${pos})`);
      const before = (await a.ev("texts()"))[0];
      if (tracking) await a.ev('document.querySelector("#track").click()');
      await press(key, key, vk);
      await settle();
      const after = (await a.ev("texts()"))[0];
      check(`${mode} ${key} at a split emoji (tracking ${tracking})`, after === left + before.slice(4), after);
      if (tracking) await a.ev('document.querySelector("#track").click()');
    }
  }
  check(`${mode} no page errors`, a.errors.length === 0, a.errors);
}
