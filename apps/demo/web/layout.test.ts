// The layout with a fixed measurer: every character half an em wide, and a
// 12pt line 12pt tall, so expected positions are plain arithmetic.
//
//   node --test web/layout.test.ts

import assert from "node:assert/strict";
import { test } from "node:test";
import type { Node as PMNode } from "prosemirror-model";
import { type Block, Blocks, type Cell, type Float, type Inline, type LayoutInput, type Paragraph, type Table, type TextStyle, TWIP } from "./blocks.ts";
import { caretX, type Line, LineBreaker, offsetAt } from "./lines.ts";
import { type Balloon, describe, markup, MOVE_COLOR, place, REPLY_HEIGHT } from "./markup.ts";
import { FixedMeasurer } from "./measure.ts";
import { buildDoc, insertionOf, leafOf, schema } from "./model.ts";
import { type Page, Paginator } from "./paginate.ts";
import type { EffectiveParagraph, Inspection, InspectNode, Section, TabStop } from "./types.ts";

import { readFileSync } from "node:fs";
import { formatNumber } from "./numbering.ts";
import { initSync } from "./pkg/docx_engine_wasm.js";
initSync({ module: readFileSync(new URL("./pkg/docx_engine_wasm_bg.wasm", import.meta.url)) });

test("lettered pages repeat the letter after Z", () => {
  assert.equal(formatNumber(28, "lowerLetter"), "bb");
  assert.equal(formatNumber(28, "upperLetter"), "BB");
});

const measurer = new FixedMeasurer();
const blocks = new Blocks((family) => family);
const style = blocks.style({ font: "Test", size_half_points: 24 });
const PROPS: EffectiveParagraph = {
  alignment: "left",
  indent: { left_twips: 0, right_twips: 0, first_line_twips: 0 },
  spacing: { before_twips: 0, after_twips: 0, line: 240, line_rule: "auto" },
  contextual_spacing: false,
  keep_next: false,
  keep_lines: false,
  widow_control: false,
  page_break_before: false,
  tabs: [],
};

/** Text and tabs (`\t`) as one paragraph's inlines, each with its position. */
function inlines(text: string, textStyle = style): Inline[] {
  const out: Inline[] = [];
  let at = 0;
  for (const part of text.split(/(\t)/)) {
    if (part === "\t") out.push({ kind: "tab", style: textStyle, source: { at: at++, atom: true } });
    else if (part) {
      out.push({ kind: "text", text: part, style: textStyle, source: { at, atom: false } });
      at += part.length;
    }
  }
  return out;
}

function para(text: string, props: Partial<EffectiveParagraph> = {}, extra: Partial<Paragraph> = {}): Paragraph {
  const content = extra.inlines ?? inlines(text);
  const size = content.reduce((n, i) => n + (i.kind === "text" && !i.source?.atom ? i.text.length : 1), 0);
  return {
    kind: "paragraph",
    key: {},
    props: { ...PROPS, ...props },
    mark: style,
    inlines: content,
    start: 1,
    size,
    formatChanged: false,
    fields: false,
    ...extra,
  };
}

function lines(p: Paragraph, width: number): Line[] {
  const breaker = new LineBreaker(p, measurer, 36);
  const out: Line[] = [];
  for (let line = breaker.next(width); line; line = breaker.next(width)) out.push(line);
  return out;
}

const shown = (line: Line) => line.pieces.map((p) => p.text).join("");
const xOf = (line: Line, text: string) => line.pieces.find((p) => p.text === text)!.x;

test("lines break at spaces, and each position is on a line", () => {
  const out = lines(para("aaaa bbbb cccc"), 60);
  assert.deepEqual(out.map(shown), ["aaaa bbbb ", "cccc"]);
  assert.deepEqual(out.map((l) => [l.from, l.to]), [[0, 10], [10, 14]]);
});

test("a word longer than the line is split, and a hyphen is a place to break", () => {
  assert.deepEqual(lines(para("abcdefghijkl"), 30).map(shown), ["abcde", "fghij", "kl"]);
  assert.deepEqual(lines(para("well-known"), 36).map(shown), ["well-", "known"]);
});

test("tabs go to custom stops by their alignment, then to default stops", () => {
  const at = (align: TabStop["align"], text: string, word: string) =>
    xOf(lines(para(text, { tabs: [{ pos_twips: 72 / TWIP, align }] }), 300)[0], word);
  assert.equal(at("left", "a\tbb", "bb"), 72);
  assert.equal(at("right", "a\tbb", "bb"), 60);
  assert.equal(at("center", "a\tbb", "bb"), 66);
  assert.equal(at("decimal", "a\t12.5", "12.5"), 60);
  assert.equal(xOf(lines(para("a\tb\tc"), 300)[0], "c"), 72);
  const dotted = lines(para("a\tbb", { tabs: [{ pos_twips: 72 / TWIP, align: "left", leader: "dot" }] }), 300)[0];
  // The tab runs from 6pt to 72pt, and each dot is 6pt wide.
  assert.equal(dotted.pieces.find((p) => p.inline?.kind === "tab")!.leader, ".".repeat(11));
});

test("a hanging indent is a tab stop on the first line", () => {
  const p = para("1.\tItem", { indent: { left_twips: 720, right_twips: 0, first_line_twips: -720 } });
  assert.equal(xOf(lines(p, 300)[0], "Item"), 36);
});

test("justified lines stretch their spaces to the edge, except the last", () => {
  const [first, last] = lines(para("aa bb cc dd", { alignment: "justify" }), 50);
  const end = first.pieces.find((p) => p.text === "cc")!;
  assert.equal(end.x + end.width, 50);
  assert.equal(xOf(last, "dd"), 0);
});

test("centred lines share the room on both sides", () => {
  assert.equal(xOf(lines(para("ab", { alignment: "center" }), 60)[0], "ab"), 24);
});

test("line spacing: multiples, exact and at least", () => {
  const height = (line: number, line_rule: EffectiveParagraph["spacing"]["line_rule"]) =>
    lines(para("a", { spacing: { ...PROPS.spacing, line, line_rule } }), 100)[0].height;
  assert.equal(height(480, "auto"), 24);
  assert.equal(height(300, "exact"), 15);
  assert.equal(height(400, "at_least"), 20);
  assert.equal(height(100, "at_least"), 12);
});

test("a caret position and a click on it agree, at every position", () => {
  for (const line of lines(para("aaaa bbbb cccc\tdd", { alignment: "justify" }), 60)) {
    for (let at = line.from; at <= line.to; at++) {
      assert.equal(offsetAt(line, caretX(line, at, measurer), measurer).offset, at);
    }
  }
});

test("past the end of a wrapped line, the caret stays on that line", () => {
  const [first, second] = lines(para("aaaa bbbb cccc"), 60);
  assert.deepEqual(offsetAt(first, 59, measurer), { offset: 10, bias: -1 });
  assert.deepEqual(offsetAt(second, 0, measurer), { offset: 10, bias: 1 });
  assert.deepEqual(offsetAt(second, 59, measurer), { offset: 14, bias: 1 });
});

test("text set in capitals has a caret position between its letters", () => {
  const caps = blocks.style({ font: "Test", size_half_points: 24, caps: true });
  const [line] = lines(para("", {}, { inlines: inlines("hello world", caps) }), 300);
  assert.equal(shown(line), "HELLO WORLD");
  assert.equal(caretX(line, 3, measurer), 18);
  assert.equal(offsetAt(line, 19, measurer).offset, 3);
});

test("capitals that draw one letter as two keep the caret positions around it", () => {
  const caps = blocks.style({ font: "Test", size_half_points: 24, caps: true });
  const [line] = lines(para("", {}, { inlines: inlines("Straße x", caps) }), 300);
  assert.equal(shown(line), "STRASSE X");
  // Each drawn letter is six points wide, and ß is drawn as two.
  const stops = [0, 6, 12, 18, 24, 36, 42, 48, 54];
  stops.forEach((x, at) => {
    assert.equal(caretX(line, at, measurer), x, `caret at ${at}`);
    assert.equal(offsetAt(line, x, measurer).offset, at, `click at ${x}`);
  });
  assert.equal(offsetAt(line, 13, measurer).offset, 2);
  assert.equal(offsetAt(line, 29, measurer).offset, 4);
  assert.equal(offsetAt(line, 31, measurer).offset, 5);

  for (const width of [6, 12]) {
    const wrapped = lines(para("", {}, { inlines: inlines("aßz", caps) }), width);
    assert.deepEqual(wrapped.map(shown), ["A", "SS", "Z"]);
    assert.deepEqual(wrapped.map(l => [l.from, l.to]), [[0, 1], [1, 2], [2, 3]]);
  }
});

/** A page five 12pt lines tall and ten characters wide, with no margins. */
const SMALL: Section = {
  start: "nextPage",
  page_width: 60 / TWIP,
  page_height: 60 / TWIP,
  margins: { top: 0, bottom: 0, left: 0, right: 0, header: 0, footer: 0 },
};

function layout(body: Block[], more: Partial<LayoutInput> = {}): Page[] {
  return new Paginator(measurer).layout({
    body: { key: null, blocks: body },
    section: SMALL,
    parts: new Map(),
    notes: new Map(),
    comments: new Map(),
    settings: { default_tab_stop_twips: 720, even_and_odd_headers: false, update_fields: false },
    ...more,
  });
}

const texts = (page: Page) =>
  page.fragments.flatMap((f) => (f.kind === "line" ? [f.line.pieces.map((p) => p.text).join("").trim()] : []));
const many = (n: number, prefix = "p") => Array.from({ length: n }, (_, i) => para(`${prefix}${i}`));

test("lines fill each page to its bottom margin", () => {
  assert.deepEqual(layout(many(12)).map((p) => texts(p).length), [5, 5, 2]);
});

test("the space between two paragraphs is the larger of one's space after and the other's before", () => {
  const tops = (blocks: Block[]) => layout(blocks)[0].fragments.flatMap((f) => (f.kind === "line" ? [f.y] : []));
  const spaced = (after: number, before: number, more: Partial<EffectiveParagraph> = {}) => ({
    ...more,
    spacing: { ...PROPS.spacing, after_twips: after * 20, before_twips: before * 20 },
  });
  // Each line is 12pt tall, so the second starts 12pt down plus the space between.
  assert.deepEqual(tops([para("a", spaced(12, 0)), para("b", spaced(0, 4))]), [0, 24]);
  assert.deepEqual(tops([para("a", spaced(4, 0)), para("b", spaced(0, 12))]), [0, 24]);
  assert.deepEqual(tops([para("a", spaced(6, 0)), para("b", spaced(0, 6))]), [0, 18]);
  // The first paragraph's space before, and the space between each pair after it.
  assert.deepEqual(tops([para("a", spaced(6, 2)), para("b", spaced(6, 4)), para("c", spaced(0, 10))]), [2, 20, 42]);

  // Contextual spacing drops a paragraph's own space next to one in its style, and only its own.
  const contextual = { contextual_spacing: true };
  assert.deepEqual(tops([para("a", spaced(12, 0, contextual)), para("b", spaced(0, 4, contextual))]), [0, 12]);
  assert.deepEqual(tops([para("a", spaced(12, 0, contextual)), para("b", spaced(0, 4))]), [0, 16]);
  assert.deepEqual(tops([para("a", spaced(12, 0)), para("b", spaced(0, 4, contextual))]), [0, 24]);
  assert.deepEqual(tops([para("a", spaced(12, 0, contextual)), para("b", spaced(0, 4, contextual), { style: "Other" })]), [0, 24]);
});

test("a paragraph kept with the next moves to the next page with it", () => {
  const pages = layout([...many(4), para("Heading", { keep_next: true }), para("body")]);
  assert.deepEqual(texts(pages[1]), ["Heading", "body"]);
});

test("widow and orphan control keep two lines together at each end", () => {
  const three = "aaaaaaaaa bbbbbbbbb ccccccccc";
  const orphan = layout([...many(4), para(three, { widow_control: true })]);
  assert.equal(texts(orphan[0]).length, 4);
  const allowed = layout([...many(4), para(three)]);
  assert.equal(texts(allowed[0]).length, 5);
  const widow = layout([...many(2), para(`${three} ddddddddd`, { widow_control: true })]);
  assert.deepEqual(widow.map((p) => texts(p).length), [4, 2]);
});

test("page break before starts a new page, and a page break inline moves what follows", () => {
  assert.deepEqual(layout([para("a"), para("b", { page_break_before: true })]).map(texts), [["a"], ["b"]]);
  const broken = para("", {}, { inlines: [...inlines("a"), { kind: "break", type: "page", style, source: { at: 1, atom: true } }, ...inlines("b").map((i) => ({ ...i, source: { at: 2, atom: false } }))] });
  assert.deepEqual(layout([broken]).map(texts), [["a"], ["b"]]);
});

test("a footnote takes room at the bottom of the page that refers to it", () => {
  const reference = para("", {}, { inlines: [{ kind: "text", text: "a1", style, source: { at: 0, atom: false }, note: "footnote:1" }] });
  const notes = new Map([["footnote:1", [para("note one"), para("note two")]]]);
  const pages = layout([reference, ...many(4)], { notes });
  // A separator and two note lines leave two lines of body.
  assert.deepEqual(texts(pages[0]), ["a1", "p0", "note one", "note two"]);
  assert.ok(pages[0].fragments.some((f) => f.kind === "rule"));
});

const cell = (content: string | Block[], rowspan = 1, column = 0): Cell => ({
  column,
  span: 1,
  rowspan,
  borders: {},
  margins: { top: 0, left: 0, bottom: 0, right: 0 },
  valign: "top",
  blocks: typeof content === "string" ? [para(content)] : content,
});

function table(rows: (string | Cell)[][], header = 0, grid = [60]): Table {
  return {
    kind: "table",
    key: {},
    grid,
    indent: 0,
    alignment: "left",
    rows: rows.map((cells, i) => ({
      header: i < header,
      cantSplit: false,
      gridBefore: 0,
      cells: cells.map((c, i) => (typeof c === "string" ? cell(c, 1, i) : c)),
    })),
  };
}

const lineAt = (page: Page, text: string) =>
  page.fragments.find((f) => f.kind === "line" && f.line.pieces.map((p) => p.text).join("").trim() === text) as
    | Extract<Page["fragments"][number], { kind: "line" }>
    | undefined;

test("header rows repeat on each page a table continues to", () => {
  const pages = layout([table([["Head"], ...Array.from({ length: 6 }, (_, i) => [`r${i}`])], 1)]);
  assert.deepEqual(pages.map(texts), [["Head", "r0", "r1", "r2", "r3"], ["Head", "r4", "r5"]]);
});

test("a cell after a merged cell sits in the next free grid column", () => {
  const [page] = layout([table([[cell("A", 2), "B1"], [cell("B2", 1, 1)]], 0, [20, 20])]);
  assert.equal(lineAt(page, "B1")!.x, 20);
  assert.equal(lineAt(page, "B2")!.x, 20);
  assert.deepEqual(
    page.fragments.flatMap((f) => (f.kind === "cell" ? [[f.x, f.y, f.height]] : [])),
    [[0, 0, 24], [20, 0, 12], [20, 12, 12]],
  );
});

test("a row that cannot split goes in whole below repeated headers, even when too tall", { timeout: 2000 }, () => {
  const tall = { ...cell(many(5, "t")) };
  const rows = table([["Head"], ["r0"], [tall]], 1);
  rows.rows[2].cantSplit = true;
  const pages = layout([rows]);
  assert.deepEqual(pages.map(texts), [["Head", "r0"], ["Head", "t0", "t1", "t2", "t3", "t4"]]);
});

test("a float anchored in a table cell is drawn", () => {
  const float: Float = {
    width: 10,
    height: 10,
    image: {},
    placement: {
      kind: "anchor",
      horizontal: { relative_to: "column", align: "right" },
      vertical: { relative_to: "paragraph" },
      wrap: "none",
      behind: false,
      distance_emu: {},
    },
  };
  const holder = para("x", {}, { inlines: [...inlines("x"), { kind: "float", float, source: { at: 1, atom: true } }] });
  const [page] = layout([para("p0"), table([["a", cell([holder], 1, 1)]], 0, [20, 40])]);
  const image = page.fragments.find((f) => f.kind === "image");
  assert.deepEqual(image && [image.x, image.y], [50, 12]);
});

test("a row taller than the room left splits between lines", () => {
  const tall = "aaaaaaaaa bbbbbbbbb ccccccccc ddddddddd";
  const pages = layout([...many(3), table([[tall]])]);
  assert.deepEqual(pages.map(texts), [["p0", "p1", "p2", "aaaaaaaaa", "bbbbbbbbb"], ["ccccccccc", "ddddddddd"]]);
});

test("headers show page fields, with the total from a second pass", () => {
  // "9/9" with the page number and the page count as fields, as Word stores their last results.
  const header = para("", {}, {
    inlines: [
      { kind: "text", text: "9", style: { ...style, pageField: { field: "n1", number: "page" } }, source: { at: 0, atom: true } },
      { kind: "text", text: "/", style, source: { at: 1, atom: false } },
      { kind: "text", text: "9", style: { ...style, pageField: { field: "n2", number: "num_pages" } }, source: { at: 2, atom: true } },
    ],
    fields: true,
  });
  const section: Section = { ...SMALL, page_height: 84 / TWIP, headers: [{ kind: "default", relationship: "rId1", part: "h" }] };
  const pages = layout(many(10), { section, parts: new Map([["h", { key: null, blocks: [header] }]]) });
  assert.deepEqual(pages.map((p) => texts(p)[0]), ["1/2", "2/2"]);
});

test("each page field is drawn once, in its own format, and other results stay as stored", () => {
  const text = (text: string, at: number, pageField?: TextStyle["pageField"]): Inline => (
    { kind: "text", text, style: pageField ? { ...style, pageField } : style, source: { at, atom: !!pageField } }
  );
  const header = para("", {}, {
    inlines: [
      // Two fields with the same instruction, with a tab between them.
      text("9", 0, { field: "n1", number: "page" }),
      { kind: "tab", style, source: { at: 1, atom: false } },
      text("9", 2, { field: "n2", number: "page" }),
      text(" ", 3),
      // One result in two runs.
      text("x", 4, { field: "n3", number: "page", format: "lowerRoman" }),
      text("x", 5, { field: "n3", number: "page", format: "lowerRoman" }),
      text(" ", 6),
      // A locked field's result is ordinary text to a layout.
      text("42", 7),
    ],
    fields: true,
  });
  const section: Section = { ...SMALL, page_height: 84 / TWIP, headers: [{ kind: "default", relationship: "rId1", part: "h" }] };
  const pages = layout(many(10), { section, parts: new Map([["h", { key: null, blocks: [header] }]]) });
  // The header wraps onto a second line.
  assert.deepEqual(pages.map((p) => texts(p).slice(0, 2)), [["11 i", "42"], ["22 ii", "42"]]);
});

test("a title page shows the first-page header, and later pages the default one", () => {
  const section: Section = {
    ...SMALL,
    page_height: 84 / TWIP,
    title_page: true,
    headers: [
      { kind: "first", relationship: "rId1", part: "first" },
      { kind: "default", relationship: "rId2", part: "default" },
    ],
  };
  const parts = new Map([
    ["first", { key: null, blocks: [para("First")] }],
    ["default", { key: null, blocks: [para("Default")] }],
  ]);
  const pages = layout(many(10), { section, parts });
  assert.deepEqual(pages.map((p) => texts(p)[0]), ["First", "Default"]);
});

test("text flows down one column and then the next", () => {
  const section: Section = { ...SMALL, page_width: 130 / TWIP, columns: { count: 2, space_twips: 10 / TWIP } };
  const [page] = layout(many(8), { section });
  const x = page.fragments.flatMap((f) => (f.kind === "line" ? [f.x] : []));
  assert.deepEqual(x, [0, 0, 0, 0, 0, 70, 70, 70]);
});

test("balloons say what changes removed, moved and reformatted, and what comments say, in Word's words", () => {
  const text = (t: string, at: number, extra: Partial<typeof style> = {}): Inline => ({
    kind: "text",
    text: t,
    style: { ...style, ...extra },
    source: { at, atom: false },
  });
  const removed = (change: string, author: string, t: string | Inline[], at: number, move?: string): Inline => ({
    kind: "anchor",
    removed: { revision: { change, author, move }, inlines: typeof t === "string" ? inlines(t) : t },
    style,
    source: { at, atom: true },
  });
  const moveTo = { change: "moveTo", author: "Bob", move: "m" };
  const bold = { by: { author: "Alice" }, before: {}, after: { bold: true } };
  const body = [
    // Part of what moved was deleted later, inside the move: the balloon quotes it all.
    para("", {}, { start: 0, inlines: [text("a", 0), removed("moveFrom", "Bob", [...inlines("moved "), removed("del", "Bob", "here", 0)], 1, "m")] }),
    para("", {}, { start: 10, inlines: [text("b", 0), removed("del", "Alice", "gone", 1)] }),
    // A moved run is one atom: its pieces share the atom's one position. This one was made bold too.
    para("", {}, { start: 20, inlines: [{ ...text("moved here", 0, { revision: moveTo, formatted: bold }), source: { at: 0, atom: true } }] }),
    para("", {}, { start: 30, inlines: [text("x ", 0, { formatted: bold }), text("y", 2, { formatted: bold })] }),
    para("", {}, { start: 40, inlines: [text("see ", 0, { comments: ["0"] }), text("note", 4, { comments: ["0"] })] }),
    para("end", {}, { start: 50, markRevision: { change: "del", author: "Alice" } }),
  ];
  const comments = new Map([["0", { author: "Carol", blocks: [para("Nice", {}, { start: null })] }]]);
  const pages = layout(body, { section: { ...SMALL, page_width: 300 / TWIP, page_height: 600 / TWIP } });
  const m = markup(pages, comments, measurer, (run) => blocks.style(run));
  const words = (b: Balloon) => b.lines.map((l) => shown(l.line).trim());

  assert.deepEqual(m.pages[0].map(words), [
    ["Bob", "Moved down [1]: moved here"],
    ["Alice", "Deleted: gone"],
    ["Bob", "Moved (insertion) [1]"],
    ["Alice", "Formatted: Font: Bold"],
    ["Alice", "Formatted: Font: Bold"],
    ["Carol", "Nice"],
    ["Alice", "Deleted: ¶"],
  ]);
  assert.deepEqual([...m.colors], [["Bob", "#c0423f"], ["Alice", "#3376cd"], ["Carol", "#7a4fa3"]]);
  assert.deepEqual(m.pages[0].map((b) => b.color), [MOVE_COLOR, "#3376cd", MOVE_COLOR, "#3376cd", "#3376cd", "#7a4fa3", "#3376cd"]);
  const [from, , to, , formatted, comment] = m.pages[0];
  assert.deepEqual([from.partner, to.partner], [20, 1]);
  assert.deepEqual([formatted.from, formatted.to], [30, 33]);
  // A comment's connector starts after the last text it covers.
  assert.deepEqual([comment.anchor, comment.from, comment.to], [{ x: 48, y: 48 }, 40, 48]);
  const tops = m.pages[0].map((b) => b.top);
  tops.slice(1).forEach((top, i) => assert.ok(top >= tops[i] + m.pages[0][i].height));
});

test("a reply is drawn in the balloon of the comment it answers, which offers to reply", () => {
  const covered = { ...style, comments: ["0", "1"] };
  const body = [para("", {}, { start: 0, inlines: [{ kind: "text", text: "see", style: covered, source: { at: 0, atom: false } }] })];
  const comments = new Map([
    ["0", { author: "Carol", blocks: [para("Nice", {}, { start: null })] }],
    ["1", { author: "Dan", parent: "0", blocks: [para("Agreed", {}, { start: null })] }],
  ]);
  const pages = layout(body, { section: { ...SMALL, page_height: 600 / TWIP } });
  const m = markup(pages, comments, measurer, (run) => blocks.style(run));
  assert.deepEqual(
    m.pages[0].map((b) => [b.comment, b.lines.map((l) => shown(l.line).trim())]),
    [["0", ["Carol", "Nice", "Dan", "Agreed"]]],
  );
  const [balloon] = m.pages[0];
  const last = balloon.lines.at(-1)!;
  assert.ok(balloon.height - (last.y + last.line.height) >= REPLY_HEIGHT, "room for the Reply button");
  assert.deepEqual([...m.comments.keys()], ["0", "1"]);
});

test("a move's arrows lead from each paragraph it took to where it went, and back", () => {
  const revision = (change: string) => ({ change, author: "Bob", move: "m" });
  const removed = (t: string): Inline => ({ kind: "anchor", removed: { revision: revision("moveFrom"), inlines: inlines(t) }, style, source: { at: 0, atom: true } });
  const moved = { kind: "text" as const, text: "one two", style: { ...style, revision: revision("moveTo") }, source: { at: 0, atom: true } };
  const body = [para("", {}, { start: 0, inlines: [removed("one")] }), para("", {}, { start: 5, inlines: [removed("two")] }), para("", {}, { start: 10, inlines: [moved] })];
  const pages = layout(body, { section: { ...SMALL, page_height: 600 / TWIP } });
  const balloons = markup(pages, new Map(), measurer, (run) => blocks.style(run)).pages[0];
  assert.deepEqual(balloons.map((b) => b.partner), [10, 10, 0]);
});

test("balloons sit level with their lines, and a crowd of them is centred on where they want to be", () => {
  assert.deepEqual(place([{ top: 10, height: 5 }, { top: 40, height: 5 }], 100, 0), [10, 40]);
  // Both want to be 5 above where they can be together: they split the difference.
  assert.deepEqual(place([{ top: 10, height: 10 }, { top: 12, height: 10 }], 100, 0), [6, 16]);
  // A run pushed into the one above it joins it, and the whole run is centred again.
  assert.deepEqual(place([{ top: 0, height: 10 }, { top: 10, height: 10 }, { top: 12, height: 10 }], 100, 0), [0, 10, 20]);
  assert.deepEqual(place([{ top: 95, height: 10 }], 100, 0), [90]);
  // What cannot fit on the page runs off its bottom, not its top.
  assert.deepEqual(place([{ top: 0, height: 60 }, { top: 0, height: 60 }], 100, 0), [0, 60]);
});

test("a formatting change is described as Word describes it", () => {
  assert.equal(describe({}, { bold: true }), "Font: Bold");
  assert.equal(describe({ italic: true, size_half_points: 22 }, { size_half_points: 24 }), "Font: 12 pt, Not Italic");
  assert.equal(describe({}, { underline: "single", color: "FF0000" }), "Font: Underline, Font color: #FF0000");
});

/** A small seeded generator, so a failing run repeats. */
function random(seed: number) {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

test("a layout started again from the last one equals a layout in full, edit after edit", () => {
  const notes = new Map([
    ["footnote:1", [para("note one")]],
    ["footnote:2", [para("note two"), para("more")]],
    ["footnote:3", [para("note three")]],
  ]);
  const numbered = para("", {}, {
    inlines: [
      { kind: "text", text: "9", style: { ...style, pageField: { field: "n1", number: "page" } }, source: { at: 0, atom: true } },
      { kind: "text", text: "/", style, source: { at: 1, atom: false } },
      { kind: "text", text: "9", style: { ...style, pageField: { field: "n2", number: "num_pages" } }, source: { at: 2, atom: true } },
    ],
    fields: true,
  });
  const parts = new Map([["h", { key: null, blocks: [numbered] }]]);
  const section: Section = { ...SMALL, page_height: 120 / TWIP, headers: [{ kind: "default", relationship: "rId1", part: "h" }] };
  const columns: Section = { ...section, start: "continuous", columns: { count: 2, space_twips: 0 } };
  const restart: Section = { ...section, page_numbers: { start: 1 } };
  const settings = { default_tab_stop_twips: 720, even_and_odd_headers: false, update_fields: false };
  for (let seed = 1; seed <= 100; seed++) {
    const rand = random(seed);
    const pick = (n: number) => Math.floor(rand() * n);
    const block = (): Block => {
      if (rand() < 0.08) return table([["Head"], ...Array.from({ length: 1 + pick(4) }, (_, i) => [`r${i} ${"w".repeat(pick(12))}`])], pick(2));
      const words = Array.from({ length: 1 + pick(8) }, () => "x".repeat(1 + pick(6))).join(" ");
      const props = {
        keep_next: rand() < 0.2,
        keep_lines: rand() < 0.1,
        widow_control: rand() < 0.5,
        page_break_before: rand() < 0.03,
        spacing: { ...PROPS.spacing, before_twips: pick(3) * 60, after_twips: pick(3) * 60 },
      };
      const r = rand();
      if (r < 0.04) return para("", props, { section: columns });
      if (r < 0.06) return para("", props, { section: restart });
      if (r < 0.16) {
        const note = `footnote:${1 + pick(3)}`;
        return para("", props, { inlines: [{ kind: "text", text: words, style, source: { at: 0, atom: false }, note }] });
      }
      return para(words, props);
    };
    const input = (blocks: Block[]): LayoutInput => ({
      body: { key: null, blocks },
      section,
      parts: seed % 2 ? parts : new Map(),
      notes,
      comments: new Map(),
      settings,
    });
    let body = Array.from({ length: 20 + pick(60) }, block);
    const incremental = new Paginator(measurer);
    incremental.layout(input(body));
    for (let step = 0; step < 40; step++) {
      body = body.slice();
      const at = pick(body.length + 1);
      const r = rand();
      if (r < 0.4 && body.length) body.splice(Math.min(at, body.length - 1), 1, block());
      else if (r < 0.7 || body.length < 2) body.splice(at, 0, ...Array.from({ length: 1 + pick(3) }, block));
      else body.splice(Math.min(at, body.length - 1), 1 + pick(3));
      assert.deepEqual(incremental.layout(input(body)), new Paginator(measurer).layout(input(body)), `seed ${seed}, step ${step}`);
    }
  }
});

test("an edit that keeps its paragraph's lines leaves the other pages as they were", () => {
  const fixed = { section: SMALL, parts: new Map(), notes: new Map(), comments: new Map(), settings: { default_tab_stop_twips: 720, even_and_odd_headers: false, update_fields: false } };
  const paginator = new Paginator(measurer);
  const body = many(200);
  const before = paginator.layout({ body: { key: null, blocks: body }, ...fixed });
  const edited = body.slice();
  edited[52] = para("q52");
  const after = paginator.layout({ body: { key: null, blocks: edited }, ...fixed });
  assert.equal(after.length, 40);
  assert.deepEqual(texts(after[10]), ["p50", "p51", "q52", "p53", "p54"]);
  assert.deepEqual(after.flatMap((page, i) => (page === before[i] ? [] : [i])), [10]);
});

test("an edited story keeps the blocks an edit left alone, at their new positions", () => {
  const info = { effective: PROPS, mark: { font: "Test", size_half_points: 24 } };
  let id = 0;
  const p = (text: string) => schema.nodes.paragraph.create({ id: `p${id++}`, info }, text ? schema.text(text) : null);
  const inspection = { stories: [{ type: "story", story: "body", id: "body", part: "document.xml" }], settings: {} } as unknown as Inspection;
  const rand = random(7);
  const pick = (n: number) => Math.floor(rand() * n);
  let doc: PMNode = schema.nodes.doc.create(null, Array.from({ length: 30 }, (_, i) => p(`paragraph ${i}`)));
  const editing = new Blocks((family) => family);
  const read = () => editing.layout(inspection, () => doc).body.blocks;
  let last = read();
  for (let step = 0; step < 200; step++) {
    const children: PMNode[] = [];
    doc.forEach((child) => children.push(child));
    const at = pick(children.length);
    const r = rand();
    if (r < 0.5) children[at] = p(`${children[at].textContent}${"x".repeat(1 + pick(5))}`);
    else if (r < 0.75 || children.length < 3) children.splice(at, 0, p("new ".repeat(pick(3))));
    else children.splice(at, 1 + pick(2));
    const kept = new Set(children);
    doc = schema.nodes.doc.create(null, children);
    const blocks = read();
    assert.deepEqual(blocks, new Blocks((family) => family).fromDoc(doc), `step ${step}`);
    // Each node the edit kept keeps its block object.
    const before = new Map(last.map((b) => [(b as Paragraph).key, b]));
    for (const b of blocks) if (kept.has((b as Paragraph).key as PMNode) && before.has(b.key)) assert.equal(b, before.get(b.key));
    last = blocks;
  }
});

test("hidden run content takes no page space and the next visible text keeps its caret offsets", () => {
  const effective = { font: "Test", size_half_points: 24 };
  const hidden = { type: "run", id: "n2", effective: { ...effective, hidden: true }, children: [
    { type: "text", id: "n3", support: "editable", text: "secret" },
    { type: "break", id: "n4", break_type: "page" },
    { type: "drawing", id: "n5", width_emu: 12700, height_emu: 12700 },
  ] };
  const visible = { type: "run", id: "n6", effective, children: [{ type: "text", id: "n7", support: "editable", text: "OK" }] };
  // In the input view the run takes eight positions, or one when held inside a field.
  for (const [first, offset] of [[hidden, 8], [{ type: "field", id: "n8", children: [hidden] }, 1]] as const) {
    const paragraph = { type: "paragraph", id: "n1", effective: PROPS, mark: effective, children: [first, visible] };
    const doc = buildDoc({ type: "story", id: "n0", children: [paragraph] } as unknown as InspectNode);
    const body = blocks.fromDoc(doc);
    const pages = layout(body);
    assert.deepEqual(pages.map(texts), [["OK"]]);
    const line = lines(body[0] as Paragraph, 60)[0];
    assert.ok(line.pieces.every(p => p.inline?.kind !== "box"));
    assert.equal(offsetAt(line, 6, measurer).offset, offset + 1);
  }
});

test("an insertion an edit made is text to edit, drawn as inserted, and a deletion or a document's own insertion is whole", () => {
  const effective = { font: "Test", size_half_points: 24 };
  const run = (id: string, child: object) => ({ type: "run", id, support: "editable", effective, children: [child] });
  const made = { support: "editable", author: "Alice", date: "2026-11-16T09:30:00Z" };
  const story = {
    type: "story",
    id: "n0",
    children: [
      {
        type: "paragraph",
        id: "n1",
        effective: PROPS,
        mark: effective,
        children: [
          run("n2", { type: "text", id: "n3", support: "editable", text: "kept " }),
          { type: "revision", id: "n4", change: "ins", ...made, children: [run("n5", { type: "text", id: "n6", support: "editable", text: "typed" })] },
          { type: "revision", id: "n7", change: "del", ...made, children: [run("n8", { type: "deleted_text", id: "n9", text: "gone" })] },
          { type: "revision", id: "n10", change: "ins", support: "read_only", author: "Bob", children: [run("n11", { type: "text", id: "n12", text: "base" })] },
        ],
      },
    ],
  } as unknown as InspectNode;
  const doc = buildDoc(story);
  const children: PMNode[] = [];
  doc.child(0).forEach((child) => children.push(child));
  assert.deepEqual(children.map(leafOf), ["n3", "n6", undefined, undefined]);
  assert.deepEqual(children.map((c) => c.text ?? c.attrs.id), ["kept ", "typed", "n7", "n10"]);
  assert.deepEqual({ ...insertionOf(children[1]) }, { id: "n4", author: "Alice", date: "2026-11-16T09:30:00Z" });
  assert.equal(insertionOf(children[0]), undefined);

  const [paragraph] = new Blocks((family) => family).fromDoc(doc) as Paragraph[];
  const typed = paragraph.inlines.find((i) => i.kind === "text" && i.text === "typed");
  assert.deepEqual(typed?.kind === "text" && typed.style.revision, { change: "ins", author: "Alice", date: "2026-11-16T09:30:00Z" });
  const anchor = paragraph.inlines.find((i) => i.kind === "anchor");
  assert.equal(anchor?.kind === "anchor" && anchor.removed.revision.change, "del");
});
