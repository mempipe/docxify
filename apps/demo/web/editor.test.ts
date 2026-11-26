// The editor's own arithmetic, run in Node: node --test web/editor.test.ts

import assert from "node:assert/strict";
import { test } from "node:test";
import { AllSelection, EditorState, TextSelection } from "prosemirror-state";
import { changedRange, clipboardContent, copiedText, hold, inParagraph, paragraphStyle, resolutionNotes, storySelection, textChange } from "./editor.ts";
import { buildDoc, schema } from "./model.ts";
import type { InspectNode, NodeId, Point } from "./types.ts";

/** A view of one paragraph, `n1`, holding `text` in the text node `n3`. */
const view = (text: string) => {
  const leaf = schema.marks.leaf.create({ id: "n3" });
  const paragraph = schema.node("paragraph", { id: "n1", info: {} }, text ? [schema.text(text, [leaf])] : []);
  return schema.node("doc", null, [paragraph]);
};

test("a view change among equal characters is placed at the caret", () => {
  // A paragraph's content starts at position 1.
  assert.deepEqual(changedRange(view("aaaa"), view("aaaaa"), 3), { start: 2, endA: 2, endB: 3 });
  assert.deepEqual(changedRange(view("aaaa"), view("aaaaa"), 6), { start: 5, endA: 5, endB: 6 });
  assert.deepEqual(changedRange(view("aaaa"), view("aaa"), 2), { start: 2, endA: 3, endB: 2 });
  assert.deepEqual(changedRange(view("one two"), view("one 2"), 6), { start: 5, endA: 8, endB: 6 });
  assert.equal(changedRange(view("aaaa"), view("aaaa"), 1), null);
});

test("a caret that is not where the change ends leaves the comparison's answer", () => {
  assert.deepEqual(changedRange(view("abc"), view("abXc"), 1), { start: 3, endA: 3, endB: 4 });
});

test("the replacement command names the caret among equal letters", () => {
  const change = textChange(view("aaaa"), view("aaaaa"), 3);
  assert.deepEqual(change, { from: { node: "n3", offset: 1 }, to: { node: "n3", offset: 1 }, at: [2, 2], text: "a" });
});

test("a replacement after an emoji uses character offsets in the engine", () => {
  assert.deepEqual(textChange(view("😀ab"), view("😀Xb"), 4), {
    from: { node: "n3", offset: 1 }, to: { node: "n3", offset: 2 }, at: [3, 4], text: "X",
  });
});

test("a held selection keeps both its ends as the engine names them, or nothing if one cannot be kept", () => {
  const paragraph = (id: string, leaf: string, text: string) =>
    schema.node("paragraph", { id, info: {} }, [schema.text(text, [schema.marks.leaf.create({ id: leaf })])]);
  const doc = schema.node("doc", null, [paragraph("n1", "n3", "First"), paragraph("n4", "n6", "Second one")]);
  const session = { sticky: (point: Point) => JSON.stringify(point) };
  const start = JSON.stringify({ node: "n3", offset: 0 });
  assert.deepEqual(hold(TextSelection.create(doc, 1), session), { anchor: start, head: start });
  // "Second", selected from its end back to the second paragraph's start at position 8.
  assert.deepEqual(hold(TextSelection.create(doc, 14, 8), session), {
    anchor: JSON.stringify({ node: "n6", offset: 6 }),
    head: JSON.stringify({ node: "n6", offset: 0 }),
  });
  const untilSecond = { sticky: (point: Point) => ("node" in point && point.node === "n3" ? "kept" : undefined) };
  assert.equal(hold(TextSelection.create(doc, 1, 14), untilSecond), null);
});

test("an empty editable text leaf resolves to its visible edge for a held caret", () => {
  const leaf = (id: NodeId, text: string) => ({ type: "text", id, support: "editable", text });
  const paragraph = (children: object[]) => ({
    id: "n1", type: "paragraph", support: "editable",
    children: [{ type: "run", id: "n2", support: "editable", children }],
  }) as unknown as InspectNode;
  for (const [children, edge] of [
    [[leaf("n3", "")], 1],
    [[leaf("n4", "😀"), leaf("n3", ""), leaf("n5", "B")], 3],
  ] as const) {
    const source = paragraph([...children]);
    const pm = buildDoc({ children: [source] } as InspectNode).firstChild!;
    const resolve = (node: NodeId, offset: number) => inParagraph(pm, 1, { node, offset }, { inspectNode: () => source });
    assert.equal(resolve("n3", 0), edge);
    assert.equal(resolve("n3", 1), null);
    assert.equal(resolve("n9", 0), null);
  }
});

test("Select All uses paragraph endpoints and includes a table or control at a story edge", () => {
  const p = view("abc").firstChild!;
  const block = (type: string) => schema.node("block_atom", { id: "n9", node: { type } });
  const select = (...children: (typeof p)[]) => storySelection(new AllSelection(schema.node("doc", null, children)));
  const text = select(p, p, block("section"));
  assert.ok(text instanceof TextSelection);
  assert.deepEqual([text.from, text.to], [1, 9]);
  assert.ok(select(block("content_control"), p) instanceof AllSelection);
  assert.ok(select(p, schema.node("table", { id: "n8", info: {} }), block("section")) instanceof AllSelection);
  assert.ok(select(block("section")) instanceof AllSelection);
});

test("a copy holds what the body shows: a tracked deletion's words stay out, at any depth", () => {
  const node = (type: string, rest: object = {}, children?: object[]) => ({ type, id: "n9", support: "read_only", ...rest, children }) as unknown as InspectNode;
  const atom = (inner: InspectNode) => schema.node("inline_atom", { id: inner.id, node: inner });
  const deleted = node("revision", { change: "del" }, [node("run", {}, [node("deleted_text", { text: "First" })])]);
  const moved = node("revision", { change: "moveFrom" }, [node("run", {}, [node("text", { text: "Moved" })])]);
  const inserted = node("revision", { change: "ins" }, [node("run", {}, [node("text", { text: "added" })])]);
  const control = node("content_control", {}, [
    node("run", {}, [node("text", { text: "Ada" }), node("tab"), node("text", { text: "L" }), node("break")]),
    deleted,
    node("run", { effective: { hidden: true } }, [node("text", { text: "hidden in control" })]),
    node("run", {}, [node("text", { text: "end" })]),
  ]);
  const leaf = schema.marks.leaf.create({ id: "n3" });
  const paragraph = (...content: Parameters<typeof schema.node>[2][]) => schema.node("paragraph", { id: "n1", info: {} }, content.flat() as never);
  const doc = schema.node("doc", null, [
    paragraph(atom(deleted), schema.text(" paragraph.", [leaf]), atom(moved), schema.text("hidden", [schema.marks.run.create({ info: { effective: { hidden: true } } })])),
    paragraph(atom(inserted), atom(control)),
  ]);
  const copy = clipboardContent(doc.content);
  assert.equal(copiedText(copy), " paragraph.\naddedAda\tL\nend");
});

test("each reason the reader reports is told to the person once, in plain words", () => {
  const reasons = [
    "merge_start_row",
    "last_row",
    "merge_start_row",
    "merge_start_wrapper",
    "last_column",
    "source_grid",
  ] as const;
  assert.deepEqual(resolutionNotes(reasons), [
    "A row is visible because a merged cell below still needs it.",
    "A table keeps its last row.",
    "A content control is visible because a merged cell below still needs it.",
    "A table keeps its last column.",
    "A table shows its original columns because a row does not fit the changed ones.",
  ]);
  assert.deepEqual(resolutionNotes([]), []);
});

test("the style picker reads the style in effect, not the ID a paragraph writes", () => {
  const paragraph = (id: string, info: object) =>
    schema.node("paragraph", { id, info }, [schema.text("Text", [schema.marks.leaf.create({ id: `${id}t` })])]);
  const picked = (...infos: object[]) => {
    const doc = schema.node("doc", null, infos.map((info, i) => paragraph(`n${i}`, info)));
    return paragraphStyle(EditorState.create({ doc, selection: new AllSelection(doc) }));
  };
  // One paragraph names the default style, one inherits it, and one names a style the file lacks.
  const normal = [{ style: "Normal", effective_style: "Normal" }, { effective_style: "Normal" }, { style: "Missing", effective_style: "Normal" }];
  assert.equal(picked(...normal), "Normal");
  assert.equal(picked(...normal, { style: "Title", effective_style: "Title" }), null);
});
