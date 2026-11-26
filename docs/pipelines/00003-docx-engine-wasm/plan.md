# DOCX engine WebAssembly bindings: plan

## Steps

1. Copy `crates/docx-engine-wasm` from the prototype at the revision in D2, unchanged. Done.
2. Derive `Cargo.lock` from the prototype's lockfile (D3). Done.
3. Add the `wasm32-unknown-unknown` target and the shared `wasm` task (D4). Done.
4. Add the `wasm` build profile (D5). Done.
5. Describe the bindings in the README. Done.
6. Bring 00001's and 00002's specs and plans up to date where they said the bindings had not moved. Done.

## Verification

Verified on macOS, by the command named:

| What | Command | Result |
| --- | --- | --- |
| The moved directory is the source's | `git rev-parse HEAD:crates/docx-engine-wasm` | The hash equals the prototype's at the D2 revision. The prototype's was read from its repository at that revision |
| The other crates and the fixtures are at the same revision | `git rev-parse HEAD:crates/docx-engine HEAD:crates/docx-engine-yrs HEAD:fixtures` | All three hashes equal the ones in 00001 D4 and 00002 D4 |
| The lockfile is the prototype's | `cmp` between `Cargo.lock` and the prototype's lockfile | No difference |
| Format, lint, tests and the WebAssembly check | `moon check --all` | All 12 tasks pass, four in each crate. The bindings have no tests |
| The `wasm` task can fail | A temporary `compile_error!` on `wasm32` only in the core, then `moon run docx-engine:wasm docx-engine-wasm:wasm` | The bindings' task failed with exit code 101. The line is not in the branch, and the core's hash was checked again after it was removed |
| The module builds | The build command in D5 | Wrote the module, 3,546 KiB raw and 897 KiB with gzip |
| The bindings work from JavaScript | `wasm-bindgen --target nodejs` 0.2.130 on the module, then a script in Node 22 on `fixtures/generated/simple-paragraphs.docx` | Two replicas each inserted text, exchanged updates and exported the same bytes. The export validated, and `DocxDocument` read "Alice Bob First paragraph." from it. A command naming a missing node threw "no node n999999". Undo took back only the first replica's text |
| A fresh clone works | `proto use`, then `moon check --all`, in a new clone of the branch, with the target removed first by `rustup target remove wasm32-unknown-unknown --toolchain 1.100.0` | The first run failed one task, `docx-engine-yrs:wasm`, in rustup's concurrent download of the target (D4). The target was installed, and a second run passed all 12 tasks with a clean tree afterwards |

The Node script was a one-off check. It is not in the branch, because the editor's Node demo covers the same ground and moves next.

Asserted, not verified here:

- D1 restates what the crate's code and documentation say. The Node script exercised part of it.
- The module was not loaded in a browser.
- Linux, Windows and a machine with none of the tools installed are untested.

## Next

The browser editor moves next, in its own pull request. It brings `web/` and the prototype's scripts, with Node.js and pnpm.

While the prototype keeps changing, the crates here fall behind it. Bringing them up to date is one copy of every crate and the fixtures at a newer revision, with new hashes in 00001 D4, 00002 D4 and D2 here.
