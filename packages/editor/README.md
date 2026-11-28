# @mempipe/docxify-editor

The browser editor for DOCX, as a package. It wraps the engine's WebAssembly module, lays a document out on pages as Word does, and lets people edit on the pages, alone or together.

It is not published to a registry. `apps/demo` is its only user, and the [demo's README](../../apps/demo/README.md) says what the editor does.

## Modules

There is no single entry point. Import a module by its name:

```ts
import { loadDocxEngine } from "@mempipe/docxify-editor/engine";
import { Surface, SURFACE_CSS } from "@mempipe/docxify-editor/surface";
import { alone, connect } from "@mempipe/docxify-editor/collab";
```

| Module | What it holds |
| --- | --- |
| `engine` | `loadDocxEngine`, a typed wrapper over the WebAssembly module |
| `types`, `wire.generated` | The types of commands and inspections. `wire.generated` is generated from the Rust types |
| `model` | One ProseMirror document for each story, built from the engine's inspection |
| `blocks`, `lines`, `paginate`, `markup` | The layout, a stage in each: blocks, lines, pages, and the balloons beside a page |
| `measure` | Text measurement behind an interface, with a fixed measurer for tests |
| `numbering` | List and note numbers, formatted by the engine |
| `fonts` | Which face draws each font family, and loading those faces |
| `paint` | Drawing the pages |
| `editor` | `createEditor`: a ProseMirror view whose every edit becomes engine commands |
| `surface` | `Surface`: the pages, with the views that take the keyboard |
| `collab` | `alone` and `connect`: a session that edits through a replica, by itself or shared through a Hocuspocus server |
| `collab-deps` | Yjs and the Hocuspocus provider, exported again so a host shares this package's copy |

Some modules run with no page. The demo's Node programs use `engine`, `collab`, `collab-deps`, `blocks`, `markup`, `measure` and `paginate`. `fonts`, `paint` and `surface` draw, so they need a browser.

## What a host provides

- **The WebAssembly module.** In a browser, `loadDocxEngine()` fetches `docx_engine_wasm_bg.wasm` from beside the script that holds the engine's code. A bundler must put the file there, or the host passes its URL. In Node, pass the bytes. The package exports the file as `@mempipe/docxify-editor/docx_engine_wasm_bg.wasm`.
- **The fonts.** `fonts` loads its stylesheets from `./node_modules/@fontsource/<family>/` under the page's address. The page must serve the five Fontsource packages there: `arimo`, `caladea`, `carlito`, `cousine` and `tinos`.
- **A bundler, or an import map.** The modules import ProseMirror, Yjs and the Hocuspocus provider by package name. Yjs and the provider also import 19 `lib0` modules by subpath, such as `lib0/encoding`, so an import map needs an entry for each. The prototype bundled those two to avoid that.

## Building and checking

From the repository's root:

```bash
moon run editor:build
moon run editor:test
moon run editor:types
```

- `build` compiles the WebAssembly module, runs `wasm-bindgen` into `src/pkg/`, and compiles `src/` to `dist/` with the TypeScript compiler. A type error fails it.
- `test` runs the layout with a fixed measurer, and where the editor places a typed change. Node runs the sources directly.
- `types` fails when `src/wire.generated.ts` is not what the Rust types generate. After changing a command or an inspection field, run `node src/generate-types.mjs` here.

The modules in `src/` are the prototype's files, unchanged. Some of their comments describe the prototype's own build, such as its import map and its `web/` directory.
