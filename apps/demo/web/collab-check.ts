// The collaboration path end to end, in Node: collab-server.ts on a spare
// port, and replicas (collab.ts) connected through Hocuspocus as the browser
// demo connects them. Exits non-zero on any failure.
//
//   node web/build.mjs && node web/collab-check.ts
//
// Checks that the server seeds a room the same way every time, that two
// replicas editing one text node at once converge to the same valid DOCX,
// that updates wait while an editor composes, that updates arriving together
// are applied together, that undo takes back only its own replica's change,
// that a selection and a name reach the other replica,
// that an uploaded file becomes a room, and that edits survive a server
// restart through the browsers' copies.

import { type ChildProcess, spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual } from "node:util";
import { connect, type Person, SEEDER, ReplicaSession } from "@mempipe/docxify-editor/collab";
import { Awareness } from "y-protocols/awareness";
import { Y } from "@mempipe/docxify-editor/collab-deps";
import type { Editor } from "@mempipe/docxify-editor/editor";
import { loadDocxEngine } from "@mempipe/docxify-editor/engine";
import type { InspectNode, NodeId } from "@mempipe/docxify-editor/types";

const root = fileURLToPath(new URL("..", import.meta.url));
const engine = await loadDocxEngine(readFileSync(fileURLToPath(import.meta.resolve("@mempipe/docxify-editor/docx_engine_wasm_bg.wasm"))));
const PORT = 18766;
const fail = (message: string): never => {
  console.error(`FAIL ${message}`);
  process.exit(1);
};

let server: ChildProcess | undefined;
// The server shares this process's stderr, so one left running after an
// uncaught error would hold a pipe reading this check open.
process.on("exit", () => server?.kill());
async function start() {
  server = spawn(process.execPath, [join(root, "web/collab-server.ts"), "--port", String(PORT)], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  await new Promise<void>((resolve, reject) => {
    server!.stdout!.on("data", (chunk: Buffer) => chunk.toString().includes("Collaboration demo") && resolve());
    server!.on("exit", (code) => reject(new Error(`server exited with ${code}`)));
  });
}
async function stop() {
  const exited = new Promise((resolve) => server!.on("exit", resolve));
  server!.kill();
  await exited;
}

const events = { refused: (reason: string) => fail(`refused: ${reason}`) };
async function enter(room: string, base: Uint8Array, name: string) {
  const presence = { status: "", people: [] as Person[] };
  const { session, provider } = await connect(engine, {
    url: `ws://127.0.0.1:${PORT}/collab`,
    room: `${engine.roomPrefix}${room}`,
    base,
    user: { name, color: "#30638e" },
    events,
    changed() {},
    presence: (now) => Object.assign(presence, now),
  });
  return { session, provider, presence };
}

let a: Awaited<ReturnType<typeof enter>> | undefined;
let b: typeof a;

async function until(what: string, ready: () => boolean) {
  for (const started = Date.now(); !ready(); ) {
    if (Date.now() - started > 10_000) fail(`timed out waiting until ${what}: ${JSON.stringify([a?.presence, b?.presence])}`);
    await new Promise((r) => setTimeout(r, 20));
  }
}

function* walk(node: InspectNode): Generator<InspectNode> {
  yield node;
  for (const child of node.children ?? []) yield* walk(child);
}
const paragraphs = (s: ReplicaSession) =>
  s.replica.inspect().stories.flatMap((story) => [...walk(story)].filter((n) => n.type === "paragraph")).length;
const leaves = (s: ReplicaSession) => [...walk(s.replica.inspect().stories[0])].filter((n): n is Extract<InspectNode, { type: "text" }> => n.type === "text" && n.support === "editable");
const has = (s: ReplicaSession, prefix: string) => leaves(s).some(n => n.text.startsWith(prefix));
const named = (s: ReplicaSession, prefix: string) => leaves(s).find(n => n.text.startsWith(prefix))?.id as NodeId | undefined;
const same = (a: Uint8Array, b: Uint8Array) => a.length === b.length && a.every((byte, i) => byte === b[i]);

const room = "fixtures/generated/complex-mixed.docx";
// A selection names the projection visible now. Formatting can replace
// those nodes before the debounced awareness publication runs.
const directEngine = engine;
const directBase = new Uint8Array(readFileSync(join(root, "fixtures/generated/simple-paragraphs.docx")));
const directReplica = directEngine.seed(51, directBase);
const awarenessDoc = new Y.Doc();
const awareness = new Awareness(awarenessDoc);
const selectionSession = new ReplicaSession(directReplica, events, () => {}, { ydoc: awarenessDoc, awareness });
const selectedLeaf = [...walk(directReplica.inspect().stories[0])].find(n => n.type === "text")!.id as NodeId;
selectionSession.selected({ node: selectedLeaf, offset: 5 }, { node: selectedLeaf, offset: 2 });
directReplica.apply([{ command: "format_range", from: { node: selectedLeaf, offset: 2 }, to: { node: selectedLeaf, offset: 5 }, format: { bold: true } }]);
if (directReplica.sticky({ node: selectedLeaf, offset: 5 })) fail("regression fixture did not replace the selected leaf");
await until("the selection is published", () => Boolean(awareness.getLocalState()?.cursor));
const announced = awareness.getLocalState()?.cursor as { head?: string; anchor?: string } | undefined;
const head = directReplica.resolve(announced?.head ?? fail("formatting before awareness publication erased the selection head"));
const anchor = directReplica.resolve(announced?.anchor ?? fail("formatting before awareness publication erased the selection anchor"));
const absolute = (where: typeof head) => {
  if (!where || !("offset" in where.point)) return -1;
  let count = 0;
  for (const node of walk(directReplica.inspect().stories[0])) {
    if (node.type !== "text") continue;
    if (node.id === where.point.node) return count + where.point.offset;
    count += [...node.text].length;
  }
  return -1;
};
if (absolute(anchor) !== 2 || absolute(head) !== 5) fail("the published selection no longer covers the same three characters");
awareness.destroy();
awarenessDoc.destroy();
directReplica.free();
console.log("a selection survives projection changes before awareness publication");

const base = new Uint8Array(readFileSync(join(root, room)));
const once = engine.seed(SEEDER, base);
// Yrs Any::Map serializes HashMap entries in arbitrary order. That changes
// update bytes, not the IDs, origins or contents of its items. Restart needs
// identical decoded structures, not a canonical ordering of embed properties.
const firstSeed = Y.decodeUpdate(once.snapshot());
for (let i = 0; i < 20; i++) {
  const again = engine.seed(SEEDER, base);
  if (!isDeepStrictEqual(firstSeed, Y.decodeUpdate(again.snapshot()))) fail("two seeds of one base have different shared items");
  again.free();
}
once.free();
console.log("21 seeds have identical item identities, origins and contents");

await start();
a = await enter(room, base, "A");
b = await enter(room, base, "B");
await until("both see two people", () => a.presence.people.length === 2 && b.presence.people.length === 2);
const names = b.presence.people.map((p) => `${p.name}${p.self ? " (you)" : ""}`).sort();
if (names.join() !== "A,B (you)") fail(`B sees ${names.join()}`);

// One text node: A types at its start while B splits it in the middle.
const leaf = [...walk(a.session.replica.inspect().stories[0])].find(
  (n) => n.type === "text" && n.support === "editable" && n.text.length >= 2,
)!.id as NodeId;
const text = a.session.text(leaf)!;
const count = paragraphs(a.session);
a.session.apply([{ command: "replace_text", node: leaf, text: `[A] ${text}` }]);
b.session.apply([{ command: "split_paragraph", at: { node: leaf, offset: Math.floor([...text].length / 2) } }]);
await until("the replicas converge", () => same(a.session.replica.export(), b.session.replica.export()));
const merged = a.session.replica.export();
if (!engine.validate(merged).valid) fail("the merged export is invalid");
if (paragraphs(a.session) !== count + 1) fail(`expected ${count + 1} paragraphs, found ${paragraphs(a.session)}`);
if (!has(a.session, "[A] ")) fail("typed text is missing from the live projection");
if (a.session.replica.resolutions().length) fail(`conflicts: ${JSON.stringify(a.session.replica.resolutions())}`);
console.log(`two replicas: typing and a split in one text node converged to the same ${merged.length} bytes`);

// While an editor composes, updates from elsewhere wait for it.
const composing = {
  story: "n0",
  view: { composing: true, state: { selection: {} } },
  redraw() {},
  splice() {},
  position: () => null,
  carets() {},
} as unknown as Editor;
b.session.attach(composing);
a.session.apply([{ command: "replace_text", node: named(a.session, "[A] ")!, text: `[A again] ${text}` }]);
await until("the transport delivers the edit during composition", () => same(Y.encodeStateVector(a!.provider.document), Y.encodeStateVector(b!.provider.document)));
if (has(b.session, "[A again]")) fail("an update was applied while an editor composed");
(composing.view as { composing: boolean }).composing = false;
b.session.resume();
if (!has(b.session, "[A again]")) fail("a held update was not applied when composition ended");
console.log("updates from elsewhere waited while an editor composed");

// Undo takes back A's own typing, and leaves B's edit elsewhere in place.
// Node IDs are local projection addresses: resolve each replica's live leaf.
const other = leaves(b.session).find(n => !n.text.startsWith("[A") && n.text.length >= 2)!.id as NodeId;
b.session.apply([{ command: "replace_text", node: other, text: `[B] ${b.session.text(other)}` }]);
await until("A sees B's edit", () => has(a!.session, "[B] "));
if (!a.session.undo()) fail("A had nothing to undo");
await until("B sees A's undo", () => !has(b!.session, "[A again]"));
if (!has(b.session, "[B] ")) fail("A's undo took B's edit");
await until("the replicas converge again", () => same(a.session.replica.export(), b.session.replica.export()));
console.log("undo took back A's typing and left B's edit");

// Two updates from elsewhere that arrive in one task reach B's replica as one.
const third = engine.join(0x7fff_fff0, base, b.session.replica.snapshot());
const updates = ["(1)", "(2)"].map((text) => {
  const before = third.stateVector();
  const node = [...walk(third.inspect().stories[0])].find(n => n.type === "text" && n.text.includes("[B] "))!.id as NodeId;
  third.apply([{ command: "replace_text", node, range: { start: 0, end: 0 }, text }]);
  return third.updateSince(before);
});
third.free();
let applications = 0;
const applyUpdate = b.session.replica.applyUpdate;
b.session.replica.applyUpdate = (update) => {
  applications += 1;
  applyUpdate(update);
};
for (const update of updates) Y.applyUpdate(b.provider.document, update);
await until("B applies both", () => has(b!.session, "(2)(1)"));
b.session.replica.applyUpdate = applyUpdate;
if (applications !== 1) fail(`B applied two updates that arrived together in ${applications} steps`);
await until("A has them too", () => has(a!.session, "(2)(1)"));
console.log("updates that arrived together were applied together");

// A's selection reaches B, which places both ends beside the same text.
const selected = named(a.session, "(2)(1)")!;
a.session.selected({ node: selected, offset: 9 }, { node: selected, offset: 6 });
const aId = a.provider.awareness!.clientID;
const cursor = () => b.provider.awareness!.getStates().get(aId)?.cursor as { head?: string; anchor?: string } | undefined;
await until("B has A's selection", () => typeof cursor()?.head === "string" && typeof cursor()?.anchor === "string");
const ends = [cursor()!.anchor!, cursor()!.head!].map((token) => b.session.replica.resolve(token)?.point);
const expected = [6, 9].map((offset) => ({ node: named(b!.session, "(2)(1)")!, offset }));
if (JSON.stringify(ends) !== JSON.stringify(expected)) fail(`B placed A's selection at ${JSON.stringify(ends)}`);
console.log("A's selection reached B beside the same text");

// An uploaded file becomes a room others can open.
const table = new Uint8Array(readFileSync(join(root, "fixtures/generated/table.docx")));
const uploaded = await fetch(`http://127.0.0.1:${PORT}/upload`, { method: "POST", body: table });
const { upload } = (await uploaded.json()) as { upload: string };
const served = new Uint8Array(await (await fetch(`http://127.0.0.1:${PORT}/upload/${upload}.docx`)).arrayBuffer());
if (!same(served, table)) fail("the uploaded file is served back changed");
const c = await enter(`upload/${upload}.docx`, table, "C");
if (c.session.replica.inspect().stories.length === 0) fail("the uploaded room has no stories");
const refused = await fetch(`http://127.0.0.1:${PORT}/upload`, { method: "POST", body: "not a docx" });
if (refused.status !== 400) fail(`a non-DOCX upload got ${refused.status}`);
const escaped = await fetch(`http://127.0.0.1:${PORT}/web/../Cargo.toml`);
if (escaped.status !== 404) fail(`a path out of web/ got ${escaped.status}`);
c.provider.destroy();
console.log(`upload: ${table.length} bytes became room upload/${upload.slice(0, 8)}…, and was served back unchanged`);

// The server keeps state only in memory. After a restart it seeds the room
// again, and the replicas' copies merge into the new seed.
const before = a.session.replica.export();
await stop();
await start();
await until("both replicas reconnect", () => a.presence.status === "connected" && b.presence.status === "connected");
const d = await enter(room, base, "D");
await until("a new replica sees the edits", () => same(d.session.replica.export(), before));
console.log("after a restart, a new replica joined the bundled fixture with every edit made before it");

for (const { provider } of [a, b, d]) provider.destroy();
await stop();
console.log("ok");
process.exit(0);
