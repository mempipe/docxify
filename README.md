# docxify

## The engine

`crates/docx-engine` is the core of a DOCX engine. It opens a DOCX into a document that can be walked, reports what it can and cannot edit, applies edits addressed by node ID, and writes a valid DOCX that keeps everything it did not change. It depends on no WebAssembly or collaboration code.

Print a document's inspection as JSON:

```bash
cargo run -p docx-engine --example inspect -- fixtures/generated/complex-mixed.docx
```

Replace a text node's text and write the result. With no node ID it edits the first editable text node:

```bash
cargo run -p docx-engine --example edit -- in.docx out.docx [node-id] [text]
```

The rules the engine follows are in the [spec](docs/pipelines/00001-docx-engine-core/spec.md). `fixtures/generated/` holds hand-written test documents, and `fixtures/real-world/` holds documents from real producers, with their [sources and licenses](fixtures/real-world/README.md).

### Shared editing

`crates/docx-engine-yrs` lets several people edit one document. Each person has a replica, which keeps the core's editing state in [Yrs](https://github.com/y-crdt/y-crdt). Replicas exchange updates, and each derives the same document. Undo takes back a person's own changes.

Run two replicas that edit at the same time, converge and export a DOCX:

```bash
cargo run --release -p docx-engine-yrs --example collab -- fixtures/generated/simple-paragraphs.docx out.docx
```

The rules for merging are in the [replica's spec](docs/pipelines/00002-docx-engine-replica/spec.md). The crate does no networking. The caller carries each replica's updates to the others.

### WebAssembly

`crates/docx-engine-wasm` exposes the engine and the replica to JavaScript through [wasm-bindgen](https://github.com/wasm-bindgen/wasm-bindgen). `DocxDocument` opens and reads a DOCX. `DocxReplica` edits one, alone or with others. Commands and inspections cross as JSON, and text offsets count Unicode scalar values, not UTF-16 units.

Build the module:

```bash
cargo build -p docx-engine-wasm --target wasm32-unknown-unknown --profile wasm
```

It is written to `target/wasm32-unknown-unknown/wasm/docx_engine_wasm.wasm`. To use it from JavaScript, generate the glue with `wasm-bindgen-cli` at the version in `Cargo.lock`. The details are in the [bindings' spec](docs/pipelines/00003-docx-engine-wasm/spec.md).

## Development

docxify is a monorepo. [proto](https://moonrepo.dev/proto) installs the tools, and [moon](https://moonrepo.dev/moon) runs the tasks.

[Install proto](https://moonrepo.dev/docs/proto/install), then install the pinned tools:

```bash
proto use
```

Check everything. This checks formatting, lints and tests every crate, and checks that each builds for WebAssembly:

```bash
moon check --all
```

Run one task in one project:

```bash
moon run docx-engine:test
```

The first run downloads the WebAssembly target that `rust-toolchain.toml` names. If a task fails while rustup downloads it, run the check again.

The `format` task only checks. To fix formatting, run `cargo fmt`.

Tool versions are in `.prototools` and `rust-toolchain.toml`. Tasks are in `.moon/tasks`. A new crate under `crates/` gets the same tasks with no moon configuration of its own.

Shared test fixtures go in `fixtures/` at the root. moon replays a cached result unless an input changed, and every crate's tasks take all of `crates/` and `fixtures/` as inputs. Test data kept anywhere else must be added to the inputs in `.moon/tasks/rust.yml`.
