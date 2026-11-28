// A ProseMirror editing view over the engine. ProseMirror does not hold the
// document: every edit becomes engine commands first, and an edit the engine
// refuses leaves the view as it was.
//
// The view is the input model. It takes the keyboard, IME composition and the
// clipboard, and the surface (surface.ts) draws its document on pages and
// steers its selection, so the view's own DOM is never seen.
//
// A contiguous typing or composition change becomes one ReplaceRange. Enter, Shift+Enter, and Backspace or Delete at a paragraph edge
// are keys, sent as SplitParagraph, InsertBreak and MergeParagraphs. Deleting,
// typing over or cutting a selection across paragraphs, Enter over a
// selection, and pasting lines are sent as ReplaceRange. Bold, italic and
// underline are sent as FormatRange over the selection, a paragraph style
// as SetParagraphStyle over it or the caret, bullets, numbering and list
// levels as SetList and ChangeListLevel over it or the caret, a link as
// SetLink over it or the link the caret is in, and a comment as AddComment
// over it, or as ReplyToComment. After any command
// that adds or removes nodes the blocks it changed are drawn again from the
// engine, so the view never guesses at structure.
//
// With Track Changes on, typing, Enter, Backspace, Delete, pasting and
// cutting are each one ReplaceRange tracked as the session's author, a
// comment is added as it is otherwise, and other changes are refused.
//
// A Session is where the commands go. ReplicaSession (collab.ts) sends them
// to a replica of the document's editing state, whether one person edits or
// several, and redraws only the blocks a change touched, here or elsewhere.

import { baseKeymap } from "prosemirror-commands";
import { keymap } from "prosemirror-keymap";
import { Fragment, Mark, Slice, type Node as PMNode, type ResolvedPos } from "prosemirror-model";
import {
  AllSelection,
  type Command as PMCommand,
  EditorState,
  type Plugin,
  Selection,
  TextSelection,
  type Transaction,
} from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { atomText, blocks, buildDoc, clipboardSerializer, drawn, insertionOf, invisibleInline, leafOf, removedRevision, schema } from "./model.ts";
import type { Resolution, Command, EffectiveRun, Fill, InspectNode, NodeId, Pages, Point, Settings, Splice, Style, Track } from "./types.ts";

const STRUCTURE =
  "That edit adds or removes paragraphs or table parts in a way the editor cannot apply. Select text within the paragraphs you want to change.";
const ORDER = "That edit would move text or items between places, which the engine cannot express yet.";
const TABLE = "A table is next to this paragraph. No command joins a paragraph with a table yet.";
const UNSELECTED = "Select the text to format first. Formatting the text you type next is not built yet.";
export const UNLINKED = "Select the text to link first.";
const UNTABLED = "Put the caret in a table cell first.";
const UNPLACED = "Put the caret in a paragraph first.";
const UNPASTED = "Nothing on the clipboard can be pasted here. Text and PNG, JPEG or GIF images can.";
const TRACKING =
  "Track Changes records typing and deleting. Turn it off to change formatting, styles, lists, links, tables or pictures, to fill in a form field, to update fields, or to break a line.";
const BESIDE = "A table or other object is next to this paragraph. Deleting it is not tracked yet.";
/** The images a document can hold that `add_media` takes. */
const PICTURES = /^image\/(png|jpeg|gif)$/;

export const FORMATTING = ["bold", "italic", "underline"] as const;
export type Formatting = (typeof FORMATTING)[number];

/** Whether text in `run` shows `property`. An underline is named by its style, and "none" is none. */
function shows(run: Mark | undefined, property: Formatting): boolean {
  const value = (run?.attrs.info.effective as EffectiveRun | undefined)?.[property];
  return property === "underline" ? !!value && value !== "none" : !!value;
}

/**
 * Whether all the selected text shows each property, or, with nothing
 * selected, the text at the caret. A toolbar shows these as pressed.
 */
export function formatted(state: EditorState): Record<Formatting, boolean> {
  const { from, to, empty, $from } = state.selection;
  const runs: (Mark | undefined)[] = [];
  if (empty) {
    runs.push(schema.marks.run.isInSet($from.marks()));
  } else {
    state.doc.nodesBetween(from, to, (node) => {
      if (node.isText) runs.push(schema.marks.run.isInSet(node.marks));
    });
  }
  const all = (property: Formatting) => runs.length > 0 && runs.every((run) => shows(run, property));
  return { bold: all("bold"), italic: all("italic"), underline: all("underline") };
}

/**
 * The style in effect in every paragraph the selection touches, or in the
 * caret's: "" in a document without styles, and null when they differ.
 */
export function paragraphStyle(state: EditorState): string | null {
  const { from, to } = state.selection;
  const styles = new Set<string>();
  state.doc.nodesBetween(from, to, (node) => {
    if (node.type !== schema.nodes.paragraph) return true;
    styles.add(node.attrs.info.effective_style ?? "");
    return false;
  });
  return styles.size === 1 ? [...styles][0] : null;
}

/** A link in a view: where it points, empty for nowhere, and the positions it spans in its paragraph. */
export interface Link {
  href: string;
  from: number;
  to: number;
}

const linkOf = (node: PMNode | null | undefined) => (node ? schema.marks.link.isInSet(node.marks) : undefined);

/**
 * The link holding the content from `$from` to `$to` in one paragraph, or,
 * where they are one position, the link on either side of it.
 */
export function linkAround($from: ResolvedPos, $to: ResolvedPos = $from): Link | null {
  if (!$from.sameParent($to) || $from.parent.type !== schema.nodes.paragraph) return null;
  const mark = $from.pos === $to.pos ? (linkOf($from.nodeAfter) ?? linkOf($from.nodeBefore)) : linkOf($from.nodeAfter);
  if (!mark) return null;
  // A link's content is the paragraph's children that carry its mark, side by side.
  const start = $from.start();
  let found: Link | null = null;
  let open: Link | null = null;
  $from.parent.forEach((child, offset) => {
    const at = start + offset;
    if (linkOf(child)?.attrs.id !== mark.attrs.id) {
      open = null;
      return;
    }
    if (open) open.to = at + child.nodeSize;
    else open = { href: mark.attrs.target ?? "", from: at, to: at + child.nodeSize };
    if (open.from <= $from.pos && $to.pos <= open.to) found = open;
  });
  return found;
}

/** The link holding the selection, or the one the caret is in or beside. A toolbar shows it as pressed. */
export const linkAt = (state: EditorState): Link | null => linkAround(state.selection.$from, state.selection.$to);

export const LISTS = ["bullet", "number"] as const;
export type ListKind = (typeof LISTS)[number];

export const TABLE_EDITS = [
  "row_above",
  "row_below",
  "column_left",
  "column_right",
  "remove_row",
  "remove_column",
  "remove_table",
] as const;
export type TableEdit = (typeof TABLE_EDITS)[number];

/** The table cell holding the caret, with its row and table, or null outside a table. */
export function cellAt(state: EditorState): { cell: PMNode; row: PMNode; table: PMNode } | null {
  const { $head } = state.selection;
  for (let depth = $head.depth; depth > 2; depth--) {
    if ($head.node(depth).type !== schema.nodes.cell) continue;
    return { cell: $head.node(depth), row: $head.node(depth - 1), table: $head.node(depth - 2) };
  }
  return null;
}

/** The kind of list a paragraph is in, read as the engine reads it: from its level's marker format. */
function listOf(paragraph: PMNode): ListKind | null {
  const format: string | undefined = paragraph.attrs.list?.definition?.format;
  if (format === undefined) return null;
  return format === "bullet" ? "bullet" : "number";
}

/**
 * The kind of list every paragraph the selection touches, or the caret's,
 * is in: null when one is in no list or they differ. A toolbar shows it as pressed.
 */
export function listKind(state: EditorState): ListKind | null {
  const { from, to } = state.selection;
  const kinds = new Set<ListKind | null>();
  state.doc.nodesBetween(from, to, (node) => {
    if (node.type !== schema.nodes.paragraph) return true;
    kinds.add(listOf(node));
    return false;
  });
  return kinds.size === 1 ? [...kinds][0] : null;
}

/**
 * What the table reader kept against someone's edit, in words for the person
 * looking at the table. The reader reports a reason for each table it held
 * back; a reason this list does not know is shown as the reader gave it.
 */
export function resolutionNotes(reasons: readonly Resolution[]): string[] {
  const message: Record<Resolution, string> = {
    merge_start_wrapper: "A content control is visible because a merged cell below still needs it.",
    merge_start_row: "A row is visible because a merged cell below still needs it.",
    last_row: "A table keeps its last row.",
    last_column: "A table keeps its last column.",
    source_grid: "A table shows its original columns because a row does not fit the changed ones.",
  };
  return [...new Set(reasons)].map((r) => message[r]);
}

/**
 * Word's built-in "heading N" style, found by the name Word gives it in every
 * language, or the default paragraph style for level 0.
 */
export function builtIn(styles: Style[], level: number): Style | undefined {
  const paragraph = styles.filter((s) => s.kind === "paragraph");
  if (level === 0) return paragraph.find((s) => s.is_default);
  return paragraph.find((s) => s.name?.toLowerCase() === `heading ${level}`);
}

/** The local date and time, written as `update_fields` takes it. */
function localNow(): string {
  const now = new Date();
  const two = (n: number) => String(n).padStart(2, "0");
  const date = `${String(now.getFullYear()).padStart(4, "0")}-${two(now.getMonth() + 1)}-${two(now.getDate())}`;
  return `${date}T${two(now.getHours())}:${two(now.getMinutes())}:${two(now.getSeconds())}`;
}

/** An engine error as a notice: the node's ID means nothing to a reader. */
function explain(e: unknown): string {
  const text = (e as Error).message.replace(/^n\d+: /, "").replace(/^n\d+ /, "this item ");
  return `${text[0].toUpperCase()}${text.slice(1).replace(/\.$/, "")}.`;
}


interface Located {
  node: PMNode;
  pos: number;
}

/**
 * Where a view change is: `before` from `start` to `endA` became `after`
 * from `start` to `endB`. Null if nothing changed. `caret` is where the
 * caret is in `after`: just after what was typed, or where something was
 * deleted. Comparing the two views alone puts a change among equal
 * characters at their end, so when everything after the caret is as it was,
 * the change ends at the caret. Typing with Track Changes on and off both
 * place a change with this.
 */
export function changedRange(before: PMNode, after: PMNode, caret: number): { start: number; endA: number; endB: number } | null {
  let start = before.content.findDiffStart(after.content);
  if (start == null) return null;
  let { a: endA, b: endB } = before.content.findDiffEnd(after.content)!;
  const overlap = start - Math.min(endA, endB);
  if (overlap > 0) {
    endA += overlap;
    endB += overlap;
  }
  const back = endB - caret;
  if (back > 0 && back <= start && before.content.cut(endA - back).eq(after.content.cut(endB - back))) {
    start -= back;
    endA -= back;
    endB -= back;
  }
  return { start, endA, endB };
}

/** Select editable paragraph endpoints, keeping a table or control at either edge whole. */
export function storySelection(all: AllSelection): Selection {
  const doc = all.$from.doc;
  // Section properties follow the body text and occupy one preserved block.
  const last = doc.lastChild?.attrs.node?.type === "section" ? doc.maybeChild(doc.childCount - 2) : doc.lastChild;
  if (!doc.firstChild?.isTextblock || !last?.isTextblock) return all;
  return TextSelection.create(doc, Selection.atStart(doc).from, Selection.atEnd(doc).to);
}

/** The engine's name for a paragraph caret or the boundary of a whole story. */
export function pointAt($pos: ResolvedPos, after = false): Point | null {
  const paragraph = $pos.parent;
  if (paragraph.type === schema.nodes.doc) {
    if (!$pos.nodeBefore && $pos.nodeAfter) return { before: $pos.nodeAfter.attrs.id };
    if (!$pos.nodeAfter && $pos.nodeBefore) return { after: $pos.nodeBefore.attrs.id };
  }
  if (paragraph.type !== schema.nodes.paragraph) return null;
  const [first, second] = after ? [$pos.nodeAfter, $pos.nodeBefore] : [$pos.nodeBefore, $pos.nodeAfter];
  const leaf = leafOf(first) ?? leafOf(second);
  if (leaf) {
    let offset = 0;
    paragraph.forEach((child, at) => {
      if (leafOf(child) === leaf && at < $pos.parentOffset) {
        offset += [...child.text!.slice(0, $pos.parentOffset - at)].length;
      }
    });
    return { node: leaf, offset };
  }
  const id: NodeId = paragraph.attrs.id;
  const { nodeBefore, nodeAfter } = $pos;
  if (!nodeBefore) return { start: id };
  if (!nodeAfter) return { end: id };
  // Text without a leaf is mid-composition and not in the engine yet.
  if (nodeBefore.isText) return null;
  return { after: nodeBefore.attrs.id };
}

/** Build the slice shared by the plain-text and HTML clipboard writers. */
export function clipboardContent(content: Fragment, fieldText?: (leaf: NodeId) => string | undefined): Fragment {
  function displayed(node: InspectNode): InspectNode {
    if (node.type === "text" && node.page_field) {
      return { ...node, text: fieldText?.(node.id) ?? node.text };
    }
    return node.children ? { ...node, children: node.children.map(displayed) } : node;
  }
  const visible: PMNode[] = [];
  content.forEach((node) => {
    if (!schema.marks.run.isInSet(node.marks)?.attrs.info.effective?.hidden) {
      if (fieldText && node.attrs.node) {
        visible.push(node.type.create({ ...node.attrs, node: displayed(node.attrs.node) }, null, node.marks));
      } else {
        visible.push(node.isLeaf ? node : node.copy(clipboardContent(node.content, fieldText)));
      }
    }
  });
  return Fragment.fromArray(visible);
}

export const copiedText = (content: Fragment): string =>
  content.textBetween(0, content.size, "\n", (leaf) => atomText(leaf.attrs.node, true));

/** Whether the selection goes beyond one paragraph, which typing cannot change. */
function spans(state: EditorState): boolean {
  const { $from, $to, empty } = state.selection;
  return !empty && (state.selection instanceof AllSelection || !$from.sameParent($to));
}

/** A range of a view, as the engine names its ends. */
interface Span {
  from: Point;
  to: Point;
  at: [number, number];
}

/**
 * When the change `author` is making beside a range was made: typing or
 * deleting on from it then joins that change, as in Word. Otherwise now, to
 * the second, as Word writes it.
 */
function dated(author: string, nodes: (PMNode | null | undefined)[]): string {
  for (const node of nodes) {
    const inserted = insertionOf(node);
    if (inserted?.author === author && inserted.date) return inserted.date;
    const info = node?.type === schema.nodes.inline_atom ? (node.attrs.node as InspectNode) : undefined;
    if (info?.type === "revision" && info.support === "editable" && info.author === author && info.date) return info.date;
  }
  return new Date().toISOString().replace(/\.\d+Z$/, "Z");
}

/**
 * A view change as one tracked replacement: the text that replaced a range
 * of one paragraph. Null if nothing changed, and why not if it is not one.
 * `caret` is where the caret is in `after`.
 */
export function textChange(before: PMNode, after: PMNode, caret: number): (Span & { text: string }) | string | null {
  const range = changedRange(before, after, caret);
  if (range == null) return null;
  const { start, endA, endB } = range;
  const [$from, $to] = [before.resolve(start), before.resolve(endA)];
  const [$start, $end] = [after.resolve(start), after.resolve(endB)];
  if (!$from.sameParent($to) || !$start.sameParent($end) || $from.parent.type !== schema.nodes.paragraph) return STRUCTURE;
  const added = $start.parent.content.cut($start.parentOffset, $end.parentOffset);
  let text = true;
  added.forEach((node) => void (text &&= node.isText));
  if (!text) return ORDER;
  const from = pointAt($from, endA > start);
  const to = pointAt($to);
  if (!from || !to) return STRUCTURE;
  return { from, to, at: [start, endA], text: added.textBetween(0, added.size) };
}

export interface EditorEvents {
  refused(reason: string): void;
  /**
   * The focused editor's selection, or the formatting it shows, may have
   * changed. `style` is as `paragraphStyle` gives it, `list` as `listKind`
   * does, and `link` is where `linkAt`'s link points, or null outside one.
   */
  formatting?(
    shown: Record<Formatting, boolean>,
    style: string | null,
    list: ListKind | null,
    link: string | null,
    inTable: boolean,
  ): void;
  /** ⌘K: someone asks to link the selection, or to change the link the caret is in. */
  linking?(): void;
  /** ⌘⌥M: someone asks to comment on the selection, or on the word at the caret. */
  commenting?(): void;
}

/** Where the caret goes after a redraw: a position, or `"change"` for where the redraw's last change ends. */
type Select = number | "change";
type Redraw = (select?: Select) => void;

/**
 * Places the caret as `select` asks and brings it into view. A redraw that
 * replaced nothing, as one that only set marks, keeps the caret it has.
 */
function reveal(tr: Transaction, select: Select) {
  let at = select === "change" ? undefined : select;
  if (select === "change") {
    tr.mapping.maps.forEach((map, i) =>
      map.forEach((_from, _to, _start, end) => void (at = tr.mapping.slice(i + 1).map(end))),
    );
  }
  if (at !== undefined) tr.setSelection(Selection.near(tr.doc.resolve(Math.min(at, tr.doc.content.size))));
  if (at !== undefined || tr.steps.length) tr.scrollIntoView();
}

/** Someone else's caret, at a position in a story's view, and the other end of their selection. */
export interface Caret {
  at: number;
  anchor?: number;
  name: string;
  color: string;
}

export interface Editor {
  /** The story it shows. */
  story: NodeId;
  view: EditorView;
  /** Draws the whole story again, and places the caret at `select` if given. */
  redraw: Redraw;
  /** Replaces the blocks the splices name, and places the caret at `select` if given. */
  splice(splices: Splice[], select?: Select): void;
  /** Where a point in one of its paragraphs is in its view, or null if the paragraph is elsewhere. */
  position(paragraph: NodeId, point: Point): number | null;
  /** Shows other people's carets and selections. */
  carets(carets: Caret[]): void;
  /** Turns a property on for the selection, or off if all of it has it. */
  format(property: Formatting): void;
  /** Gives the paragraphs the selection touches, or the caret's, the style with this ID. */
  style(style: string): void;
  /** Makes the paragraphs the selection touches, or the caret's, a list of this kind, or no list if all are. */
  list(kind: ListKind): void;
  /** Applies or removes a link over exactly this selection. */
  link(target: string | null, selection: Selection): boolean;
  /** Asks, as ⌘K does, for a link for the selection or the caret's link, or explains why there is none to ask for. */
  startLink(): void;
  /** Comments on the selection, or at the caret, as `by`. */
  comment(text: string, by: Track): boolean;
  /** Asks, as ⌘⌥M does, for a comment on the selection or on the word at the caret. */
  startComment(): void;
  /** Replies to the comment with this ID, as `by`. */
  reply(comment: string, text: string, by: Track): boolean;
  /** Asks Word to update the document's fields when it opens the file, or stops asking. */
  updateFields(on: boolean): void;
  /** Fills the content control with this ID as a form does. */
  fill(control: NodeId, fill: Fill): boolean;
  /** Updates the body's fields as Word does when every field is selected and updated, as of now, with each paragraph on its page in `pages`. */
  updateFieldResults(pages: Pages): void;
  /** Adds or removes a row or column beside the caret's cell, or removes its table. */
  table(edit: TableEdit): void;
  /**
   * Puts a PNG, JPEG or GIF image this many EMUs wide and high at the supplied caret
   * or in place of the supplied range.
   */
  picture(bytes: Uint8Array, widthEmu: number, heightEmu: number, range: { anchor: number; head: number }): void;
}

/** The document the editors of its stories edit. */
export interface Session {
  readonly events: EditorEvents;
  /** Who changes are tracked as, or null while Track Changes is off. */
  tracking: string | null;
  inspectNode(node: NodeId): InspectNode;
  inspectChildren(parent: NodeId, start: number, end: number): InspectNode[];
  settings(): Settings;
  text(node: NodeId): string | undefined;
  /** Applies commands in order as one change, all of them or none. */
  apply(commands: Command[], media?: Uint8Array): void;
  /** Validates and names an image; apply receives its bytes with the insertion. */
  mediaName(bytes: Uint8Array): string;
  /** A caret as a token that stays beside the same content while the document changes, if the point is in the document. */
  sticky(point: Point): string | undefined;
  /** Where a caret from `sticky` is now, if it is in the document. */
  resolve(token: string): { paragraph: NodeId; point: Point } | undefined;
  /**
   * An editor's change was applied. The session brings the editors up to
   * date, and `select` is where that editor's caret goes.
   */
  record(editor: Editor, select?: number): void;
  undo(): boolean;
  redo(): boolean;
  attach(editor: Editor): void;
  /** An editor's IME composition ended, and its text was applied. */
  resume(): void;
  /** Called after a change with the stories it changed that no editor shows, or "all" after a redraw of everything. */
  touched?: (stories: NodeId[] | "all") => void;
  /**
   * The focused editor's selection moved: `head` is where the caret is, and
   * `anchor` the other end when it spans content. Each is null where the
   * engine has no name for it.
   */
  selected?(head: Point | null, anchor: Point | null): void;
}

/** A selection's ends as `Session.sticky` gives them. */
export interface Held {
  anchor: string;
  head: string;
}

/**
 * Holds a selection's place for something that finishes later, such as
 * reading a pasted image, or null where the engine has no name for an end.
 */
export function hold(selection: Selection, session: Pick<Session, "sticky">): Held | null {
  const token = ($pos: ResolvedPos) => {
    const point = pointAt($pos);
    return point && session.sticky(point);
  };
  const anchor = token(selection.$anchor);
  const head = token(selection.$head);
  return anchor && head ? { anchor, head } : null;
}

const SYNC = "docx-engine-sync";

/** A copy with only node types and text: no attributes or marks. */
function shape(node: PMNode): PMNode {
  if (node.isText) return schema.text(node.text!);
  const children: PMNode[] = [];
  node.forEach((child) => children.push(shape(child)));
  return node.type.create(Object.fromEntries(Object.keys(node.attrs).map((key) => [key, null])), children);
}

/**
 * Replaces the part of `tr.doc` whose shape differs from `fresh`, comparing
 * `now` and `want`, their shapes. Blocks added or removed are inserted or
 * deleted whole, and where a single block changed the comparison goes on
 * inside it, so a caret outside the change keeps its place.
 */
function patch(tr: Transaction, now: PMNode, want: PMNode, fresh: PMNode, start: number) {
  if (fresh.inlineContent) {
    const from = now.content.findDiffStart(want.content);
    if (from == null) return;
    let { a, b } = now.content.findDiffEnd(want.content)!;
    const overlap = from - Math.min(a, b);
    if (overlap > 0) {
      a += overlap;
      b += overlap;
    }
    tr.replaceWith(start + from, start + a, fresh.content.cut(from, b));
    return;
  }
  const same = (i: number, j: number) => now.child(i).eq(want.child(j));
  let head = 0;
  while (head < now.childCount && head < want.childCount && same(head, head)) head++;
  let tail = 0;
  while (
    tail < now.childCount - head &&
    tail < want.childCount - head &&
    same(now.childCount - 1 - tail, want.childCount - 1 - tail)
  ) {
    tail++;
  }
  let from = start;
  for (let i = 0; i < head; i++) from += now.child(i).nodeSize;
  const removed = now.childCount - tail - head;
  const added = want.childCount - tail - head;
  const block = now.maybeChild(head);
  if (removed === 1 && added === 1 && block && !block.isLeaf && block.sameMarkup(want.child(head))) {
    patch(tr, block, want.child(head), fresh.child(head), from + 1);
    return;
  }
  let to = from;
  for (let i = head; i < head + removed; i++) to += now.child(i).nodeSize;
  const nodes: PMNode[] = [];
  for (let i = head; i < head + added; i++) nodes.push(fresh.child(i));
  tr.replaceWith(from, to, nodes);
}

/**
 * Gives the nodes and text of `tr.doc` from `start` on the attributes and
 * marks they have in `fresh`'s content, which has the same shape.
 *
 * The two are walked side by side, so the cost follows the size of `fresh`
 * and not the document's. Blocks line up child by child; inside a paragraph
 * the text can be split into nodes differently, so it is compared by offset.
 */
function relabel(tr: Transaction, fresh: PMNode, start = 0) {
  const changes: (() => void)[] = [];
  const walk = (now: PMNode, first: number, want: PMNode, start: number) => {
    want.forEach((node, offset, index) => {
      const pos = start + offset;
      const end = pos + node.nodeSize;
      if (node.isText) {
        let same = true;
        now.nodesBetween(offset, offset + node.nodeSize, (was) => {
          if (was.isText && !Mark.sameSet(was.marks, node.marks)) same = false;
          return same;
        });
        if (!same) {
          changes.push(() => {
            tr.removeMark(pos, end, null);
            for (const mark of node.marks) tr.addMark(pos, end, mark);
          });
        }
        return;
      }
      const was = want.inlineContent ? now.nodeAt(offset)! : now.child(first + index);
      if (!was.hasMarkup(node.type, node.attrs, node.marks)) {
        changes.push(() => tr.setNodeMarkup(pos, node.type, node.attrs, node.marks));
      }
      walk(was, 0, node, pos + 1);
    });
  };
  const $start = tr.doc.resolve(start);
  walk($start.parent, $start.index(), fresh, start);
  // None of these moves a position, so each applies where it was found.
  for (const change of changes) change();
}

/** The story, or the table cell in it, a splice names, and where its content starts. */
function container(doc: PMNode, story: NodeId, flow: NodeId): Located | null {
  if (flow === story) return { node: doc, pos: 0 };
  let found: Located | null = null;
  doc.descendants((node, pos) => {
    if (found) return false;
    if (node.type === schema.nodes.cell && node.attrs.id === flow) found = { node, pos: pos + 1 };
    return node.type === schema.nodes.table || node.type === schema.nodes.row || node.type === schema.nodes.cell;
  });
  return found;
}

/** Where `point` is in `paragraph`, whose content starts at `start`. */
export function inParagraph(paragraph: PMNode, start: number, point: Point, session: Pick<Session, "inspectNode">): number | null {
  if ("start" in point) return start;
  if ("end" in point) return start + paragraph.content.size;
  let at: number | null = null;
  let left = "node" in point ? point.offset : 0;
  paragraph.forEach((child, offset) => {
    if (at !== null) return;
    if ("node" in point) {
      if (leafOf(child) !== point.node) return;
      const chars = [...child.text!];
      if (left <= chars.length) at = start + offset + chars.slice(0, left).join("").length;
      else left -= chars.length;
    } else if ("after" in point && child.attrs.id === point.after) {
      at = start + offset + child.nodeSize;
    } else if ("before" in point && child.attrs.id === point.before) {
      at = start + offset;
    }
  });
  if (at !== null || ("node" in point && point.offset !== 0)) return at;

  // An empty text leaf, field delimiter or bookmark can sit at a visible edge.
  // Walk the source in order, counting only children the paragraph actually drew.
  const target = "node" in point ? point.node : "after" in point ? point.after : point.before;
  const ends = new Map<NodeId, number>();
  paragraph.forEach((child, offset) => ends.set(leafOf(child) ?? child.attrs.id, start + offset + child.nodeSize));
  let edge = start;
  const find = (node: InspectNode): boolean => {
    if (node.id === target) {
      return "node" in point
        ? node.type === "text" && node.support === "editable" && node.text === ""
        : invisibleInline(node);
    }
    const end = ends.get(node.id);
    if (end === undefined) return node.children?.some(find) ?? false;
    edge = end;
    return false;
  };
  return find(session.inspectNode(paragraph.attrs.id)) ? edge : null;
}

export interface EditorOptions {
  /** Plugins of whatever draws the view and steers its selection. */
  plugins?: Plugin[];
  /** Shows other people's carets and selections. */
  carets?(carets: Caret[]): void;
  /** The document's styles, for the keys that give a heading or the default style. */
  styles?: Style[];
  /** Text actually laid out for page-field result leaves in the story being copied. */
  clipboardFields?(): ReadonlyMap<NodeId, string>;
  /** Sizes and places an image pasted into this story. */
  pasteImage?(image: Blob): void;
}

/** A view of one story, drawn from the session's inspection of it. */
export function createEditor(mount: HTMLElement, story: InspectNode, session: Session, options: EditorOptions = {}) {
  const { events } = session;
  const draw = () => buildDoc(session.inspectNode(story.id));
  // The last document the engine agrees with. Edits made while an IME is
  // composing are committed against it when the composition ends.
  let synced = buildDoc(story);

  /** Replaces what differs from the session's story, and places the caret at `select` if given. */
  const redraw: Redraw = (select) => {
    const fresh = draw();
    const tr = view.state.tr.setMeta(SYNC, true);
    if (!tr.doc.eq(fresh)) {
      // Only types and text are compared, and attributes and marks are set in
      // place: a list item added above renumbers the items below it, and a
      // shared session's view can renumber the nodes it created after any
      // change. Replacing a paragraph for that would move the caret out of it.
      patch(tr, shape(tr.doc), shape(fresh), fresh, 0);
      relabel(tr, fresh);
    }
    if (select !== undefined) reveal(tr, select);
    view.dispatch(tr);
  };

  /**
   * Replaces the blocks each splice names. They are compared first, as in
   * `redraw`, so a caret in a block that kept its shape keeps its place.
   */
  const splice = (splices: Splice[], select?: Select) => {
    const tr = view.state.tr.setMeta(SYNC, true);
    const block = blocks();
    // A table drawn again here already shows its cells as they are now.
    const drawn = new Set<NodeId>();
    for (const { flow, at, removed, blocks: ids } of splices) {
      if (drawn.has(flow)) continue;
      const parent = container(tr.doc, story.id, flow);
      // A cell a vertical merge hides is not drawn.
      if (!parent || at + removed > parent.node.childCount) return redraw(select);
      let from = parent.pos;
      for (let i = 0; i < at; i++) from += parent.node.child(i).nodeSize;
      const old: PMNode[] = [];
      for (let i = at; i < at + removed; i++) old.push(parent.node.child(i));
      const fresh = session.inspectChildren(flow, at, at + ids.length).map(block);
      for (const node of fresh) node.descendants((n) => void (n.type === schema.nodes.cell && drawn.add(n.attrs.id)));
      const want = schema.nodes.doc.create(null, fresh);
      patch(tr, shape(schema.nodes.doc.create(null, old)), shape(want), want, from);
      relabel(tr, want, from);
    }
    if (select !== undefined) reveal(tr, select);
    view.dispatch(tr);
  };

  const position = (paragraph: NodeId, point: Point) => {
    let found: number | null = null;
    view.state.doc.descendants((node, pos) => {
      if (found !== null) return false;
      if (node.type !== schema.nodes.paragraph) return !node.isAtom;
      if (node.attrs.id === paragraph) found = inParagraph(node, pos + 1, point, session);
      return false;
    });
    return found;
  };

  const carets = options.carets ?? (() => {});

  /** Whether Track Changes is off, so a change it does not record may go ahead; if not, says so. */
  const untracked = () => {
    if (session.tracking) events.refused(TRACKING);
    return !session.tracking;
  };

  const run = (command: Command | Command[], select?: number) => {
    if (untracked()) apply(Array.isArray(command) ? command : [command], select);
  };

  /** Applies commands whatever Track Changes says, as a comment, which it does not record, is. */
  const apply = (commands: Command[], select?: number, media?: Uint8Array) => {
    try {
      session.apply(commands, media);
    } catch (e) {
      events.refused(explain(e));
      return false;
    }
    session.record(editor, select);
    return true;
  };

  /**
   * Replaces the selection, or `start` to `stop`, with text whose line feeds
   * start paragraphs. Text typed over a selection takes the formatting of its
   * first character, as in Word, and pasted at a caret that of the text
   * before it.
   */
  const replace = (state: EditorState, text: string, start = state.selection.from, stop = state.selection.to): boolean => {
    const [$from, $to] = [state.doc.resolve(start), state.doc.resolve(stop)];
    const from = pointAt($from, start !== stop);
    const to = pointAt($to);
    if (!from || !to) return false;
    const lines = text.split("\n");
    // Each line feed closes one paragraph and opens the next.
    const end = $from.pos + lines.reduce((n, line) => n + line.length, 0) + 2 * (lines.length - 1);
    run({ command: "replace_range", from, to, text }, end);
    return true;
  };

  /** Bold, italic or underline: off if all the selected text has it, and otherwise on, as in Word. */
  const format =
    (property: Formatting): PMCommand =>
    (state) => {
      if (state.selection.empty) {
        events.refused(UNSELECTED);
        return true;
      }
      const { $from, $to } = state.selection;
      const from = pointAt($from, true);
      const to = pointAt($to);
      // The text does not change, so the redrawn blocks keep the selection.
      if (from && to) run({ command: "format_range", from, to, format: { [property]: !formatted(state)[property] } });
      return true;
    };

  const style =
    (id: string): PMCommand =>
    (state) => {
      const { $from, $to } = state.selection;
      const from = pointAt($from, true);
      const to = pointAt($to);
      if (from && to) run({ command: "set_paragraph_style", from, to, style: id });
      return true;
    };

  /** ⌘⌥1 to ⌘⌥3 give a heading and ⌘⌥0 the default style, as in Google Docs. */
  const level =
    (n: number): PMCommand =>
    (state, dispatch, view) => {
      const found = builtIn(options.styles ?? [], n);
      if (!found) {
        events.refused(`This document has no ${n ? `Heading ${n}` : "default paragraph"} style.`);
        return true;
      }
      return style(found.id)(state, dispatch, view);
    };

  /** Bullets or numbering: off if every paragraph is already that kind of list, and otherwise on, as in Word. */
  const list =
    (kind: ListKind): PMCommand =>
    (state) => {
      const { $from, $to } = state.selection;
      const from = pointAt($from, true);
      const to = pointAt($to);
      if (from && to) run({ command: "set_list", from, to, kind: listKind(state) === kind ? null : kind });
      return true;
    };

  /**
   * Tab and Shift+Tab move list paragraphs a level in or out when the caret
   * is at the start of one or the selection spans paragraphs, as in Word.
   */
  const indent =
    (by: 1 | -1): PMCommand =>
    (state) => {
      const { $from, $to, from: start, to: end, empty } = state.selection;
      let listed = false;
      state.doc.nodesBetween(start, end, (node) => {
        if (node.type !== schema.nodes.paragraph) return true;
        listed ||= listOf(node) !== null;
        return false;
      });
      if (!listed || !(spans(state) || (empty && $from.parentOffset === 0))) return false;
      const from = pointAt($from, true);
      const to = pointAt($to);
      if (from && to) run({ command: "change_list_level", from, to, by });
      return true;
    };

  const link = (target: string | null, selection: Selection) => {
    if (!untracked()) return false;
    const from = pointAt(selection.$from, true);
    const to = pointAt(selection.$to);
    if (!from || !to || selection.empty) {
      events.refused(UNLINKED);
      return false;
    }
    return apply([{ command: "set_link", from, to, target }]);
  };

  const startLink: PMCommand = () => {
    if (untracked()) events.linking?.();
    return true;
  };

  /** A comment on the selection, or at the caret, as Word adds one at the insertion point. */
  const comment =
    (text: string, by: Track): PMCommand =>
    (state) => {
      const { $from, $to, empty } = state.selection;
      const from = pointAt($from, !empty);
      const to = pointAt($to);
      if (from && to) return apply([{ command: "add_comment", from, to, text, by }]);
      events.refused(UNPLACED);
      return false;
    };

  const startComment: PMCommand = () => {
    events.commenting?.();
    return true;
  };

  /**
   * As Word's table menu does: a new row or column copies the caret's, and
   * a column is the grid column the caret's cell starts in, or for one to
   * its right, the last it covers.
   */
  const table =
    (edit: TableEdit): PMCommand =>
    (state) => {
      const found = cellAt(state);
      if (!found) {
        events.refused(UNTABLED);
        return true;
      }
      const row = found.row.attrs.id;
      const tableId = found.table.attrs.id;
      const { column, colspan: span } = found.cell.attrs;
      const commands: Record<TableEdit, Command> = {
        row_above: { command: "insert_row", row, below: false },
        row_below: { command: "insert_row", row, below: true },
        column_left: { command: "insert_column", table: tableId, index: column, like: column },
        column_right: { command: "insert_column", table: tableId, index: column + span, like: column + span - 1 },
        remove_row: { command: "remove_node", node: row },
        remove_column: { command: "remove_column", table: tableId, index: column },
        remove_table: { command: "remove_node", node: tableId },
      };
      run(commands[edit]);
      return true;
    };

  /** One change, as in Word, so one undo takes the picture out and gives the selection back. */
  const picture =
    (bytes: Uint8Array, width_emu: number, height_emu: number, range: { anchor: number; head: number }): PMCommand =>
    (state) => {
      if (!untracked()) return true;
      const selection = TextSelection.create(state.doc, range.anchor, range.head);
      const { $from, $to, empty } = selection;
      const from = pointAt($from, !empty);
      const to = pointAt($to);
      if (!from || !to) {
        events.refused(UNPLACED);
        return true;
      }
      let media: string;
      try {
        media = session.mediaName(bytes);
      } catch (e) {
        events.refused(explain(e));
        return true;
      }
      const insert: Command = { command: "insert_picture", at: from, ...(empty ? {} : { to }), media, width_emu, height_emu };
      // A picture is one atom wide. A selection that is no longer the range
      // is where the person works now, so it stays.
      apply([insert], selection.eq(state.selection) ? $from.pos + 1 : undefined, bytes);
      return true;
    };

  /** Takes the caret's paragraph out of its list. */
  const unlist = ($cursor: ResolvedPos) => {
    const at = pointAt($cursor);
    if (at) run({ command: "set_list", from: at, to: at, kind: null });
  };

  /**
   * Replaces a range of `before`, the view it is in, with `text`, tracked as
   * the session's author: what they inserted is removed, anything else is
   * marked deleted, and the text is marked inserted. The caret goes before
   * the range for Backspace, and otherwise after what replaced it.
   */
  const track = (before: PMNode, { from, to, at }: Span, text: string, back = false) => {
    const author = session.tracking!;
    const [$from, $to] = [before.resolve(at[0]), before.resolve(at[1])];
    const date = dated(author, [$from.nodeBefore, $to.nodeAfter, $from.nodeAfter, $to.nodeBefore]);
    // Nothing after the range changes, so its distance from the end holds.
    const tail = before.content.size - at[1];
    try {
      session.apply([{ command: "replace_range", from, to, text, track: { author, date } }]);
    } catch (e) {
      events.refused(explain(e));
      if (!view.state.doc.eq(synced)) redraw();
      return;
    }
    session.record(editor);
    const { doc } = view.state;
    const caret = Math.max(0, Math.min(back ? at[0] : doc.content.size - tail, doc.content.size));
    view.dispatch(view.state.tr.setSelection(Selection.near(doc.resolve(caret))).scrollIntoView());
  };

  /** Replaces `from`..`to` of the view with `text` as a tracked change, if the engine has names for both ends. */
  const typed = (from: number, to: number, text: string, back = false): boolean => {
    const { doc } = view.state;
    const start = pointAt(doc.resolve(from), from !== to);
    const end = pointAt(doc.resolve(to));
    if (!start || !end) return false;
    track(doc, { from: start, to: end, at: [from, to] }, text, back);
    return true;
  };

  /** Backspace, or Delete going `forward`: deletes the selection, or what is beside the caret, as a tracked change. */
  const erase =
    (forward: boolean): PMCommand =>
    (state) => {
      const { from, to, empty } = state.selection;
      if (!empty) return typed(from, to, "", !forward);
      // Text uses the browser's native deletion range. Move past any existing
      // deletion before letting the browser choose the character to erase.
      let pos = from;
      for (;;) {
        const $pos = state.doc.resolve(pos);
        const node = forward ? $pos.nodeAfter : $pos.nodeBefore;
        if (node?.type === schema.nodes.inline_atom && removedRevision(node.attrs.node)) {
          pos += forward ? node.nodeSize : -node.nodeSize;
          continue;
        }
        if (node?.isText) {
          if (pos !== from) view.dispatch(state.tr.setSelection(TextSelection.create(state.doc, pos)));
          return false;
        }
        if (node) {
          const edge = pos + (forward ? node.nodeSize : -node.nodeSize);
          return typed(Math.min(pos, edge), Math.max(pos, edge), "", !forward);
        }
        if ($pos.parent.type !== schema.nodes.paragraph) return true;
        const parent = $pos.node(-1);
        const index = $pos.index(-1) + (forward ? 1 : -1);
        const neighbour = parent.maybeChild(index);
        if (!neighbour) return true;
        if (neighbour.type !== schema.nodes.paragraph) {
          events.refused(BESIDE);
          return true;
        }
        const [first, second] = forward ? [$pos.parent, neighbour] : [neighbour, $pos.parent];
        const across = pos + (forward ? 2 : -2);
        if (first.attrs.info.mark_revision === "del") {
          pos = across;
          continue;
        }
        track(state.doc, {
          from: { end: first.attrs.id }, to: { start: second.attrs.id },
          at: [Math.min(pos, across), Math.max(pos, across)],
        }, "", !forward);
        return true;
      }
    };

  /**
   * A key that deletes a word or to a line's edge: the browser deletes, and
   * the change is tracked when it settles. At a paragraph's edge it takes the
   * paragraph mark, as Backspace or Delete does.
   */
  const eraseFurther =
    (forward: boolean): PMCommand =>
    (state, dispatch, view) => {
      const $cursor = (state.selection as TextSelection).$cursor;
      if ($cursor && $cursor.parentOffset !== (forward ? $cursor.parent.content.size : 0)) return false;
      return erase(forward)(state, dispatch, view);
    };

  /** The keys below, while Track Changes is on. */
  const whileTracking = (command: PMCommand): PMCommand => (state, dispatch, view) =>
    !!session.tracking && command(state, dispatch, view);

  /** A change the browser made, such as an IME's, as one tracked change. */
  const settleTracked = (next: EditorState) => {
    const change = textChange(synced, next.doc, next.selection.head);
    if (change === null) return view.updateState(next);
    if (typeof change === "string") {
      events.refused(change);
      if (!view.state.doc.eq(synced)) redraw();
      return;
    }
    const before = synced;
    // The view shows the browser's change until the engine's replaces it.
    view.updateState(next);
    track(before, change, change.text, !change.text);
  };

  const settle = (next: EditorState) => {
    if (session.tracking) return settleTracked(next);
    const change = textChange(synced, next.doc, next.selection.head);
    try {
      if (typeof change === "string") throw new Error(change);
      if (change) session.apply([{ command: "replace_range", from: change.from, to: change.to, text: change.text }]);
    } catch (error) {
      events.refused(explain(error));
      if (!view.state.doc.eq(synced)) redraw();
      return;
    }
    view.updateState(next);
    synced = next.doc;
    session.record(editor);
  };

  /** Enter and Shift+Enter: split the paragraph, or break the line, at the caret. */
  const split =
    (command: "split_paragraph" | "insert_break"): PMCommand =>
    (state, _dispatch, view) => {
      if (!view) return false;
      const $cursor = (state.selection as TextSelection).$cursor;
      // Enter in an empty list item ends the list, as in Word.
      if (command === "split_paragraph" && $cursor?.parent.content.size === 0 && listOf($cursor.parent)) {
        unlist($cursor);
        return true;
      }
      if (!state.selection.empty && command === "split_paragraph") return replace(state, "\n");
      if (!state.selection.empty) {
        const { $from, $to } = state.selection;
        const from = pointAt($from, true);
        const to = pointAt($to);
        if (from && to) run({ command: "insert_break", at: from, to }, $from.pos + 1);
        return true;
      }
      const { $from } = view.state.selection;
      const at = pointAt($from);
      // A split closes one paragraph and opens another; a break is one atom wide.
      if (at) run({ command, at }, $from.pos + (command === "split_paragraph" ? 2 : 1));
      return true;
    };

  /** Backspace at a paragraph's start, or Delete at its end, joins it with its neighbour. */
  const join =
    (forward: boolean): PMCommand =>
    (state) => {
      if (spans(state)) return replace(state, "");
      const $cursor = (state.selection as TextSelection).$cursor;
      if (!$cursor || $cursor.parent.type !== schema.nodes.paragraph) return false;
      if ($cursor.parentOffset !== (forward ? $cursor.parent.content.size : 0)) return false;
      // Backspace at a list item's start takes it out of the list, as in Word.
      if (!forward && listOf($cursor.parent)) {
        unlist($cursor);
        return true;
      }
      const parent = $cursor.node(-1);
      const index = $cursor.index(-1) + (forward ? 1 : -1);
      const neighbour = index >= 0 && index < parent.childCount ? parent.child(index) : null;
      if (!neighbour) return true;
      if (neighbour.type === schema.nodes.block_atom) {
        // An atom nothing of which is drawn, such as a section's properties, is not removed unseen.
        if (drawn(neighbour.attrs.node)) run({ command: "remove_node", node: neighbour.attrs.id });
      } else if (neighbour.type !== schema.nodes.paragraph) {
        events.refused(TABLE);
      } else if (forward) {
        run({ command: "merge_paragraphs", paragraph: $cursor.parent.attrs.id }, $cursor.pos);
      } else {
        // The caret ends up where the previous paragraph's text ended.
        run({ command: "merge_paragraphs", paragraph: neighbour.attrs.id }, $cursor.pos - 2);
      }
      return true;
    };

  const view: EditorView = new EditorView(mount, {
    state: EditorState.create({
      doc: synced,
      plugins: [
        keymap(
          Object.fromEntries(
            Object.entries({
              Enter: (state: EditorState) => typed(state.selection.from, state.selection.to, "\n"),
              "Shift-Enter": () => !untracked(),
              Backspace: erase(false),
              "Shift-Backspace": erase(false),
              Delete: erase(true),
              "Mod-Backspace": eraseFurther(false),
              "Alt-Backspace": eraseFurther(false),
              "Mod-Delete": eraseFurther(true),
              "Alt-Delete": eraseFurther(true),
            } satisfies Record<string, PMCommand>).map(([key, command]) => [key, whileTracking(command)]),
          ),
        ),
        keymap({
          Enter: split("split_paragraph"),
          "Shift-Enter": split("insert_break"),
          Backspace: join(false),
          Delete: join(true),
          "Mod-b": format("bold"),
          "Mod-i": format("italic"),
          "Mod-u": format("underline"),
          "Mod-Alt-0": level(0),
          "Mod-Alt-1": level(1),
          "Mod-Alt-2": level(2),
          "Mod-Alt-3": level(3),
          "Shift-Mod-8": list("bullet"),
          "Shift-Mod-7": list("number"),
          "Mod-k": startLink,
          "Mod-Alt-m": startComment,
          Tab: indent(1),
          "Shift-Tab": indent(-1),
          "Mod-z": session.undo,
          "Shift-Mod-z": session.redo,
          "Mod-y": session.redo,
        }),
        keymap(baseKeymap),
        ...(options.plugins ?? []),
      ],
    }),
    // A tab is an OOXML element, not text, so a pasted one becomes a space, and
    // characters XML cannot hold are dropped.
    handlePaste(view, event) {
      const raw = event.clipboardData?.getData("text/plain") ?? "";
      // Text comes first: an app such as Word copies a picture of the text with it.
      const image = [...(event.clipboardData?.files ?? [])].find((f) => PICTURES.test(f.type));
      if (!raw && image && options.pasteImage) {
        options.pasteImage(image);
        return true;
      }
      const text = raw
        .replace(/\r\n?/g, "\n")
        .replace(/\t/g, " ")
        .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\uFFFE\uFFFF]/g, "");
      // A paste with nothing the document can hold is not a deletion of the selection.
      if (!text) {
        events.refused(UNPASTED);
        return true;
      }
      if (session.tracking) return typed(view.state.selection.from, view.state.selection.to, text);
      // What is pasted replaces the selection whole, as typing over it does.
      if (!replace(view.state, text)) events.refused(STRUCTURE);
      return true;
    },
    handleTextInput(view, from, to, text) {
      if (view.composing) return false;
      if (session.tracking) return typed(from, to, text);
      if (spans(view.state)) return replace(view.state, text);
      return from !== to && replace(view.state, text, from, to);
    },
    transformCopied(slice) {
      let fields: ReadonlyMap<NodeId, string> | undefined;
      const readFields = options.clipboardFields;
      const fieldText = readFields && ((id: NodeId) => {
        fields ??= readFields();
        // Subsequent result leaves of a field can have no painted text.
        return fields.get(id) ?? "";
      });
      const content = clipboardContent(slice.content, fieldText);
      return new Slice(content, slice.openStart, slice.openEnd);
    },
    clipboardSerializer,
    clipboardTextSerializer: (slice) => copiedText(slice.content),
    handleDrop: () => true,
    handleDOMEvents: {
      beforeinput(view, event) {
        const { inputType } = event;
        // The Edit menu, and undo gestures on touch keyboards.
        if (inputType === "historyUndo" || inputType === "historyRedo") {
          event.preventDefault();
          if (inputType === "historyUndo") session.undo();
          else session.redo();
          return true;
        }
        if (!event.cancelable || event.isComposing || view.composing) return false;

        let { from, to, empty } = view.state.selection;
        const deleting = inputType === "deleteContentBackward" || inputType === "deleteContentForward";
        let text = "";
        if (deleting) {
          if (!session.tracking) return false;
        } else {
          if (empty || (inputType !== "insertText" && inputType !== "insertReplacementText")) return false;
          const data = event.data ?? event.dataTransfer?.getData("text/plain");
          if (!data) return false;
          text = data;
        }

        // Plain typing replaces the selection whole, even if every letter is
        // identical. For a correction or deletion the browser names its target.
        if (inputType !== "insertText") {
          const [range] = event.getTargetRanges();
          if (!range) return false;
          from = view.posAtDOM(range.startContainer, range.startOffset);
          to = view.posAtDOM(range.endContainer, range.endOffset);
        }
        event.preventDefault();
        const handled = session.tracking
          ? typed(from, to, text, inputType === "deleteContentBackward")
          : replace(view.state, text, from, to);
        if (!handled) events.refused(STRUCTURE);
        return true;
      },
      compositionend(view) {
        // ProseMirror reads the composed text after this event.
        setTimeout(() => {
          if (view.composing) return;
          if (!view.state.doc.eq(synced)) settle(view.state);
          session.resume();
        });
        return false;
      },
    },
    dispatchTransaction(tr: Transaction) {
      // ProseMirror has written the clipboard before it deletes a cut selection.
      if (tr.getMeta("uiEvent") === "cut" && session.tracking) {
        typed(view.state.selection.from, view.state.selection.to, "");
        return;
      }
      if (tr.getMeta("uiEvent") === "cut" && spans(view.state)) {
        replace(view.state, "");
        return;
      }
      if (tr.selectionSet && tr.selection instanceof AllSelection) tr.setSelection(storySelection(tr.selection));
      const next = view.state.apply(tr);
      if (tr.getMeta(SYNC)) {
        view.updateState(next);
        synced = next.doc;
      } else if (!tr.docChanged || view.composing) {
        view.updateState(next);
      } else {
        settle(next);
      }
      // A local command that places the caret shares it. A remote redraw only maps it.
      if ((tr.selectionSet || (tr.docChanged && !tr.getMeta(SYNC))) && !view.composing && view.hasFocus()) {
        const { $head, $anchor, empty } = view.state.selection;
        session.selected?.(pointAt($head), empty ? null : pointAt($anchor));
      }
      if (view.hasFocus()) {
        const link = linkAt(view.state);
        events.formatting?.(
          formatted(view.state),
          paragraphStyle(view.state),
          listKind(view.state),
          link && link.href,
          cellAt(view.state) !== null,
        );
      }
    },
  });
  const editor: Editor = {
    story: story.id,
    view,
    redraw,
    splice,
    position,
    carets,
    format: (property) => format(property)(view.state, view.dispatch, view),
    style: (id) => style(id)(view.state, view.dispatch, view),
    list: (kind) => list(kind)(view.state, view.dispatch, view),
    link,
    startLink: () => startLink(view.state, view.dispatch, view),
    comment: (text, by) => comment(text, by)(view.state, view.dispatch, view),
    startComment: () => startComment(view.state, view.dispatch, view),
    reply: (id, text, by) => apply([{ command: "reply_to_comment", comment: id, text, by }]),
    // Word tracks no change to a setting.
    updateFields: (on) => apply([{ command: "set_update_fields", on }]),
    fill: (control, fill) => untracked() && apply([{ command: "fill_content_control", control, fill }]),
    updateFieldResults: (pages) => {
      if (untracked()) apply([{ command: "update_fields", now: localNow(), pages }]);
    },
    table: (edit) => table(edit)(view.state, view.dispatch, view),
    picture: (bytes, width, height, range) => picture(bytes, width, height, range)(view.state, view.dispatch, view),
  };
  session.attach(editor);
  return editor;
}
