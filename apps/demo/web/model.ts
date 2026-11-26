// The editor's view model: one ProseMirror document per story, built from
// the engine's inspection. Nodes carry the engine's view of themselves as
// data, so the layout reads the same resolved properties the engine gives,
// and nothing here decides how anything is drawn.

import { DOMSerializer, type Mark, type Node as PMNode, Schema } from "prosemirror-model";
import type { InspectNode, NodeId } from "./types.ts";

/** The engine's view of a node, without its children. */
export type Info<T extends InspectNode["type"] = InspectNode["type"]> = Omit<
  Extract<InspectNode, { type: T }>,
  "children" | "paragraph_text"
>;

const info = (node: InspectNode) => {
  const { children: _children, paragraph_text: _text, ...rest } = node as InspectNode & { paragraph_text?: string };
  return rest;
};

/** Revisions drawn in the margin, outside the editable body text. */
export const removedRevision = (node: InspectNode): boolean =>
  node.type === "revision" && (node.change === "del" || node.change === "moveFrom");

/**
 * Text an atom shows, for the hidden input view and assistive technology.
 * With `body`, only what the page's body shows of it: text a tracked change
 * deleted or moved away is drawn in a balloon beside the page.
 */
export const atomText = (node: InspectNode, body = false): string => {
  if (body && node.type === "run" && node.effective?.hidden) return "";
  if (body && removedRevision(node)) return "";
  if (node.type === "text" || node.type === "deleted_text") return node.text;
  if (node.type === "opaque") return node.text_preview ?? "";
  if (node.type === "break") return "\n";
  if (node.type === "tab") return "\t";
  if (node.type === "drawing") return node.description ?? node.name ?? "";
  if (node.type === "note_reference" || node.type === "note_mark") return node.number ?? "";
  const children = node.children ?? [];
  const separator = children.some((child) => child.type === "paragraph" || child.type === "row") ? "\n"
    : node.type === "row" ? "\t" : "";
  return children.map((child) => atomText(child, body)).join(separator);
};

const atomDOM = (tag: "div" | "span", body = false) => (node: PMNode) => {
  const dom = document.createElement(tag);
  dom.dataset.node = node.attrs.id;
  dom.contentEditable = "false";
  dom.textContent = atomText(node.attrs.node, body);
  return dom;
};

export const schema = new Schema({
  nodes: {
    doc: { content: "block*" },
    paragraph: {
      group: "block",
      content: "inline*",
      attrs: {
        id: {},
        tag: { default: "p" },
        info: {},
        // The list numbering the marker is counted from, and the marker.
        list: { default: null },
        marker: { default: null },
      },
      toDOM: (node) => [node.attrs.tag, { "data-node": node.attrs.id }, 0],
    },
    table: {
      group: "block",
      content: "(row | block_atom)*",
      attrs: { id: {}, info: {} },
      toDOM: (node) => ["table", { "data-node": node.attrs.id }, ["tbody", 0]],
    },
    row: {
      content: "(cell | block_atom)*",
      attrs: { id: {}, info: {} },
      toDOM: (node) => ["tr", { "data-node": node.attrs.id }, 0],
    },
    cell: {
      content: "block*",
      isolating: true,
      attrs: { id: {}, info: {}, column: {}, colspan: { default: 1 }, rowspan: { default: 1 } },
      toDOM: (node) => ["td", { "data-node": node.attrs.id }, 0],
    },
    block_atom: { group: "block", atom: true, selectable: false, attrs: { id: {}, node: {} }, toDOM: atomDOM("div") },
    inline_atom: {
      group: "inline",
      inline: true,
      atom: true,
      selectable: false,
      attrs: { id: {}, node: {} },
      toDOM: atomDOM("span"),
    },
    text: { group: "inline" },
  },
  // The parse rules let ProseMirror read back DOM the browser re-created while
  // typing. Without them such text would lose its leaf and could be credited
  // to a neighbouring text node.
  marks: {
    link: {
      attrs: { id: {}, target: { default: null } },
      toDOM: (mark) => ["a", { "data-node": mark.attrs.id, title: mark.attrs.target ?? "" }, 0],
      parseDOM: [{ tag: "a[data-node]", getAttrs: (dom) => ({ id: dom.dataset.node, target: dom.title || null }) }],
    },
    run: {
      attrs: { info: {} },
      toDOM: (mark) => ["span", { "data-run": JSON.stringify(mark.attrs.info) }, 0],
      parseDOM: [{ tag: "span[data-run]", getAttrs: (dom) => ({ info: JSON.parse(dom.dataset.run!) }) }],
    },
    leaf: {
      attrs: { id: {} },
      toDOM: (mark) => ["span", { "data-node": mark.attrs.id, class: "t" }, 0],
      parseDOM: [{ tag: "span.t[data-node]", getAttrs: (dom) => ({ id: dom.dataset.node }) }],
    },
    // A tracked insertion an edit made, whose text stays editable.
    revision: {
      attrs: { id: {}, author: { default: null }, date: { default: null } },
      toDOM: (mark) => ["ins", { "data-node": mark.attrs.id, title: mark.attrs.author ?? "", "data-date": mark.attrs.date ?? "" }, 0],
      parseDOM: [
        {
          tag: "ins[data-node]",
          getAttrs: (dom) => ({ id: dom.dataset.node, author: dom.title || null, date: dom.dataset.date || null }),
        },
      ],
    },
  },
});

/** Serialize the same table and paragraph structure, with only the body text of preserved atoms. */
export const clipboardSerializer = new DOMSerializer(
  { ...DOMSerializer.nodesFromSchema(schema), block_atom: atomDOM("div", true), inline_atom: atomDOM("span", true) },
  DOMSerializer.marksFromSchema(schema),
);

/** The tracked insertion an edit made that holds a view node: who made it, and when. */
export const insertionOf = (node: PMNode | null | undefined): { author: string | null; date: string | null } | undefined =>
  node ? schema.marks.revision.isInSet(node.marks)?.attrs as { author: string | null; date: string | null } | undefined : undefined;

export const leafOf = (node: PMNode | null | undefined): NodeId | undefined =>
  node ? schema.marks.leaf.isInSet(node.marks)?.attrs.id : undefined;

/** Where a hyperlink points as the engine names it: its target, and `#` before a place in the document. */
export const hrefOf = (link: { target?: string; anchor?: string }): string | null =>
  link.anchor !== undefined ? `${link.target ?? ""}#${link.anchor}` : (link.target ?? null);

/** Whether a preserved block exposes visible content that Backspace can target. */
export const drawn = (node: InspectNode): boolean => node.type === "content_control" && (node.children ?? []).some((child) => child.type === "paragraph" || child.type === "table" || drawn(child));

/** Kinds that hold nothing a reader sees: they get no position in the view. */
export const invisibleInline = (node: InspectNode): boolean => node.type === "marker" || node.type === "field_code";

/**
 * Builds blocks as ProseMirror nodes. Every block of a story or table cell
 * becomes one node, an atom if it is not a paragraph or table, so a
 * container's children are the engine's and a splice of them applies by index.
 */
export function blocks() {
  const { nodes, marks } = schema;

  const inline = (node: InspectNode, outer: readonly Mark[], out: PMNode[]) => {
    const kids = (m: readonly Mark[]) => node.children?.forEach((c) => inline(c, m, out));
    switch (node.type) {
      case "run":
        return kids(marks.run.create({ info: info(node) }).addToSet(outer));
      case "hyperlink":
        return kids(marks.link.create({ id: node.id, target: hrefOf(node) }).addToSet(outer));
      // An insertion an edit made holds text to edit; a deletion, or one the
      // document came with, is kept whole.
      case "revision":
        if (node.support !== "editable" || node.change !== "ins") break;
        return kids(marks.revision.create({ id: node.id, author: node.author ?? null, date: node.date ?? null }).addToSet(outer));
      case "text":
        if (node.support !== "editable") break;
        if (node.text) out.push(schema.text(node.text, marks.leaf.create({ id: node.id }).addToSet(outer)));
        return;
    }
    if (!invisibleInline(node)) out.push(nodes.inline_atom.create({ id: node.id, node }, null, outer));
  };

  const block = (node: InspectNode): PMNode => {
    switch (node.type) {
      case "paragraph": {
        const content: PMNode[] = [];
        node.children?.forEach((c) => inline(c, [], content));
        const level = node.heading_level ? Math.min(node.heading_level, 6) : 0;
        const attrs = {
          id: node.id,
          tag: level ? `h${level}` : "p",
          info: info(node),
          list: node.list ?? null,
          marker: node.list?.marker ?? null,
        };
        return nodes.paragraph.create(attrs, content);
      }
      case "table": {
        const rows = (node.children ?? []).map((row) => {
          if (row.type !== "row") return block(row);
          const cells = (row.children ?? []).map((cell) => {
            if (cell.type !== "cell") return block(cell);
            const { column, rows: rowspan } = node.cells[cell.id];
            if (rowspan === 0) return null;
            const content = (cell.children ?? []).map(block);
            return nodes.cell.create({ id: cell.id, info: info(cell), colspan: cell.grid_span, rowspan, column }, content);
          });
          return nodes.row.create({ id: row.id, info: info(row) }, cells.filter((c) => c !== null));
        });
        return nodes.table.create({ id: node.id, info: info(node) }, rows);
      }
    }
    return nodes.block_atom.create({ id: node.id, node });
  };
  return block;
}

/** One story as a ProseMirror document. */
export function buildDoc(story: InspectNode): PMNode {
  return schema.nodes.doc.create(null, (story.children ?? []).map(blocks()));
}
