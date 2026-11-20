---
status: in-progress
---

# Monorepo for the Rust and TypeScript libraries

## Problem

docxify holds its license, its contributor documents and its agent guidance, and no code. There is no place to put a library, no pinned tool versions and no command that checks a change. The project is a Rust library and a TypeScript library developed together, so both need one repository layout, one set of tool versions and one way to check a change.

The operator asked on 2026-11-20: "can you use moon and proto to set up a monorepo of rust and typescript library? create PR". This records that request as approval and moves the pipeline to in-progress. Acceptance remains with the operator.

## Proposed outcome

The outcome is true when, on a fresh clone:

- `proto use` installs every tool the repository needs, at pinned versions.
- The repository holds a Rust library and a TypeScript library, each a moon project.
- `moon check --all` checks formatting, lints, type-checks, tests and builds both libraries.
- That command fails when any one of those checks fails.

## Affected users and systems

Contributors and agents working in this repository. Nothing is published, and nothing else consumes the repository yet.

## Constraints

- Use moon to run tasks and proto to install tools.

## Open questions

None.
