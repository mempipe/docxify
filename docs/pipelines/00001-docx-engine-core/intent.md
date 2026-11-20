---
status: in-progress
---

# DOCX engine core

## Problem

docxify is a DOCX toolkit with no engine. Nothing in the repository can open a DOCX, change it and write it back without losing what it does not understand.

The engine exists as a prototype built outside this repository. It has a Rust core, a replica for shared editing, WebAssembly bindings and a browser editor. On 2026-11-20 the operator decided where it is published: "my goal is for this to be publish under https://github.com/mempipe/docxify", and "this is source available. that’s the decision".

The prototype is still being built. Moving it one part at a time lets each part be checked against its source.

On 2026-11-20 the operator asked for the first part: "move core/engine now. make sure its mostly byte to bypte." The crate keeps the name it has in the prototype, "so that they are one to one same if possible". The scope is the core alone: "only move core engine now". The operator also asked for this pipeline: "help me write a pipeline based on what import." This records that request as approval and moves the pipeline to in-progress. Acceptance remains with the operator.

## Proposed outcome

The outcome is true when:

- `crates/docx-engine` holds the prototype's core crate and `fixtures/` holds its fixtures. Each is identical to its source at the revision the spec names, and the spec says how to check that.
- The crate depends on no WebAssembly, Yrs or Mempipe code.
- The engine opens a DOCX into a document that can be walked: paragraphs, runs, styles and tables. Content it cannot edit is kept as read-only or opaque, never dropped.
- An edit is a command that names a node by ID. The edited document exports as a DOCX that passes package validation, and unrelated content is preserved in the tested cases.
- The crate's tests, its formatting check and its lint pass through `moon check --all` in this repository.

## Affected users and systems

People and agents who build on docxify. Nothing consumes the crate yet, and it is not published to a registry.

## Constraints

- Move only the core crate and the fixtures its tests read. The replica, the WebAssembly bindings, the browser editor and the prototype's scripts are not part of this pipeline.
- The moved files are byte for byte the source's. This pipeline does not edit them.
- The crate keeps the name `docx-engine`.
- Preserve what the engine does not understand. Never claim lossless round-tripping without tests that support the claim.
- Do not copy implementation code from third-party editors. Dependencies must be compatible with this repository's source-available license.

## Open questions

None remain. The prototype set out to answer ten questions about the document model and editing. The spec quotes each one with its answer.
