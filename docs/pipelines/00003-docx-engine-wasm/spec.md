# DOCX engine WebAssembly bindings: decisions

D1 describes the bindings as they stand. It is carried from the crate and the prototype's design record. D2 to D5 are about the move into this repository.

The core's rules are in [00001](../00001-docx-engine-core/spec.md), and the replica's in [00002](../00002-docx-engine-replica/spec.md).

## Set by the operator

These are not the builder's to change.

- 2026-11-24, publication: "ok then let’s just expose, since not super important". The engine is published with every part.
- 2026-11-26, scope: "lets expose everything lets do the next pr step by step,", then "ok merge it and start the wasm pr". One part moves in each pull request, and this is the third.
- Fidelity and name follow 00001: the moved files are the prototype's, and the crate keeps its name, `docx-engine-wasm`.

## The bindings

### D1 — One JSON contract across the boundary

Proposed: awaiting operator approval.

- **Structured values cross as JSON strings.** Commands go in as the JSON the Rust `Command` type reads. Inspections, settings, redraws and validation reports come out as JSON. So JavaScript and Rust share one versioned contract, such as `docx-engine.inspect/2`, and the bindings add no shapes of their own.
- **Text offsets are Unicode scalar values,** as in the Rust API. They are not JavaScript's UTF-16 units. A host converts at its edge.
- **Two classes.** `DocxDocument` opens a DOCX and reads it. All editing goes through `DocxReplica`, including when one person edits alone. It wraps the replica in 00002 and exposes its updates, undo and redo, carets that follow content, and export.
- **A refusal is a JavaScript error** carrying the engine's reason. A refused command changes nothing, as 00001 D2 says.
- **Free functions** validate a package, format a note number in one of Word's formats, and name the replica's shared format, `docx-engine.yrs/9`.

## The move

### D2 — The crate is the prototype's, unchanged

Proposed: awaiting operator approval.

The source is the prototype at revision `2210c69073059d82b4bba72b1349fbbed49169ca`. That is the revision of the core and the fixtures in 00001 D4, and of the replica in 00002 D4.

| Here | Git tree hash |
| --- | --- |
| `crates/docx-engine-wasm` | `027d9f56d346a1c7af73bc3c13e1644ed2a04932` |

A git tree hash covers the bytes, name and mode of every file below a directory. The prototype's directory has this hash at that revision. To check this repository's side:

```bash
git rev-parse HEAD:crates/docx-engine-wasm
```

The bindings call both other crates, so all three crates and the fixtures move together from now on. A later sync copies them at one revision and records it in each spec.

### D3 — The lockfile is the prototype's

Proposed: awaiting operator approval.

- **`Cargo.lock` is now byte for byte the prototype's.** Every crate the prototype's workspace holds is here, so Cargo keeps all 78 of its packages.
- **No third-party package is new.** `wasm-bindgen` 0.2.130 and its support crates were already locked. On `wasm32`, `getrandom` uses them, and both the core's `zip` and the replica's `yrs` reach `getrandom`.
- **The crate's manifest names its own dependencies.** The root `Cargo.toml` gains no workspace dependency.

### D4 — Every crate is checked for WebAssembly

Proposed: awaiting operator approval.

- **`rust-toolchain.toml` adds the `wasm32-unknown-unknown` target,** so rustup installs it with the toolchain.
- **A shared `wasm` task runs `cargo check --target wasm32-unknown-unknown`** in every crate.

Every crate needs the check, not only the bindings. The bindings compile the other two for `wasm32`, and the core's `zip` features were chosen so that it builds there with no C toolchain. Until this pipeline, no task compiled any crate for `wasm32`.

A check is enough. No crate has code that only compiles for one target. The host's `lint` and `test` tasks already cover the code itself.

The task can fail. A `compile_error!` that applies only on `wasm32`, added to the core for the trial, failed the bindings' `wasm` task with exit code 101. The host tasks do not see such an error.

Costs:

- Each contributor downloads the target once.
- The first check on this machine took 14 s for the core, 4 s for the replica and 3 s for the bindings. Each was timed once, on a machine doing other work.

What would overturn this: code that is specific to one target, which needs a build or a test on `wasm32` instead of a check.

### D5 — The `wasm` build profile is the prototype's

Proposed: awaiting operator approval.

The root `Cargo.toml` gains the prototype's `[profile.wasm]`. It inherits `release`, optimizes for size, and turns on whole-program optimization with one code generation unit and `panic = "abort"`. A browser downloads the module, so size comes first.

Build the module with:

```bash
cargo build -p docx-engine-wasm --target wasm32-unknown-unknown --profile wasm
```

Built this way, `target/wasm32-unknown-unknown/wasm/docx_engine_wasm.wasm` was 3,546 KiB raw and 897 KiB compressed with gzip at level 9.

The prototype's `[profile.release]` sets `opt-level = 3`, which is Cargo's default. It is left out, so nothing else in the workspace changes.

## Answers to the prototype's questions

| Question | Answer | Decision |
| --- | --- | --- |
| "Can native Rust, WASM, and TypeScript bindings share one stable public API?" | Yes, as one JSON contract for commands and inspection. Rust and WebAssembly share it here. The TypeScript types are generated from the Rust types, and move with the browser editor | D1 |

## Deferred

Each of these waits for the condition named.

- **The JavaScript glue and the browser editor.** `wasm-bindgen-cli` turns the module into a JavaScript package. The editor's build runs it, so it moves with the editor, in the next pull request.
- **Tests for the bindings.** The crate has none of its own. In the prototype the editor's Node demo and its browser checks exercise them, and those move with the editor.
- **An npm package.** It waits for the bindings to be published.
- **Continuous integration.** Nothing runs the checks on a pull request. It waits for the operator to ask for it.
