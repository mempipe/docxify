// A moveFrom is one atom in the input view and takes no space on the page.
import { check } from "./harness.mjs";
export { helpers } from "./track.mjs";

export const fixture = "generated/tracked-changes.docx";

export async function run(mode, a) {
  const move = () => a.ev(`(() => {
    let at;
    pm().state.doc.descendants((node, pos) => {
      if (node.attrs.node?.change === "moveFrom") at = pos;
    });
    return at;
  })()`);
  for (const [side, text] of [[0, "AB"], [1, "CD"]]) {
    const at = await move() + side;
    await a.ev(`select(${at}, ${at})`);
    await a.send("Input.insertText", { text });
    await a.paint();
  }
  await a.ev('author("Cara"); toggle()');
  for (const [key, side, letter] of [["Backspace", 1, "B"], ["Delete", 0, "C"]]) {
    const at = await move() + side;
    await a.ev(`select(${at}, ${at})`);
    for (const type of ["keyDown", "keyUp"]) {
      await a.send("Input.dispatchKeyEvent", { type, key, code: key, windowsVirtualKeyCode: side ? 8 : 46 });
    }
    await a.paint();
    const { revisions, valid } = await a.ev("exported()");
    check(`${mode} ${key} tracks the visible letter across a moved-away run`,
      valid && revisions.some(([kind, author, , text]) => kind === "del" && author === "Cara" && text === letter), revisions);
    check(`${mode} ${key} preserves the original move`,
      revisions.some(([kind, author, , text]) => kind === "moveFrom" && author === "Bob" && text === "Moved sentence."), revisions);
    await a.ev('key("z", { metaKey: true })');
    await a.paint();
  }
}
