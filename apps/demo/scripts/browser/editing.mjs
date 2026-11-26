// Semantic editing and relative-caret regressions on the default replica in the
// browser demo, alone and in two shared tabs: typing, Backspace, Enter,
// joining, bold and undo. On this path a command is written to the shared
// state as what it means there, and the view is built with no command.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs editing

import { check, click, waitForSharedCaret } from "./harness.mjs";

export const fixture = "generated/simple-paragraphs.docx";

export const helpers = `(() => {
  window.texts = () => { const out = []; pm().state.doc.forEach((n) => out.push(n.type.name === "paragraph" ? n.textContent : "[" + n.type.name + "]")); return out; };
  window.center = (selector) => { const e = document.querySelector(selector); e.scrollIntoView({ block: "center", inline: "center" }); const r = e.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; };
  // The characters of paragraph i that the view shows bold.
  window.bold = (i) => { const p = pm().state.doc.child(i); let out = "";
    p.forEach((c) => { if (c.isText && c.marks.find((m) => m.type.name === "run")?.attrs.info.effective?.bold) out += c.text; });
    return out; };
})()`;

export async function run(mode, a, b) {
  await a.front(b);
  const settle = a.paint;
  const press = async (key, code, vk, text) => {
    for (const type of ["keyDown", "keyUp"]) await a.send("Input.dispatchKeyEvent", { type, key, code, text: type === "keyDown" ? text : undefined, windowsVirtualKeyCode: vk });
  };
  const texts = (tab = a) => tab.ev(`texts()`);
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  // The migration is now the default and includes field settings too.
  const settings = await a.ev(`(() => { const s=docxDemo.session; const before=s.settings().update_fields; s.apply([{command:"set_update_fields",on:!before}]); const changed=s.settings().update_fields===!before; s.undo(); return changed&&s.settings().update_fields===before; })()`);
  check(`${mode} the default replica applies and undoes field settings`, settings);
  await settle();
  const before = await texts();
  const [first, second] = before;

  await a.ev(`select(start(0) + 5, start(0) + 5)`);
  await press("x", "KeyX", 88, "x");
  await settle();
  const typed = first.slice(0, 5) + "x" + first.slice(5);
  check(`${mode} typed`, (await texts())[0] === typed, await texts());

  await press("Backspace", "Backspace", 8);
  await settle();
  check(`${mode} Backspace`, (await texts())[0] === first, await texts());

  await press("Enter", "Enter", 13, "\r");
  await settle();
  let now = await texts();
  check(`${mode} Enter`, now[0] === first.slice(0, 5) && now[1] === first.slice(5) && now.length === before.length + 1, now);

  await press("Backspace", "Backspace", 8);
  await settle();
  check(`${mode} Backspace at a paragraph's start joins`, JSON.stringify(await texts()) === JSON.stringify(before), await texts());

  await a.ev(`select(start(1) + 0, start(1) + 6)`);
  await click(a, "#bold");
  await settle();
  check(`${mode} bold`, (await a.ev(`bold(1)`)) === second.slice(0, 6), await a.ev(`bold(1)`));
  check(`${mode} nothing was refused`, (await notice()) === "", await notice());

  if (b) {
    await b.front(a);
    await b.wait("a named remote caret", `document.querySelector(".pages .caret:not(.local) .name") !== null`);
    const names = await b.ev(`Array.from(document.querySelectorAll(".pages .caret:not(.local) .name"), n => n.textContent)`);
    check(`${mode} the other tab shows a named remote caret`, names.length > 0, names);
    check(`${mode} the other tab shows the same text`, JSON.stringify(await texts(b)) === JSON.stringify(before), await texts(b));
    check(`${mode} the other tab shows the bold`, (await b.ev(`bold(1)`)) === second.slice(0, 6), await b.ev(`bold(1)`));
    // The other person types too, and it comes back.
    await b.ev(`select(start(2) + 0, start(2) + 0)`);
    for (const type of ["keyDown", "keyUp"]) await b.send("Input.dispatchKeyEvent", { type, key: "y", code: "KeyY", text: type === "keyDown" ? "y" : undefined, windowsVirtualKeyCode: 89 });
    await waitForSharedCaret(b);
    await a.front(b);
    check(`${mode} the other person's typing arrives`, (await texts())[2] === "y" + before[2], await texts());
    await a.wait("the remote caret after typing", `document.querySelector(".pages .caret:not(.local) .name") !== null`);
    check(`${mode} the remote caret remains after typing`, (await a.ev(`document.querySelectorAll(".pages .caret:not(.local) .name").length`)) > 0);
    check(`${mode} no page errors in the other tab`, b.errors.length === 0, b.errors);
  }

  await a.ev(`pm().focus()`);
  await a.send("Input.dispatchKeyEvent", { type: "keyDown", key: "z", code: "KeyZ", windowsVirtualKeyCode: 90, modifiers: 4 });
  await a.send("Input.dispatchKeyEvent", { type: "keyUp", key: "z", code: "KeyZ", windowsVirtualKeyCode: 90, modifiers: 4 });
  await settle();
  check(`${mode} undo takes the bold back`, (await a.ev(`bold(1)`)) === "", await a.ev(`bold(1)`));

  // Typing over a selection is one change: one undo brings the old text back.
  const undo = async () => {
    for (const type of ["keyDown", "keyUp"]) await a.send("Input.dispatchKeyEvent", { type, key: "z", code: "KeyZ", windowsVirtualKeyCode: 90, modifiers: 4 });
  };
  const line = (await texts())[0];
  await a.ev(`select(start(0) + 0, start(0) + 5)`);
  await press("Z", "KeyZ", 90, "Z");
  await settle();
  check(`${mode} typed over a selection`, (await texts())[0] === "Z" + line.slice(5), await texts());
  if (b) {
    // Each history step moves Alice's caret away from the position Bob last saw.
    for (const redo of [false, true]) {
      await a.ev(`select(start(2) + 2, start(2) + 2)`);
      await waitForSharedCaret(a);
      await a.ev(`key("z", { metaKey: true, shiftKey: ${redo} })`);
      const head = await a.ev("pm().state.selection.head");
      check(`${mode} ${redo ? "redo" : "undo"} reveals the change in the first paragraph`, head < await a.ev("start(1)"), head);
      await waitForSharedCaret(a);
      await b.front(a);
      const remote = `[...docxDemo.surface.remote.values()].flat().some(c => c.at === ${head})`;
      await b.wait("the peer to show the history caret", remote);
      check(`${mode} the peer sees the caret after ${redo ? "redo" : "undo"}`, await b.ev(remote));
      await a.front(b);
    }
  }
  await undo();
  await settle();
  check(`${mode} one undo brings the old text back`, (await texts())[0] === line, await texts());
  await a.ev(`(() => { const input = document.getElementById("name"); input.value = "Alice"; input.dispatchEvent(new Event("change")); document.getElementById("track").click(); })()`);
  await a.ev(`select(start(0) + 2, start(0) + 2)`);
  await press("T", "KeyT", 84, "T");
  await settle();
  const insertions = (tab = a) => tab.ev(`(() => { const out = []; pm().state.doc.descendants(n => { const revision = n.isText && n.marks.find(m => m.type.name === "revision"); if (revision) out.push([n.text, revision.attrs.author]); }); return out; })()`);
  check(`${mode} tracked typing keeps its author`, (await insertions()).some(([text, author]) => text === "T" && author === "Alice"), await insertions());
  await press("Enter", "Enter", 13, "\r");
  await settle();
  const marks = (tab = a) => tab.ev(`(() => { const out = []; pm().state.doc.forEach(p => out.push(p.attrs.info?.mark_revision ?? null)); return out; })()`);
  check(`${mode} tracked Enter marks the paragraph it ends`, (await marks())[0] === "ins" && (await marks())[1] === null, await marks());
  check(`${mode} tracked insertion is not refused`, (await notice()) === "", await notice());
  if (b) {
    await b.front(a);
    check(`${mode} other tab sees insertion authors`, JSON.stringify(await insertions(b)) === JSON.stringify(await insertions()), await insertions(b));
    check(`${mode} other tab sees inserted marks`, JSON.stringify(await marks(b)) === JSON.stringify(await marks()), await marks(b));
    await a.front(b);
  }
  await undo(); await settle();
  await undo(); await settle();
  check(`${mode} tracked typing and Enter undo`, (await texts())[0] === line && (await insertions()).length === 0, await texts());
  if (b) {
    await a.ev('document.getElementById("track").click()');
    await b.front(a);
    const head = await b.ev('select(start(2) + 10, start(2) + 10); pm().state.selection.head');
    await waitForSharedCaret(b);
    await a.front(b);
    await a.wait("the peer caret inside the text to replace", `[...docxDemo.surface.remote.values()].flat().some(c => c.at === ${head})`);
    await a.ev("select(start(0), start(2) + pm().state.doc.child(2).content.size - 1)");
    for (const text of ["X", "Y"]) await a.send("Input.insertText", { text });
    await a.paint();
    await b.front(a);
    check("both tabs keep the next key after replacing text around a peer caret",
      (await texts())[0] === "XY." && (await texts(b))[0] === "XY.", [await texts(), await texts(b)]);
  } else {
    // A host may take the surface out of the page for a while. A change made meanwhile is still drawn.
    await a.ev(`(() => { const { surface } = docxDemo; window.host = surface.root.parentElement; surface.root.remove();
      pm().dispatch(pm().state.tr.insertText("Q", start(0))); })()`);
    await settle();
    await a.ev(`host.append(docxDemo.surface.root)`);
    await settle();
    const drawn = await a.ev(`Array.from(document.querySelectorAll(".pages .line"), (l) => l.textContent)`);
    check("a change made while the surface was out of the page is drawn", (await texts())[0] === "Q" + line && drawn[0]?.startsWith("Q"), [await texts(), drawn[0]]);
    await a.ev(`pm().focus()`);
    await undo(); await settle();
    check("and undone", (await texts())[0] === line, await texts());
    // Hold a file read open so the picker cannot start another open ahead of it.
    await a.ev(`(async () => {
      const picker = document.querySelector("#file");
      const bytes = await (await fetch("../fixtures/generated/table.docx")).arrayBuffer();
      const file = new File([bytes], "table.docx");
      file.arrayBuffer = () => new Promise(resolve => { window.finishRead = () => resolve(bytes); });
      const files = new DataTransfer();
      files.items.add(file);
      picker.files = files.files;
      picker.dispatchEvent(new Event("change"));
    })()`);
    check("the picker is disabled throughout the pending read", await a.ev('document.querySelector("#file").disabled'));
    await a.ev('finishRead()');
    await a.wait("opened table and ready picker", 'texts()[0] === "Before the table." && !document.querySelector("#file").disabled');
    // A change can be waiting for the next frame when a host disposes of its surface.
    await a.ev(`(() => {
      const s = docxDemo.surface;
      pm().dispatch(pm().state.tr.insertText("Q", start(0)));
      s.destroy();
      window.lastPages = s.pages;
    })()`);
    await a.paint();
    check("a destroyed surface leaves its last layout alone", await a.ev("docxDemo.surface.pages === lastPages"));
    // A page with no document to show says why for as long as it shows none.
    await a.send("Page.navigate", { url: await a.ev(`location.origin + location.pathname + "?fixture=generated/missing.docx"`) });
    await a.wait("the open to end", `document.querySelector("#file")?.disabled === false`);
    check("a failed first open leaves its reason where the pages go", await a.ev(`document.querySelector("#view").textContent.includes("generated/missing.docx: 404")`));
  }
  check(`${mode} no page errors`, a.errors.length === 0, a.errors);
}
