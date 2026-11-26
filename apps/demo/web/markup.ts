// Word's markup area: a balloon beside the page for each thing a tracked
// change removed, moved or reformatted, and for each comment, level with the
// line it belongs to. Balloons are found on the laid-out pages, so each sits
// where its content was drawn, and their text goes through the pages' own
// line breaker, so it measures and paints like the rest of the page.

import type { Comment, Inline, Paragraph, TextStyle } from "./blocks.ts";
import { type Line, LineBreaker, type Piece } from "./lines.ts";
import type { Measurer } from "./measure.ts";
import type { Fragment, Page } from "./paginate.ts";
import type { EffectiveParagraph, EffectiveRun, NodeId, RunProps } from "./types.ts";

/** The markup area's width beside a page, as Word shows it. */
export const MARKUP_WIDTH = 3.3 * 72;
/** The label on a comment or revision, including the first one added after open. */
export const BALLOON_FONT = "Times New Roman";
/** Where balloon text starts, from the markup area's left edge. */
export const BALLOON_INSET = 7.5;
/** Room kept at the right for a move's arrow. */
export const ARROW_WIDTH = 18;
/** Above and below a balloon's text. */
const PADDING = 2;
/** Below a comment's text, for its Reply button. */
export const REPLY_HEIGHT = 14;
const GAP = 4.5;
/** Word's colours for the first two authors, sampled from Word; it gives each author the next colour in order. */
const AUTHOR_COLORS = ["#c0423f", "#3376cd", "#7a4fa3", "#b5651d", "#1f7a7a", "#a63d74", "#5c6f2b", "#4d5a99"];
export const MOVE_COLOR = "#3a6139";

export interface Balloon {
  color: string;
  /** Where its connector starts on the page: after what it describes, at the top of its line. */
  anchor: { x: number; y: number };
  top: number;
  height: number;
  /** From its top to the middle of the line under the author's name, where the connector and a move's arrow meet it. */
  point: number;
  /** Its text, each line with its offset from the balloon's top. */
  lines: { line: Line; paragraph: Paragraph; y: number }[];
  /** The positions it describes in an editor's view, which the caret selects it by. */
  story: NodeId | null;
  from: number;
  to: number;
  /** For a move, the position of its other end. */
  partner?: number;
  /** For a comment, its ID, which a reply answers. */
  comment?: string;
}

export interface Markup {
  /** The markup area's width beside every page: 0 when nothing needs it. */
  width: number;
  /** Each page's balloons, top to bottom. */
  pages: Balloon[][];
  /** Each author's colour, in the order the authors first appear. */
  colors: Map<string, string>;
  /** The colour each comment's range is shaded in, by comment ID. */
  comments: Map<string, string>;
}

export const NO_MARKUP: Markup = { width: 0, pages: [], colors: new Map(), comments: new Map() };

type Kind = "deleted" | "moveFrom" | "moveTo" | "formatted" | "comment";

/** Where something was found on the pages, and the positions it covers in its story; -1 where no editor addresses it. */
interface Found {
  page: number;
  x: number;
  y: number;
  story: NodeId | null;
  from: number;
  to: number;
}

/** Something a balloon will describe. */
interface Item extends Found {
  kind: Kind;
  author?: string;
  /** What the balloon quotes: removed content or a description; for a comment, the comment's ID. */
  body: Inline[] | string;
  /** The revision or description that adjacent pieces share when they are one change. */
  same?: unknown;
  move?: string;
}

const PLAIN: EffectiveParagraph = {
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

/** Word's wording for a change to a run's direct formatting. */
export function describe(before: RunProps, after: RunProps): string {
  const font: string[] = [];
  const other: string[] = [];
  if (after.font && after.font !== before.font) font.push(after.font);
  if (after.size_half_points && after.size_half_points !== before.size_half_points) font.push(`${after.size_half_points / 2} pt`);
  const flags: [keyof RunProps, string][] = [
    ["bold", "Bold"],
    ["italic", "Italic"],
    ["strike", "Strikethrough"],
    ["caps", "All caps"],
    ["small_caps", "Small caps"],
    ["hidden", "Hidden"],
  ];
  for (const [key, name] of flags) if (!!after[key] !== !!before[key]) font.push(after[key] ? name : `Not ${name}`);
  const underlined = (p: RunProps) => !!p.underline && p.underline !== "none";
  if (underlined(after) !== underlined(before)) font.push(underlined(after) ? "Underline" : "No underline");
  if (after.vert_align !== before.vert_align) {
    font.push(after.vert_align === "superscript" ? "Superscript" : after.vert_align === "subscript" ? "Subscript" : "Not Superscript/ Subscript");
  }
  if (after.color !== before.color) other.push(`Font color: ${after.color ? `#${after.color}` : "Auto"}`);
  if (after.highlight !== before.highlight) other.push(after.highlight ? "Highlight" : "Not Highlight");
  return [...(font.length ? [`Font: ${font.join(", ")}`] : []), ...other].join(", ");
}

/**
 * Places balloons as near their lines as they fit without overlapping. Each
 * run of balloons that would overlap is centred on where its members want to
 * be; then the page bounds them, and any that cannot fit run off the bottom.
 */
export function place(wanted: { top: number; height: number }[], bottom: number, gap = GAP): number[] {
  // `sum` adds each member's wanted top less its offset in the run, so the run's best top is their mean.
  const runs: { first: number; height: number; sum: number; count: number; top: number }[] = [];
  wanted.forEach((b, i) => {
    let run = { first: i, height: b.height, sum: b.top, count: 1, top: b.top };
    for (let before = runs.at(-1); before && before.top + before.height + gap > run.top; before = runs.at(-1)) {
      runs.pop();
      const sum = before.sum + run.sum - run.count * (before.height + gap);
      const count = before.count + run.count;
      run = { first: before.first, height: before.height + gap + run.height, sum, count, top: sum / count };
    }
    runs.push(run);
  });
  const tops: number[] = [];
  for (const run of runs) {
    let y = run.top;
    for (let i = run.first; i < run.first + run.count; i++) {
      tops.push(y);
      y += wanted[i].height + gap;
    }
  }
  let ceiling = bottom;
  for (let i = tops.length - 1; i >= 0; i--) {
    tops[i] = Math.min(tops[i], ceiling - wanted[i].height);
    ceiling = tops[i] - gap;
  }
  let floor = 0;
  for (let i = 0; i < tops.length; i++) {
    tops[i] = Math.max(tops[i], floor);
    floor = tops[i] + wanted[i].height + gap;
  }
  return tops;
}

const showing = new WeakMap<object, boolean>();

/** Whether a paragraph holds anything the markup area describes or colours, checked once per content. */
function shows(p: Paragraph): boolean {
  let shown = showing.get(p.key);
  if (shown === undefined) {
    shown =
      p.markRevision?.change === "del" ||
      p.inlines.some((i) => i.kind === "anchor" || ("style" in i && !!(i.style.revision || i.style.formatted || i.style.comments)));
    showing.set(p.key, shown);
  }
  return shown;
}

function found(page: number, f: Extract<Fragment, { kind: "line" }>, piece: Piece): Found {
  const start = f.paragraph.start;
  const span = piece.span;
  return {
    page,
    x: f.x + piece.x,
    y: f.y,
    story: start === null ? null : f.story,
    from: start === null || !span ? -1 : start + span.from,
    to: start === null || !span ? -1 : start + span.to,
  };
}

/**
 * The balloons for laid-out pages. `style` gives a run's text style, with its
 * font resolved as the pages resolve it; `comments` are the document's.
 */
export function markup(
  pages: Page[],
  comments: Map<string, Comment>,
  measurer: Measurer,
  style: (run: EffectiveRun) => TextStyle,
): Markup {
  const items: Item[] = [];
  // A reply is drawn in the balloon of the comment it answers.
  const threaded = (comment: Comment | undefined) => !!comment?.parent && comments.has(comment.parent);
  const authors: string[] = [];
  const met = (author = "") => void (authors.includes(author) || authors.push(author));
  const covered = new Map<string, Item>();
  const last = new Map<Kind, Item>();
  // A change goes on as one balloon while its pieces follow each other; an atom's pieces share its one position.
  const add = (at: Found, kind: Kind, author: string | undefined, body: Item["body"], same?: unknown, move?: string) => {
    met(author);
    const before = last.get(kind);
    if (same !== undefined && before?.same === same && before.story === at.story && at.from <= before.to) {
      before.to = at.to;
      return;
    }
    const item = { ...at, kind, author, body, same, move };
    items.push(item);
    last.set(kind, item);
  };

  pages.forEach((page, index) => {
    for (const f of page.fragments) {
      if (f.kind !== "line" || !shows(f.paragraph)) continue;
      for (const piece of f.line.pieces) {
        const { inline, style: s } = piece;
        const mark = !inline && !!piece.span && f.paragraph.markRevision?.change === "del";
        if (inline?.kind !== "anchor" && !mark && !(inline?.kind === "text" && (s.revision || s.formatted || s.comments))) continue;
        const at = found(index, f, piece);
        if (inline?.kind === "anchor") {
          const { revision, inlines } = inline.removed;
          add(at, revision.change === "moveFrom" ? "moveFrom" : "deleted", revision.author, inlines, undefined, revision.move);
        } else if (mark) {
          add(at, "deleted", f.paragraph.markRevision!.author, "¶");
        } else {
          const { revision, formatted } = s;
          if (revision) met(revision.author);
          if (revision?.change === "moveTo") add(at, "moveTo", revision.author, "", revision, revision.move);
          if (formatted) {
            const description = describe(formatted.before, formatted.after);
            add(at, "formatted", formatted.by.author, description, `${formatted.by.author}\n${description}`);
          }
          for (const id of s.comments ?? []) {
            if (threaded(comments.get(id))) continue;
            const range = covered.get(id);
            const end = { ...at, x: at.x + piece.width };
            if (range && range.story === at.story) Object.assign(range, end, { from: range.from });
            else covered.set(id, { ...end, kind: "comment", author: comments.get(id)?.author, body: id });
            met(comments.get(id)?.author);
          }
        }
      }
    }
  });
  // A comment whose range covers nothing is shown at the top of the first page rather than dropped.
  for (const [id, comment] of comments) {
    met(comment.author);
    if (threaded(comment)) continue;
    items.push(covered.get(id) ?? { kind: "comment", author: comment.author, page: 0, x: 0, y: 0, story: null, from: -1, to: -1, body: id });
  }

  const colors = new Map(authors.map((a, i) => [a, AUTHOR_COLORS[i % AUTHOR_COLORS.length]]));
  const moves = [...new Set(items.flatMap((i) => (i.move ? [i.move] : [])))];
  const label = (item: Item): string => {
    switch (item.kind) {
      case "deleted":
        return "Deleted:";
      case "formatted":
        return "Formatted:";
      case "comment":
        return "";
      case "moveTo":
        return `Moved (insertion) [${moves.indexOf(item.move!) + 1}]`;
      case "moveFrom": {
        const to = items.findIndex((i) => i.kind === "moveTo" && i.move === item.move);
        if (to < 0) return "Moved:";
        return `Moved ${to > items.indexOf(item) ? "down" : "up"} [${moves.indexOf(item.move!) + 1}]:`;
      }
    }
  };

  const plain = (bold: boolean) => style({ font: BALLOON_FONT, size_half_points: 18, bold });
  const paragraph = (inlines: Inline[]): Paragraph => ({
    kind: "paragraph",
    key: {},
    props: PLAIN,
    mark: plain(false),
    inlines,
    start: null,
    size: 0,
    formatChanged: false,
    fields: false,
  });
  const text = (text: string, s: TextStyle): Inline => ({ kind: "text", text, style: s, source: { at: 0, atom: true } });
  // Quoted content keeps its own font and emphasis, at the balloon's size,
  // with what a later change removed from inside it, as a deletion inside a move.
  const quoted = (inlines: Inline[]): Inline[] =>
    inlines.flatMap((i) => {
      if (i.kind === "anchor") return quoted(i.removed.inlines);
      return i.kind === "text" && i.text ? [text(i.text, style({ ...i.style.run, size_half_points: 18 }))] : [];
    });
  const width = MARKUP_WIDTH - BALLOON_INSET - ARROW_WIDTH;

  const balloon = (item: Item): Balloon => {
    const paragraphs = [paragraph([text(item.author ?? "Unknown", plain(true))])];
    if (item.kind === "comment") {
      const id = item.body as string;
      const replies = [...comments.values()].filter((c) => c.parent === id);
      [comments.get(id), ...replies].forEach((comment, i) => {
        if (i > 0) paragraphs.push(paragraph([text(comment?.author ?? "Unknown", plain(true))]));
        for (const block of comment?.blocks ?? []) {
          if (block.kind === "paragraph") paragraphs.push({ ...block, props: { ...block.props, spacing: PLAIN.spacing } });
        }
      });
    } else {
      const quote = typeof item.body !== "string" ? quoted(item.body) : item.body ? [text(item.body, plain(false))] : [];
      paragraphs.push(paragraph([text(label(item), plain(true)), ...(quote.length ? [text(" ", plain(false)), ...quote] : [])]));
    }
    const lines: Balloon["lines"] = [];
    let y = PADDING;
    for (const p of paragraphs) {
      const breaker = new LineBreaker(p, measurer, 36);
      for (let l = breaker.next(width); l; l = breaker.next(width)) {
        lines.push({ line: l, paragraph: p, y });
        y += l.height;
      }
    }
    const first = lines[1] ?? lines[0];
    const moved = item.kind === "moveFrom" || item.kind === "moveTo";
    const partner = moved ? items.find((i) => i.kind !== item.kind && i.move === item.move && i.story === item.story) : undefined;
    return {
      color: moved ? MOVE_COLOR : colors.get(item.author ?? "")!,
      anchor: { x: item.x, y: item.y },
      top: 0,
      height: y + PADDING + (item.kind === "comment" ? REPLY_HEIGHT : 0),
      point: first.y + first.line.height / 2,
      lines,
      story: item.story,
      from: item.from,
      to: item.to,
      partner: partner && partner.from >= 0 ? partner.from : undefined,
      comment: item.kind === "comment" ? (item.body as string) : undefined,
    };
  };

  const out: Balloon[][] = pages.map(() => []);
  for (const item of items) out[item.page].push(balloon(item));
  out.forEach((balloons, index) => {
    balloons.sort((a, b) => a.anchor.y - b.anchor.y || a.anchor.x - b.anchor.x);
    const tops = place(
      balloons.map((b) => ({ top: b.anchor.y, height: b.height })),
      pages[index].height,
    );
    balloons.forEach((b, i) => (b.top = tops[i]));
  });
  const shaded = new Map([...comments].map(([id, c]) => [id, colors.get(c.author ?? "")!]));
  return { width: items.length ? MARKUP_WIDTH : 0, pages: out, colors, comments: shaded };
}
