// Painting: each page as a box of absolutely placed pieces, in points, each
// drawn at the place the layout measured for it. Only pages near the viewport
// hold content; the others keep their size, so the scrollbar stays honest.
//
// A line's element is reused while the layout returns the same line, so a
// repaint after typing rebuilds only the lines that changed.

import type { Image, Paragraph, Revision, TextStyle } from "./blocks.ts";
import type { Line, Piece } from "./lines.ts";
import { type Balloon, BALLOON_INSET, MARKUP_WIDTH, MOVE_COLOR, NO_MARKUP, REPLY_HEIGHT } from "./markup.ts";
import { cssFont, type Measurer } from "./measure.ts";
import type { Fragment, Page } from "./paginate.ts";
import type { Border, NodeId } from "./types.ts";

/** A rectangle on a page, in points. Someone else's selection or caret has their colour. */
export interface Rect {
  page: number;
  x: number;
  y: number;
  width: number;
  height: number;
  color?: string;
}

/** A caret. Someone else's shows their name; the local one blinks. */
export interface Caret extends Rect {
  name?: string;
}

interface View {
  frame: HTMLElement;
  /** From the bottom: pictures behind the text, table cells, lines, pictures in front, balloons, then selections and carets. */
  layers: { behind: HTMLElement; boxes: HTMLElement; text: HTMLElement; front: HTMLElement; balloons: HTMLElement; overlay: HTMLElement };
  page: Page | null;
  /** The page's width with the markup area beside it, in points. */
  width: number;
  lines: Map<Line, HTMLElement>;
}

const HIGHLIGHT: Record<string, string> = {
  yellow: "#ffff00",
  green: "#00ff00",
  cyan: "#00ffff",
  magenta: "#ff00ff",
  blue: "#0000ff",
  red: "#ff0000",
  darkBlue: "#000080",
  darkCyan: "#008080",
  darkGreen: "#008000",
  darkMagenta: "#800080",
  darkRed: "#800000",
  darkYellow: "#808000",
  darkGray: "#808080",
  lightGray: "#c0c0c0",
  black: "#000000",
};
const UNDERLINE: Record<string, string> = { double: "double", dotted: "dotted", dash: "dashed", wave: "wavy" };
const BORDER: Record<string, string> = { double: "double", dotted: "dotted", dashed: "dashed", dotDash: "dashed", inset: "inset", outset: "outset" };
const ARROW = `<svg viewBox="0 0 10 9" width="10pt" height="9pt"><path d="M0 4.5H9M5 .8l3.7 3.7L5 8.2" fill="none" stroke="#4870b2" stroke-width="1.3"/></svg>`;

const el = (className: string, css = "") => {
  const node = document.createElement("div");
  node.className = className;
  node.style.cssText = css;
  return node;
};

const hex = (color: string | undefined, fallback: string) =>
  color && /^[0-9a-f]{6}$/i.test(color) ? `#${color}` : fallback;

/** Whether a line shows a tracked change, which a bar in the margin marks. */
function changed(line: Line, paragraph: Paragraph) {
  if (paragraph.formatChanged || (line.end === "paragraph" && paragraph.markRevision)) return true;
  return line.pieces.some((p) => p.inline?.kind === "anchor" || (p.inline && (p.style.revision || p.style.formatted)));
}

export class Painter {
  readonly root: HTMLElement;
  private readonly measurer: Measurer;
  private readonly images: (image: Image) => string | undefined;
  private views: View[] = [];
  private visible = new Set<number>();
  private readonly observer: IntersectionObserver;
  private selection: Rect[] = [];
  private carets: Caret[] = [];
  private markup = NO_MARKUP;
  private balloon: Balloon | null = null;
  private readonly activeRule: HTMLStyleElement;
  private readonly colorRule: HTMLStyleElement;

  /** `images` gives a URL for an image's package part the browser can show, or nothing. */
  constructor(root: HTMLElement, measurer: Measurer, images: (image: Image) => string | undefined) {
    this.root = root;
    this.measurer = measurer;
    this.images = images;
    root.classList.add("pages");
    this.activeRule = document.createElement("style");
    this.colorRule = document.createElement("style");
    root.append(this.activeRule, this.colorRule);
    this.observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const index = this.views.findIndex((v) => v.frame === entry.target);
          if (index < 0) continue;
          if (entry.isIntersecting) {
            this.visible.add(index);
            this.paint(index);
          } else {
            this.visible.delete(index);
            for (const layer of Object.values(this.views[index].layers)) layer.replaceChildren();
            this.views[index].lines.clear();
          }
        }
      },
      { rootMargin: "100% 0px" },
    );
  }

  /**
   * Draws the pages, with the markup area beside each when there are
   * balloons. Colours go through a style rule per author and comment, so a
   * line kept from the last paint follows when the authors' order changes.
   */
  render(pages: Page[], markup = NO_MARKUP) {
    this.markup = markup;
    this.colorRule.textContent = [
      ...[...markup.colors].map(([author, color]) => `.pages [data-author="${CSS.escape(author)}"]{--author:${color}}`),
      ...[...markup.comments].map(
        ([id, color]) => `.pages [data-comments~="${CSS.escape(id)}"]{background:color-mix(in srgb, ${color} 18%, transparent)}`,
      ),
    ].join("\n");
    while (this.views.length > pages.length) {
      const view = this.views.pop()!;
      this.observer.unobserve(view.frame);
      view.frame.remove();
      this.visible.delete(this.views.length);
    }
    pages.forEach((page, index) => {
      let view = this.views[index];
      if (!view) {
        const frame = el("page");
        const layers = {
          behind: el("layer"),
          boxes: el("layer"),
          text: el("layer"),
          front: el("layer"),
          balloons: el("layer"),
          overlay: el("layer overlay"),
        };
        view = { frame, layers, page: null, width: 0, lines: new Map() };
        frame.append(...Object.values(layers));
        this.root.append(frame);
        this.views.push(view);
        this.observer.observe(frame);
      }
      view.page = page;
      view.width = page.width + markup.width;
      view.frame.style.width = `${view.width}pt`;
      view.frame.style.height = `${page.height}pt`;
      if (this.visible.has(index)) this.paint(index);
    });
  }

  /** Dims every story but the one being edited, as Word dims the body while a header is edited. */
  activate(story: NodeId) {
    this.activeRule.textContent = `.pages .line[data-story]:not([data-story="${CSS.escape(story)}"]){opacity:.45}`;
  }

  /** Draws the selections and carets, and the connector from the active balloon to what it describes. */
  decorate(selection: Rect[], carets: Caret[], balloon: Balloon | null = null) {
    this.selection = selection;
    this.carets = carets;
    this.balloon = balloon;
    for (const index of this.visible) this.overlay(index);
  }

  /** The page under a point in the viewport, and the point on it in points. */
  pageAt(clientX: number, clientY: number): { page: number; x: number; y: number } | null {
    let best: { page: number; x: number; y: number } | null = null;
    let distance = Infinity;
    this.views.forEach((view, page) => {
      const rect = view.frame.getBoundingClientRect();
      const d = clientY < rect.top ? rect.top - clientY : clientY > rect.bottom ? clientY - rect.bottom : 0;
      if (d >= distance) return;
      const scale = view.width / rect.width;
      best = { page, x: (clientX - rect.left) * scale, y: (clientY - rect.top) * scale };
      distance = d;
    });
    return best;
  }

  /** Where a rectangle on a page is in the viewport. */
  clientRect(rect: Rect): DOMRect {
    const view = this.views[rect.page];
    const frame = view.frame.getBoundingClientRect();
    const scale = frame.width / view.width;
    return new DOMRect(frame.left + rect.x * scale, frame.top + rect.y * scale, rect.width * scale, rect.height * scale);
  }

  /** Scrolls the least distance that shows a rectangle. */
  reveal(rect: Rect) {
    const target = el("reveal", `position:absolute;left:${rect.x}pt;top:${rect.y}pt;width:1px;height:${rect.height}pt`);
    this.views[rect.page]?.frame.append(target);
    target.scrollIntoView({ block: "nearest", inline: "nearest" });
    target.remove();
  }

  private paint(index: number) {
    const view = this.views[index];
    const page = view.page;
    if (!page) return;
    const lines = new Map<Line, HTMLElement>();
    const shown = new Set<HTMLElement>();
    const behind: HTMLElement[] = [];
    const boxes: HTMLElement[] = [];
    const front: HTMLElement[] = [];
    for (const fragment of page.fragments) {
      if (fragment.kind === "line") {
        let node = view.lines.get(fragment.line);
        if (!node || lines.has(fragment.line)) {
          node = this.line(fragment.line, fragment.paragraph, fragment.story);
          if (changed(fragment.line, fragment.paragraph)) node.append(el("change", `top:0;height:${fragment.line.height}pt`));
        }
        node.style.left = `${fragment.x}pt`;
        node.style.top = `${fragment.y}pt`;
        const bar = node.querySelector<HTMLElement>(":scope > .change");
        if (bar) bar.style.left = `${page.margins.left - 10 - fragment.x}pt`;
        lines.set(fragment.line, node);
        shown.add(node);
      } else if (fragment.kind === "image") {
        (fragment.behind ? behind : front).push(this.image(fragment));
      } else {
        boxes.push(...this.box(fragment));
      }
    }
    // Lines are placed absolutely, so their order does not matter: lines
    // kept from the last paint stay in the DOM, and only the others change.
    const { text } = view.layers;
    for (const node of [...text.children]) if (!shown.has(node as HTMLElement)) node.remove();
    for (const node of shown) if (node.parentNode !== text) text.append(node);
    view.layers.behind.replaceChildren(...behind);
    view.layers.boxes.replaceChildren(...boxes);
    view.layers.front.replaceChildren(...front);
    view.layers.balloons.replaceChildren(...this.balloons(index, page));
    view.lines = lines;
    this.overlay(index);
  }

  private overlay(index: number) {
    const view = this.views[index];
    const nodes: HTMLElement[] = [];
    for (const r of this.selection) {
      if (r.page !== index) continue;
      const tint = r.color ? `;background:color-mix(in srgb, ${r.color} 25%, transparent)` : "";
      nodes.push(el("selection", `left:${r.x}pt;top:${r.y}pt;width:${r.width}pt;height:${r.height}pt${tint}`));
    }
    for (const c of this.carets) {
      if (c.page !== index) continue;
      const caret = el(c.color ? "caret" : "caret local", `left:${c.x}pt;top:${c.y}pt;height:${c.height}pt`);
      if (c.color) {
        caret.style.borderColor = c.color;
        const name = el("name", `background:${c.color}`);
        name.textContent = c.name ?? "";
        caret.append(name);
      }
      nodes.push(caret);
    }
    const balloon = this.balloon;
    const page = view.page;
    if (balloon && page && this.markup.pages[index]?.includes(balloon)) {
      // Word's connector: along the line to the right margin, then across to the balloon.
      const { x, y } = balloon.anchor;
      const path = `M${x} ${y}H${page.width - page.margins.right}L${page.width} ${balloon.top + balloon.point}`;
      const svg = `<svg viewBox="0 0 ${view.width} ${page.height}"><path d="${path}" fill="none" stroke="${balloon.color}" stroke-width=".75" stroke-dasharray="3 2"/></svg>`;
      const connector = el("connector");
      connector.innerHTML = svg;
      nodes.push(connector);
    }
    view.layers.overlay.replaceChildren(...nodes);
  }

  /** The markup area beside a page: its edge, then each balloon with its author's bar. */
  private balloons(index: number, page: Page): HTMLElement[] {
    if (!this.markup.width) return [];
    const out = [el("separator", `left:${page.width}pt`)];
    for (const balloon of this.markup.pages[index] ?? []) {
      const node = el("balloon", `left:${page.width}pt;top:${balloon.top}pt;height:${balloon.height}pt;--bar:${balloon.color}`);
      for (const { line, paragraph, y } of balloon.lines) {
        const drawn = this.line(line, paragraph, null);
        drawn.style.left = `${BALLOON_INSET}pt`;
        drawn.style.top = `${y}pt`;
        node.append(drawn);
      }
      if (balloon.partner !== undefined) {
        const arrow = el("arrow", `top:${balloon.point}pt`);
        arrow.innerHTML = ARROW;
        node.append(arrow);
      }
      if (balloon.comment !== undefined) {
        const reply = Object.assign(document.createElement("button"), { type: "button", className: "reply", textContent: "Reply" });
        reply.dataset.comment = balloon.comment;
        node.append(reply);
      }
      out.push(node);
    }
    return out;
  }

  private line(line: Line, paragraph: Paragraph, story: NodeId | null): HTMLElement {
    const node = el("line", `height:${line.height}pt`);
    if (story) node.dataset.story = story;
    for (const piece of line.pieces) {
      const drawn = this.piece(piece, line, paragraph);
      if (drawn) node.append(drawn);
    }
    return node;
  }

  private piece(piece: Piece, line: Line, paragraph: Paragraph): HTMLElement | null {
    const inline = piece.inline;
    if (inline?.kind === "box") {
      const top = line.ascent - inline.height;
      return this.picture(inline.image, `left:${piece.x}pt;top:${top}pt;width:${inline.width}pt;height:${inline.height}pt`);
    }
    if (inline?.kind === "tab") return piece.leader ? this.text(piece.leader, piece.x, line, piece.style) : null;
    if (!piece.text) return null;
    const span = this.text(piece.text, piece.x, line, piece.style, piece.style.revision);
    if (piece.style.link) span.title = piece.style.link;
    if (piece.style.comments?.length) span.dataset.comments = piece.style.comments.join(" ");
    if (piece.style.control) span.dataset.control = piece.style.control;
    return span;
  }

  private text(text: string, x: number, line: Line, style: TextStyle, revision?: Revision): HTMLElement {
    const { run, font } = style;
    const top = line.ascent - this.measurer.metrics(font).ascent - style.shift;
    const css = [`left:${x}pt`, `top:${top}pt`, `font:${cssFont(font)}`];
    const moved = revision?.change === "moveTo";
    const color = moved ? MOVE_COLOR : revision ? "var(--author)" : hex(run.color, "#000");
    css.push(`color:${color}`);
    if (run.highlight && HIGHLIGHT[run.highlight]) css.push(`background:${HIGHLIGHT[run.highlight]}`);
    const lines: string[] = [];
    let decoration = "solid";
    if (run.underline && run.underline !== "none") {
      lines.push("underline");
      decoration = UNDERLINE[run.underline] ?? "solid";
    }
    if (run.strike) lines.push("line-through");
    if (revision?.change === "ins") lines.push("underline");
    if (moved) {
      lines.push("underline");
      decoration = "double";
    }
    if (lines.length) css.push(`text-decoration:${[...new Set(lines)].join(" ")} ${decoration}`);
    const span = document.createElement("span");
    span.style.cssText = css.join(";");
    span.textContent = text;
    if (revision && !moved) span.dataset.author = revision.author ?? "";
    if (revision) span.title = `${revision.change} by ${revision.author ?? "unknown"}${revision.date ? ` on ${revision.date}` : ""}`;
    return span;
  }

  private picture(image: Image, css: string): HTMLElement {
    const url = image.part ? this.images(image) : undefined;
    if (!url) {
      const box = el("picture missing", css);
      box.textContent = image.description ?? "";
      return box;
    }
    const img = document.createElement("img");
    img.src = url;
    img.alt = image.description ?? "";
    img.style.cssText = css;
    return img;
  }

  private image(f: Extract<Fragment, { kind: "image" }>): HTMLElement {
    return this.picture(f.image, `left:${f.x}pt;top:${f.y}pt;width:${f.width}pt;height:${f.height}pt`);
  }

  private box(f: Extract<Fragment, { kind: "cell" | "rule" }>): HTMLElement[] {
    if (f.kind === "rule") return [el("edge", `left:${f.x}pt;top:${f.y}pt;width:${f.width}pt;border-top:0.5pt solid #000`)];
    const out = [el("cell", `left:${f.x}pt;top:${f.y}pt;width:${f.width}pt;height:${f.height}pt;${f.shading ? `background:${hex(f.shading, "transparent")}` : ""}`)];
    // Word centres a border on the cell's edge.
    const edge = (side: "top" | "bottom" | "left" | "right", border: Border | undefined) => {
      if (!border || border.style === "nil" || border.style === "none") return;
      const style = BORDER[border.style] ?? "solid";
      const width = Math.max(0.25, border.size_eighths / 8) * (style === "double" ? 3 : 1);
      const color = hex(border.color, "#000");
      const half = width / 2;
      const horizontal = side === "top" || side === "bottom";
      const at = side === "top" ? f.y : side === "bottom" ? f.y + f.height : side === "left" ? f.x : f.x + f.width;
      const css = horizontal
        ? `left:${f.x - half}pt;top:${at - half}pt;width:${f.width + width}pt;border-top:${width}pt ${style} ${color}`
        : `left:${at - half}pt;top:${f.y - half}pt;height:${f.height + width}pt;border-left:${width}pt ${style} ${color}`;
      out.push(el("edge", css));
    };
    edge("top", f.borders.top);
    edge("bottom", f.borders.bottom);
    edge("left", f.borders.left);
    edge("right", f.borders.right);
    return out;
  }
}

export const PAINT_CSS = `
.pages{display:flex;flex-direction:column;align-items:center;gap:16pt;padding:16pt 0;background:#e6e6e6}
.pages .page{position:relative;flex:none;background:#fff;box-shadow:0 1px 3px rgba(0,0,0,.3);overflow:hidden;cursor:text;content-visibility:auto}
.pages .layer{position:absolute;inset:0}
.pages .overlay{pointer-events:none}
.pages .line{position:absolute;width:0}
.pages .line>span{position:absolute;white-space:pre;line-height:normal}
.pages .line>span[data-control]{cursor:pointer}
.pages .line>span[data-control]:hover{outline:1px solid #8ab4f8}
.pages .separator{position:absolute;top:0;bottom:0;width:0;border-left:.75pt solid #c8c8c8}
.pages .balloon{position:absolute;width:${MARKUP_WIDTH}pt;box-shadow:inset 2pt 0 var(--bar);cursor:default}
.pages .arrow{position:absolute;right:7.5pt;transform:translateY(-50%);line-height:0;cursor:pointer}
.pages .reply{position:absolute;right:7.5pt;bottom:2pt;height:${REPLY_HEIGHT - 2}pt;padding:0 2pt;border:0;background:none;color:var(--bar);font:600 7.5pt/1 system-ui,sans-serif;cursor:pointer}
.pages .connector>svg{position:absolute;left:0;top:0;width:100%;height:100%;overflow:visible}
.pages img,.pages .picture,.pages .cell,.pages .edge{position:absolute}
.pages .edge{width:0;height:0}
.pages .picture.missing{box-sizing:border-box;border:1px dashed #999;background:#f4f4f4;color:#666;font:9pt sans-serif;overflow:hidden;padding:2pt}
.pages .change{position:absolute;width:0;border-left:1pt solid #444}
.pages .selection{position:absolute;background:rgba(51,132,255,.28)}
.pages .caret{position:absolute;width:0;border-left:1.5px solid #000}
.pages .caret.local{animation:caret-blink 1.06s steps(1) infinite}
.pages .caret>.name{position:absolute;left:-1px;bottom:100%;color:#fff;font:600 9px/1.4 system-ui,sans-serif;padding:0 4px;border-radius:3px 3px 3px 0;white-space:nowrap}
@keyframes caret-blink{50%{opacity:0}}
`;
