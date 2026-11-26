---
status: in-progress
---

# Editor package

## Problem

The browser editor is in docxify, but only as part of the demo. Its modules sit in `apps/demo/web` beside the demo page and the demo's server, and nothing else can depend on them. A program that wants the editor would have to copy the demo.

On 2026-11-26 the operator named the fix: "in subsequent PR i wannt pull out the editor into a library under @mempipe/docxify-editor?" The builder agreed with the direction and named its cost: the moved files stop being a plain copy of the prototype, so each later sync from the prototype is more work. On 2026-11-27, with the demo merged, the operator asked for it: "then create the pr for mempipe package editor", and then "continue with the package PR". This records that request as approval and moves the pipeline to in-progress. Acceptance remains with the operator.

## Proposed outcome

The outcome is true when:

- The editor is a package named `@mempipe/docxify-editor`, in this repository, that another program can depend on.
- The demo uses the editor only through that package.
- The editor's own files are unchanged by the move, and the spec says how to check that.
- The editor behaves as before. Every check that passed on the demo before the move passes after it: the tests, the fixtures through the WebAssembly module, shared editing in Node, and the browser check.
- The package's build, its tests and the demo's checks pass through `moon check --all`.

## Affected users and systems

People and agents who build on docxify. The demo is the package's only user. Nothing is published to a registry.

## Constraints

- One package, with the name the operator gave.
- Do not change what the editor does. This pipeline moves the editor and changes how it is built and loaded.
- Do not edit the editor's own files, so a later sync from the prototype can still replace each of them.
- The prototype's scripts stay unchanged.
- Do not publish the package. Publishing is the operator's call.
- Dependencies must be compatible with this repository's source-available license.

## Open questions

None.
