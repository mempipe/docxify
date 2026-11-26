// Pages: the body flows down columns and across pages, each page with its
// header, footer and footnotes, as Word lays a document out.
//
// A paragraph's lines are cached by its content's identity and width, so only
// changed paragraphs break again. Placement starts again where the last
// layout placed the first changed block, and stops once a later unchanged
// block would be placed where it was: the pages from there on are the last
// layout's. A change to a header, a footer, a note or a section break lays
// the document out in full.
//
// Word's keep rules move content that has already been placed, so placement
// takes checkpoints and goes back to one when a rule needs it.

import {
  type Block,
  type Cell,
  EMU,
  type Float,
  type Image,
  type LayoutInput,
  type Paragraph,
  type Table,
  TWIP,
} from "./blocks.ts";
import { type Fields, type Line, LineBreaker } from "./lines.ts";
import type { Measurer } from "./measure.ts";
import { formatNumber } from "./numbering.ts";
import type { Border, Edges, NodeId, Section, StoryRef } from "./types.ts";

/** Something drawn on a page. Lengths are points from the page's top left. */
export type Fragment =
  | { kind: "line"; story: NodeId | null; paragraph: Paragraph; line: Line; x: number; y: number }
  | { kind: "cell"; x: number; y: number; width: number; height: number; borders: Edges<Border>; shading?: string }
  | { kind: "rule"; x: number; y: number; width: number }
  | { kind: "image"; x: number; y: number; width: number; height: number; image: Image; behind: boolean };

export interface Page {
  index: number;
  /** The number the page shows, which a section can restart. */
  number: number;
  /** How its section writes page numbers, as `w:pgNumType` names the format. */
  format?: string;
  width: number;
  height: number;
  margins: { top: number; right: number; bottom: number; left: number };
  fragments: Fragment[];
}

/** Content laid out without page breaks: a cell's, a header's or a note's. */
interface Stacked {
  fragments: Fragment[];
  height: number;
  /** Where the content may be cut across pages: below each line and table row. */
  breaks: number[];
  notes: string[];
  /** Floats anchored in the content, each with the area it is positioned in. */
  floats: Placed[];
}

interface Placed {
  float: Float;
  x: number;
  y: number;
  width: number;
}

/** Rows a page break cannot separate: one row, or rows joined by vertically merged cells. */
interface Group {
  heights: number[];
  cells: { cell: Cell; row: number; rows: number; x: number; width: number; stack: Stacked }[];
  height: number;
  header: boolean;
  splittable: boolean;
  notes: string[];
}

/** The part of a column a line may use beside floats, from the column's left edge. */
interface Room {
  left: number;
  right: number;
}

interface Exclusion {
  left: number;
  right: number;
  top: number;
  bottom: number;
  wrap: string;
}

interface Sheet {
  page: Page;
  section: number;
  /** It is the first page of its section. */
  first: boolean;
  bodyTop: number;
  bodyBottom: number;
  notes: { key: string; stack: Stacked }[];
  noteHeight: number;
  exclusions: Exclusion[];
}

/** Columns across the page from `top` down. A continuous section break starts a new band. */
interface Band {
  top: number;
  left: number;
  count: number;
  width: number;
  space: number;
  deepest: number;
}

/**
 * How the current column began, while nothing is in it yet: after a natural
 * break, where Word drops the next paragraph's space before, or a forced one.
 */
type Top = "soft" | "hard" | null;

interface Checkpoint {
  index: number;
  sheets: number;
  fragments: number;
  notes: number;
  noteHeight: number;
  exclusions: number;
  column: number;
  y: number;
  band: Band;
  top: Top;
}

/** Where placement stood before a body block, as the block was last placed. */
interface Mark extends Checkpoint {
  section: number;
  /** How many blocks back the run of keep-with-next paragraphs before it began; 0 outside one. */
  chain: number;
}

/** A layout, kept to start the next one from. */
interface Laid {
  input: LayoutInput;
  sections: Resolved[];
  sheets: Sheet[];
  marks: Mark[];
  totalsShown: boolean;
}

/** How a layout starts again from the last one. */
interface Resume {
  laid: Laid;
  /** The first body block placed again. */
  from: number;
  /** From this block on, the blocks and the one before each are the last layout's. */
  meet: number;
  /** How many more blocks the body has than the last layout's. */
  delta: number;
}

interface Resolved {
  section: Section;
  /** Its blocks: the body's from `from` up to `to`. */
  from: number;
  to: number;
  refs: Record<"headers" | "footers", Partial<Record<StoryRef["kind"], string>>>;
}

const SLACK = 0.01;
/** The footnote separator: a short rule in a line of its own. */
const SEPARATOR = { height: 12, width: 144 };
/** Narrower room beside a float than this, and a line goes below the float instead. */
const NARROWEST = 36;

/**
 * The space between two blocks: the larger of one's space after and the
 * other's before, as ISO/IEC 29500-1 17.3.1.33 has it, and not their sum.
 * Contextual spacing drops a paragraph's own space next to one in its style.
 */
function spacing(previous: Block | null, next: Block | null): number {
  const contextual = (p: Paragraph, other: Block | null) =>
    p.props.contextual_spacing && other?.kind === "paragraph" && other.style === p.style;
  const after = previous?.kind === "paragraph" && !contextual(previous, next) ? previous.props.spacing.after_twips : 0;
  const before = next?.kind === "paragraph" && !contextual(next, previous) ? next.props.spacing.before_twips : 0;
  return Math.max(after, before) * TWIP;
}

const floating = new WeakMap<object, Float[]>();

/** A paragraph's anchored floats, found once per content. */
function floatsOf(p: Paragraph): Float[] {
  let floats = floating.get(p.key);
  if (!floats) floating.set(p.key, (floats = p.inlines.flatMap((i) => (i.kind === "float" ? [i.float] : []))));
  return floats;
}

const shift = (fragment: Fragment, x: number, y: number): Fragment => ({ ...fragment, x: fragment.x + x, y: fragment.y + y });

const sameBand = (a: Band, b: Band) =>
  a.top === b.top && a.left === b.left && a.count === b.count && a.width === b.width && a.space === b.space && a.deepest === b.deepest;

const sameExclusion = (a: Exclusion, b: Exclusion) =>
  a.left === b.left && a.right === b.right && a.top === b.top && a.bottom === b.bottom && a.wrap === b.wrap;

/** Header and footer stories that are the same, block for block. */
function sameParts(a: Map<string, { blocks: Block[] }>, b: Map<string, { blocks: Block[] }>) {
  if (a.size !== b.size) return false;
  for (const [part, story] of a) if (b.get(part)?.blocks !== story.blocks) return false;
  return true;
}

/** Where an anchored float sits along one axis of the area it is positioned in. */
function along(axis: { offset_emu?: number; align?: string }, [start, end]: number[], size: number) {
  switch (axis.align) {
    case "center":
      return (start + end - size) / 2;
    case "right":
    case "bottom":
    case "outside":
      return end - size;
    case "left":
    case "top":
    case "inside":
      return start;
  }
  return start + (axis.offset_emu ?? 0) * EMU;
}

export class Paginator {
  private readonly measurer: Measurer;
  private cache = new WeakMap<object, { width: number; lines: Line[] }>();
  /** The last layout's page counts, which NUMPAGES and SECTIONPAGES show. */
  private totals = { pages: 0, sections: [] as number[] };
  private totalsShown = false;

  private input!: LayoutInput;
  private sections: Resolved[] = [];
  private current = 0;
  private defaultTab = 36;
  private sheets: Sheet[] = [];
  private band!: Band;
  private column = 0;
  private y = 0;
  private top: Top = "hard";
  /** Where the run of keep-with-next paragraphs before the current block began. */
  private chain: Checkpoint | null = null;
  private notes = new Map<string, Stacked>();
  private marks: Mark[] = [];
  private laid: Laid | null = null;
  private resume: Resume | null = null;
  /** The sheet where placement met the last layout, or -1. */
  private met = -1;

  constructor(measurer: Measurer) {
    this.measurer = measurer;
  }

  layout(input: LayoutInput): Page[] {
    const before = this.totals;
    const sections = this.resolve(input);
    let pages = this.pass(input, sections, this.resumable(input, sections));
    const changed = before.pages !== this.totals.pages || before.sections.join() !== this.totals.sections.join();
    if (this.totalsShown && changed) pages = this.pass(input, sections, null);
    return pages;
  }

  /**
   * Where this layout can start again from the last one: at the first body
   * block that changed, or the run of keep-with-next paragraphs it ends. Null
   * when anything else that places blocks changed.
   */
  private resumable(input: LayoutInput, sections: Resolved[]): Resume | null {
    const laid = this.laid;
    if (!laid) return null;
    const last = laid.input;
    if (input.body.key !== last.body.key || input.section !== last.section || input.notes !== last.notes) return null;
    if (input.settings !== last.settings || !sameParts(input.parts, last.parts)) return null;
    const before = last.body.blocks;
    const after = input.body.blocks;
    if (!before.length) return null;
    const n = Math.min(before.length, after.length);
    let head = 0;
    while (head < n && before[head] === after[head]) head++;
    let tail = 0;
    while (tail < n - head && before[before.length - 1 - tail] === after[after.length - 1 - tail]) tail++;
    const delta = after.length - before.length;

    // Section breaks stay where they were, outside the blocks that changed.
    if (sections.length !== laid.sections.length) return null;
    for (let k = 0; k < sections.length; k++) {
      const was = laid.sections[k];
      if (sections[k].section !== was.section) return null;
      if (was.to <= head ? sections[k].to !== was.to : was.to < before.length - tail || sections[k].to !== was.to + delta) return null;
    }

    let from = Math.min(head, before.length - 1);
    from -= laid.marks[from].chain;
    return { laid, from, meet: after.length - tail + 1, delta };
  }

  private pass(input: LayoutInput, sections: Resolved[], resume: Resume | null): Page[] {
    this.input = input;
    this.defaultTab = input.settings.default_tab_stop_twips * TWIP || 36;
    this.chain = null;
    this.sections = sections;
    this.resume = resume;
    this.met = -1;
    const blocks = input.body.blocks;
    let first = 0;
    let fresh = 0;
    if (resume) {
      const mark = resume.laid.marks[resume.from];
      this.restoreFrom(resume.laid, mark);
      this.totalsShown = resume.laid.totalsShown;
      this.marks = resume.laid.marks.slice(0, resume.from);
      first = mark.section;
      fresh = mark.sheets - 1;
    } else {
      this.sheets = [];
      this.notes = new Map();
      this.marks = [];
      this.totalsShown = false;
    }

    for (let index = first; index < sections.length; index++) {
      const { section, from, to } = sections[index];
      this.current = index;
      if (resume && index === first) {
        if (this.flow(blocks, input.body.key, from, resume.from, to, true)) break;
        continue;
      }
      this.start(section, index === 0);
      if (this.flow(blocks, input.body.key, from, from, to, true)) break;
    }
    const met = this.met >= 0;
    if (!met) {
      this.marks.length = blocks.length;
      const endnotes = [...input.notes].filter(([key]) => key.startsWith("endnote:")).flatMap(([, blocks]) => blocks);
      if (endnotes.length) {
        if (this.y + SEPARATOR.height > this.bottom() && this.top === null) this.nextColumn("soft");
        this.rule(this.sheet(), this.columnX(), this.y);
        this.y += SEPARATOR.height;
        this.top = null;
        this.flow(endnotes, null, 0, 0, endnotes.length, false);
      }
    }
    // A sheet taken from the last layout has its footnotes already.
    for (let k = fresh; k < (met ? this.met : this.sheets.length); k++) this.footnotes(this.sheets[k]);

    const counts = sections.map(() => 0);
    for (const sheet of this.sheets) counts[sheet.section]++;
    this.totals = { pages: this.sheets.length, sections: counts };
    this.laid = { input, sections, sheets: this.sheets, marks: this.marks, totalsShown: this.totalsShown };
    this.resume = null;
    return this.sheets.map((sheet) => sheet.page);
  }

  /** Placement as it stood at a mark of the last layout. Its sheet is copied, so the last layout's stays whole. */
  private restoreFrom(laid: Laid, mark: Mark) {
    this.sheets = laid.sheets.slice(0, mark.sheets - 1);
    const was = laid.sheets[mark.sheets - 1];
    this.sheets.push({
      ...was,
      page: { ...was.page, fragments: was.page.fragments.slice(0, mark.fragments) },
      notes: was.notes.slice(0, mark.notes),
      noteHeight: mark.noteHeight,
      exclusions: was.exclusions.slice(0, mark.exclusions),
    });
    this.current = mark.section;
    this.column = mark.column;
    this.y = mark.y;
    this.band = mark.band;
    this.top = mark.top;
  }

  /** Where placement stands before body block `index`. */
  private mark(index: number): Mark {
    const sheet = this.sheet();
    return {
      index,
      sheets: this.sheets.length,
      fragments: sheet.page.fragments.length,
      notes: sheet.notes.length,
      noteHeight: sheet.noteHeight,
      exclusions: sheet.exclusions.length,
      column: this.column,
      y: this.y,
      band: this.band,
      top: this.top,
      section: this.current,
      chain: this.chain ? index - this.chain.index : 0,
    };
  }

  /**
   * Whether placement stands before body block `index` where the last layout
   * placed it. Then everything after is placed as it was, so the rest of the
   * pages are taken from the last layout.
   */
  private meet(index: number): boolean {
    const { laid, delta } = this.resume!;
    const mark = laid.marks[index - delta];
    const sheet = this.sheet();
    if (!mark || mark.chain || this.chain || mark.section !== this.current || mark.sheets !== this.sheets.length) return false;
    if (mark.column !== this.column || mark.y !== this.y || mark.top !== this.top || !sameBand(mark.band, this.band)) return false;
    if (mark.noteHeight !== sheet.noteHeight || mark.notes !== sheet.notes.length || mark.exclusions !== sheet.exclusions.length) return false;
    const was = laid.sheets[mark.sheets - 1];
    if (was.first !== sheet.first || was.page.number !== sheet.page.number) return false;
    if (was.bodyTop !== sheet.bodyTop || was.bodyBottom !== sheet.bodyBottom) return false;
    for (let k = 0; k < mark.notes; k++) if (was.notes[k].key !== sheet.notes[k].key) return false;
    for (let k = 0; k < mark.exclusions; k++) if (!sameExclusion(was.exclusions[k], sheet.exclusions[k])) return false;

    const moved = sheet.page.fragments.length - mark.fragments;
    for (let k = mark.fragments; k < was.page.fragments.length; k++) sheet.page.fragments.push(was.page.fragments[k]);
    for (let k = mark.notes; k < was.notes.length; k++) sheet.notes.push(was.notes[k]);
    for (let k = mark.exclusions; k < was.exclusions.length; k++) sheet.exclusions.push(was.exclusions[k]);
    sheet.noteHeight = was.noteHeight;
    this.met = this.sheets.length - 1;
    for (let k = this.sheets.length; k < laid.sheets.length; k++) this.sheets.push(laid.sheets[k]);

    // The marks on this sheet count its fragments, which now start from this layout's.
    this.marks.length = index;
    let k = index - delta;
    for (; k < laid.marks.length && laid.marks[k].sheets === mark.sheets; k++) {
      const m = laid.marks[k];
      this.marks.push(moved ? { ...m, fragments: m.fragments + moved } : m);
    }
    this.marks = this.marks.concat(laid.marks.slice(k));
    return true;
  }

  /** Splits the body into its sections, each with the headers and footers it has or inherits. */
  private resolve(input: LayoutInput): Resolved[] {
    const out: Resolved[] = [];
    let from = 0;
    const close = (section: Section, to: number) => {
      const before = out.at(-1)?.refs ?? { headers: {}, footers: {} };
      const refs = { headers: { ...before.headers }, footers: { ...before.footers } };
      for (const ref of section.headers ?? []) if (ref.part) refs.headers[ref.kind] = ref.part;
      for (const ref of section.footers ?? []) if (ref.part) refs.footers[ref.kind] = ref.part;
      out.push({ section, from, to, refs });
      from = to;
    };
    const blocks = input.body.blocks;
    for (let i = 0; i < blocks.length; i++) {
      const block = blocks[i];
      if (block.kind === "paragraph" && block.section) close(block.section, i + 1);
    }
    if (from < blocks.length || !out.length) close(input.section, blocks.length);
    return out;
  }

  private sheet() {
    return this.sheets.at(-1)!;
  }

  private columnX() {
    return this.band.left + this.column * (this.band.width + this.band.space);
  }

  private bottom() {
    const sheet = this.sheet();
    return sheet.bodyBottom - sheet.noteHeight;
  }

  private bandFor(page: Page, top: number): Band {
    const columns = this.sections[this.current].section.columns;
    const count = Math.max(1, columns?.count ?? 1);
    const space = (columns?.space_twips ?? 720) * TWIP;
    const text = page.width - page.margins.left - page.margins.right;
    return { top, left: page.margins.left, count, width: (text - (count - 1) * space) / count, space, deepest: top };
  }

  /** Starts a section the way its break says: on a new page, an odd or even one, a new column, or below what is there. */
  private start(section: Section, first: boolean) {
    if (first) return this.newPage(true);
    switch (section.start) {
      case "continuous": {
        const top = Math.max(this.band.deepest, this.y);
        this.band = this.bandFor(this.sheet().page, top);
        this.column = 0;
        this.y = top;
        this.top = "hard";
        return;
      }
      case "nextColumn":
        return this.nextColumn("hard");
      case "oddPage":
      case "evenPage": {
        const number = section.page_numbers?.start ?? this.sheet().page.number + 1;
        if (number % 2 !== (section.start === "oddPage" ? 1 : 0)) this.newPage(false);
        return this.newPage(true);
      }
    }
    this.newPage(true);
  }

  private newPage(first = false) {
    const { section } = this.sections[this.current];
    const twips = (value: number | undefined, fallback: number) => (value ?? fallback) * TWIP;
    const m = section.margins ?? {};
    const width = twips(section.page_width, 12240);
    const height = twips(section.page_height, 15840);
    const margins = {
      top: Math.abs(twips(m.top, 1440)),
      right: twips(m.right, 1440),
      bottom: Math.abs(twips(m.bottom, 1440)),
      left: twips(m.left, 1440),
    };
    const previous = this.sheets.at(-1)?.page.number ?? 0;
    const number = (first ? section.page_numbers?.start : undefined) ?? previous + 1;
    const format = section.page_numbers?.format;
    const page: Page = { index: this.sheets.length, number, format, width, height, margins, fragments: [] };
    const sheet: Sheet = {
      page,
      section: this.current,
      first,
      bodyTop: margins.top,
      bodyBottom: height - margins.bottom,
      notes: [],
      noteHeight: 0,
      exclusions: [],
    };
    this.sheets.push(sheet);

    // A header or footer taller than its margin pushes the body away; a negative margin keeps it fixed.
    const text = width - margins.left - margins.right;
    const header = this.storyFor(sheet, "headers");
    if (header) {
      const at = twips(m.header, 720);
      const stack = this.stack(header.blocks, header.key, text, this.fields(sheet));
      this.put(sheet, stack, margins.left, at);
      if ((m.top ?? 0) >= 0) sheet.bodyTop = Math.max(sheet.bodyTop, at + stack.height);
    }
    const footer = this.storyFor(sheet, "footers");
    if (footer) {
      const stack = this.stack(footer.blocks, footer.key, text, this.fields(sheet));
      const at = height - twips(m.footer, 720) - stack.height;
      this.put(sheet, stack, margins.left, at);
      if ((m.bottom ?? 0) >= 0) sheet.bodyBottom = Math.min(sheet.bodyBottom, at);
    }
    this.band = this.bandFor(page, sheet.bodyTop);
    this.column = 0;
    this.y = sheet.bodyTop;
    this.top = "hard";
  }

  /** The header or footer a page shows: its section's first-page or even-page one where those apply. */
  private storyFor(sheet: Sheet, which: "headers" | "footers") {
    const { section, refs } = this.sections[sheet.section];
    let kind: StoryRef["kind"] = "default";
    if (sheet.first && section.title_page) kind = "first";
    else if (this.input.settings.even_and_odd_headers && sheet.page.number % 2 === 0) kind = "even";
    const part = refs[which][kind];
    return part ? this.input.parts.get(part) : undefined;
  }

  /** What the page fields show on a page. The totals come from the last layout and are checked after this one. */
  private fields(sheet: Sheet): Fields {
    return ({ number, format }) => {
      if (number === "page") return formatNumber(sheet.page.number, format ?? this.sections[sheet.section].section.page_numbers?.format);
      this.totalsShown = true;
      const total = number === "num_pages" ? this.totals.pages : (this.totals.sections[sheet.section] ?? 0);
      return format ? formatNumber(total, format) : String(total);
    };
  }

  private nextColumn(top: Exclude<Top, null>) {
    // A band is never changed in place, so checkpoints share it.
    this.band = { ...this.band, deepest: Math.max(this.band.deepest, this.y) };
    if (this.column + 1 < this.band.count) {
      this.column++;
      this.y = this.band.top;
    } else {
      this.newPage();
    }
    this.top = top;
  }

  /** Where placement stands, in a new checkpoint or written into `into`. */
  private checkpoint(index: number, into?: Checkpoint): Checkpoint {
    const sheet = this.sheet();
    const at = into ?? ({} as Checkpoint);
    at.index = index;
    at.sheets = this.sheets.length;
    at.fragments = sheet.page.fragments.length;
    at.notes = sheet.notes.length;
    at.noteHeight = sheet.noteHeight;
    at.exclusions = sheet.exclusions.length;
    at.column = this.column;
    at.y = this.y;
    at.band = this.band;
    at.top = this.top;
    return at;
  }

  private restore(at: Checkpoint) {
    this.sheets.length = at.sheets;
    const sheet = this.sheet();
    sheet.page.fragments.length = at.fragments;
    sheet.notes.length = at.notes;
    sheet.noteHeight = at.noteHeight;
    sheet.exclusions.length = at.exclusions;
    this.column = at.column;
    this.y = at.y;
    this.band = at.band;
    this.top = at.top;
    this.chain = null;
  }

  /**
   * Lays blocks into the body's columns and pages: from `from` up to `to`, in
   * a run that begins at `first`. For the body it marks where each block
   * starts, and returns true when it met the last layout.
   */
  private flow(blocks: Block[], story: NodeId | null, first: number, from: number, to: number, body: boolean): boolean {
    // A block placed again after a keep rule moved it keeps the mark from before
    // the move: a changed block starts from there, where it may now fit.
    for (let i = from, again = false; i < to; ) {
      if (body) {
        if (this.resume && i >= this.resume.meet && this.meet(i)) return true;
        if (!again) this.marks[i] = this.mark(i);
      }
      const block = blocks[i];
      const previous = i > first ? blocks[i - 1] : null;
      const keep = block.kind === "paragraph" && block.props.keep_next;
      const at = keep && !this.chain ? this.checkpoint(i) : null;
      const back = block.kind === "paragraph" ? this.paragraph(block, i, story, previous) : this.table(block, story, previous);
      if (back !== null) {
        i = back;
        again = true;
        continue;
      }
      this.chain = keep ? (this.chain ?? at) : null;
      i++;
      again = false;
    }
    return false;
  }

  /**
   * When the block after a run of keep-with-next paragraphs does not fit, the
   * run moves to the next column with it, unless the run already starts one.
   * Returns the block to continue from.
   */
  private keepChain(): number | null {
    const chain = this.chain;
    if (!chain || chain.top !== null || chain.sheets !== this.sheets.length || chain.column !== this.column) return null;
    this.restore(chain);
    this.nextColumn("soft");
    return chain.index;
  }

  /** Places a paragraph's lines, keeping them together as its keep rules say. Returns where to continue when it moved. */
  private paragraph(p: Paragraph, index: number, story: NodeId | null, previous: Block | null): number | null {
    if (p.props.page_break_before && !(this.column === 0 && this.top !== null)) this.newPage();
    const start = this.checkpoint(index);
    if (this.top !== "soft") this.y += spacing(previous, p);
    for (const float of floatsOf(p)) this.anchor(this.sheet(), float, this.y, this.columnX(), this.band.width);
    const source = this.source(p);
    const metrics = this.measurer.metrics(p.mark.font);
    const next = () => source(this.besideFloats(metrics.ascent + metrics.descent));

    // The line placed last, and where placement stood before it, for a
    // widowed last line to take along. One checkpoint is reused for every line.
    let last: Line | null = null;
    const before = { ...start };
    let placed = 0;
    let here = 0;
    for (let line = next(); line; ) {
      if (this.top === null && !this.fits(line.height, line.notes)) {
        const startedHere = here === placed && start.top === null;
        if (here === 0) {
          const back = this.keepChain();
          if (back !== null) return back;
          // Starting again at the next column drops the space before and puts its floats there.
          this.restore(start);
          this.nextColumn("soft");
          return index;
        }
        const widow = p.props.widow_control && line.end === "paragraph";
        if (startedHere && (p.props.keep_lines || (p.props.widow_control && here === 1) || (widow && here === 2))) {
          this.restore(start);
          this.nextColumn("soft");
          return index;
        }
        if (widow && here >= 2) {
          // The paragraph's last line takes the line before it along.
          this.restore(before);
          this.nextColumn("soft");
          this.place(last!, story, p);
          here = 1;
          continue;
        }
        this.nextColumn("soft");
        here = 0;
        continue;
      }
      last = line;
      this.checkpoint(index, before);
      this.place(line, story, p);
      placed++;
      here++;
      if (line.end === "page") {
        this.newPage();
        here = 0;
      } else if (line.end === "column") {
        this.nextColumn("hard");
        here = 0;
      }
      line = next();
    }
    return null;
  }

  /**
   * A paragraph's lines in the body, each given the room floats leave it.
   * They come from the cache unless a float on the page can narrow them or
   * the paragraph shows a page field.
   */
  private source(p: Paragraph): (room: Room | undefined) => Line | null {
    const sheet = this.sheet();
    const width = this.band.width;
    const fields = p.fields ? this.fields(sheet) : undefined;
    if (!sheet.exclusions.length) {
      const lines = this.lines(p, width, fields);
      let k = 0;
      return () => lines[k++] ?? null;
    }
    const breaker = new LineBreaker(p, this.measurer, this.defaultTab, fields);
    return (room) => breaker.next(width, room);
  }

  private lines(p: Paragraph, width: number, fields?: Fields): Line[] {
    const live = p.fields && fields;
    const cached = live ? undefined : this.cache.get(p.key);
    if (cached?.width === width) return cached.lines;
    const breaker = new LineBreaker(p, this.measurer, this.defaultTab, fields);
    const lines: Line[] = [];
    for (let line = breaker.next(width); line; line = breaker.next(width)) lines.push(line);
    if (!live) this.cache.set(p.key, { width, lines });
    return lines;
  }

  /**
   * Moves below the floats the next line cannot share a line with, and
   * returns the part of the column it may use beside the others.
   */
  private besideFloats(height: number): Room | undefined {
    const x = this.columnX();
    const width = this.band.width;
    for (;;) {
      let left = 0;
      let right = width;
      let below = -Infinity;
      for (const ex of this.sheet().exclusions) {
        if (ex.bottom <= this.y || ex.top >= this.y + height || ex.right <= x || ex.left >= x + width) continue;
        below = Math.max(below, ex.bottom);
        if (ex.wrap === "top_and_bottom") {
          left = right;
          break;
        }
        // The text takes the wider side.
        if (ex.left - x >= x + width - ex.right) right = Math.min(right, ex.left - x);
        else left = Math.max(left, ex.right - x);
      }
      if (right - left >= NARROWEST || below === -Infinity) {
        return left > 0 || right < width ? { left, right } : undefined;
      }
      this.y = below;
    }
  }

  private fits(height: number, notes: string[]) {
    return this.y + height + this.noteGrowth(notes) <= this.bottom() + SLACK;
  }

  private place(line: Line, story: NodeId | null, paragraph: Paragraph) {
    this.sheet().page.fragments.push({ kind: "line", story, paragraph, line, x: this.columnX(), y: this.y });
    this.addNotes(line.notes);
    this.y += line.height;
    this.top = null;
  }

  /** Places a table row by row, splitting rows across pages where they allow it and repeating header rows. */
  private table(table: Table, story: NodeId | null, previous: Block | null): number | null {
    if (this.top !== "soft") this.y += spacing(previous, table);
    // A row's lines are broken before it is placed, when its page is not yet
    // known, so page fields in a body table keep the result Word stored.
    const groups = this.groups(table, story, this.band.width, undefined);
    const headers = groups.slice(0, Math.max(0, groups.findIndex((g) => !g.header)));
    /** Nothing but repeated header rows is in the column yet, so a row too tall for it goes in anyway. */
    let fresh = this.top !== null;
    const nextColumn = (g: number) => {
      this.nextColumn("soft");
      if (g >= headers.length) for (const header of headers) this.putGroup(header);
      fresh = true;
    };
    for (let g = 0; g < groups.length; g++) {
      let group = groups[g];
      for (;;) {
        const room = this.bottom() - this.y - this.noteGrowth(group.notes);
        const parts = group.height > room + SLACK && group.splittable ? this.split(group, room) : null;
        if (group.height <= room + SLACK || (fresh && !parts)) {
          this.putGroup(group);
          fresh = false;
          break;
        }
        if (parts) {
          this.putGroup(parts[0]);
          group = parts[1];
        } else if (g === 0) {
          const back = this.keepChain();
          if (back !== null) return back;
        }
        nextColumn(g);
      }
    }
    return null;
  }

  private putGroup(group: Group) {
    const sheet = this.sheet();
    const { fragments, floats } = this.emit(group, this.columnX(), this.y);
    sheet.page.fragments.push(...fragments);
    for (const f of floats) this.anchor(sheet, f.float, f.y, f.x, f.width);
    this.addNotes(group.notes);
    this.y += group.height;
    this.top = null;
  }

  /** A table's rows, with every cell laid out at its width from the table grid. */
  private groups(table: Table, story: NodeId | null, width: number, fields: Fields | undefined): Group[] {
    let grid = table.grid;
    if (!grid.some((w) => w > 0)) {
      // Without a grid, the cells share the width equally.
      const columns = Math.max(1, ...table.rows.map((r) => r.cells.reduce((end, cell) => Math.max(end, cell.column + cell.span), r.gridBefore)));
      grid = Array.from({ length: columns }, () => width / columns);
    }
    const offsets = [0];
    for (const w of grid) offsets.push(offsets.at(-1)! + w);
    const at = (column: number) => offsets[Math.min(column, offsets.length - 1)];
    const total = at(grid.length);
    const left = table.alignment === "center" ? (width - total) / 2 : table.alignment === "right" ? width - total : table.indent;

    const rows = table.rows.map((row, r) => {
      return row.cells.map((cell) => {
        const column = cell.column;
        const x = left + at(column);
        const cellWidth = at(column + cell.span) - at(column);
        const spanned = Math.max(1, cell.rowspan);
        const stack = this.stack(cell.blocks, story, cellWidth - cell.margins.left - cell.margins.right, fields);
        return { cell, row: r, rows: spanned, x, width: cellWidth, stack };
      });
    });
    const need = (c: Group["cells"][number]) => c.stack.height + c.cell.margins.top + c.cell.margins.bottom;
    const heights = rows.map((cells, r) => {
      const natural = Math.max(0, ...cells.filter((c) => c.rows === 1).map(need));
      const rule = table.rows[r].height;
      if (rule?.rule === "exact") return rule.value;
      return Math.max(natural, rule?.value ?? 0);
    });
    // A merged cell taller than the rows it spans makes the last of them taller.
    for (const cells of rows) {
      for (const c of cells) {
        if (c.rows === 1) continue;
        const last = Math.min(c.row + c.rows, rows.length) - 1;
        const have = heights.slice(c.row, last + 1).reduce((a, b) => a + b, 0);
        if (need(c) > have && table.rows[last].height?.rule !== "exact") heights[last] += need(c) - have;
      }
    }

    const groups: Group[] = [];
    for (let r = 0; r < rows.length; ) {
      let end = r;
      for (let k = r; k <= end; k++) for (const c of rows[k]) end = Math.min(rows.length - 1, Math.max(end, k + c.rows - 1));
      const cells = rows.slice(r, end + 1).flat().map((c) => ({ ...c, row: c.row - r }));
      const groupHeights = heights.slice(r, end + 1);
      groups.push({
        heights: groupHeights,
        cells,
        height: groupHeights.reduce((a, b) => a + b, 0),
        header: table.rows[r].header,
        splittable: end === r && !table.rows[r].cantSplit && table.rows[r].height?.rule !== "exact",
        notes: cells.flatMap((c) => c.stack.notes),
      });
      r = end + 1;
    }
    return groups;
  }

  /** Divides a row so its first part fits in `room`, cutting each cell below a line. */
  private split(group: Group, room: number): [Group, Group] | null {
    const heads: Group["cells"] = [];
    const tails: Group["cells"] = [];
    for (const c of group.cells) {
      const fit = room - c.cell.margins.top - c.cell.margins.bottom;
      const cut = Math.max(0, ...c.stack.breaks.filter((b) => b <= fit + SLACK));
      const [head, tail] = cutStack(c.stack, cut);
      heads.push({ ...c, stack: head });
      tails.push({ ...c, stack: tail });
    }
    if (!heads.some((c) => c.stack.fragments.length) || !tails.some((c) => c.stack.fragments.length)) return null;
    const part = (cells: Group["cells"]): Group => {
      const height = Math.max(...cells.map((c) => c.stack.height + c.cell.margins.top + c.cell.margins.bottom));
      return { ...group, cells, heights: [height], height, notes: cells.flatMap((c) => c.stack.notes) };
    };
    return [part(heads), part(tails)];
  }

  /** A group's cells, borders and content at `x` and `y`, and the floats anchored in its cells. */
  private emit(group: Group, x: number, y: number): { fragments: Fragment[]; floats: Placed[] } {
    const fragments: Fragment[] = [];
    const floats: Placed[] = [];
    const offsets = [0];
    for (const h of group.heights) offsets.push(offsets.at(-1)! + h);
    for (const c of group.cells) {
      const top = y + offsets[c.row];
      const height = offsets[Math.min(c.row + c.rows, group.heights.length)] - offsets[c.row];
      const { margins, valign, borders, shading } = c.cell;
      fragments.push({ kind: "cell", x: x + c.x, y: top, width: c.width, height, borders, shading });
      const free = Math.max(0, height - margins.top - margins.bottom - c.stack.height);
      const contentX = x + c.x + margins.left;
      const contentY = top + margins.top + (valign === "center" ? free / 2 : valign === "bottom" ? free : 0);
      for (const fragment of c.stack.fragments) fragments.push(shift(fragment, contentX, contentY));
      for (const f of c.stack.floats) floats.push({ ...f, x: contentX + f.x, y: contentY + f.y });
    }
    return { fragments, floats };
  }

  /** Lays blocks out at `width` without page breaks. */
  private stack(blocks: Block[], story: NodeId | null, width: number, fields: Fields | undefined): Stacked {
    const out: Stacked = { fragments: [], height: 0, breaks: [], notes: [], floats: [] };
    let y = 0;
    let previous: Block | null = null;
    for (const block of blocks) {
      y += spacing(previous, block);
      if (block.kind === "paragraph") {
        for (const float of floatsOf(block)) out.floats.push({ float, x: 0, y, width });
        for (const line of this.lines(block, width, fields)) {
          out.fragments.push({ kind: "line", story, paragraph: block, line, x: 0, y });
          out.notes.push(...line.notes);
          y += line.height;
          out.breaks.push(y);
        }
      } else {
        for (const group of this.groups(block, story, width, fields)) {
          const { fragments, floats } = this.emit(group, 0, y);
          out.fragments.push(...fragments);
          out.floats.push(...floats);
          out.notes.push(...group.notes);
          y += group.height;
          out.breaks.push(y);
        }
      }
      previous = block;
    }
    out.height = y + spacing(previous, null);
    return out;
  }

  /** Draws stacked content on a page at `x` and `y`, with its floats. */
  private put(sheet: Sheet, stack: Stacked, x: number, y: number) {
    for (const fragment of stack.fragments) sheet.page.fragments.push(shift(fragment, x, y));
    for (const f of stack.floats) this.anchor(sheet, f.float, y + f.y, x + f.x, f.width);
  }

  /**
   * Places a float where its anchor says, and keeps text off it unless it
   * wraps none. `paragraphTop` is where its paragraph starts, and `left` and
   * `width` the column or cell it is in.
   */
  private anchor(sheet: Sheet, float: Float, paragraphTop: number, left: number, width: number) {
    const { page } = sheet;
    const m = page.margins;
    const { horizontal, vertical, wrap, behind, distance_emu: d } = float.placement;
    const across: Record<string, number[]> = {
      page: [0, page.width],
      margin: [m.left, page.width - m.right],
      leftMargin: [0, m.left],
      rightMargin: [page.width - m.right, page.width],
      insideMargin: [0, m.left],
      outsideMargin: [page.width - m.right, page.width],
    };
    const down: Record<string, number[]> = {
      page: [0, page.height],
      margin: [m.top, page.height - m.bottom],
      topMargin: [0, m.top],
      bottomMargin: [page.height - m.bottom, page.height],
    };
    const x = along(horizontal, across[horizontal.relative_to] ?? [left, left + width], float.width);
    const y = along(vertical, down[vertical.relative_to] ?? [paragraphTop, paragraphTop], float.height);
    page.fragments.push({ kind: "image", x, y, width: float.width, height: float.height, image: float.image, behind });
    if (wrap === "none") return;
    sheet.exclusions.push({
      left: x - (d.left ?? 0) * EMU,
      right: x + float.width + (d.right ?? 0) * EMU,
      top: y - (d.top ?? 0) * EMU,
      bottom: y + float.height + (d.bottom ?? 0) * EMU,
      wrap,
    });
  }

  /** A note laid out at the width of the current page's text, kept for later pages and layouts at that width. */
  private noteStack(key: string): Stacked | undefined {
    const page = this.sheet().page;
    const width = page.width - page.margins.left - page.margins.right;
    let stack = this.notes.get(`${key}@${width}`);
    const blocks = this.input.notes.get(key);
    if (!stack && blocks) {
      stack = this.stack(blocks, null, width, undefined);
      this.notes.set(`${key}@${width}`, stack);
    }
    return stack;
  }

  /** How much taller the current page's footnotes grow with the footnotes `keys` refer to. */
  private noteGrowth(keys: string[]): number {
    if (!keys.length) return 0;
    const sheet = this.sheet();
    let height = 0;
    for (const key of new Set(keys)) {
      if (!key.startsWith("footnote:") || sheet.notes.some((n) => n.key === key)) continue;
      height += this.noteStack(key)?.height ?? 0;
    }
    return height && !sheet.notes.length ? height + SEPARATOR.height : height;
  }

  private addNotes(keys: string[]) {
    if (!keys.length) return;
    const sheet = this.sheet();
    for (const key of new Set(keys)) {
      if (!key.startsWith("footnote:") || sheet.notes.some((n) => n.key === key)) continue;
      const stack = this.noteStack(key);
      if (!stack) continue;
      if (!sheet.notes.length) sheet.noteHeight += SEPARATOR.height;
      sheet.notes.push({ key, stack });
      sheet.noteHeight += stack.height;
    }
  }

  /** Draws a page's footnotes at the bottom of its body, below a separator. */
  private footnotes(sheet: Sheet) {
    if (!sheet.notes.length) return;
    const { margins } = sheet.page;
    let y = sheet.bodyBottom - sheet.noteHeight;
    this.rule(sheet, margins.left, y);
    y += SEPARATOR.height;
    for (const { stack } of sheet.notes) {
      this.put(sheet, stack, margins.left, y);
      y += stack.height;
    }
  }

  private rule(sheet: Sheet, x: number, y: number) {
    const { margins, width } = sheet.page;
    const length = Math.min(SEPARATOR.width, width - margins.left - margins.right);
    sheet.page.fragments.push({ kind: "rule", x, y: y + SEPARATOR.height / 2, width: length });
  }
}

/** Stacked content cut at `y`: what is above, and what is below moved up to start at zero. */
function cutStack(stack: Stacked, y: number): [Stacked, Stacked] {
  const notesOf = (fragments: Fragment[]) => fragments.flatMap((f) => (f.kind === "line" ? f.line.notes : []));
  const above = stack.fragments.filter((f) => f.y < y);
  const below = stack.fragments.filter((f) => f.y >= y).map((f) => shift(f, 0, -y));
  return [
    { fragments: above, height: y, breaks: stack.breaks.filter((b) => b <= y), notes: notesOf(above), floats: stack.floats.filter((f) => f.y < y) },
    {
      fragments: below,
      height: stack.height - y,
      breaks: stack.breaks.filter((b) => b > y).map((b) => b - y),
      notes: notesOf(below),
      floats: stack.floats.filter((f) => f.y >= y).map((f) => ({ ...f, y: f.y - y })),
    },
  ];
}
