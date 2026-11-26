// The style picker shows the style a paragraph uses, whatever ID it writes.
import { check } from "./harness.mjs";

export const fixture = "generated/undefined-paragraph-style.docx";

export const helpers = `(() => { window.picked = () => document.getElementById("style").value; })()`;

export async function run(mode, a) {
  // The second paragraph names a style the file does not define, so it uses the default.
  await a.ev("select(start(1) + 2, start(1) + 2)");
  await a.paint();
  check(`${mode} a paragraph naming an undefined style shows the default style`, (await a.ev("picked()")) === "Normal", await a.ev("picked()"));
  await a.ev("select(start(0) + 2, start(1) + 2)");
  await a.paint();
  check(`${mode} paragraphs that use one style show it, though one names no style`, (await a.ev("picked()")) === "Normal", await a.ev("picked()"));
}
