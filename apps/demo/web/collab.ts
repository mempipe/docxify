import { TextSelection } from "prosemirror-state";
// Editing through the engine's Yrs replica (docx-engine-yrs), alone or
// together. Every edit changes the replica's editing state, and the replica
// derives the displayed document. Plain text edits can rebuild one paragraph;
// structural and formatting changes may require a full rebuild.
// Undo takes back this person's own changes.
//
// One person editing alone has a replica and nothing else (`alone`).
//
// People editing together each have a replica, kept in step through a Y.Doc
// (`connect`). A Hocuspocus provider keeps the Y.Doc in step with the server,
// which seeded it from the base DOCX (collab-server.ts). The Y.Doc only
// carries the state:
//
// - what a local edit, undo or redo wrote to the replica goes into the Y.Doc,
//   which sends it;
// - an update from elsewhere goes from the Y.Doc into the replica.
//
// Updates from elsewhere are applied once per frame, together, and undo
// leaves everyone else's changes.
//
// Awareness carries each person's name and colour, and their selection as
// positions the replica keeps beside the same content while others edit.

import { pointAt, type Caret, type Editor, type EditorEvents, type Session } from "./editor.ts";
import type { DocxEngine, DocxShared } from "./engine.ts";
import { HocuspocusProvider, Y } from "./collab-deps.ts";
import type { Command, NodeId, Point } from "./types.ts";

/** The client ID the server seeds with. Browsers never use it. */
export const SEEDER = 1;

const LOCAL = Symbol("replica");

export interface User {
  name: string;
  color: string;
}

export interface Person extends User {
  self: boolean;
}

export interface Presence {
  status: string;
  people: Person[];
}

type Awareness = NonNullable<HocuspocusProvider["awareness"]>;

/** What keeps a replica in step with the others editing the document. */
interface Link {
  ydoc: Y.Doc;
  awareness: Awareness | null;
}

export class ReplicaSession implements Session {
  readonly events: EditorEvents;
  tracking: string | null = null;
  readonly replica: DocxShared;
  touched?: (stories: NodeId[] | "all") => void;
  /** Absent while one person edits alone. */
  private readonly ydoc: Y.Doc | null;
  private readonly awareness: Awareness | null;
  private readonly changed: () => void;
  private editors: Editor[] = [];
  private queued: Uint8Array[] = [];
  private scheduled = false;
  private moved: ReturnType<typeof setTimeout> | undefined;

  constructor(replica: DocxShared, events: EditorEvents, changed: () => void, link?: Link) {
    this.replica = replica;
    if (!link) replica.stopLocalUpdates();
    this.ydoc = link?.ydoc ?? null;
    this.awareness = link?.awareness ?? null;
    this.events = events;
    this.changed = changed;
    this.ydoc?.on("update", (update: Uint8Array, origin: unknown) => {
      if (origin === LOCAL) return;
      this.queued.push(update);
      this.schedule();
    });
    this.awareness?.on("change", () => this.showCarets());
  }

  inspectNode = (node: NodeId) => this.replica.inspectNode(node);
  inspectChildren = (parent: NodeId, start: number, end: number) => this.replica.inspectChildren(parent, start, end);
  settings = () => this.replica.settings();
  text = (node: NodeId) => this.replica.text(node);
  // Preparation is read-only; apply commits bytes with the accepted picture.
  mediaName = (bytes: Uint8Array) => this.replica.mediaName(bytes);
  sticky = (point: Point) => this.replica.sticky(point);
  resolve = (token: string) => this.replica.resolve(token);

  apply(commands: Command[], media?: Uint8Array) {
    this.send(() => {
      this.replica.apply(commands, media);
      return true;
    });
  }

  record(editor: Editor, select?: number) {
    this.refresh(editor, select);
  }

  undo = () => this.step(() => this.replica.undo());
  redo = () => this.step(() => this.replica.redo());

  attach(editor: Editor) {
    this.editors.push(editor);
  }

  /**
   * Applies updates from elsewhere. They wait while an editor is composing:
   * its text is applied against the view it started from when composition
   * ends, and a newer view would read the other edits as deleted by it.
   */
  resume() {
    if (!this.queued.length || this.composing()) return;
    const selections = this.editors.flatMap(editor => {
      const selection = editor.view.state.selection;
      if (!(selection instanceof TextSelection)) return [];
      const anchor = pointAt(selection.$anchor);
      const head = pointAt(selection.$head);
      if (!anchor || !head) return [];
      return [{ editor, anchor: this.replica.sticky(anchor), head: this.replica.sticky(head) }];
    });
    this.replica.applyUpdate(Y.mergeUpdates(this.queued.splice(0)));
    this.refresh();
    for (const { editor, anchor, head } of selections) {
      const position = (token?: string) => {
        const resolved = token && this.replica.resolve(token);
        return resolved ? editor.position(resolved.paragraph, resolved.point) : null;
      };
      const a = position(anchor), h = position(head);
      const { state } = editor.view;
      if (a !== null && h !== null && (state.selection.anchor !== a || state.selection.head !== h)) {
        editor.view.dispatch(state.tr.setSelection(TextSelection.create(state.doc, a, h)));
      }
    }
  }

  /** Shares where this person's selection is, once it has rested briefly. */
  selected(head: Point | null, anchor: Point | null) {
    clearTimeout(this.moved);
    if (!this.awareness) return;
    // Points name this projection's nodes. Capture their stable positions
    // now: an edit may rebuild those nodes before publication is due.
    const sticky = (point: Point | null) => (point && this.replica.sticky(point)) ?? undefined;
    const at = sticky(head);
    const cursor = at ? { head: at, anchor: sticky(anchor) } : null;
    this.moved = setTimeout(() => {
      this.awareness?.setLocalStateField("cursor", cursor);
    }, 50);
  }

  /**
   * Applies updates from elsewhere at the next frame. Applying them walks
   * each story they touch in Yrs, so updates that arrive together, from one
   * person or several, are applied together. A hidden page gets no frames,
   * so it applies them when it is shown; nobody can see it meanwhile.
   */
  private schedule() {
    if (this.scheduled) return;
    this.scheduled = true;
    frame(() => {
      this.scheduled = false;
      this.resume();
    });
  }

  /** Undo or redo. The caret goes to what changed, as in Word, so the change is seen. */
  private step(change: () => boolean): boolean {
    if (!this.send(change)) return false;
    this.refresh(undefined, undefined, true);
    return true;
  }

  /** Makes a change to the replica, and sends what it wrote. */
  private send(change: () => boolean): boolean {
    if (!change()) return false;
    if (this.ydoc) {
      const update = this.replica.takeUpdate();
      if (update) Y.applyUpdate(this.ydoc, update, LOCAL);
    }
    return true;
  }

  private composing() {
    return this.editors.some((e) => e.view.composing);
  }

  /**
   * Redraws what changed in every editor. It places `selecting`'s caret at
   * `select`, or with `changed` each editor's caret at what changed in it.
   */
  private refresh(selecting?: Editor, select?: number, changed = false) {
    const redraw = this.replica.takeRedraw();
    for (const editor of this.editors) {
      const at = changed ? "change" : editor === selecting ? select : undefined;
      if (redraw === "all") {
        editor.redraw(at);
        continue;
      }
      const mine = redraw.filter((s) => s.story === editor.story);
      if (mine.length || typeof at === "number") editor.splice(mine, at);
    }
    this.touched?.(redraw === "all" ? "all" : redraw.map((change) => change.story));
    this.showCarets();
    this.changed();
  }

  private showCarets() {
    if (!this.awareness || this.composing()) return;
    const carets = new Map<Editor, Caret[]>(this.editors.map((e) => [e, []]));
    for (const [client, state] of this.awareness.getStates()) {
      const cursor = state.cursor as { head?: unknown; anchor?: unknown } | null | undefined;
      if (client === this.awareness.clientID || typeof cursor?.head !== "string") continue;
      const head = this.replica.resolve(cursor.head);
      if (!head) continue;
      const anchor = typeof cursor.anchor === "string" ? this.replica.resolve(cursor.anchor) : undefined;
      const { name, color } = user(state);
      for (const editor of this.editors) {
        const at = editor.position(head.paragraph, head.point);
        if (at === null) continue;
        const from = anchor ? editor.position(anchor.paragraph, anchor.point) : null;
        carets.get(editor)!.push({ at, anchor: from ?? undefined, name, color });
        break;
      }
    }
    for (const [editor, list] of carets) editor.carets(list);
  }
}

const frame = (run: () => void) =>
  typeof requestAnimationFrame === "function" ? requestAnimationFrame(() => run()) : setTimeout(run, 0);

const COLORS = ["#d1495b", "#00798c", "#edae49", "#30638e", "#6a4c93", "#2a9d8f", "#e76f51", "#8d6e63"];

/** A name and a colour for someone who has none yet. */
export function guest(): User {
  const n = crypto.getRandomValues(new Uint32Array(1))[0];
  return { name: `Guest ${100 + (n % 900)}`, color: COLORS[n % COLORS.length] };
}

/**
 * Someone's name and colour from their awareness state. Another browser
 * sets them, and the colour goes into a style attribute, so only a hex
 * colour is taken.
 */
const user = (state: Record<string, unknown>): User => {
  const { name, color } = (state.user ?? {}) as Partial<User>;
  const hex = typeof color === "string" && /^#[0-9a-f]{3}([0-9a-f]{3})?$/i.test(color);
  return { name: (typeof name === "string" && name) || "Someone", color: hex ? color : "#888" };
};

export interface Connection {
  /** The collaboration server's WebSocket URL. */
  url: string;
  /** The shared state's name on the server: the base's path there. */
  room: string;
  base: Uint8Array;
  user: User;
  events: EditorEvents;
  /** The document changed, here or elsewhere. */
  changed: () => void;
  presence: (presence: Presence) => void;
}

// Yjs client IDs are 32-bit; one that matched another replica's would
// corrupt the shared state.
const clientId = () => SEEDER + 1 + (crypto.getRandomValues(new Uint32Array(1))[0] % (2 ** 32 - SEEDER - 1));

/** One person's session on a document nobody else is editing. */
export function alone(engine: DocxEngine, base: Uint8Array, events: EditorEvents, changed: () => void): ReplicaSession {
  return new ReplicaSession(engine.seed(clientId(), base), events, changed);
}

/** Joins a shared state, and resolves once the server's state has arrived. */
export async function connect(
  engine: DocxEngine,
  { url, room, base, user: me, events, changed, presence }: Connection,
): Promise<{ session: ReplicaSession; provider: HocuspocusProvider }> {
  const ydoc = new Y.Doc();
  const provider = new HocuspocusProvider({ url, name: room, document: ydoc });
  // A first awareness state has clock 0, which a peer that never saw this
  // client ignores as already known, until the 15-second renewal. Setting a
  // field sends clock 1.
  provider.setAwarenessField("user", me);
  let status = "connecting";
  const report = () => {
    const states = provider.awareness?.getStates() ?? new Map();
    const self = provider.awareness?.clientID;
    const people = [...states].map(([client, state]) => ({ ...user(state), self: client === self }));
    presence({ status, people });
  };
  provider.on("status", (event: { status: string }) => {
    status = event.status;
    report();
  });
  provider.awareness?.on("change", report);
  await new Promise<void>((resolve) => {
    const synced = () => {
      provider.off("synced", synced);
      resolve();
    };
    provider.on("synced", synced);
  });
  const replica = engine.join(clientId(), base, Y.encodeStateAsUpdate(ydoc));
  report();
  return { session: new ReplicaSession(replica, events, changed, { ydoc, awareness: provider.awareness }), provider };
}
