import { check } from "./harness.mjs";

export const fixture = "generated/unknown-ooxml.docx";

// This paragraph starts with an opaque w:customXml, followed by editable text.
export const helpers = `window.opaqueParagraph = () => {
  let result;
  pm().state.doc.forEach((p, at) => {
    if (p.textContent.endsWith(" then plain text.")) {
      result = { start: at + 1, hasAtom: p.firstChild?.type.name === "inline_atom", text: p.textContent };
    }
  });
  return result;
}`;

export async function run(mode, a, b) {
  const original = await a.digest();
  const before = await a.ev("opaqueParagraph()");
  check(`${mode} source atom precedes plain text`, before.hasAtom, before);
  const caret = before.start + 1;
  await a.ev(`select(${caret}, ${caret})`);
  for (const type of ["keyDown", "keyUp"]) {
    await a.send("Input.dispatchKeyEvent", { type, key: "Backspace", code: "Backspace", windowsVirtualKeyCode: 8 });
  }
  await a.wait("Backspace to remove the whole opaque inline", "!opaqueParagraph().hasAtom");
  check(`${mode} Backspace preserves adjacent text`, (await a.ev("opaqueParagraph().text")) === before.text, before.text);
  if (b) {
    await b.front(a);
    check("peer removes the same atom", !(await b.ev("opaqueParagraph().hasAtom")));
    await a.front(b);
  }
  await a.ev('key("z", { metaKey: true })');
  await a.wait("undo to restore the opaque inline", "opaqueParagraph().hasAtom");
  check(`${mode} undo restores the exported source`, await a.digest() === original);
}
