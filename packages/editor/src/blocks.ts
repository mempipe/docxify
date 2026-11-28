// The layout's input: paragraphs as runs of styled inline content, and tables
// as rows of cells holding blocks, read from each story's ProseMirror
// document. An editor's documents give positions it addresses; content no
// editor addresses, such as notes, is built into the same model and drawn
// read-only.
//
// Positions inside a paragraph are relative to where its content starts, so
// a paragraph's layout stays valid when blocks above it change. An edited
// story keeps the block objects of the top-level nodes an edit left alone, and
// moves their starts in place, so the layout can tell what changed by identity.

import type { Mark, Node as PMNode } from "prosemirror-model";
import type { Font } from "./measure.ts";
import { blocks, buildDoc, hrefOf, type Info, insertionOf, removedRevision, schema } from "./model.ts";
import type {
  Border,
  Edges,
  EffectiveParagraph,
  EffectiveRun,
  Inspection,
  InspectNode,
  Level,
  NodeId,
  PageField,
  Placement,
  RunProps,
  Section,
  Settings,
  Tracked,
} from "./types.ts";

export const TWIP = 1 / 20;
export const EMU = 1 / 12700;

export interface Revision {
  change: string;
  author?: string;
  date?: string;
  /** The move a `moveFrom` or `moveTo` belongs to. */
  move?: string;
}

/** A run's tracked formatting change: who made it, and its direct formatting before and after. */
export interface Formatted {
  by: Tracked;
  before: RunProps;
  after: RunProps;
}

export interface TextStyle {
  font: Font;
  run: EffectiveRun;
  /** Baseline shift in points, up positive: superscript and subscript. */
  shift: number;
  link?: string;
  revision?: Revision;
  formatted?: Formatted;
  /** The comments whose ranges hold the text, by ID. */
  comments?: string[];
  /** The page number drawn in place of this text, which a page field's result holds. */
  pageField?: PageField;
  /** The innermost content control a form fills that holds the text, by ID. */
  control?: NodeId;
}

export interface Image {
  part?: string;
  type?: string;
  description?: string;
}

/**
 * Where an inline sits in its story's document, relative to its paragraph's
 * content. Editable text maps one position to each code unit; an atom's
 * content maps to the atom's single position as a whole.
 */
export interface Source {
  at: number;
  atom: boolean;
}

export type Inline =
  | { kind: "text"; text: string; style: TextStyle; source: Source; fieldResult?: NodeId; note?: string }
  | { kind: "tab"; style: TextStyle; source: Source }
  | { kind: "break"; type: "line" | "page" | "column"; style: TextStyle; source: Source }
  | { kind: "box"; width: number; height: number; image: Image; style: TextStyle; source: Source }
  | { kind: "float"; float: Float; source: Source }
  | { kind: "anchor"; removed: Removed; style: TextStyle; source: Source };

/** Content a tracked change removed. Word shows it in a balloon beside the page, not on the line. */
export interface Removed {
  revision: Revision;
  inlines: Inline[];
}

export interface Float {
  width: number;
  height: number;
  image: Image;
  placement: Extract<Placement, { kind: "anchor" }>;
}

export interface Paragraph {
  kind: "paragraph";
  /** Unchanged content keeps its key, so its layout is reused. */
  key: object;
  props: EffectiveParagraph;
  mark: TextStyle;
  style?: string;
  marker?: { text: string; style: TextStyle; level: Level };
  inlines: Inline[];
  /** Where the content starts in the story's document; null where no editor addresses it. */
  start: number | null;
  /** The content's size in positions. */
  size: number;
  section?: Section;
  markRevision?: Revision;
  /** Its paragraph properties are a tracked change. */
  formatChanged: boolean;
  /** It shows a page field, so its text depends on the page it is drawn on. */
  fields: boolean;
}

export interface Table {
  kind: "table";
  key: object;
  grid: number[];
  indent: number;
  alignment: "left" | "center" | "right";
  rows: Row[];
}

export interface Row {
  header: boolean;
  cantSplit: boolean;
  height?: { value: number; rule: "at_least" | "exact" };
  gridBefore: number;
  revision?: Revision;
  cells: Cell[];
}

export interface Cell {
  column: number;
  span: number;
  rowspan: number;
  borders: Edges<Border>;
  shading?: string;
  margins: Required<Edges<number>>;
  valign: "top" | "center" | "bottom";
  blocks: Block[];
}

export type Block = Paragraph | Table;

/** A story's blocks, and the ID an editor addresses its positions by, when one does. */
export interface Story {
  key: NodeId | null;
  blocks: Block[];
}

/** A comment, drawn read-only in a balloon beside the text it comments on. */
export interface Comment {
  author?: string;
  date?: string;
  /** The comment it replies to, in whose balloon it is drawn. */
  parent?: string;
  blocks: Block[];
}

export interface LayoutInput {
  body: Story;
  /** The body's own section properties, which the last section takes. */
  section: Section;
  /** Header and footer stories by part name. */
  parts: Map<string, Story>;
  /** Footnotes and endnotes as `kind:id`, separators left out. */
  notes: Map<string, Block[]>;
  /** Comments by ID. */
  comments: Map<string, Comment>;
  settings: Settings;
}

/** What a run's tracked formatting change and comments add to its style. */
const marked = (run: Info<"run">): Partial<TextStyle> => ({
  formatted: run.previous ? { by: run.format_change ?? {}, before: run.previous, after: run.props ?? {} } : undefined,
  comments: run.comments,
});

const NO_SECTION: Section = { start: "nextPage" };

/** The section properties a body ends with, which are its own. */
function finalSection(body: PMNode): Section {
  for (let i = body.childCount - 1; i >= 0; i--) {
    const node = body.child(i);
    if (node.type === schema.nodes.block_atom && node.attrs.node.type === "section") return node.attrs.node;
  }
  return NO_SECTION;
}

/** A story's blocks as last built, and how many each top-level node gave, to build the next from. */
interface Built {
  doc: PMNode;
  blocks: Block[];
  counts: number[];
}

/** Moves a block's positions, and those of the blocks in its cells, by `delta`. */
function shift(block: Block, delta: number) {
  if (block.kind === "paragraph") {
    if (block.start !== null) block.start += delta;
    return;
  }
  for (const row of block.rows) for (const cell of row.cells) for (const b of cell.blocks) shift(b, delta);
}

/** Builds the layout's input, resolving each requested family to the face that draws it. */
export class Blocks {
  private readonly resolve: (family: string) => string;
  private paragraphs = new WeakMap<PMNode, Omit<Paragraph, "start">>();
  private atoms = new WeakMap<InspectNode, Block[]>();
  private notes = new WeakMap<Inspection, Map<string, Block[]>>();
  private comments = new WeakMap<Inspection, Map<string, Comment>>();
  private built = new Map<NodeId, Built>();

  constructor(resolve: (family: string) => string) {
    this.resolve = resolve;
  }

  style(run: EffectiveRun, extra?: Partial<TextStyle>): TextStyle {
    const shifted = run.vert_align === "superscript" || run.vert_align === "subscript";
    const size = run.size_half_points / 2;
    const font = {
      family: this.resolve(run.font),
      size: shifted ? size * 0.65 : size,
      bold: !!run.bold,
      italic: !!run.italic,
      smallCaps: !!run.small_caps && !run.caps,
    };
    const shift = run.vert_align === "superscript" ? size * 0.33 : run.vert_align === "subscript" ? -size * 0.14 : 0;
    return { font, run, shift, ...extra };
  }

  /**
   * A document's layout input. `doc` gives the ProseMirror document an editor
   * holds for each story, and the stories are then addressed by their IDs.
   * Without it each story is built from the inspection and drawn read-only.
   * Notes are always read-only.
   */
  layout(inspection: Inspection, doc?: (story: InspectNode) => PMNode): LayoutInput {
    const read = doc ?? ((node: InspectNode) => buildDoc(node));
    const story = (node: InspectNode, built = read(node)): Story =>
      doc ? { key: node.id, blocks: this.edited(node.id, built) } : { key: null, blocks: this.fromDoc(built, false) };
    const stories = inspection.stories.filter((s) => s.type === "story");
    const parts = new Map<string, Story>();
    for (const s of stories) if (s.story === "header" || s.story === "footer") parts.set(s.part, story(s));
    const node = stories.find((s) => s.story === "body")!;
    const body = read(node);
    return {
      body: story(node, body),
      section: finalSection(body),
      parts,
      notes: this.notesOf(inspection),
      comments: this.commentsOf(inspection),
      settings: inspection.settings,
    };
  }

  /** The blocks of a story's document, with the positions an editor addresses when `editable`. */
  fromDoc(doc: PMNode, editable = true): Block[] {
    const out: Block[] = [];
    doc.forEach((node, offset) => out.push(...this.fromPM(node, offset, editable)));
    return out;
  }

  /**
   * An edited story's blocks. The top-level nodes before and after the ones
   * an edit replaced keep their blocks, those after moved to their new
   * positions, and an unchanged document gives the same array.
   */
  private edited(key: NodeId, doc: PMNode): Block[] {
    const last = this.built.get(key);
    if (last?.doc === doc) return last.blocks;
    const old = last?.doc;
    const n = old ? Math.min(old.childCount, doc.childCount) : 0;
    let head = 0;
    while (head < n && old!.child(head) === doc.child(head)) head++;
    let tail = 0;
    while (tail < n - head && old!.child(old!.childCount - 1 - tail) === doc.child(doc.childCount - 1 - tail)) tail++;

    let kept = 0;
    let pos = 0;
    for (let i = 0; i < head; i++) {
      kept += last!.counts[i];
      pos += doc.child(i).nodeSize;
    }
    const middle: Block[] = [];
    const counts: number[] = [];
    for (let i = head; i < doc.childCount - tail; i++) {
      const node = doc.child(i);
      const out = this.fromPM(node, pos, true);
      for (const block of out) middle.push(block);
      counts.push(out.length);
      pos += node.nodeSize;
    }
    let after = last?.blocks.length ?? 0;
    for (let i = 0; i < tail; i++) after -= last!.counts[old!.childCount - 1 - i];
    const moved = last?.blocks.slice(after) ?? [];
    const delta = doc.content.size - (old?.content.size ?? 0);
    if (delta) for (const block of moved) shift(block, delta);

    const built: Built = {
      doc,
      blocks: (last?.blocks.slice(0, kept) ?? []).concat(middle, moved),
      counts: (last?.counts.slice(0, head) ?? []).concat(counts, last?.counts.slice(old!.childCount - tail) ?? []),
    };
    this.built.set(key, built);
    return built.blocks;
  }

  /** A document's footnotes and endnotes by `kind:id`, without the separators Word draws itself. */
  private notesOf(inspection: Inspection): Map<string, Block[]> {
    let notes = this.notes.get(inspection);
    if (notes) return notes;
    notes = new Map();
    for (const story of inspection.stories) {
      if (story.type !== "story" || (story.story !== "footnotes" && story.story !== "endnotes")) continue;
      for (const note of story.children ?? []) {
        if (note.type !== "note" || note.separator) continue;
        notes.set(`${note.kind}:${note.note_id}`, this.readOnly(note));
      }
    }
    this.notes.set(inspection, notes);
    return notes;
  }

  /** A document's comments by ID. */
  private commentsOf(inspection: Inspection): Map<string, Comment> {
    let comments = this.comments.get(inspection);
    if (comments) return comments;
    comments = new Map();
    for (const story of inspection.stories) {
      if (story.type !== "story" || story.story !== "comments") continue;
      for (const comment of story.children ?? []) {
        if (comment.type !== "comment") continue;
        const { author, date, parent } = comment;
        comments.set(comment.comment_id, { author, date, parent, blocks: this.readOnly(comment) });
      }
    }
    this.comments.set(inspection, comments);
    return comments;
  }

  /** The blocks inside a node no editor addresses. */
  private readOnly(node: InspectNode): Block[] {
    return this.fromDoc(buildDoc(node), false);
  }

  private fromPM(node: PMNode, pos: number, editable: boolean): Block[] {
    if (node.type === schema.nodes.paragraph) {
      let paragraph = this.paragraphs.get(node);
      if (!paragraph) {
        paragraph = this.pmParagraph(node);
        this.paragraphs.set(node, paragraph);
      }
      return [{ ...paragraph, start: editable ? pos + 1 : null }];
    }
    if (node.type === schema.nodes.table) {
      const table = node.attrs.info as Info<"table">;
      const rows: Row[] = [];
      node.forEach((row, rowOffset) => {
        if (row.type !== schema.nodes.row) return;
        const cells: Cell[] = [];
        row.forEach((cell, cellOffset) => {
          if (cell.type !== schema.nodes.cell) return;
          const blocks: Block[] = [];
          const start = pos + 1 + rowOffset + 1 + cellOffset + 1;
          cell.forEach((child, offset) => blocks.push(...this.fromPM(child, start + offset, editable)));
          cells.push(this.cell(cell.attrs.info, cell.attrs.column, cell.attrs.rowspan, blocks));
        });
        rows.push(this.row(row.attrs.info, cells));
      });
      return [this.table(node, table, rows)];
    }
    if (node.type === schema.nodes.block_atom) return this.atom(node.attrs.node);
    return [];
  }

  /** A block an editor keeps whole. A content control's blocks are drawn read-only; nothing else is drawn. */
  private atom(node: InspectNode): Block[] {
    if (node.type !== "content_control") return [];
    let out = this.atoms.get(node);
    if (!out) {
      const block = blocks();
      out = (node.children ?? []).flatMap((child) => this.fromPM(block(child), 0, false));
      if (node.control.type !== "other") controlled(out, node.id);
      this.atoms.set(node, out);
    }
    return out;
  }

  private pmParagraph(node: PMNode): Omit<Paragraph, "start"> {
    const info = node.attrs.info as Info<"paragraph">;
    const inlines: Inline[] = [];
    node.forEach((child, offset) => {
      const run = runOf(child.marks);
      const effective = run?.effective ?? info.mark;
      // A whole field or revision can contain runs with their own visibility.
      if ((run || child.isText) && effective.hidden) return;
      const link = child.marks.find((m) => m.type === schema.marks.link)?.attrs.target ?? undefined;
      const inserted = insertionOf(child);
      const revision = inserted && { change: "ins", author: inserted.author ?? undefined, date: inserted.date ?? undefined };
      const style = this.style(effective, { link, revision, ...(run && marked(run)) });
      if (child.isText) {
        inlines.push({ kind: "text", text: child.text!, style, source: { at: offset, atom: false } });
      } else {
        this.inline(child.attrs.node, style, { at: offset, atom: true }, inlines);
      }
    });
    return this.paragraph(node, info, node.attrs.marker, inlines, node.content.size);
  }

  private paragraph(
    key: object,
    info: Info<"paragraph">,
    marker: string | null,
    inlines: Inline[],
    size: number,
  ): Omit<Paragraph, "start"> {
    const level = info.list?.definition;
    const mark = this.style(info.mark);
    return {
      kind: "paragraph",
      key,
      props: info.effective,
      mark,
      style: info.effective_style,
      marker: marker && level ? { text: marker, style: this.style({ ...info.mark, ...level.run }), level } : undefined,
      inlines,
      size,
      section: info.section,
      markRevision: info.mark_revision ? { change: info.mark_revision, ...info.mark_change } : undefined,
      formatChanged: !!info.format_revision,
      fields: inlines.some((i) => i.kind === "text" && !!i.style.pageField),
    };
  }

  /** One atom of a paragraph, flattened into styled inline content at the atom's position. */
  private inline(node: InspectNode, style: TextStyle, source: Source, out: Inline[]) {
    const kids = (s: TextStyle) => node.children?.forEach((c) => this.inline(c, s, source, out));
    switch (node.type) {
      case "run":
        if (node.effective.hidden) return;
        return kids(
          this.style(node.effective, { link: style.link, revision: style.revision, control: style.control, ...marked(node) }),
        );
      case "hyperlink":
        return kids({ ...style, link: hrefOf(node) ?? undefined });
      case "revision": {
        const revision = { change: node.change, author: node.author, date: node.date, move: node.move_name };
        if (!removedRevision(node)) return kids({ ...style, revision });
        const removed: Inline[] = [];
        node.children?.forEach((c) => this.inline(c, { ...style, revision }, source, removed));
        return out.push({ kind: "anchor", removed: { revision, inlines: removed }, style, source });
      }
      case "field":
        return kids(style);
      case "content_control":
        return kids(node.control.type === "other" ? style : { ...style, control: node.id });
      case "text":
        return out.push({ kind: "text", text: node.text, style: node.page_field ? { ...style, pageField: node.page_field } : style, fieldResult: node.page_field ? node.id : undefined, source });
      case "deleted_text":
        return out.push({ kind: "text", text: node.text, style, source });
      case "tab":
        return out.push({ kind: "tab", style, source });
      case "break": {
        const type = node.break_type === "page" ? "page" : node.break_type === "column" ? "column" : "line";
        return out.push({ kind: "break", type, style, source });
      }
      case "note_reference":
      case "note_mark": {
        const text = node.number ?? "";
        const raised = { ...style, ...this.style({ ...style.run, vert_align: "superscript" }), revision: style.revision };
        const note = node.type === "note_reference" ? `${node.kind}:${node.note_id}` : undefined;
        return out.push({ kind: "text", text, style: raised, source, note });
      }
      case "drawing": {
        const width = (node.width_emu ?? 0) * EMU;
        const height = (node.height_emu ?? 0) * EMU;
        const media = node.media?.find((m) => !m.external && m.part);
        const image = { part: media?.part, type: media?.content_type, description: node.description ?? node.name };
        if (node.placement?.kind === "anchor") {
          return out.push({ kind: "float", float: { width, height, image, placement: node.placement }, source });
        }
        return out.push({ kind: "box", width, height, image, style, source });
      }
      case "opaque":
        if (node.text_preview) out.push({ kind: "text", text: node.text_preview, style, source });
        return;
    }
  }

  private table(key: object, info: Info<"table">, rows: Row[]): Table {
    return {
      kind: "table",
      key,
      grid: info.grid.map((w) => w * TWIP),
      indent: info.indent_twips * TWIP,
      alignment: info.alignment,
      rows,
    };
  }

  private row(info: Info<"row">, cells: Cell[]): Row {
    return {
      header: !!info.header,
      cantSplit: !!info.cant_split,
      height: info.height ? { value: info.height.twips * TWIP, rule: info.height.rule } : undefined,
      gridBefore: info.grid_before ?? 0,
      revision: info.revision ? { change: info.revision } : undefined,
      cells,
    };
  }

  private cell(info: Info<"cell">, column: number, rowspan: number, blocks: Block[]): Cell {
    const m = info.margins_twips;
    return {
      column,
      span: info.grid_span,
      rowspan,
      borders: info.borders,
      shading: info.shading,
      margins: { top: m.top * TWIP, left: m.left * TWIP, bottom: m.bottom * TWIP, right: m.right * TWIP },
      valign: info.vertical_align,
      blocks,
    };
  }
}

/** Marks the text of a block-level content control's blocks as the control's, inside any control nested in it. */
function controlled(blocks: Block[], control: NodeId) {
  for (const block of blocks) {
    if (block.kind === "table") {
      for (const row of block.rows) for (const cell of row.cells) controlled(cell.blocks, control);
      continue;
    }
    for (const inline of block.inlines) {
      if ("style" in inline) inline.style = { ...inline.style, control: inline.style.control ?? control };
    }
  }
}

const runOf = (marks: readonly Mark[]) => marks.find((m) => m.type === schema.marks.run)?.attrs.info as Info<"run"> | undefined;
