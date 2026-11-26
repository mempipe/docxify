---
status: in-progress
---

# DOCX engine replica

## Problem

docxify has the engine's core, which edits a DOCX for one person. Nothing in the repository lets several people edit the same document and reach the same result.

The prototype has a replica that does this. It keeps the core's editing state in Yrs, so replicas exchange updates and each derives the same document. The prototype's browser editor also depends on it: every edit there goes through a replica, including when one person edits alone.

On 2026-11-24 the operator weighed keeping the replica out of the public repository: "can we not publish the yrs and still keep most of the part public?" The decision was to publish it: "ok then let’s just expose, since not super important". On 2026-11-26 the operator asked for the move, one part at a time: "lets expose everything lets do the next pr step by step,". This records that request as approval and moves the pipeline to in-progress. Acceptance remains with the operator.

## Proposed outcome

The outcome is true when:

- `crates/docx-engine-yrs` holds the prototype's replica crate. It is identical to its source at the revision the spec names, and the spec says how to check that.
- The core crate and the fixtures are at that same revision.
- Two replicas started from one DOCX can each edit, exchange updates in either order, and show the same document. That document exports as a valid DOCX. This holds in the tested cases.
- A person's undo takes back their own change and leaves everyone else's.
- The crate's tests, its formatting check and its lint pass through `moon check --all` in this repository.

## Affected users and systems

People and agents who build on docxify. Nothing consumes the crate yet, and it is not published to a registry. The WebAssembly bindings and the browser editor depend on it, and they move later.

## Constraints

- Move only the replica crate. The WebAssembly bindings, the browser editor and the prototype's scripts are not part of this pipeline.
- The moved files are byte for byte the source's. This pipeline does not edit them.
- The crate keeps the name `docx-engine-yrs`.
- The replica and the core are copies of one source revision. The replica calls into the core and shares its test helpers.
- The operator's requirement for shared editing stands: "Shared state may contain conflicting choices. Every client must interpret them consistently into usable output, without automatic repair writes."
- Never claim that merges are correct beyond what the tests cover.
- Dependencies must be compatible with this repository's source-available license.

## Open questions

None remain. The prototype set out to answer six questions about shared editing. The spec quotes each one with its answer.
