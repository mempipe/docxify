// Fills the content controls of the generated fixture in the browser demo,
// locally and in two shared tabs, by clicking them as a person would, and
// checks the painted text, the export, refusals, undo, redo and the other tab.
//
//   node prototypes/docx-engine/scripts/check-browser.mjs fill

import { check, click, same } from "./harness.mjs";

export const fixture = "generated/content-controls.docx";
export const windowSize = "1500,1400";

export const helpers = `(() => {
  window.focused = () => document.activeElement === pm().dom;
  window.controlsOf = (inspection) => { const out = {};
    const text = (n) => n.type === "text" ? n.text : n.type === "break" ? "\\n" : (n.children ?? []).map(text).join("") + (n.type === "paragraph" ? "¶" : "");
    const walk = (n) => { if (n.type === "content_control" && n.alias) out[n.alias] = { id: n.id, type: n.control.type, checked: n.control.checked ?? null, last: n.control.last_value ?? null,
      date: n.control.full_date ?? null, placeholder: !!n.placeholder, text: text(n) }; n.children?.forEach(walk); };
    walk(inspection.stories[0]); return out; };
  window.ids = controlsOf(docxDemo.surface.inspection);
  window.exportBytes = () => docxDemo.session.replica.export();
  window.exported = () => { const bytes = exportBytes(); const valid = docxDemo.engine.validate(bytes).valid; const re = docxDemo.engine.open(bytes);
    const out = { valid, controls: controlsOf(re.inspect()), hash: [...bytes].reduce((h, b) => (h * 31 + b) >>> 0, 7) }; re.free(); return out; };
  window.painted = (alias) => [...document.querySelectorAll('.pages [data-control="' + ids[alias].id + '"]')].map((s) => s.textContent).join("");
  window.paintedAll = () => Object.fromEntries(Object.keys(ids).map((a) => [a, painted(a)]));
  window.box = () => { const f = document.querySelector("#fillfield > *"); return { open: !document.getElementById("fillbar").hidden, tag: f?.tagName ?? null, type: f?.type ?? null, value: f?.value ?? null, label: f?.getAttribute("aria-label") ?? null, focused: !!f && document.activeElement === f }; };
  window.setField = (value) => { const f = document.querySelector("#fillfield > *"); f.value = value; };
  window.toggle = () => document.getElementById("track").click();
  window.center = (selector) => { const e = document.querySelector(selector); e.scrollIntoView({ block: "center", inline: "center" }); const r = e.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; };
})()`;

const field = (t, alias) => t.ev(`'.pages [data-control="' + ids[${JSON.stringify(alias)}].id + '"]'`);
async function enter(t, modifiers = 0) {
  for (const type of ["keyDown", "keyUp"]) await t.send("Input.dispatchKeyEvent", { type, key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, modifiers, ...(type === "keyDown" ? { text: "\r" } : {}) });
}
async function escape(t) {
  for (const type of ["keyDown", "keyUp"]) await t.send("Input.dispatchKeyEvent", { type, key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
}

export async function run(mode, a, b) {
  await a.front(b);
  const settle = a.paint;
  const notice = () => a.ev(`document.getElementById("notice").textContent`);
  const clearNotice = () => a.ev(`document.getElementById("notice").textContent = ""`);
  const controls = async () => (await a.ev(`exported()`)).controls;
  const other = async (label) => {
    if (!b) return;
    await b.front(a);
    check(`${mode} ${label}: the other tab paints the same fields`, same(await b.ev(`paintedAll()`), await a.ev(`paintedAll()`)), [await b.ev(`paintedAll()`), await a.ev(`paintedAll()`)]);
    const [ea, eb] = [await a.ev(`exported()`), await b.ev(`exported()`)];
    check(`${mode} ${label}: both tabs export the same bytes`, same(ea, eb) && ea.valid, [ea.hash, eb.hash, ea.valid]);
    await a.front(b);
  };
  const ids = await a.ev(`ids`);
  const original = await a.ev(`exported()`);
  const paintedBefore = await a.ev(`paintedAll()`);
  check(`${mode} opened: every control a form fills is painted as one, and the citation is not`,
    ["Name", "Agree", "Size", "Colour", "Start date", "Address", "Notes", "Reference", "Company", "Signature"].every((k) => paintedBefore[k] !== "") && paintedBefore.Source === "", paintedBefore);

  await click(a, await field(a, "Notes"));
  const notes = await a.ev('box().value');
  check(`${mode} the fill box keeps the tab and final line break`, notes === "West\tWing\n", notes);
  await a.ev('setField(box().value + "!")');
  await enter(a); await settle();
  check(`${mode} a refused tab leaves the exported control intact`,
    (await notice()).includes("tabs") && (await a.ev('exported().hash')) === original.hash);
  const draft = await a.ev('box()');
  check(`${mode} the refused draft stays open and focused`, draft.open && draft.focused && draft.value === notes + "!", draft);
  await a.ev('setField(box().value.replace("\t", " "))');
  await enter(a); await settle();
  const saved = (await controls()).Notes.text;
  check(`${mode} the corrected draft saves and focus returns to the editor`,
    saved === "West Wing\n!" && await a.ev('!box().open && focused()'), saved);
  await a.ev('key("z", { metaKey: true })'); await settle();
  await clearNotice();

  // A check box ticks on a click, with no box to fill.
  await click(a, await field(a, "Agree")); await settle();
  let now = await controls();
  check(`${mode} a click ticks the check box`, now.Agree.checked === true && (await a.ev(`box().open`)) === false, now.Agree);
  check(`${mode} the box shows the ticked symbol`, (await a.ev(`painted("Agree")`)) !== paintedBefore.Agree && (await a.ev(`painted("Agree")`)) === now.Agree.text.replace(/¶/g, ""), [await a.ev(`painted("Agree")`), now.Agree]);
  check(`${mode} the editor keeps the focus`, await a.ev(`focused()`), null);

  // A text control: the box opens empty over the placeholder, and Enter fills it.
  await click(a, await field(a, "Name")); await a.paint();
  let box = await a.ev(`box()`);
  check(`${mode} a text control opens an empty input, labelled with its title and focused`, box.open && box.tag === "INPUT" && box.type === "text" && box.value === "" && box.label === "Name" && box.focused, box);
  await a.send("Input.insertText", { text: "Ada Lovelace" });
  await enter(a); await settle();
  now = await controls();
  check(`${mode} Enter fills the text control`, now.Name.text === "Ada Lovelace" && !now.Name.placeholder && (await a.ev(`painted("Name")`)) === "Ada Lovelace" && !(await a.ev(`box().open`)), [now.Name, await a.ev(`painted("Name")`)]);
  check(`${mode} the editor has the focus again`, await a.ev(`focused()`), null);

  // Escape leaves it as it was, and the box opens with the value it holds.
  await click(a, await field(a, "Name")); await a.paint();
  box = await a.ev(`box()`);
  check(`${mode} the box opens with the control's value`, box.open && box.value === "Ada Lovelace", box);
  await a.send("Input.insertText", { text: "zzz" });
  await escape(a); await settle();
  check(`${mode} Escape closes the box and changes nothing`, !(await a.ev(`box().open`)) && (await controls()).Name.text === "Ada Lovelace" && (await a.ev(`focused()`)), await a.ev(`box()`));

  // Clicking away closes the box, and a shift-click extends the selection instead of opening one.
  await click(a, await field(a, "Name")); await a.paint();
  await click(a, ".pages .line > span:not([data-control])"); await a.paint();
  check(`${mode} clicking away closes the box`, !(await a.ev(`box().open`)) && (await a.ev(`focused()`)), await a.ev(`box()`));
  const [sx, sy] = await a.ev(`center(${JSON.stringify(await field(a, "Name"))})`);
  for (const type of ["mousePressed", "mouseReleased"]) await a.send("Input.dispatchMouseEvent", { type, x: sx, y: sy, button: "left", clickCount: 1, modifiers: 8 });
  await a.paint();
  check(`${mode} a shift-click opens no box`, !(await a.ev(`box().open`)), await a.ev(`box()`));

  // A drop-down list and a combo box choose an item; the combo box opens on its last value.
  await click(a, await field(a, "Size")); await a.paint();
  box = await a.ev(`box()`);
  check(`${mode} a drop-down list opens a select`, box.open && box.tag === "SELECT", box);
  await a.ev(`setField("M")`);
  await click(a, "#fillapply"); await settle();
  now = await controls();
  const sizeShown = now.Size.text.replace(/¶/g, "");
  check(`${mode} Fill chooses the item`, now.Size.last === "M" && !now.Size.placeholder && (await a.ev(`painted("Size")`)) === sizeShown && sizeShown !== "", [now.Size, await a.ev(`painted("Size")`)]);
  await click(a, await field(a, "Colour")); await a.paint();
  box = await a.ev(`box()`);
  check(`${mode} a combo box opens on its last value`, box.open && box.tag === "SELECT" && box.value === "Red", box);
  await a.ev(`setField("Green")`);
  await click(a, "#fillapply"); await settle();
  now = await controls();
  check(`${mode} the combo box shows the item's display text`, now.Colour.last === "Green" && (await a.ev(`painted("Colour")`)) === "Sea green", [now.Colour, await a.ev(`painted("Colour")`)]);
  await click(a, await field(a, "Colour")); await a.paint();
  box = await a.ev(`box()`);
  check(`${mode} the combo box opens on the item chosen last`, box.open && box.value === "Green", box);
  await escape(a); await settle();

  // A date control shows the date through its picture.
  await click(a, await field(a, "Start date")); await a.paint();
  box = await a.ev(`box()`);
  check(`${mode} a date control opens a date input`, box.open && box.type === "date", box);
  await a.ev(`setField("2026-11-16")`);
  await click(a, "#fillapply"); await settle();
  now = await controls();
  check(`${mode} the date is shown through the picture`, now["Start date"].date === "2026-11-16T00:00:00Z" && (await a.ev(`painted("Start date")`)) === "Monday, November 16, 2026", [now["Start date"], await a.ev(`painted("Start date")`)]);

  // A multi-line control takes Shift+Enter as a line break, and Enter fills it.
  await click(a, await field(a, "Address")); await a.paint();
  box = await a.ev(`box()`);
  check(`${mode} a multi-line control opens a text area`, box.open && box.tag === "TEXTAREA", box);
  await a.ev(`setField("")`);
  await a.send("Input.insertText", { text: "1 Main St" });
  await enter(a, 8);
  await a.send("Input.insertText", { text: "Springfield" });
  await enter(a); await settle();
  now = await controls();
  check(`${mode} the address has two lines`, now.Address.text === "1 Main St\nSpringfield¶" && (await a.ev(`painted("Address")`)) === "1 Main StSpringfield", [now.Address, await a.ev(`painted("Address")`)]);

  await click(a, await field(a, "Address")); await a.paint();
  box = await a.ev(`box()`);
  check(`${mode} the box opens with the address's two lines`, box.open && box.value === "1 Main St\nSpringfield", box);
  await escape(a); await settle();

  const copied = await a.ev(`key("a", { metaKey: true });
    (() => { const dt = new DataTransfer(); pm().dom.dispatchEvent(new ClipboardEvent("copy", { clipboardData: dt, bubbles: true, cancelable: true })); return dt.getData("text/plain"); })()`);
  check(`${mode} copied form includes filled inline and multiline control values`, copied.includes("Ada Lovelace") && copied.includes("1 Main St\nSpringfield"), copied);

  const cut = await a.ev(`(() => {
    let pos; pm().state.doc.descendants((node, at) => { if (node.attrs.id === ids.Name.id) pos = at; });
    select(pos, pos + 1);
    const dt = new DataTransfer();
    pm().dom.dispatchEvent(new ClipboardEvent("cut", { clipboardData: dt, bubbles: true, cancelable: true }));
    return dt.getData("text/plain");
  })()`);
  await settle();
  check(`${mode} cutting the name preserves its text on the clipboard and removes the control`,
    cut === "Ada Lovelace" && !(await controls()).Name, cut);
  await a.ev(`key("z", { metaKey: true })`); await settle();
  check(`${mode} undo restores the cut control and its value`, (await controls()).Name?.text === cut, cut);

  const filled = await a.ev(`exported()`);
  check(`${mode} export validates`, filled.valid, filled.valid);
  await other("filled");

  // What a form would not let anyone fill is refused, and changes nothing.
  for (const [alias, reason] of [["Reference", "locked against editing"], ["Company", "bound to custom XML"], ["Signature", ""]]) {
    await clearNotice();
    await click(a, await field(a, alias)); await a.paint();
    await a.send("Input.insertText", { text: "X" });
    await enter(a); await settle();
    const after = await a.ev(`exported()`);
    const said = await notice();
    check(`${mode} ${alias} is refused${reason ? ` as ${reason}` : ""}`, said !== "" && said.includes(reason) && after.hash === filled.hash, [said, after.hash === filled.hash]);
    await escape(a);
  }
  await clearNotice();

  // Track Changes records no fill, so it refuses one.
  await a.ev(`toggle()`);
  await click(a, await field(a, "Agree")); await settle();
  const said = await notice();
  check(`${mode} with Track Changes on, a fill is refused`, said.startsWith("Track Changes records") && said.includes("form field") && (await a.ev(`exported()`)).hash === filled.hash, said);
  await a.ev(`toggle()`);
  await clearNotice();

  // Typing beside the fields keeps them.
  const p0 = await a.ev(`start(0)`);
  await a.ev(`select(${p0}, ${p0})`);
  await a.send("Input.insertText", { text: "X" }); await settle();
  const typed = await a.ev(`paintedAll()`);
  check(`${mode} typing beside the fields keeps them`, same((await controls()), filled.controls) && typed.Name === "Ada Lovelace" && typed.Colour === "Sea green" && typed.Address === "1 Main StSpringfield", typed);
  await other("typed beside");

  // Each fill undoes on its own, after the typing.
  await a.ev(`key("z", { metaKey: true })`); await settle();
  await a.ev(`key("z", { metaKey: true })`); await settle();
  now = await controls();
  check(`${mode} undo: the address is back, the date stays`, now.Address.text === original.controls.Address.text && now["Start date"].date === "2026-11-16T00:00:00Z", [now.Address, now["Start date"]]);
  for (let i = 0; i < 5; i++) { await a.ev(`key("z", { metaKey: true })`); await settle(); }
  const undone = await a.ev(`exported()`);
  check(`${mode} undo: every fill is gone`, same(undone.controls, original.controls) && same(await a.ev(`paintedAll()`), paintedBefore), [undone.controls, await a.ev(`paintedAll()`)]);
  await other("undone");
  for (let i = 0; i < 7; i++) { await a.ev(`key("z", { metaKey: true, shiftKey: true })`); await settle(); }
  const redone = await a.ev(`exported()`);
  check(`${mode} redo: every fill is back`, redone.valid && same(redone.controls, filled.controls), [redone.controls, filled.controls]);
  check(`${mode} redo: the painted fields agree`, same(await a.ev(`paintedAll()`), typed), [await a.ev(`paintedAll()`), typed]);
  await other("redone");

  if (b) {
    // The other person clears the box from their tab.
    await b.front(a);
    await click(b, await field(b, "Agree")); await b.paint();
    const theirs = (await b.ev(`exported()`)).controls.Agree;
    check(`${mode} the other tab clears the check box`, theirs.checked === false, theirs);
    await a.front(b);
    check(`${mode} this tab shows it clear`, (await controls()).Agree.checked === false && (await a.ev(`painted("Agree")`)) === paintedBefore.Agree, [await a.ev(`painted("Agree")`), paintedBefore.Agree]);
    await other("the other tab's fill");
  }

  if (!b) {
    // The old document is still editable while its replacement is being read.
    await a.ev(`(async () => {
      const bytes = await (await fetch("../fixtures/generated/content-controls.docx")).arrayBuffer();
      const file = new File([bytes], "another-form.docx");
      file.arrayBuffer = () => new Promise(resolve => { window.finishOpen = () => resolve(bytes); });
      const files = new DataTransfer();
      files.items.add(file);
      const input = document.getElementById("file");
      input.files = files.files;
      input.dispatchEvent(new Event("change"));
    })()`);
    await click(a, await field(a, "Name"));
    await a.send("Input.insertText", { text: "Old document only" });
    await a.ev("finishOpen()");
    await a.wait("the replacement form", '!document.getElementById("file").disabled');
    check("opening another document closes the old fill box", !(await a.ev("box().open")));
    await a.ev('document.getElementById("fillbar").requestSubmit()');
    check("the old fill cannot write to the new document", same((await controls()).Name, original.controls.Name));
  }

  const errors = [...a.errors, ...(b?.errors ?? [])];
  check(`${mode}: no exceptions`, errors.length === 0, errors);
  check(`${mode}: no refusal left`, (await notice()) === "", await notice());
  return ids;
}
