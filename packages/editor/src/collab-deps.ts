// Yjs and the Hocuspocus provider, which build.mjs bundles into one browser
// module. They import lib0 by subpath, which the import map cannot list one
// by one, and the provider must share the one copy of Yjs.

export * as Y from "yjs";
export { HocuspocusProvider } from "@hocuspocus/provider";
