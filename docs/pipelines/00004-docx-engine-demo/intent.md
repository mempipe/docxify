---
status: in-progress
---

# DOCX engine demo

## Problem

docxify has the engine's crates and its WebAssembly bindings. Nothing in the repository shows a document on pages, or lets a person edit one.

The prototype has a browser editor that does this, with a demo page, a collaboration server, and checks that drive the page in Chrome. They are the last parts of the prototype that have not moved.

On 2026-11-26 the operator asked for them, and said where they go: "bring in browser editor? web/ and scripts? but this is the demo app right? then put it under apps/demo first." On 2026-11-27 the operator chose the layout, of the two offered: "go with 1, nested unchanged, start the pr". This records that request as approval and moves the pipeline to in-progress. Acceptance remains with the operator.

## Proposed outcome

The outcome is true when:

- `apps/demo/web` and `apps/demo/scripts` hold the prototype's `web/` and `scripts/`. Each is identical to its source at the revision the spec names, and the spec says how to check that.
- The crates and the fixtures are at that same revision.
- The demo builds in this repository with the tools the repository pins, and the spec says how.
- A person can open a DOCX in the demo, see it on pages, edit it and download it. Two browsers can edit one document together through the demo's server. This holds in the tested cases.
- The demo's checks that need no browser pass through `moon check --all`. The check that needs Chrome runs by one command.

## Affected users and systems

People and agents who build on docxify, and anyone who wants to try the engine in a browser. Nothing is published to a registry.

## Constraints

- Move only `web/` and `scripts/`. The prototype's README is not part of this pipeline.
- The files go under `apps/demo`, nested: `apps/demo/web` and `apps/demo/scripts`.
- The moved files are byte for byte the source's. This pipeline does not edit them. On 2026-11-27 the operator chose this over editing 19 comment lines that name the prototype's repository: "Copy unchanged".
- The demo, the crates and the fixtures are copies of one source revision.
- Do not package the editor as a library here. The operator plans that for a later pull request: "in subsequent PR i wannt pull out the editor into a library under @mempipe/docxify-editor?"
- Dependencies must be compatible with this repository's source-available license.

## Open questions

None.
