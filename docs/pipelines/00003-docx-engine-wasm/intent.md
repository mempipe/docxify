---
status: in-progress
---

# DOCX engine WebAssembly bindings

## Problem

docxify has the engine's core and its replica for shared editing. Both are Rust crates. Nothing in the repository lets a browser or another JavaScript host use them.

The prototype has WebAssembly bindings that do this. The browser editor calls the engine only through them.

On 2026-11-26 the operator asked for the remaining parts to move one at a time: "lets expose everything lets do the next pr step by step,". After the replica merged, the operator asked for this part: "ok merge it and start the wasm pr". This records that request as approval and moves the pipeline to in-progress. Acceptance remains with the operator.

## Proposed outcome

The outcome is true when:

- `crates/docx-engine-wasm` holds the prototype's bindings crate. It is identical to its source at the revision the spec names, and the spec says how to check that.
- The core, the replica and the fixtures are at that same revision.
- The bindings build for `wasm32-unknown-unknown` in this repository, and the spec says how.
- From JavaScript, a host can open a DOCX, inspect it, edit it through a replica, exchange updates with another replica, and export a valid DOCX. This holds in the tested cases.
- The crate's formatting check and lint, and a check that every crate builds for WebAssembly, pass through `moon check --all`.

## Affected users and systems

People and agents who build on docxify. Nothing consumes the bindings yet, and they are not published to a registry. The browser editor depends on them and moves later.

## Constraints

- Move only the bindings crate. The browser editor and the prototype's scripts are not part of this pipeline.
- The moved files are byte for byte the source's. This pipeline does not edit them.
- The crate keeps the name `docx-engine-wasm`.
- The bindings, the replica and the core are copies of one source revision.
- Dependencies must be compatible with this repository's source-available license.

## Open questions

None.
