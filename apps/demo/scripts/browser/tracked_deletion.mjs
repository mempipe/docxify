// Tracked-deletion and authorship regressions on the default replica.
import { check } from "./harness.mjs";
export { fixture, helpers } from "./track.mjs";

export async function run(mode, a, b) {
  const before = await a.ev(`exported()`);
  await a.ev(`author("Alice"); toggle(); select(start(0), start(0))`);
  await a.send("Input.insertText", { text: "blue" }); await a.paint();
  const deleter = b ?? a;
  await deleter.front(b ? a : undefined);
  await deleter.ev(`author("Bob"); ${b ? 'toggle();' : ''} select(start(0), start(0) + 4); key("Backspace")`);
  await deleter.paint();
  const revisions = await deleter.ev(`engineRevisions()`);
  check(`${mode} deleting another's addition retains the inserter`, revisions.some(([kind, author, , text]) => kind === "ins" && author === "Alice" && text === "blue"), revisions);
  check(`${mode} deleting another's addition records the deleter`, revisions.some(([kind, author, , text]) => kind === "del" && author === "Bob" && text === "blue"), revisions);
  check(`${mode} the deleted addition leaves the live line`, !(await deleter.ev(`texts()`))[0].startsWith("blue"));
  const out = await deleter.ev(`exported()`);
  check(`${mode} both authors survive export and reopen`, out.valid && out.revisions.filter(r => r[3] === "blue").length === 2, out);
  await deleter.ev(`key("z", { metaKey: true })`); await deleter.paint();
  check(`${mode} undo deletion restores the pending addition`, (await deleter.ev(`texts()`))[0].startsWith("blue"));
  await a.front(b);
  await a.ev(`key("z", { metaKey: true })`); await a.paint();
  check(`${mode} undo insertion restores the original document`, (await a.ev(`exported()`)).hash === before.hash);

  // A copy holds the words the body shows, so pasting it cannot bring back what a tracked change deleted.
  await a.ev(`select(start(0), start(0) + 5); key("Backspace")`); await a.paint();
  const copied = await a.ev(`(() => { select(start(0), start(0) + pm().state.doc.child(0).content.size);
    const dt = new DataTransfer(); pm().dom.dispatchEvent(new ClipboardEvent("copy", { clipboardData: dt, bubbles: true, cancelable: true })); return {plain: dt.getData("text/plain"), html: new DOMParser().parseFromString(dt.getData("text/html"), "text/html").body.textContent}; })()`);
  check(`${mode} a copy leaves out the words a tracked change deleted`, copied.plain === " paragraph." && copied.html === copied.plain, copied);
  await a.ev(`toggle(); select(start(1), start(1)); paste(${JSON.stringify(copied.plain)})`); await a.paint();
  const pasted = (await a.ev(`texts()`))[1];
  check(`${mode} pasting the copy adds only what the body showed`, pasted === " paragraph.Second paragraph.", pasted);
}
