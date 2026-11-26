// The editing surface: the document laid out on pages as Word lays it out,
// edited on the pages themselves. Each story keeps a ProseMirror view as its
// input model (editor.ts). The views are hidden but focused as usual, so the
// keyboard, IME composition and the clipboard work as they do in any editor.
// A view draws only the blocks near its selection (`nearby`). After a change
// the surface lays their documents out again, paints the pages, and draws the
// selection. A click becomes a selection in a view, and keys that move by
// lines move by the painted lines.

import { DOMSerializer, type Node as PMNode, type ResolvedPos } from "prosemirror-model";
import { type EditorState, Plugin, Selection, TextSelection } from "prosemirror-state";
import { Decoration, DecorationSet, type EditorView, type NodeView } from "prosemirror-view";
import { Blocks, EMU, type Image } from "./blocks.ts";
import {
  type Caret as Remote,
  createEditor,
  type Editor,
  type EditorEvents,
  type Formatting,
  type Held,
  hold,
  linkAround,
  linkAt,
  type ListKind,
  type Session,
  type TableEdit,
  UNLINKED,
} from "./editor.ts";
import { FontResolver, installed } from "./fonts.ts";
import { caretX, extent, type Line, offsetAt } from "./lines.ts";
import { ARROW_WIDTH, type Balloon, BALLOON_FONT, markup, MARKUP_WIDTH, NO_MARKUP } from "./markup.ts";
import { CanvasMeasurer } from "./measure.ts";
import { formatNumber } from "./numbering.ts";
import { type Fragment, type Page, Paginator } from "./paginate.ts";
import { type Caret, PAINT_CSS, Painter, type Rect } from "./paint.ts";
import type { Fill, InspectNode, Inspection, NodeId, Track } from "./types.ts";

type LineFragment = Extract<Fragment, { kind: "line" }>;

/**
 * A painted line of an editable story, with the positions it holds in the
 * story's view. They are read from its paragraph, whose start moves when an
 * edit above it moves it.
 */
class Placed {
  readonly page: number;
  readonly fragment: LineFragment;

  constructor(page: number, fragment: LineFragment) {
    this.page = page;
    this.fragment = fragment;
  }

  get from() {
    return this.fragment.paragraph.start! + this.fragment.line.from;
  }

  get to() {
    return this.fragment.paragraph.start! + this.fragment.line.to;
  }
}

/** Each painted line of a paragraph, in order and then by page, in the order Up and Down keys visit them. */
const byPosition = (a: Placed, b: Placed) => a.fragment.line.from - b.fragment.line.from || a.page - b.page;

interface Edited {
  key: NodeId;
  kind: "body" | "header" | "footer";
  part: string;
  view: EditorView;
  editor: Editor;
}

/** Where a caret at a wrap is drawn: at the end of the line before (-1) or the start of the next (1). */
type Bias = -1 | 1;

const BROWSER_IMAGES = /^image\/(png|jpeg|gif|bmp|webp|svg\+xml)$/;
const MOVES: Record<string, "up" | "down" | "home" | "end" | "pageup" | "pagedown"> = {
  ArrowUp: "up",
  ArrowDown: "down",
  Home: "home",
  End: "end",
  PageUp: "pageup",
  PageDown: "pagedown",
};
const mac = typeof navigator !== "undefined" && /Mac|iP(hone|ad)/.test(navigator.platform);
const words = new Intl.Segmenter(undefined, { granularity: "word" });

export interface Source {
  inspect(): Inspection;
  part(name: string): Uint8Array | undefined;
}

/** How many blocks on each side of the selection's ends a view draws. */
const NEARBY = 8;

/** The top-level blocks within NEARBY of `$pos`'s, each as where it starts and ends. */
function around($pos: ResolvedPos, out: Map<number, number>) {
  const { doc } = $pos;
  const index = Math.min($pos.index(0), doc.childCount - 1);
  if (index < 0) return;
  const start = $pos.posAtIndex(index, 0);
  for (let i = index, pos = start; i < Math.min(doc.childCount, index + NEARBY + 1); i++) {
    const size = doc.child(i).nodeSize;
    out.set(pos, pos + size);
    pos += size;
  }
  for (let i = index - 1, pos = start; i >= Math.max(0, index - NEARBY); i--) {
    const size = doc.child(i).nodeSize;
    pos -= size;
    out.set(pos, pos + size);
  }
}

/**
 * The decorations marking the blocks near the selection, changed from the
 * last ones. Building a set walks every top-level block, so the last set is
 * mapped through the change and only the blocks that came near are added.
 */
function marked(last: DecorationSet, state: EditorState): DecorationSet {
  const want = new Map<number, number>();
  around(state.selection.$anchor, want);
  around(state.selection.$head, want);
  const stale: Decoration[] = [];
  for (const d of last.find()) {
    if (want.get(d.from) === d.to) want.delete(d.from);
    else stale.push(d);
  }
  const kept = stale.length ? last.remove(stale) : last;
  if (!want.size) return kept;
  return kept.add(state.doc, [...want].map(([from, to]) => Decoration.node(from, to, {}, { nearby: true })));
}

const drawn = (decorations: readonly Decoration[]) => decorations.some((d) => d.spec.nearby);

/**
 * A top-level block: drawn as usual near the selection, and elsewhere an
 * empty element that is not displayed. Drawn whole, a view of 10,000
 * paragraphs made each key cost the browser about 20 ms of layout,
 * compositing and painting for text nobody sees.
 */
function block(node: PMNode, view: EditorView, getPos: () => number | undefined, decorations: readonly Decoration[]): NodeView {
  // A block in a table cell gets a view only when its table is drawn, and
  // lies inside that table's mark. Resolving the position instead costs a
  // scan of the blocks before it, so opening a story would cost their square.
  const pos = getPos()!;
  const top = !nearby.getState(view.state)!.find(pos, pos).some((d) => d.from < pos && pos < d.to);
  let current = node;
  if (!top || drawn(decorations)) {
    const { dom, contentDOM } = DOMSerializer.renderSpec(document, node.type.spec.toDOM!(node));
    return {
      dom,
      contentDOM,
      update: (next, decorations) => {
        if (!next.sameMarkup(current) || (top && !drawn(decorations))) return false;
        current = next;
        return true;
      },
    };
  }
  const dom = document.createElement("div");
  dom.hidden = true;
  return { dom, update: (next, decorations) => next.type === current.type && !drawn(decorations) };
}

/** What a comment or link being drafted is on: a story, and the selection held in it. */
export interface HeldSelection {
  story: NodeId;
  at: Held;
}

/**
 * Draws only the blocks near the selection's ends, so the browser lays out
 * and paints a few blocks however long the story is. The keyboard, IME
 * composition and the browser's own caret moves act at the selection, and a
 * move past the drawn blocks lands in a drawn neighbour, which moves the
 * window with it. Assistive technology reading the view sees only these
 * blocks too.
 */
const nearby: Plugin<DecorationSet> = new Plugin<DecorationSet>({
  state: {
    init: (_, state) => marked(DecorationSet.empty, state),
    apply: (tr, last, _, state) => {
      if (!tr.docChanged && !tr.selectionSet) return last;
      return marked(tr.docChanged ? last.map(tr.mapping, tr.doc) : last, state);
    },
  },
  props: {
    decorations: (state: EditorState) => nearby.getState(state),
    nodeViews: { paragraph: block, table: block },
  },
});

export class Surface {
  readonly root: HTMLElement;
  private readonly input: HTMLElement;
  private readonly measurer = new CanvasMeasurer();
  private readonly fonts = new FontResolver(installed());
  private readonly blocks = new Blocks((family) => this.fonts.resolve(family));
  private readonly paginator = new Paginator(this.measurer);
  private readonly painter: Painter;
  private readonly source: Source;
  /** The document as the views were first drawn from it, with its comments as they are now. */
  inspection: Inspection;
  /** Called with where a link points when someone ⌘-clicks it, or Ctrl-clicks it off a Mac. */
  onFollow: ((href: string) => void) | null = null;
  /** Called with a comment's ID, and where its Reply button is on screen, when someone clicks the button. */
  onReply: ((comment: string, at: DOMRect) => void) | null = null;
  /** Called with a content control a form fills, as drawn, and where its clicked text is on screen, when someone clicks it. */
  onControl: ((control: InspectNode, at: DOMRect) => void) | null = null;
  private readonly urls = new Map<string, string | undefined>();
  private stories: Edited[] = [];
  private active!: Edited;
  private session!: Session;
  private events!: EditorEvents;
  private laid: Page[] = [];
  private markup = NO_MARKUP;
  /** The painted lines of each paragraph of an editable story, by its content. A paragraph repeated on many pages, as a header's, has a line on each. */
  private index = new Map<object, Placed[]>();
  /** Each page's entries in the index, to take out when a layout replaces the page. */
  private placed = new WeakMap<Page, Placed[]>();
  private laidOut: PMNode[] = [];
  private remote = new Map<NodeId, Remote[]>();
  private dirty = false;
  /** Layout may run after the fonts settle and until the surface is destroyed. */
  private ready = false;
  /** The page whose copy of a repeated header or footer the caret is in. */
  private page = 0;
  private bias: Bias = 1;
  /** The x a run of Up and Down keys keeps to, and the position it last put the caret at. */
  private goal: { x: number; at: number } | null = null;

  private constructor(source: Source) {
    this.source = source;
    this.inspection = source.inspect();
    this.root = document.createElement("div");
    this.painter = new Painter(this.root, this.measurer, (image) => this.image(image));
    this.input = this.root.appendChild(document.createElement("div"));
    this.input.className = "input";
    this.root.addEventListener("mousedown", (event) => this.mousedown(event));
    this.root.addEventListener("click", (event) => {
      const reply = (event.target as Element).closest<HTMLElement>(".reply");
      if (reply) this.onReply?.(reply.dataset.comment!, reply.getBoundingClientRect());
    });
  }

  /** Opens a document's stories for editing through `session`, and draws them once the fonts are ready. */
  static async open(source: Source, session: Session, warn: (message: string) => void): Promise<Surface> {
    const surface = new Surface(source);
    try {
      surface.edit(session);
      // Building blocks resolves every font used by the current stories without measuring text.
      surface.layoutInput();
      surface.fonts.resolve(BALLOON_FONT);
      for (const style of surface.inspection.styles) {
        if ((style.kind === "paragraph" || style.kind === "character") && style.run?.font) surface.fonts.resolve(style.run.font);
      }
      if (!await surface.fonts.load()) warn("Some fonts could not be loaded. Line breaks may differ until you reopen the document.");
      surface.ready = true;
      surface.layout();
      return surface;
    } catch (error) {
      surface.destroy();
      throw error;
    }
  }

  destroy() {
    this.ready = false;
    for (const { view } of this.stories) view.destroy();
    for (const url of this.urls.values()) if (url) URL.revokeObjectURL(url);
    this.root.remove();
  }

  /** Turns a property on or off for the selection in the story being edited. */
  format(property: Formatting) {
    this.active.editor.format(property);
  }

  /** Makes the selected paragraphs, or the caret's, a list of this kind, or no list if all are. */
  list(kind: ListKind) {
    this.active.editor.list(kind);
  }

  /**
   * Links what `held` held when its address was asked for, or without it the
   * selection or the link holding it, to `target`, or unlinks it for null,
   * and returns to editing on success. A refusal leaves the form open.
   */
  link(target: string | null, held?: HeldSelection): boolean {
    let story = this.active;
    let selection: Selection;
    if (held) {
      const range = this.now(held);
      if (!range || range.anchor === range.head) {
        this.events.refused("The text to link was removed. Select other text and link it again.");
        return false;
      }
      story = range.story;
      this.select(story, range.anchor, range.head);
      selection = story.view.state.selection;
    } else {
      selection = this.linkSelection();
    }
    if (!story.editor.link(target, selection)) return false;
    story.view.focus();
    return true;
  }

  /** Opening or removing a link targets the entire enclosing link, or the selected text. */
  private linkSelection(): Selection {
    const state = this.active.view.state;
    const link = linkAt(state);
    return link ? TextSelection.create(state.doc, link.from, link.to) : state.selection;
  }

  holdLink(): HeldSelection | null {
    const { key, view } = this.active;
    const range = this.linkSelection();
    const at = range.empty ? null : hold(range, this.session);
    if (!at) {
      this.events.refused(UNLINKED);
      view.focus();
      return null;
    }
    return { story: key, at };
  }

  /** Where a held selection is now, wherever edits have moved it, or null if an end left the document. */
  private now(held: HeldSelection): { story: Edited; anchor: number; head: number } | null {
    const story = this.story(held.story);
    const position = (token: string) => {
      const resolved = this.session.resolve(token);
      return resolved ? story.editor.position(resolved.paragraph, resolved.point) : null;
    };
    const anchor = position(held.at.anchor);
    const head = position(held.at.head);
    return anchor === null || head === null ? null : { story, anchor, head };
  }

  /** Asks for a link for the selection, as ⌘K does. */
  startLink() {
    this.active.editor.startLink();
  }

  /**
   * Holds what a comment about to be drafted is on: the selection in the
   * story being edited, or with none the word at the caret, as Word does. A
   * comment needs content, since one at a bare caret would have nothing to
   * be drawn beside, so that is refused here and gives null.
   */
  holdComment(): HeldSelection | null {
    const { key, view } = this.active;
    const { doc, selection } = view.state;
    let { anchor, head } = selection;
    if (selection.empty) [anchor, head] = this.word(doc, head);
    const at = anchor === head ? null : hold(TextSelection.create(doc, anchor, head), this.session);
    if (!at) {
      this.events.refused("Put the caret in a word, or select the text to comment on.");
      view.focus();
      return null;
    }
    return { story: key, at };
  }

  /**
   * Comments as `by` on what `target` held when the draft began, wherever
   * edits have moved that since, and returns to editing after a successful post. A refusal keeps the draft open.
   */
  comment(text: string, by: Track, target: HeldSelection): boolean {
    const range = this.now(target);
    if (!range || range.anchor === range.head) {
      this.events.refused("The selected text was removed. Select other text and use Add comment to move this draft.");
      return false;
    }
    const { story } = range;
    this.select(story, range.anchor, range.head);
    if (!story.editor.comment(text, by)) return false;
    story.view.focus();
    return true;
  }

  /** Asks for a comment on the selection, or on the word at the caret, as ⌘⌥M does. */
  startComment() {
    this.active.editor.startComment();
  }

  /** Replies to a comment as `by`, and returns to editing. */
  reply(comment: string, text: string, by: Track) {
    if (!this.active.editor.reply(comment, text, by)) return false;
    this.active.view.focus();
    return true;
  }

  /** Asks Word to update the document's fields when it opens the file, or stops asking. */
  updateFields(on: boolean) {
    this.active.editor.updateFields(on);
  }

  /**
   * Updates the body's fields as Word does when every field is selected and
   * updated, as of now and with the pages as laid out: a paragraph is on the
   * page its first line is drawn on.
   */
  updateFieldResults() {
    this.flush();
    const body = this.stories.find((s) => s.kind === "body")!;
    const numbers: Record<NodeId, string> = {};
    for (const page of this.laid) {
      for (const f of page.fragments) {
        if (f.kind !== "line" || f.story !== body.key || f.paragraph.start === null) continue;
        const id = body.view.state.doc.resolve(f.paragraph.start).parent.attrs.id as NodeId;
        numbers[id] ??= formatNumber(page.number, page.format);
      }
    }
    body.editor.updateFieldResults({ total: this.laid.length, numbers });
  }

  /**
   * Read the painted field results. A header or footer repeats, and is read
   * on the caret's page, or on the first page that shows it, as the caret
   * is drawn there.
   */
  private clipboardFields(story: NodeId): Map<NodeId, string> {
    this.flush();
    const values = new Map<NodeId, string>();
    const seen = new Set<Line>();
    const shows = (page: Page) => page.fragments.some((f) => f.kind === "line" && f.story === story);
    const caret = this.laid[this.page];
    const pages = this.story(story).kind === "body" ? this.laid
      : caret && shows(caret) ? [caret]
      : this.laid.filter(shows).slice(0, 1);
    for (const page of pages) {
      for (const fragment of page.fragments) {
        if (fragment.kind !== "line" || fragment.story !== story || !fragment.paragraph.fields || seen.has(fragment.line)) continue;
        seen.add(fragment.line); // A table header can repeat the same line.
        for (const piece of fragment.line.pieces) {
          const id = piece.inline?.kind === "text" && piece.inline.fieldResult;
          if (id) values.set(id, (values.get(id) ?? "") + piece.text);
        }
      }
    }
    return values;
  }

  /** Fills a content control as a form does, and returns to editing. */
  fill(control: NodeId, fill: Fill) {
    if (!this.active.editor.fill(control, fill)) return false;
    this.active.view.focus();
    return true;
  }

  /** Adds or removes a row or column beside the caret's table cell, or removes its table. */
  table(edit: TableEdit) {
    this.active.editor.table(edit);
  }

  /**
   * Puts an image at the caret, or in place of the selection, at its size at
   * 96 dots per inch, made smaller to fit between the margins of the caret's
   * page. Resolve the initiating selection after the asynchronous image read.
   */
  async picture(image: Blob, storyKey: NodeId = this.active.key) {
    const story = this.story(storyKey);
    const at = hold(story.view.state.selection, this.session);
    if (!at) return this.events.refused("Select a place in a paragraph for the picture.");
    let bytes: Uint8Array;
    let width: number;
    let height: number;
    try {
      bytes = new Uint8Array(await image.arrayBuffer());
      const bitmap = await createImageBitmap(image);
      // A CSS pixel is 3/4 of a point.
      [width, height] = [bitmap.width * 0.75, bitmap.height * 0.75];
      bitmap.close();
    } catch {
      if (!story.view.isDestroyed) this.events.refused("That image could not be read. Pictures can be PNG, JPEG or GIF images.");
      return;
    }
    if (story.view.isDestroyed) return;
    const range = this.now({ story: storyKey, at });
    if (!range) {
      this.events.refused("The place selected for the picture was removed. Select another place and try again.");
      return;
    }
    const caret = this.caret(story, range.head);
    const page = this.laid[caret?.page ?? 0];
    const fits = page ? page.width - page.margins.left - page.margins.right : width;
    const scale = width > fits ? fits / width : 1;
    const emu = (points: number) => Math.max(1, Math.round((points * scale) / EMU));
    story.editor.picture(bytes, emu(width), emu(height), { anchor: range.anchor, head: range.head });
  }

  /** Returns to editing where the selection was. */
  focus() {
    this.active.view.focus();
  }

  /** Where the caret of the story being edited is drawn on screen, or null if it is not drawn. */
  caretRect(): DOMRect | null {
    const caret = this.caret(this.active, this.active.view.state.selection.head);
    return caret && this.painter.clientRect(caret);
  }

  /** Gives the selected paragraphs, or the caret's, a paragraph style, and returns to editing. */
  style(style: string) {
    this.active.editor.style(style);
    this.active.view.focus();
  }

  get body(): EditorView {
    return this.stories.find((s) => s.kind === "body")!.view;
  }

  get pages(): readonly Page[] {
    return this.laid;
  }

  private edit(session: Session) {
    this.session = session;
    this.events = session.events;
    session.touched = (stories) => this.touched(stories);
    for (const story of this.inspection.stories) {
      if (story.type !== "story" || (story.story !== "body" && story.story !== "header" && story.story !== "footer")) continue;
      const mount = this.input.appendChild(document.createElement("div"));
      const key = story.id;
      const editor = createEditor(mount, story, session, {
        plugins: [this.plugin(key), nearby],
        carets: (list) => {
          this.remote.set(key, list);
          this.schedule();
        },
        styles: this.inspection.styles,
        clipboardFields: () => this.clipboardFields(key),
        pasteImage: (image) => void this.picture(image, key),
      });
      this.stories.push({ key, kind: story.story, part: story.part, view: editor.view, editor });
    }
    this.active = this.stories.find((s) => s.kind === "body") ?? this.stories[0];
    this.painter.activate(this.active.key);
  }

  /** Refresh read-only stories that have no ProseMirror view of their own. */
  private touched(stories: NodeId[] | "all") {
    const ids = new Set(this.inspection.stories.filter((s) => s.type === "story"
      && (s.story === "footnotes" || s.story === "endnotes" || s.story === "comments")).map((s) => s.id));
    if (this.inspection.comments_story) ids.add(this.inspection.comments_story);
    const changed = [...ids].filter((id) => stories === "all" || stories.includes(id));
    if (!changed.length) return;
    const updated = this.inspection.stories.filter((s) => !changed.includes(s.id));
    this.inspection = { ...this.inspection, stories: [...updated, ...changed.map((id) => this.session.inspectNode(id))] };
    this.laidOut = [];
    this.schedule();
  }

  /** Lays the views out again when their documents change, and steers line-wise keys. */
  private plugin(key: NodeId) {
    return new Plugin({
      view: () => ({
        update: (view, previous) => {
          // Peer offsets belong to the old document until the session resolves them again.
          if (view.state.doc !== previous.doc) this.remote.delete(key);
          this.schedule();
        },
      }),
      props: {
        handleKeyDown: (view, event) => {
          let move = MOVES[event.key];
          if (mac && event.metaKey && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
            move = event.key === "ArrowLeft" ? "home" : "end";
          } else if (!move || event.altKey || event.metaKey || event.ctrlKey) {
            return false;
          }
          return this.navigate(this.story(key), move, event.shiftKey);
        },
        handleScrollToSelection: (view) => {
          if (!view.hasFocus()) return true;
          // The view's state changed just before this, so the pages may not show it yet.
          this.dirty = true;
          this.flush();
          const caret = this.caret(this.active, this.active.view.state.selection.head);
          if (caret) this.painter.reveal(caret);
          return true;
        },
      },
    });
  }

  private story(key: NodeId) {
    return this.stories.find((s) => s.key === key)!;
  }

  /** Media added since the document opened is not among its parts, so the picture's own content type comes first. */
  private image({ part, type: given }: Image): string | undefined {
    if (!part) return undefined;
    if (!this.urls.has(part)) {
      const type = given ?? this.inspection.package.parts.find((p) => p.name === part)?.content_type ?? "";
      const bytes = BROWSER_IMAGES.test(type) ? this.source.part(part) : undefined;
      this.urls.set(part, bytes ? URL.createObjectURL(new Blob([bytes as BlobPart], { type })) : undefined);
    }
    return this.urls.get(part);
  }

  private schedule() {
    if (this.dirty) return;
    this.dirty = true;
    requestAnimationFrame(() => this.flush());
  }

  /** Brings the pages up to date with the views now, if a change is waiting. */
  private flush() {
    // Open lays out the latest story views once it is ready.
    if (!this.dirty || !this.ready) return;
    this.dirty = false;
    const docs = this.stories.map((s) => s.view.state.doc);
    if (docs.some((doc, i) => doc !== this.laidOut[i])) this.layout();
    else this.decorate();
  }

  private layoutInput() {
    return this.blocks.layout(this.inspection, (story) => this.story(story.id).view.state.doc);
  }

  private layout() {
    this.dirty = false;
    this.laidOut = this.stories.map((s) => s.view.state.doc);
    const input = this.layoutInput();
    const pages = this.paginator.layout(input);
    this.reindex(this.laid, pages);
    this.laid = pages;
    this.markup = markup(this.laid, input.comments, this.measurer, (run) => this.blocks.style(run));
    this.painter.render(this.laid, this.markup);
    this.decorate();
  }

  /** Brings the index of painted lines up to date for the pages the layout replaced; the others' lines stay. */
  private reindex(before: readonly Page[], after: readonly Page[]) {
    // A paragraph with lines on several pages, such as a header's, is filtered and sorted once at the end.
    const gone = new Set<Placed>();
    const touched = new Set<Placed[]>();
    for (let i = 0; i < before.length; i++) {
      if (after[i] === before[i]) continue;
      for (const placed of this.placed.get(before[i]) ?? []) {
        const key = placed.fragment.paragraph.key;
        const list = this.index.get(key)!;
        if (list.length === 1) {
          this.index.delete(key);
        } else {
          gone.add(placed);
          touched.add(list);
        }
      }
    }
    for (let i = 0; i < after.length; i++) {
      if (after[i] === before[i]) continue;
      const placed: Placed[] = [];
      for (const f of after[i].fragments) {
        if (f.kind !== "line" || !f.story || f.paragraph.start === null) continue;
        const line = new Placed(i, f);
        placed.push(line);
        const list = this.index.get(f.paragraph.key);
        if (!list) {
          this.index.set(f.paragraph.key, [line]);
        } else {
          list.push(line);
          touched.add(list);
        }
      }
      this.placed.set(after[i], placed);
    }
    for (const list of touched) {
      const key = list[0].fragment.paragraph.key;
      const kept = gone.size ? list.filter((p) => !gone.has(p)) : list;
      if (kept.length) this.index.set(key, kept.sort(byPosition));
      else this.index.delete(key);
    }
  }

  /**
   * The painted lines of the paragraph at `$pos`, or of the nearest one. The
   * same content at two places in a story shares a key, so a line is checked
   * to be this paragraph's by where it starts.
   */
  private linesAt($pos: ResolvedPos): Placed[] {
    const $text = $pos.parent.isTextblock ? $pos : (Selection.findFrom($pos, 1, true) ?? Selection.findFrom($pos, -1, true))?.$head;
    if (!$text) return [];
    const start = $text.start();
    return (this.index.get($text.parent) ?? []).filter((p) => p.fragment.paragraph.start === start);
  }

  /** The painted line holding a position, preferring the caret's page for a repeated header. */
  private locate(story: Edited, pos: number, bias = this.bias): Placed | undefined {
    const lines = this.linesAt(story.view.state.doc.resolve(pos));
    const holding = lines.filter((p) => p.from <= pos && pos <= p.to);
    if (!holding.length) return lines[0];
    // At a wrap the position ends one line and starts the next.
    const preferred = holding.filter((p) => (bias < 0 ? p.to === pos : p.from === pos));
    const pick = preferred.length ? preferred : holding;
    return pick.find((p) => p.page === this.page) ?? pick[0];
  }

  private caret(story: Edited, pos: number, bias = this.bias): Rect | null {
    const placed = this.locate(story, pos, bias);
    if (!placed) return null;
    const { fragment: f } = placed;
    const metrics = this.measurer.metrics(f.paragraph.mark.font);
    const height = Math.min(f.line.height, metrics.ascent + metrics.descent);
    const x = f.x + caretX(f.line, pos - f.paragraph.start!, this.measurer);
    return { page: placed.page, x, y: f.y + Math.max(0, f.line.ascent - metrics.ascent), width: 0, height };
  }

  /** The lines a range covers, as rectangles. A line the range runs past is covered to its end. */
  private ranges(story: Edited, from: number, to: number, color?: string): Rect[] {
    const covered: Placed[] = [];
    const { doc } = story.view.state;
    doc.nodesBetween(from, to, (node, pos) => {
      if (!node.isTextblock) return true;
      covered.push(...this.linesAt(doc.resolve(pos + 1)));
      return false;
    });
    const out: Rect[] = [];
    for (const placed of covered) {
      if (placed.to < from || placed.from > to || (placed.to === from && placed.from !== from)) continue;
      const { fragment: f } = placed;
      const start = f.paragraph.start!;
      const left = f.x + caretX(f.line, Math.max(from, placed.from) - start, this.measurer);
      const end = to > placed.to ? f.x + extent(f.line).right + 4 : f.x + caretX(f.line, to - start, this.measurer);
      if (end > left) out.push({ page: placed.page, x: left, y: f.y, width: end - left, height: f.line.height, color });
    }
    return out;
  }

  private decorate() {
    if (!this.active) return;
    const { view } = this.active;
    const { from, to, empty, head } = view.state.selection;
    const selection = empty ? [] : this.ranges(this.active, from, to);
    const carets: Caret[] = [];
    const local = this.caret(this.active, head);
    if (local && empty && view.hasFocus()) carets.push(local);
    for (const [key, list] of this.remote) {
      const story = this.story(key);
      for (const { at, anchor, name, color } of list) {
        const caret = this.caret(story, at, 1);
        if (caret) carets.push({ ...caret, color, name });
        if (anchor !== undefined && anchor !== at) selection.push(...this.ranges(story, Math.min(at, anchor), Math.max(at, anchor), color));
      }
    }
    this.painter.decorate(selection, carets, this.balloonAt(this.active, head));
    if (local) this.follow(local);
  }

  /** The balloon for what the caret is in or beside: the narrowest, where several hold it. */
  private balloonAt(story: Edited, pos: number): Balloon | null {
    let best: Balloon | null = null;
    for (const balloons of this.markup.pages) {
      for (const b of balloons) {
        if (b.story !== story.key || pos < b.from || pos > b.to) continue;
        if (!best || b.to - b.from < best.to - best.from) best = b;
      }
    }
    return best;
  }

  /**
   * Keeps the focused view's DOM caret under the painted one, where the
   * browser draws an IME's candidates: the input box moves to the caret, and
   * scrolls the view's DOM caret to its corner.
   */
  private follow(caret: Rect) {
    const client = this.painter.clientRect(caret);
    const root = this.root.getBoundingClientRect();
    this.input.style.left = `${client.left - root.left}px`;
    this.input.style.top = `${client.top - root.top}px`;
    this.input.style.height = `${client.height}px`;
    const { view } = this.active;
    if (!view.hasFocus()) return;
    const at = view.coordsAtPos(view.state.selection.head);
    const box = this.input.getBoundingClientRect();
    this.input.scrollLeft += at.left - box.left;
    this.input.scrollTop += at.top - box.top;
  }

  /** The editable line nearest a point on a page, in `story` or in any story. */
  private hit(page: number, x: number, y: number, story?: Edited) {
    let best: { fragment: LineFragment; dy: number; dx: number } | null = null;
    for (const fragment of this.laid[page]?.fragments ?? []) {
      if (fragment.kind !== "line" || !fragment.story || fragment.paragraph.start === null) continue;
      if (story && fragment.story !== story.key) continue;
      const { line } = fragment;
      const left = fragment.x + extent(line).left;
      const right = fragment.x + extent(line).right;
      const dy = y < fragment.y ? fragment.y - y : y > fragment.y + line.height ? y - fragment.y - line.height : 0;
      const dx = x < left ? left - x : x > right ? x - right : 0;
      if (!best || dy < best.dy || (dy === best.dy && dx < best.dx)) best = { fragment, dy, dx };
    }
    if (!best) return null;
    const { fragment } = best;
    const { offset, bias } = offsetAt(fragment.line, x - fragment.x, this.measurer);
    return { story: this.story(fragment.story!), pos: fragment.paragraph.start! + offset, bias };
  }

  private select(story: Edited, anchor: number, head: number, bias: Bias = 1) {
    const { view } = story;
    const { doc } = view.state;
    const selection = TextSelection.create(doc, Math.min(anchor, doc.content.size), Math.min(head, doc.content.size));
    this.bias = bias;
    view.dispatch(view.state.tr.setSelection(selection).scrollIntoView());
  }

  /** Switches editing to another story, as a double-click on a header does in Word. */
  private activate(story: Edited) {
    this.active = story;
    this.painter.activate(story.key);
  }

  private mousedown(event: MouseEvent) {
    if (event.button !== 0) return;
    const at = this.painter.pageAt(event.clientX, event.clientY);
    if (!at) return;
    event.preventDefault();
    this.flush();
    if (this.balloonClick(at.page, at.x, at.y)) return;
    // The painted text is under the page's other layers, so the event's target is never it.
    const clicked = document.elementsFromPoint(event.clientX, event.clientY).find((e) => e instanceof HTMLElement && e.dataset.control) as
      | HTMLElement
      | undefined;
    const control = clicked && this.onControl && !event.shiftKey && this.control(clicked.dataset.control as NodeId);
    if (control) return this.onControl!(control, clicked.getBoundingClientRect());
    const anywhere = this.hit(at.page, at.x, at.y);
    if (event.detail >= 2 && anywhere && anywhere.story !== this.active) this.activate(anywhere.story);
    const story = this.active;
    const target = this.hit(at.page, at.x, at.y, story);
    if (!target) return;
    if (mac ? event.metaKey : event.ctrlKey) {
      const link = linkAround(story.view.state.doc.resolve(target.pos));
      if (link) return this.onFollow?.(link.href);
    }
    this.page = at.page;
    this.goal = null;
    story.view.focus();
    const { doc } = story.view.state;
    if (event.detail === 2) {
      const [from, to] = this.word(doc, target.pos);
      this.select(story, from, to);
    } else if (event.detail >= 3) {
      const $pos = doc.resolve(target.pos);
      this.select(story, $pos.start(), $pos.end());
    } else {
      const anchor = event.shiftKey ? story.view.state.selection.anchor : target.pos;
      this.select(story, anchor, target.pos, target.bias);
    }

    const anchor = story.view.state.selection.anchor;
    const move = (e: MouseEvent) => {
      const point = this.painter.pageAt(e.clientX, e.clientY);
      const over = point && this.hit(point.page, point.x, point.y, story);
      if (!over) return;
      this.page = point.page;
      this.select(story, anchor, over.pos, over.bias);
    };
    const up = () => {
      removeEventListener("mousemove", move);
      removeEventListener("mouseup", up);
    };
    addEventListener("mousemove", move);
    addEventListener("mouseup", up);
  }

  /** The content control with this ID as the stories draw it, inside the atom that holds it. */
  private control(id: NodeId): InspectNode | null {
    const find = (node: InspectNode): InspectNode | null => {
      if (node.id === id) return node;
      for (const child of node.children ?? []) {
        const found = find(child);
        if (found) return found;
      }
      return null;
    };
    let found: InspectNode | null = null;
    for (const { view } of this.stories) {
      view.state.doc.descendants((node) => {
        const held = node.attrs.node as InspectNode | undefined;
        if (held && !found) found = find(held);
        return !found;
      });
      if (found) break;
    }
    return found;
  }

  /**
   * A click on a balloon puts the caret at what it describes, and a click on
   * a move's arrow at the other end of the move, as Word's do.
   */
  private balloonClick(page: number, x: number, y: number): boolean {
    const left = this.laid[page].width;
    if (x < left) return false;
    const balloon = this.markup.pages[page]?.find((b) => y >= b.top && y < b.top + b.height);
    if (!balloon) return false;
    // A balloon for content no editor addresses, such as a change in a note, has nowhere to put the caret.
    if (!balloon.story || balloon.from < 0) return true;
    const story = this.story(balloon.story);
    const arrow = balloon.partner !== undefined && x >= left + MARKUP_WIDTH - ARROW_WIDTH;
    const at = arrow ? balloon.partner! : balloon.from;
    if (story !== this.active) this.activate(story);
    this.page = page;
    this.goal = null;
    story.view.focus();
    this.select(story, at, at);
    return true;
  }

  /** The word around a position, by the paragraph's text with each atom as one character. */
  private word(doc: PMNode, pos: number): [number, number] {
    const $pos = doc.resolve(pos);
    const start = $pos.start();
    const text = doc.textBetween(start, $pos.end(), undefined, "￼");
    for (const { index, segment } of words.segment(text)) {
      if (start + index <= pos && pos <= start + index + segment.length && segment.trim()) {
        return [start + index, start + index + segment.length];
      }
    }
    return [pos, pos];
  }

  /** Moves the caret by painted lines, or to a line's start or end. Shift extends the selection. */
  private navigate(story: Edited, move: (typeof MOVES)[string], extend: boolean): boolean {
    this.flush();
    const { selection, doc } = story.view.state;
    const head = selection.head;
    const placed = this.locate(story, head);
    if (!placed) return false;
    const { fragment: f } = placed;
    const start = f.paragraph.start!;
    const goal = this.goal?.at === head ? this.goal.x : f.x + caretX(f.line, head - start, this.measurer);
    let target: number;
    let bias: Bias = 1;
    if (move === "home") target = placed.from;
    else if (move === "end") {
      target = placed.to;
      bias = -1;
    } else if (move === "up" || move === "down") {
      const next = this.neighbour(story, placed, move === "up" ? -1 : 1);
      if (!next) target = move === "up" ? Selection.atStart(doc).from : Selection.atEnd(doc).to;
      else {
        const at = offsetAt(next.fragment.line, goal - next.fragment.x, this.measurer);
        target = next.fragment.paragraph.start! + at.offset;
        bias = at.bias;
        this.page = next.page;
      }
    } else {
      const caret = this.caret(story, head);
      if (!caret) return false;
      const client = this.painter.clientRect(caret);
      const distance = (move === "pageup" ? -1 : 1) * innerHeight * 0.9;
      scrollBy(0, distance);
      const point = this.painter.pageAt(client.left, client.top + client.height / 2);
      const hit = point && this.hit(point.page, point.x, point.y, story);
      if (!hit) return true;
      target = hit.pos;
      this.page = point.page;
    }
    this.select(story, extend ? selection.anchor : target, target, bias);
    this.goal = move === "up" || move === "down" ? { x: goal, at: story.view.state.selection.head } : null;
    return true;
  }

  /** The painted line before or after one, in document order, on the caret's page where a header repeats. */
  private neighbour(story: Edited, placed: Placed, step: -1 | 1): Placed | undefined {
    const { doc } = story.view.state;
    const $start = doc.resolve(placed.fragment.paragraph.start!);
    const pick = (lines: Placed[]) => {
      const same = lines.filter((l) => l.from === lines[0].from);
      return same.find((l) => l.page === placed.page) ?? same[0];
    };
    const own = this.linesAt($start).filter((l) => (step > 0 ? l.from > placed.from : l.from < placed.from));
    if (own.length) return pick(step > 0 ? own : own.reverse());
    const next = Selection.findFrom(doc.resolve(step > 0 ? $start.after() : $start.before()), step, true);
    if (!next) return undefined;
    const lines = this.linesAt(next.$head);
    return lines.length ? pick(step > 0 ? lines : lines.reverse()) : undefined;
  }
}

// The input box is contained strictly, so a keystroke does not make the
// browser walk and repaint the whole hidden document inside it.
export const SURFACE_CSS = `${PAINT_CSS}
.pages{position:relative}
.pages .input{position:absolute;left:0;top:0;width:1px;height:1em;overflow:hidden;contain:strict;opacity:0;pointer-events:none;z-index:-1}
.pages .input .ProseMirror{width:600px;outline:none;white-space:pre-wrap;white-space:break-spaces;caret-color:transparent}
`;
