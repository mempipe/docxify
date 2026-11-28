// A typed facade over the wasm-bindgen output in ./pkg. The same module runs
// in Node and in the browser; only how the .wasm bytes are obtained differs.

import init, { DocxDocument, DocxReplica, initSync, sharedFormat, validate } from "./pkg/docx_engine_wasm.js";
import type {
  Resolution,
  Command,
  InspectNode,
  Inspection,
  NodeId,
  Point,
  Settings,
  Splice,
  ValidationReport,
} from "./types.ts";

export interface DocxDoc {
  inspect(): Inspection;
  text(node: NodeId): string | undefined;
  free(): void;
}

/**
 * One participant's copy of a shared editing state: the base DOCX with the
 * shared semantic state synchronized by Yjs v1 updates. The core derives
 * the displayed tree; a text-only edit can rebuild its changed paragraph.
 * View node IDs may change, so redraw what `takeRedraw` names.
 */
export interface DocxShared {
  commentIdentities(): Record<string, string>;
  stopLocalUpdates(): void;
  inspect(): Inspection;
  inspectNode(node: NodeId): InspectNode;
  inspectChildren(parent: NodeId, start: number, end: number): InspectNode[];
  settings(): Settings;
  text(node: NodeId): string | undefined;
  /** A package part's bytes, such as an image's. */
  part(name: string): Uint8Array | undefined;
  /** Validates and names an image without changing shared state; returns the part name `insert_picture` takes. */
  mediaName(bytes: Uint8Array): string;
  /** Editor commands as one change: all of them, or none if any is refused. */
  apply(commands: Command[], media?: Uint8Array): void;
  /** Undoes this participant's last change, and nobody else's. False if there is none. */
  undo(): boolean;
  redo(): boolean;
  /** What changed since this was last called: everything, or splices to apply in order. */
  takeRedraw(): "all" | Splice[];
  /** A caret as a token that stays beside the same content while others edit. */
  sticky(point: Point): string | undefined;
  /** Where a caret from `sticky` is now, if it is in the view. */
  resolve(token: string): { paragraph: NodeId; point: Point } | undefined;
  snapshot(): Uint8Array;
  stateVector(): Uint8Array;
  updateSince(stateVector: Uint8Array): Uint8Array;
  /** What this participant's own changes wrote since this was last called, to send; costs only what they wrote. */
  takeUpdate(): Uint8Array | undefined;
  applyUpdate(update: Uint8Array): void;
  resolutions(): Resolution[];
  export(): Uint8Array;
  free(): void;
}

export interface DocxEngine {
  readonly roomPrefix: string;
  open(bytes: Uint8Array): DocxDoc;
  /** Start a shared state from a DOCX. Exactly one participant seeds; the rest join its state. */
  seed(clientId: number, base: Uint8Array): DocxShared;
  join(clientId: number, base: Uint8Array, state: Uint8Array): DocxShared;
  validate(bytes: Uint8Array): ValidationReport;
}

function shared(replica: DocxReplica): DocxShared {
  return {
    inspect: () => JSON.parse(replica.inspect()) as Inspection,
    commentIdentities: () => JSON.parse(replica.commentIdentities()),
    stopLocalUpdates: () => replica.stopLocalUpdates(),
    inspectNode: (node) => JSON.parse(replica.inspectNode(node)) as InspectNode,
    inspectChildren: (parent, start, end) => JSON.parse(replica.inspectChildren(parent, start, end)) as InspectNode[],
    settings: () => JSON.parse(replica.settings()) as Settings,
    text: (node) => replica.text(node) ?? undefined,
    part: (name) => replica.part(name) ?? undefined,
    mediaName: (bytes) => replica.mediaName(bytes),
    apply: (commands, media) => replica.apply(JSON.stringify(commands), media),
    undo: () => replica.undo(),
    redo: () => replica.redo(),
    takeRedraw: () => JSON.parse(replica.takeRedraw()),
    sticky: (point) => replica.sticky(JSON.stringify(point)) ?? undefined,
    resolve: (token) => {
      const where = replica.resolve(token);
      return where ? JSON.parse(where) : undefined;
    },
    snapshot: () => replica.snapshot(),
    stateVector: () => replica.stateVector(),
    updateSince: (stateVector) => replica.updateSince(stateVector),
    takeUpdate: () => replica.takeUpdate() ?? undefined,
    applyUpdate: (update) => replica.applyUpdate(update),
    resolutions: () => JSON.parse(replica.resolutions()),
    export: () => replica.export(),
    free: () => replica.free(),
  };
}

/**
 * Load from bytes (Node) or a URL (browser; defaults to the module's sibling .wasm).
 * Shared states must use the current format; older states are refused.
 */
export async function loadDocxEngine(
  wasm?: BufferSource | URL,
): Promise<DocxEngine> {
  if (wasm instanceof URL || wasm === undefined) {
    await init(wasm ? { module_or_path: wasm } : undefined);
  } else {
    initSync({ module: wasm });
  }
  return {
    roomPrefix: `yrs-v${sharedFormat().split("/").at(-1)}:`,
    open(bytes) {
      const doc = DocxDocument.open(bytes);
      return {
        inspect: () => JSON.parse(doc.inspect()) as Inspection,
        text: (node) => doc.text(node) ?? undefined,
        free: () => doc.free(),
      };
    },
    seed: (clientId, base) => shared(DocxReplica.seed(clientId, base)),
    join: (clientId, base, state) => shared(DocxReplica.join(clientId, base, state)),
    validate: (bytes) => JSON.parse(validate(bytes)) as ValidationReport,
  };
}
