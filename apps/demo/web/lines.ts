// A paragraph's lines. Text breaks greedily at spaces, after hyphens and
// around CJK characters; tabs go to the paragraph's stops; a list item's
// marker comes before its text. Each piece of a line knows the positions it
// covers, so an editor can find a caret on the line and a click on it.
//
// Lines come one at a time, each given the room it may use, because a float
// can narrow some lines of a paragraph and not others.

import { type Inline, type Paragraph, type TextStyle, TWIP } from "./blocks.ts";
import type { Measurer } from "./measure.ts";
import type { NodeId, PageField, TabStop } from "./types.ts";

/**
 * The positions a piece covers, relative to its paragraph's content. Editable
 * text shown with as many code units as it stores has a position between each
 * of them; anything else covers its positions as a whole.
 */
export interface Span {
  from: number;
  to: number;
  text: boolean;
}

/**
 * A drawn piece of a line: a word, its spaces, a tab, an image, a break, or,
 * without an inline, a list marker or the paragraph mark that ends the last line.
 */
export interface Piece {
  inline: Inline | null;
  style: TextStyle;
  text: string;
  x: number;
  width: number;
  span: Span | null;
  /** What a tab's leader draws across it: its character, as many times as fit. */
  leader?: string;
}

export interface Line {
  pieces: Piece[];
  /** From the line's top to its baseline, including the line spacing added above the text. */
  ascent: number;
  descent: number;
  height: number;
  /** The positions the line holds, relative to the paragraph's content. */
  from: number;
  to: number;
  end: "wrap" | "line" | "page" | "column" | "paragraph";
  /** Notes referenced on the line, as `kind:id`. */
  notes: string[];
}

/** The text a page field shows where it is being laid out. */
export type Fields = (field: PageField) => string;

interface Part {
  inline: Inline;
  style: TextStyle;
  text: string;
  width: number;
  span: Span | null;
}

/** What breaks no further: a word with the spaces after it, or one tab, break, image or float. */
interface Token {
  kind: Inline["kind"];
  parts: Part[];
  width: number;
  space: Part[];
  /** Nothing joins it: it ends at a hyphen or is a CJK character. */
  sealed: boolean;
}

const OPEN = { left: -Infinity, right: Infinity };
/** How far a line may overrun its room by rounding alone. */
const SLACK = 0.01;
const CJK = /[⺀-鿿가-힯豈-﫿＀-￯]/;
const LEADERS: Record<string, string> = { dot: ".", hyphen: "-", underscore: "_", heavy: "_", middle_dot: "·" };
const graphemes = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/** Each inline's shown text: caps applied, page fields filled in. */
function shown(inlines: Inline[], fields?: Fields): string[] {
  let drawn: NodeId | undefined;
  return inlines.map((inline) => {
    if (inline.kind !== "text") return "";
    const { run, pageField } = inline.style;
    let text = inline.text;
    if (pageField && fields) {
      // A field's result can span several text inlines: the value replaces the first.
      text = pageField.field === drawn ? "" : fields(pageField);
      drawn = pageField.field;
    }
    return run.caps ? text.toUpperCase() : text;
  });
}

const styleOf = (inline: Inline, fallback: TextStyle) => ("style" in inline ? inline.style : fallback);

/** A stretch of shown text and the stored text it draws; `exact` when they agree unit for unit. */
interface Stretch {
  shown: number;
  length: number;
  from: number;
  to: number;
  exact: boolean;
}

/**
 * Where shown text is in the text it was stored as. Capitals can draw one
 * stored character as several, as ß is drawn SS: that character is a stretch
 * of its own, with a caret position on each side and none inside. Null when
 * the shown text is not the stored text or its capitals.
 */
function capitals(stored: string, shown: string): Stretch[] | null {
  if (stored === shown) return [{ shown: 0, length: shown.length, from: 0, to: stored.length, exact: true }];
  const out: Stretch[] = [];
  let [from, at] = [0, 0];
  for (const character of stored) {
    const drawn = character.toUpperCase();
    if (!shown.startsWith(drawn, at)) return null;
    const exact = drawn.length === character.length;
    const last = out.at(-1);
    if (exact && last?.exact) {
      last.length += drawn.length;
      last.to += character.length;
    } else out.push({ shown: at, length: drawn.length, from, to: from + character.length, exact });
    from += character.length;
    at += drawn.length;
  }
  return at === shown.length ? out : null;
}

/** Pieces placed left to right, with the tab whose alignment waits on what follows it. */
class Cursor {
  readonly pieces: Piece[] = [];
  x: number;
  /** The width of the spaces ending the line, which hang past its end. */
  trailing = 0;
  content = false;
  private pending: { tab: Piece; stop: number; align: TabStop["align"]; from: number; leader?: string } | null = null;
  private readonly measurer: Measurer;

  constructor(x: number, measurer: Measurer) {
    this.x = x;
    this.measurer = measurer;
  }

  place(piece: Omit<Piece, "x">, space = false) {
    this.pieces.push({ ...piece, x: this.x });
    this.x += piece.width;
    this.trailing = space ? this.trailing + piece.width : 0;
    if (piece.inline) this.content = true;
  }

  /** A tab to the stop `find` gives for where the text before it ends, once an earlier tab has aligned that text. */
  tab(find: (x: number) => TabStop, piece: Omit<Piece, "x">) {
    this.settle();
    const stop = find(this.x);
    const at = stop.pos_twips * TWIP;
    const leader = stop.leader && LEADERS[stop.leader];
    this.place(piece);
    const tab = this.pieces.at(-1)!;
    if (stop.align === "left") {
      this.fill(tab, at - tab.x, leader);
      this.x = tab.x + tab.width;
    } else {
      this.pending = { tab, stop: at, align: stop.align, from: this.pieces.length, leader };
    }
  }

  private fill(tab: Piece, width: number, leader?: string) {
    tab.width = Math.max(0, width);
    if (leader) tab.leader = leader.repeat(Math.floor(tab.width / (this.measurer.width(leader, tab.style.font) || 1)));
  }

  /** How far the pieces after an open tab shift so they align at its stop. */
  private shift(width: number) {
    const pending = this.pending!;
    let anchor = width;
    if (pending.align === "center") anchor = width / 2;
    if (pending.align === "decimal") {
      let before = 0;
      for (const piece of this.pieces.slice(pending.from)) {
        const dot = piece.text.indexOf(".");
        if (dot >= 0) {
          anchor = before + this.measurer.width(piece.text.slice(0, dot), piece.style.font);
          break;
        }
        before += piece.width;
      }
    }
    return Math.max(0, pending.stop - anchor - pending.tab.x);
  }

  /** Where the line would end with `width` more placed, once an open tab is aligned. */
  end(width = 0) {
    if (!this.pending) return this.x + width;
    return this.x + width + this.shift(this.x + width - this.pending.tab.x);
  }

  settle() {
    if (!this.pending) return;
    const shift = this.shift(this.x - this.pending.tab.x);
    this.fill(this.pending.tab, shift, this.pending.leader);
    for (const piece of this.pieces.slice(this.pending.from)) piece.x += shift;
    this.x += shift;
    this.pending = null;
  }
}

export class LineBreaker {
  private readonly paragraph: Paragraph;
  private readonly measurer: Measurer;
  private readonly defaultTab: number;
  private readonly tokens: Token[] = [];
  private index = 0;
  private count = 0;
  private last = 0;
  private done = false;

  /** `defaultTab` is the document's default tab stop interval in points. */
  constructor(paragraph: Paragraph, measurer: Measurer, defaultTab: number, fields?: Fields) {
    this.paragraph = paragraph;
    this.measurer = measurer;
    this.defaultTab = defaultTab;
    this.tokenize(shown(paragraph.inlines, fields));
  }

  private measure(text: string, style: TextStyle) {
    return text ? this.measurer.width(text, style.font) : 0;
  }

  private tokenize(texts: string[]) {
    let token: Token | null = null;
    const open = (kind: Inline["kind"]): Token => {
      token = { kind, parts: [], width: 0, space: [], sealed: false };
      this.tokens.push(token);
      return token;
    };
    const add = (part: Part, space: boolean): Token => {
      const joins = token?.kind === "text" && (space || (!token.sealed && !token.space.length));
      const target = joins ? token! : open("text");
      if (space) target.space.push(part);
      else {
        target.parts.push(part);
        target.width += part.width;
      }
      return target;
    };
    const seal = () => {
      if (token) token.sealed = true;
    };

    this.paragraph.inlines.forEach((inline, i) => {
      const source = inline.source;
      const style = styleOf(inline, this.paragraph.mark);
      const whole = { from: source.at, to: source.at + (source.atom || inline.kind !== "text" ? 1 : inline.text.length), text: false };
      // Removed content leaves an anchor for its balloon, which joins the
      // word around it, as the hidden text would, so no line breaks there.
      if (inline.kind === "anchor") {
        add({ inline, style, text: "", width: 0, span: whole }, false);
        return;
      }
      if (inline.kind !== "text") {
        const width = inline.kind === "box" ? inline.width : 0;
        const own = open(inline.kind);
        own.parts.push({ inline, style, text: "", width, span: whole });
        own.width = width;
        own.sealed = true;
        return;
      }
      const text = texts[i];
      // A note reference with its own mark shows no number, but the note still goes on the page.
      if (!text && inline.note) {
        add({ inline, style, text: "", width: 0, span: whole }, false);
        return;
      }
      // Capitals keep a caret position between letters; text shown with other
      // code units than it stores, such as a field's value, is one whole.
      const stored = source.atom ? null : capitals(inline.text, text);
      const part = (from: number, to: number, span: Span): Part => {
        const chunk = text.slice(from, to);
        return { inline, style, text: chunk, width: this.measure(chunk, style), span };
      };
      let stretchIndex = 0;
      const place = (start: number, end: number, space: boolean) => {
        if (!stored) return void add(part(start, end, whole), space);
        // Chunks arrive in display order, so resume at the last source stretch.
        while (start < end) {
          const stretch = stored[stretchIndex];
          const stretchEnd = stretch.shown + stretch.length;
          const to = Math.min(end, stretchEnd);
          const at = source.at + stretch.from;
          const span = stretch.exact
            ? { from: at + start - stretch.shown, to: at + to - stretch.shown, text: true }
            : { from: at, to: source.at + stretch.to, text: false };
          add(part(start, to, span), space);
          start = to;
          if (to === stretchEnd) stretchIndex++;
        }
      };
      for (const match of text.matchAll(/( +)|([^ ]*?-(?=[^ -]))|([^ ]+)/g)) {
        const [chunk, space, hyphenated] = match;
        const start = match.index!;
        if (space) {
          place(start, start + chunk.length, true);
          continue;
        }
        let from = 0;
        for (let k = 0; k <= chunk.length; k++) {
          if (k < chunk.length && !CJK.test(chunk[k])) continue;
          if (k > from) place(start + from, start + k, false);
          if (k < chunk.length) {
            open("text");
            place(start + k, start + k + 1, false);
            seal();
          }
          from = k + 1;
        }
        if (hyphenated) seal();
      }
    });
  }

  /**
   * The next line, or null after the last. `width` is the text area's, and
   * `free` the part of it a float leaves this line, when one intrudes.
   */
  next(width: number, free = OPEN): Line | null {
    if (this.done) return null;
    const { props, marker, mark } = this.paragraph;
    const first = this.count++ === 0;
    const indent = props.indent.left_twips * TWIP;
    const left = Math.max(free.left, indent + (first ? props.indent.first_line_twips * TWIP : 0));
    const right = Math.min(free.right, width - props.indent.right_twips * TWIP);
    const cursor = new Cursor(left, this.measurer);
    const piece = (part: Part) => ({ inline: part.inline, style: part.style, text: part.text, width: part.width, span: part.span });

    if (first && marker) {
      const { text, style, level } = marker;
      const markerWidth = this.measure(text, style);
      if (level.justification !== "left") cursor.x -= level.justification === "right" ? markerWidth : markerWidth / 2;
      cursor.place({ inline: null, style, text, width: markerWidth, span: null });
      const suffix = { inline: null, style, text: "", width: 0, span: null };
      if (level.suffix === "tab") cursor.tab((x) => this.stop(x, indent), suffix);
      if (level.suffix === "space") cursor.place({ ...suffix, text: " ", width: this.measure(" ", style) }, true);
    }

    let end: Line["end"] = "paragraph";
    while (this.index < this.tokens.length) {
      const token = this.tokens[this.index];
      if (token.kind === "break") {
        cursor.place(piece(token.parts[0]));
        this.index++;
        end = (token.parts[0].inline as Extract<Inline, { kind: "break" }>).type;
        break;
      }
      if (token.kind === "tab") {
        cursor.tab((x) => this.stop(x, first ? indent : null), piece(token.parts[0]));
        this.index++;
        continue;
      }
      if (cursor.end(token.width) > right + SLACK) {
        if (cursor.content) {
          end = "wrap";
          break;
        }
        if (token.kind === "text") this.split(token, right - cursor.end());
      }
      const placed = this.tokens[this.index++];
      for (const part of placed.parts) cursor.place(piece(part));
      for (const part of placed.space) cursor.place(piece(part), true);
    }
    if (end === "paragraph") {
      this.done = true;
      const at = this.paragraph.size;
      cursor.place({ inline: null, style: mark, text: "", width: 0, span: { from: at, to: at, text: false } });
    }
    cursor.settle();
    return this.finish(cursor.pieces, cursor.x - cursor.trailing, right, end);
  }

  /** Splits a word too long for any line so its head fits in `room`, leaving the tail as the next token. */
  private split(token: Token, room: number) {
    const head: Part[] = [];
    const tail: Part[] = [];
    let used = 0;
    const cut = (part: Part, from: number, to: number): Part => {
      const text = part.text.slice(from, to);
      const span = part.span?.text ? { from: part.span.from + from, to: part.span.from + to, text: true } : part.span;
      return { ...part, text, width: this.measure(text, part.style), span };
    };
    for (const part of token.parts) {
      if (tail.length) {
        tail.push(part);
        continue;
      }
      // A capital such as ß draws several letters at one stored character.
      const expanded = part.span?.text === false && part.inline.kind === "text" && !part.inline.source.atom;
      const segments = expanded ? [{ index: 0, segment: part.text }] : graphemes.segment(part.text);
      let at = 0;
      for (const { segment, index } of segments) {
        const width = this.measure(segment, part.style);
        if (used + width > room && (head.length || index > 0)) break;
        used += width;
        at = index + segment.length;
      }
      if (at > 0) head.push(cut(part, 0, at));
      if (at < part.text.length) tail.push(cut(part, at, part.text.length));
    }
    if (!tail.length) return;
    const sum = (parts: Part[]) => parts.reduce((total, p) => total + p.width, 0);
    this.tokens.splice(
      this.index,
      1,
      { kind: "text", parts: head, width: sum(head), space: [], sealed: true },
      { kind: "text", parts: tail, width: sum(tail), space: token.space, sealed: token.sealed },
    );
  }

  /** The first stop after `x`: a custom one, the hanging indent on a first line, or a default one. */
  private stop(x: number, hanging: number | null): TabStop {
    const stops = this.paragraph.props.tabs.filter((t) => t.align !== "bar" && t.pos_twips * TWIP > x + SLACK);
    if (hanging !== null && hanging > x + SLACK) stops.push({ pos_twips: hanging / TWIP, align: "left" });
    if (stops.length) return stops.reduce((a, b) => (b.pos_twips < a.pos_twips ? b : a));
    const at = (Math.floor(x / this.defaultTab + 1e-6) + 1) * this.defaultTab;
    return { pos_twips: at / TWIP, align: "left" };
  }

  private finish(pieces: Piece[], contentEnd: number, right: number, end: Line["end"]): Line {
    const { props, mark } = this.paragraph;
    let ascent = 0;
    let descent = 0;
    const notes: string[] = [];
    const grow = (style: TextStyle) => {
      const metrics = this.measurer.metrics(style.font);
      ascent = Math.max(ascent, metrics.ascent + Math.max(0, style.shift));
      descent = Math.max(descent, metrics.descent - Math.min(0, style.shift));
    };
    for (const piece of pieces) {
      const inline = piece.inline;
      if (inline?.kind === "box") ascent = Math.max(ascent, inline.height);
      else if (piece.text || inline?.kind === "tab") grow(piece.style);
      if (inline?.kind === "text" && inline.note && !notes.includes(inline.note)) notes.push(inline.note);
    }
    // The paragraph mark's font counts on the last line, and sets an empty line's height.
    if (end === "paragraph" || ascent + descent === 0) grow(mark);

    const natural = ascent + descent;
    const { line, line_rule } = props.spacing;
    let height = natural;
    if (line_rule === "auto") height = (natural * line) / 240;
    if (line_rule === "exact") height = line * TWIP;
    if (line_rule === "at_least") height = Math.max(natural, line * TWIP);
    ascent = line_rule === "exact" ? height - descent : ascent + height - natural;

    const free = Math.max(0, right - contentEnd);
    if (props.alignment === "center" || props.alignment === "right") {
      const shift = props.alignment === "center" ? free / 2 : free;
      for (const piece of pieces) piece.x += shift;
    } else if (props.alignment === "justify" && end !== "paragraph") {
      justify(pieces, free);
    }

    const spanned = pieces.filter((p) => p.span);
    const from = spanned[0]?.span!.from ?? this.last;
    const to = spanned.at(-1)?.span!.to ?? from;
    this.last = to;
    return { pieces, ascent, descent, height, from, to, end, notes };
  }
}

/** Stretches the spaces between words after the line's last tab, leaving trailing spaces alone. */
function justify(pieces: Piece[], free: number) {
  const lastTab = pieces.findLastIndex((p) => p.inline?.kind === "tab");
  const lastWord = pieces.findLastIndex((p) => p.text.trim() !== "");
  const spaces = pieces.map((p, i) => (i > lastTab && i < lastWord && p.text && !p.text.trim() ? p.text.length : 0));
  const count = spaces.reduce((a, b) => a + b, 0);
  if (!count || !free) return;
  let shift = 0;
  pieces.forEach((piece, i) => {
    piece.x += shift;
    piece.width += (spaces[i] * free) / count;
    shift += (spaces[i] * free) / count;
  });
}

/** The x of the first `chars` of a piece's text, scaled to its drawn width, which justifying may stretch. */
function prefixX(piece: Piece, chars: number, measurer: Measurer) {
  if (chars <= 0) return piece.x;
  if (chars >= piece.text.length) return piece.x + piece.width;
  const natural = measurer.width(piece.text, piece.style.font) || 1;
  return piece.x + (measurer.width(piece.text.slice(0, chars), piece.style.font) * piece.width) / natural;
}

/** The x of a caret at relative position `offset`, which the line holds. */
export function caretX(line: Line, offset: number, measurer: Measurer): number {
  const spanned = line.pieces.filter((p) => p.span);
  let x = spanned[0]?.x ?? line.pieces[0]?.x ?? 0;
  for (const piece of spanned) {
    const span = piece.span!;
    if (offset < span.from) break;
    if (offset === span.from) return piece.x;
    if (offset < span.to && span.text) return prefixX(piece, offset - span.from, measurer);
    x = piece.x + piece.width;
  }
  return x;
}

/** Where a line's pieces start and end, relative to the line. */
export function extent(line: Line): { left: number; right: number } {
  const last = line.pieces.at(-1);
  return { left: line.pieces[0]?.x ?? 0, right: last ? last.x + last.width : 0 };
}

/**
 * The relative position nearest to `x` on the line. At the end of a wrapped
 * line the position also starts the next line, and `bias` is -1 to keep the
 * caret on this one; elsewhere it is 1.
 */
export function offsetAt(line: Line, x: number, measurer: Measurer): { offset: number; bias: -1 | 1 } {
  const offset = nearest(line, x, measurer);
  return { offset, bias: offset === line.to && line.end === "wrap" ? -1 : 1 };
}

function nearest(line: Line, x: number, measurer: Measurer): number {
  let last = line.from;
  for (const piece of line.pieces) {
    const span = piece.span;
    if (!span) continue;
    if (x < piece.x + piece.width) {
      if (x <= piece.x) return span.from;
      if (!span.text) return x < piece.x + piece.width / 2 ? span.from : span.to;
      let best = span.from;
      let distance = Infinity;
      for (const { index } of [...graphemes.segment(piece.text), { index: piece.text.length }]) {
        const d = Math.abs(prefixX(piece, index, measurer) - x);
        if (d < distance) [best, distance] = [span.from + index, d];
      }
      return best;
    }
    last = span.to;
  }
  return last;
}
