# Editor package: decisions

The editor's own rules are 00004 D1 and D2, in the [demo's spec](../00004-docx-engine-demo/spec.md). This pipeline does not change them. It decides where the editor's files live, how they are built, and how the demo loads them.

## Set by the operator

These are not the builder's to change.

- 2026-11-26, the package: "in subsequent PR i wannt pull out the editor into a library under @mempipe/docxify-editor?"
- 2026-11-27, the request: "then create the pr for mempipe package editor", and "continue with the package PR".

## Decisions

### D1 — One package, in `packages/editor`

Proposed: awaiting operator approval.

`packages/editor` is the package `@mempipe/docxify-editor`. It holds 19 of the 28 files of the prototype's `web/`:

- **16 modules:** the engine wrapper, the wire types, the view model, the layout, painting, fonts, the editing view, the surface and the collaboration session.
- **Two tests,** for the layout and for where the editor places a typed change.
- **The type generator,** beside the file it writes.

`apps/demo/web` keeps what is the demo: the page and its script, the collaboration server, the two Node programs that check the editor, the build script and the manifest.

The line between them was already there. No module of the editor imported a demo file, and none looks up an element of the demo's page.

The engine wrapper is in the same package, as the operator asked for one. A program with no page imports the `engine` module alone, which loads no editor code.

### D2 — The editor's files are unchanged, and `web/` is no longer one copy

Proposed: awaiting operator approval.

Overturns: 00004 D3

00004 D3 gave one tree hash for `apps/demo/web`. That hash no longer holds, because the directory is split. What holds now, against the prototype at the revision 00004 D3 names:

| Files | State |
| --- | --- |
| The 19 files in `packages/editor/src` | Unchanged. Each has the hash of the prototype's `web/` file of the same name |
| `apps/demo/scripts`, 25 files | Unchanged. The tree hash in 00004 D3 still holds |
| `apps/demo/.gitignore` | Unchanged |
| `browser-demo.ts`, `node-demo.ts`, `collab-server.ts`, `collab-check.ts` | Edited: they import the package, and the three Node programs ask the package for the WebAssembly module |
| `index.html` | Edited: the import map is removed (D5) |
| `build.mjs`, `package.json` | Rewritten for this repository (D5) |
| `pnpm-lock.yaml`, `pnpm-workspace.yaml` | Moved to the root and changed (D4) |

To check the 19 files, list their hashes beside those of the prototype's `web` tree. That tree is the one 00004 D3 records, and this repository holds it:

```bash
git ls-tree 76c36b7f0ee062888fe8f204fa5612336607b2ec
git ls-tree HEAD:packages/editor/src
```

Each of the 19 names has the same hash in both lists.

Keeping those files unchanged leaves three things that read oddly here:

- Some comments describe the prototype's build, such as `collab-deps.ts` on an import map and a bundle that no longer exist.
- The modules sit flat in `src/`, as they did in `web/`, so their imports of each other and of `./pkg/` need no edit.
- The font stylesheets are loaded from a fixed address under the page (D5).

The edits to the four programs are kept to what they import, so a sync can apply them again. Their comments are the prototype's, and some usage lines are out of date:

- `collab-check.ts` and `collab-server.ts` say to run `node web/build.mjs` first. That build now needs the package built, so it fails on a clean tree. The READMEs give `moon run demo:build`, which builds both.
- `browser-demo.ts` names the editor's modules as files beside it, such as `surface.ts`.

**A later sync from the prototype is no longer one copy.** The crates, the fixtures and `scripts/` still are. For `web/`:

1. Each of the 19 files replaces its copy in `packages/editor/src`.
2. The four edited programs and `index.html` take the prototype's new text, with the same edits applied again.
3. A file that is new in the prototype's `web/` is placed by hand: in the package if the editor needs it, in the demo otherwise. A new test also joins the `test` script of the manifest that runs it, because each `test` script names its files.
4. A changed dependency in the prototype's `package.json` is carried into the manifest that uses it.
5. `build.mjs` and the `scripts` of both manifests are this repository's. A change to the prototype's build or to its scripts is carried over by hand.
6. A replaced file that fails the type check (D3) stops the sync. The operator chooses between fixing the file in the prototype first, editing it here as an exception to this decision, and loosening the compiler's options.

### D3 — Each module is an entry point, compiled with its types

Proposed: awaiting operator approval.

- **The package exports each module by name,** such as `@mempipe/docxify-editor/surface`. There is no single entry point. The prototype has none, and choosing what one would export is a design of its own.
- **It also exports the WebAssembly file,** as `@mempipe/docxify-editor/docx_engine_wasm_bg.wasm`, so a host can find it without knowing the package's layout.
- **Exports point at compiled JavaScript with declarations,** in `dist/`. The TypeScript compiler writes both. The prototype stripped types with a Node API and rewrote import paths with a regular expression. That gave no declarations and checked no types.
- **The sources pass a strict type check with no edit.** Nothing checked them before. A type error now fails the build.
- **The tests still run the sources directly in Node.** Two compiler options refuse syntax that Node's type stripping could not run.
- **`wasm-bindgen` writes into `src/pkg/`,** where the sources import it, and the build copies it to `dist/pkg/`.
- **The package is `private` and at version `0.0.0`.** It is not published.

The new development dependency is `typescript` 5.9.3, under Apache-2.0.

### D4 — One pnpm workspace at the root

Proposed: awaiting operator approval.

Overturns: 00004 D5

00004 D5 locked the npm packages with the prototype's `web/pnpm-lock.yaml`, and installed `wasm-bindgen-cli` in `demo:tools`. Both move. Its pins of Node.js, pnpm and the `wasm-bindgen-cli` version stand. That version is now written in `packages/editor/moon.yml`, in the task `editor:tools`.

- **`pnpm-workspace.yaml` and `pnpm-lock.yaml` are at the root.** The workspace holds `apps/demo/web` and `packages/*`. The demo depends on the package as `workspace:*`, so both share one copy of Yjs.
- **The lockfile holds the same 67 packages at the same versions,** and `typescript` as the 68th. It was moved, and pnpm rewrote which package asks for what.
- **pnpm's default linker replaces the hoisted one.** The prototype hoisted so that its import map could point at fixed paths. The import map is gone (D5).
- **Each manifest names what its own code imports.** The package takes ProseMirror, Yjs, `y-protocols` and the Hocuspocus provider. The demo keeps the Hocuspocus server, Yjs, `y-protocols`, the five fonts and esbuild.
- **The release-age guard and the esbuild build rule are carried over** from the prototype's workspace file.

### D5 — The demo loads the package through a bundle

Proposed: awaiting operator approval.

- **esbuild bundles the page's script with the package** into `web/dist/`. The prototype served each module as its own file and resolved package names with an import map in `index.html`. An import map lists fixed paths, which a package installed by pnpm does not have.
- **The editor module is a second entry of the bundle,** written as `web/dist/editor.js`. The browser check imports that path in the page to call one of the editor's functions. With code splitting, both entries share one copy of the code. This keeps `scripts/` unchanged.
- **The build copies the WebAssembly file into `web/dist/`,** where the engine looks for it by default: beside the script that holds its code.
- **The Node programs ask the package for the WebAssembly file** with `import.meta.resolve`, in place of a path into `web/pkg/`.
- **The page still serves the fonts.** The `fonts` module loads stylesheets from `./node_modules/@fontsource/` under the page's address. The demo depends on the five font packages, so pnpm links them at that place, and the demo's server follows the links.

The page's script is two files: `browser-demo.js`, and a chunk it shares with the editor entry. Together they are 953 KiB raw and 220 KiB with gzip.

- **They are not minified,** so an error in the browser names real functions.
- **Minified, they would be 429 KiB raw and 136 KiB with gzip.** That was measured with the same build and `minify: true`.
- **The prototype minified its bundle of Yjs and the Hocuspocus provider.** That code is now in `browser-demo.js`, unminified.

### D6 — The package builds first, and the demo's tasks follow

Proposed: awaiting operator approval.

Overturns: 00004 D6

| Task | Command | In `moon check` |
| --- | --- | --- |
| `editor:install` | `pnpm install --frozen-lockfile` | yes |
| `editor:tools` | `cargo install --locked wasm-bindgen-cli --version 0.2.130` | yes |
| `editor:build` | `node build.mjs` | yes |
| `editor:test` | `pnpm run test` | yes |
| `editor:types` | `pnpm run types` | yes |
| `demo:build` | `node web/build.mjs` | yes |
| `demo:test` | `pnpm --dir web run test` | yes |
| `demo:fixtures` | `node web/node-demo.ts` | yes |
| `demo:collab` | `node web/collab-check.ts` | yes |
| `demo:browser` | `node scripts/check-browser.mjs` | no |

- **`editor:install` installs the whole pnpm workspace,** the demo's packages too. There is one lockfile, so there is one install.
- **The demo's tasks that use the package depend on `editor:build`.** `editor:build` declares its outputs, so moon reruns them when the package changes. Their own inputs do not list the package's files. A change to `packages/editor/src` was seen to rerun `demo:fixtures` this way.
- **`demo:test` now runs only the test of how the Word comparison scores lines.** The editor's two tests moved to `editor:test`. The same 52 tests run in all.
- **`demo:browser` stays out of `moon check`,** for the reason 00004 D6 gives.

### D7 — The demo keeps its layout and its two links

Proposed: awaiting operator approval.

00004 D4 names packaging the editor as what would decide the demo's layout again. This pipeline is that packaging, and the layout stands. `apps/demo` still holds `web/` and `scripts/`, with the links `fixtures` and `target` beside them.

The moved-out editor needs neither link. The demo's own files still read through them:

- `node-demo.ts` writes to `target/web-demo`.
- `scripts/compare-with-word.mjs` writes to `target/word-compare`.
- `scripts/check-with-python-docx.sh` runs `./target/debug/examples/edit`.
- The demo's server serves `fixtures/`.

One example in 00004 D4 no longer holds. The demo's build does not read the module from `../target`. The package's build reads it from `target/` at the root.

## Deferred

Each of these waits for the condition named.

- **Publishing the package.** It waits for the operator to ask for it. A published package also needs a license field and a decision on whether the WebAssembly file ships inside it.
- **A single entry point.** It waits for a second user of the package, which shows what one would export.
- **A font address the host can set.** The fixed address under the page (D5) suits the demo. It waits for a host that serves fonts from somewhere else, because the fix edits `fonts.ts`.
- **A type check of the demo's own files and of the tests.** The demo's page script was checked once against the package's declarations, by hand, and passed. The Node programs and the tests need Node's type declarations, which are not installed. It waits for the operator to ask for it.
- **A separate package for the engine wrapper.** A program with no page can import the `engine` module today (D1). A second package waits for a user that must not install ProseMirror.
