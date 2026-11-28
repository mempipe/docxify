# DOCX engine demo: decisions

D1 and D2 describe the editor as it stands. They are carried from the prototype's design record. D3 to D6 are about the move into this repository.

The core's rules are in [00001](../00001-docx-engine-core/spec.md), the replica's in [00002](../00002-docx-engine-replica/spec.md) and the bindings' in [00003](../00003-docx-engine-wasm/spec.md).

## Set by the operator

These are not the builder's to change.

- 2026-11-15, speed: "ideal is build a very performant engine and editor", and "The fix is to rebuild only the parts that changed."
- 2026-11-26, place: "but this is the demo app right? then put it under apps/demo first."
- 2026-11-27, layout: "go with 1, nested unchanged, start the pr". The two choices were `apps/demo/web` and `apps/demo/scripts` unchanged, or `web/`'s files directly in `apps/demo` with their paths edited.
- 2026-11-27, fidelity: "Copy unchanged". The moved files keep 19 comment lines that name the prototype's repository (D3).
- 2026-11-26, packaging: "in subsequent PR i wannt pull out the editor into a library under @mempipe/docxify-editor?" It is not done here.

## The editor

### D1 — Layout is derived and never stored

Proposed: awaiting operator approval.

- **The engine resolves what Word resolves:** each node's effective properties through defaults, styles, numbering and direct formatting. Layout applies no style rules of its own.
- **Layout runs in TypeScript on the browser's text measurement,** behind an interface that a Rust line breaker can take over. It is five stages, each one module: blocks, lines, pages, markup, paint.
- **Nothing on a page holds state.** Layout output is never stored in the document or in Yrs. It is computed again.
- **Word's core fonts are drawn with open fonts of the same widths,** and each substitution is recorded.
- **Tracked changes and comments are drawn as Word draws them,** with balloons beside the page.
- **The space between two paragraphs is the larger of the two,** not their sum.
- **Word is the measure,** and the operator runs the comparison.

### D2 — An edit costs what it changes

Proposed: awaiting operator approval. The aim is the operator's.

- **People edit on the painted pages through hidden ProseMirror views,** so the keyboard, composition and the clipboard work as in any editor. Every edit goes through the engine. A view is not a second document.
- **A hidden view draws only the blocks near the selection.**
- **The replica reports the blocks it rebuilt.** Typing, formatting and paragraph boundaries use the same paragraph writer. Joined source strands are read together. Typing inside an active comment follows its paired anchors. New independent paragraphs keep their unchanged neighbours. A cell that a row or column insertion created is rebuilt alone too, starting from its paragraph as the table writer left it. A change in list numbering can affect later blocks and requires a full redraw. Global changes, including comment activation, table settlement and drawing-ID allocation, use full materialization. A multi-command batch reports a full redraw after its atomic commit.
- After a local commit, the replica reuses the committed command state alongside the view. A remote update, undo or redo invalidates both. A refused command leaves the preceding state available.
- **One function says where a view change is,** for tracked and untracked typing alike.

## The move

### D3 — The files are the prototype's, unchanged

Proposed: awaiting operator approval.

The source is the prototype at revision `2210c69073059d82b4bba72b1349fbbed49169ca`. That is the revision of the crates and the fixtures, in 00001 D4, 00002 D4 and 00003 D2.

| Here | The prototype's | Git hash |
| --- | --- | --- |
| `apps/demo/web` | `web` | `76c36b7f0ee062888fe8f204fa5612336607b2ec` |
| `apps/demo/scripts` | `scripts` | `dead48b54d720b2e753fc68c275d238d90524acd` |
| `apps/demo/.gitignore` | `.gitignore` | `6d3fcfb67e24c3e148f74aade966c461ba18bf1c` |

The first two are tree hashes. A git tree hash covers the bytes, name and mode of every file below a directory. The third is one file's hash. The prototype has these hashes at that revision. To check this repository's side:

```bash
git rev-parse HEAD:apps/demo/web HEAD:apps/demo/scripts HEAD:apps/demo/.gitignore
```

Two things follow from keeping the bytes:

- **19 comment lines name the prototype's repository.** 17 script headers give a usage path that starts `prototypes/docx-engine/`. Read it as relative to `apps/demo`. One comment cites a plan by its number in that repository, and one mentions that repository's pnpm workspace. No code depends on them. The operator chose to keep them.
- **The prototype's README does not move.** It describes the prototype's own directory and links into its design record, so it cannot be copied unchanged. `apps/demo/README.md` is written for this repository. Its sections on running, editing and what the pages show reuse the prototype's wording.

The demo, the crates and the fixtures move together from now on. A later sync copies them at one revision and records it in each spec.

### D4 — The demo sits in `apps/demo`, with two links

Proposed: awaiting operator approval.

The moved files expect one directory that holds `web/`, `scripts/`, `fixtures/` and Cargo's `target/`. For example, the build reads the module from `../target`, and the server serves `/web/` and `/fixtures/`. In the prototype that directory is the workspace root. Here it is `apps/demo`, so two symbolic links supply the rest:

| Link | Points at |
| --- | --- |
| `apps/demo/fixtures` | `fixtures/` at the root |
| `apps/demo/target` | `target/` at the root |

- **The files stay unchanged.** With the links, no path in them needs an edit.
- **Cargo needs no link.** A Cargo command run in `apps/demo` finds the workspace above it.
- **The root `.gitignore` excepts `apps/demo/target`,** so the link is tracked while Cargo's output stays ignored. It also ignores `node_modules`.
- **`.moon/workspace.yml` adds `apps/*`** as projects. The demo's project ID is `demo`.

Costs:

- `apps/demo/target` points at nothing until Cargo first builds. The demo's build runs Cargo first, so nothing reads the link before then.
- A checkout without symbolic links, such as Git for Windows with its default settings, gets two text files in place of the links. The demo does not run there.

What would overturn this: packaging the editor as a library. That moves and edits its files, so the layout is decided again then.

### D5 — Node.js, pnpm and wasm-bindgen are pinned

Proposed: awaiting operator approval.

- **`.prototools` pins Node.js 26.13.1 and pnpm 11.17.0.** The demo runs its TypeScript directly in Node, with no compile step. The prototype was built with Node 26 and this pnpm version, so these are the versions it was tested with. Older Node versions were not tried.
- **The npm packages are locked by the prototype's `web/pnpm-lock.yaml`,** which is part of the moved files. The install uses `--frozen-lockfile`.
- **`wasm-bindgen-cli` is installed by the task `demo:tools`,** with `cargo install`. proto does not manage it. Its version, 0.2.130, must equal the `wasm-bindgen` crate's in `Cargo.lock`, and `wasm-bindgen` refuses a module built with another version. The version is written in `apps/demo/moon.yml`, so a sync that moves the crate's version must move it there too.
- **Licenses.** `pnpm licenses list` reports 42 installed packages: 35 MIT, five OFL-1.1, one 0BSD and one Python-2.0. The five under the SIL Open Font License are the fonts the pages are drawn with.

### D6 — The checks that need no browser run in `moon check`

Proposed: awaiting operator approval.

`apps/demo/moon.yml` defines the demo's tasks. Each runs a command the prototype already had.

| Task | Command | In `moon check` |
| --- | --- | --- |
| `install` | `pnpm --dir web install --frozen-lockfile` | yes |
| `tools` | `cargo install --locked wasm-bindgen-cli --version 0.2.130` | yes |
| `build` | `node web/build.mjs` | yes |
| `test` | `pnpm --dir web run test` | yes |
| `types` | `node web/generate-types.mjs --check` | yes |
| `fixtures` | `node web/node-demo.ts` | yes |
| `collab` | `node web/collab-check.ts` | yes |
| `browser` | `node scripts/check-browser.mjs` | no |

- **`browser` is left out of `moon check`.** It needs Google Chrome, at the path the prototype's script names, which is a Mac's. With it in, `moon check --all` would fail on a machine without Chrome. It took 61 s here for 1,717 checks.
- **The crates' tasks do not depend on the demo.** Their inputs are `crates/` and `fixtures/`, so a change in `apps/` reruns only the demo's tasks.
- **`moon check --all` now needs Node.js, pnpm and a network** on its first run, for the npm packages and for `wasm-bindgen-cli`.

The scripts that compare with Word, compare with python-docx and time the editor have no task. Each needs a tool this repository does not pin: Microsoft Word, uv, or large documents made on request.

What would overturn this: a pinned browser that the check can find on any machine, which would let `browser` join `moon check`.

## Answers to the prototype's questions

| Question | Answer | Decision |
| --- | --- | --- |
| "What information must the engine expose for layout?" | The tree of blocks with their IDs, and each node's resolved properties | D1 |
| "Should layout calculations run in Rust/WASM or TypeScript?" | TypeScript first, behind an interface | D1 |
| "Can we use browser text layout initially without making DOM authoritative?" | Yes. Nothing on a page holds state | D1 |
| "What would prevent a future transition to deterministic pagination?" | Storing layout output, which is ruled out | D1 |

## Deferred

The editor became the package `@mempipe/docxify-editor` in [00005](../00005-editor-package/spec.md). That pipeline overturns D3, D5 and D6 here, and keeps D4 (00005 D7).

Each of these waits for the condition named.

- **Rules for storing a shared session.** The demo's server keeps uploads and edits in memory. The prototype's rules for what is stored wait for a part that stores a session.
- **The rest of the prototype's README.** Its measurements, its list of limitations and its account of the engine need rewriting for this repository. It waits for the operator to ask for it.
- **Continuous integration.** Nothing runs the checks on a pull request. It waits for the operator to ask for it.
