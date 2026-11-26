// The collaboration demo's server: Hocuspocus relays each document's shared
// state between browsers, and the same port serves the demo page, the
// fixtures and uploaded files.
//
//   node web/build.mjs
//   node web/collab-server.ts [--host 127.0.0.1] [--port 8766]
//
// Then open http://<host>:<port>/ in two browsers. A room is named by its
// base's path on this server: fixtures/<path>.docx, or upload/<sha256>.docx
// for a file a browser uploaded. The server seeds a room's shared state from
// its base the first time it is opened, so no two browsers race to seed it.
//
// Everything is in memory: uploads and shared states are lost when the
// server stops. Anyone who can reach the port can read and edit every room.

import { Server } from "@hocuspocus/server";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import type { IncomingMessage, ServerResponse } from "node:http";
import { extname, join, normalize, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import * as Y from "yjs";
import { SEEDER } from "@mempipe/docxify-editor/collab";
import { loadDocxEngine } from "@mempipe/docxify-editor/engine";

const { values } = parseArgs({
  options: { host: { type: "string", default: "127.0.0.1" }, port: { type: "string", default: "8766" } },
});
const root = fileURLToPath(new URL("..", import.meta.url));
const wasm = await readFile(fileURLToPath(import.meta.resolve("@mempipe/docxify-editor/docx_engine_wasm_bg.wasm")));
const engine = await loadDocxEngine(wasm);

const MAX_UPLOAD = 32 * 1024 * 1024;
const uploads = new Map<string, Uint8Array>();
const states = new Map<string, Uint8Array>();
const TYPES: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".css": "text/css; charset=utf-8",
  ".wasm": "application/wasm",
  ".docx": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
};

/** A room's base, or undefined if no file has that name. */
async function base(name: string): Promise<Uint8Array | undefined> {
  const upload = /^upload\/([0-9a-f]{64})\.docx$/.exec(name);
  if (upload) return uploads.get(upload[1]);
  return name.endsWith(".docx") ? file(name, ["fixtures"]) : undefined;
}

/** A file under one of `dirs` of the engine's directory, refusing any path out of them. */
async function file(name: string, dirs: string[]): Promise<Uint8Array | undefined> {
  const path = normalize(join(root, name));
  if (!dirs.some((dir) => path.startsWith(join(root, dir) + sep))) return undefined;
  try {
    return await readFile(path);
  } catch {
    return undefined;
  }
}

async function body(request: IncomingMessage): Promise<Uint8Array | undefined> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > MAX_UPLOAD) return undefined;
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

function send(response: ServerResponse, status: number, type: string, content: string | Uint8Array) {
  response.writeHead(status, { "Content-Type": type, "Cache-Control": "no-store" });
  response.end(content);
}

async function handle(request: IncomingMessage, response: ServerResponse) {
  const path = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname).slice(1);
  if (request.method === "POST" && path === "upload") {
    const bytes = await body(request);
    if (!bytes) return send(response, 413, "text/plain", `Uploads are limited to ${MAX_UPLOAD / 1024 / 1024} MiB.`);
    try {
      engine.open(bytes).free();
    } catch (e) {
      return send(response, 400, "text/plain", `Not a DOCX the engine opens: ${(e as Error).message}`);
    }
    const sha = createHash("sha256").update(bytes).digest("hex");
    uploads.set(sha, bytes);
    return send(response, 200, "application/json", JSON.stringify({ upload: sha }));
  }
  if (request.method !== "GET" && request.method !== "HEAD") return send(response, 405, "text/plain", "Not allowed.");
  if (path === "") {
    response.writeHead(302, { Location: "/web/index.html?collab" });
    return response.end();
  }
  const content = path.startsWith("upload/") ? await base(path) : await file(path, ["web", "fixtures"]);
  if (!content) return send(response, 404, "text/plain", "Not found.");
  send(response, 200, TYPES[extname(path)] ?? "application/octet-stream", content);
}

const server = new Server({
  address: values.host,
  port: Number(values.port),
  quiet: true,
  async onRequest({ request, response }) {
    try {
      await handle(request, response);
    } catch (e) {
      console.error(e);
      if (!response.headersSent) send(response, 500, "text/plain", "Server error.");
    }
    // Rejecting with no error stops Hocuspocus answering the request itself.
    throw null;
  },
  async onLoadDocument({ documentName }) {
    const state = states.get(documentName);
    if (state) return state;
    if (!documentName.startsWith(engine.roomPrefix)) throw new Error("This room needs its original editor; open a new room with this version.");
    const bytes = await base(documentName.slice(engine.roomPrefix.length));
    if (!bytes) throw new Error(`no base named ${documentName}`);
    // A fixed seeder ID makes the seed the same items every time, so after a
    // restart the browsers' copies merge into the new seed instead of
    // conflicting with it.
    const seeded = engine.seed(SEEDER, bytes);
    try {
      return seeded.snapshot();
    } finally {
      seeded.free();
    }
  },
  async onStoreDocument({ documentName, document }) {
    states.set(documentName, Y.encodeStateAsUpdate(document));
  },
});
await server.listen();
console.log(`Collaboration demo: http://${values.host}:${values.port}/`);
