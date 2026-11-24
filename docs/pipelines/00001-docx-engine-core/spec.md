# DOCX engine core: decisions

D1 to D3 describe the core engine as it stands. They are carried from the prototype's design record, where they are also proposed. D4 to D6 are about the move into this repository.

## Set by the operator

These are not the builder's to change.

- 2026-11-20, publication: "my goal is for this to be publish under https://github.com/mempipe/docxify", and "this is source available. that’s the decision".
- 2026-11-20, scope: "only move core engine now".
- 2026-11-20, fidelity: "move core/engine now. make sure its mostly byte to bypte."
- 2026-11-20, name: the crate keeps the name it has in the prototype, `docx-engine`, "so that they are one to one same if possible".

## The engine

### D1 — Keep what the file holds

Proposed: awaiting operator approval.

- **The original package stays in memory, and the document model points into it.** That is the compressed ZIP and the decoded text of each part the engine parses.
- **Every node is editable, read-only or opaque.** An opaque node keeps its source bytes and is written back unchanged. So are unknown attributes and whitespace inside an edited parent.
- **Export splices.** An untouched subtree is copied from the source bytes. Only changed elements are written anew. An edited source run stays that run, so the markup around its children is kept. When the content of a run or link is written as several parts, the first part keeps the markup before its first child and the last part the markup after its last child. Content that moves to another paragraph declares the namespaces its source paragraph declared for it. When an edit rewrites a table whose properties follow a range marker, the table head emits those properties before the marker, each after the markup that led to it. An untouched table keeps its original order.
- **A property edit reads markup as import does.** The XML parser that import uses says where each property is and which namespace names it. The edit splices at the byte ranges the parser gives. Markup the parser rejects refuses the command.
- **No command adds or removes a relationship, a part or a content type.** Export derives the package additions from visible content. Callers can run `validate_package` on the exported bytes to check package references.
- **Strict OOXML and packages that are not Word documents are refused at open.**
- **Source nodes receive IDs in import order,** so the same bytes give the same source IDs. Projected nodes belong to a single view. A rebuild can reuse slots from nodes the view discarded.
- **Byte ranges into the source are internal. Where content came from can be captured at import,** for a host that maps content back to its source: the part, the XPath of the element in that part's original XML, and a paragraph's `w14:paraId` where it has one. The capture is optional, and nothing keeps it up to date while a document is edited. The part and the `paraId` are public today. The XPath is not built: an opaque node gives only its element name, which does not say which occurrence it is.

### D2 — An edit is a command that applies whole or is refused

Proposed: awaiting operator approval.

- **Headless and shared edits use one command implementation.** `editing::Editor` prepares its writes on a private copy. The replica for shared editing, which has not moved here, prepares the same writes. A refusal leaves live state and undo history untouched. `Document` exposes the projected DOCX for inspection and export.
- **A command on anything that is not editable is refused.** A read-only or opaque node may be removed whole, never changed inside.
- **A place in text is a text node's ID and offsets in Unicode scalar values,** so no range can cut a character in two.
- **Undo belongs to the editing session.** A split keeps the first half's identity. Splits and merges refuse edits that would discard protected content.
- **Built:** typing, line breaks, splitting and merging paragraphs, removing items, replacing a range across paragraphs, bold, italic and underline, paragraph styles, lists, links, table rows and columns, pictures, tracked typing, comments and replies, filling content controls, and updating fields on command and on open.
- **A refusal settles one person's command on one document.** It is never how a merge of two people's edits is settled. The rules for that move here with the replica.

### D3 — Tracked changes and comments are content

Proposed: awaiting operator approval.

- **A tracked change is part of the document, with its author and date.** It is not read out of a collaboration history.
- **Built:** typing and deleting as tracked changes, and comments with replies.
- Removing an imported insertion rejects it. Removing an imported deletion accepts it.
- **Not built:** tracked formatting, general accept and reject commands, comparing two documents, and editing, deleting or resolving a comment. These will use the command preparation and document semantics in D2.

## The move

### D4 — The crate and the fixtures are the prototype's, unchanged

Proposed: awaiting operator approval.

The source is the prototype at revision `2210c69073059d82b4bba72b1349fbbed49169ca`.

| Here | Git tree hash |
| --- | --- |
| `crates/docx-engine` | `45268e3cc24e12b8aacebaf87fb431e119f69177` |
| `fixtures` | `4c4153813adfc342f367567b53026d6431cb3d1d` |

A git tree hash covers the bytes, name and mode of every file below a directory. The prototype's two directories have these hashes at that revision. To check this repository's side:

```bash
git rev-parse HEAD:crates/docx-engine HEAD:fixtures
```

The hashes change as soon as a moved file is edited here, so they describe the commit that last copied the files.

One link is left broken to keep the bytes. `fixtures/real-world/README.md` points at a "Real-world corpus" section of the prototype's README, which has not moved.

The placeholder crate `docxify-engine` is removed. It existed so the tasks had something to run on.

### D5 — Dependencies are locked at the prototype's versions

Proposed: awaiting operator approval.

- The root `Cargo.toml` gains the five workspace dependencies the crate names, with the prototype's version requirements.
- `Cargo.lock` starts from the prototype's lockfile. Cargo kept 49 packages, each at the prototype's version, and dropped 29 that only the parts not moved here use.
- `license-file` and `repository` are removed from `[workspace.package]`. The crate's manifest is unchanged and does not inherit them, and Cargo warns about workspace fields that no member uses. The repository's `LICENSE.md` still governs the crate. The crate stays `publish = false`.

### D6 — The shared tasks test in release and lint every feature

Proposed: awaiting operator approval.

- **`test` runs `cargo test --release`.** At the D4 revision the crate's 258 tests ran in 4.3 s in release and 48 s in debug, with the binaries already built. Each was timed once on a machine doing other work, so the ratio is the finding. Release is also the profile the prototype's own check uses.
- **`lint` runs Clippy with `--all-features`.** The crate has an optional `schema` feature and an example that requires it. Without the flag neither is compiled by any task.

The cost of release is that debug assertions and overflow checks are off while the tests run.

What would overturn this: a defect that a debug assertion would have caught and a release test run missed, or two features of one crate that cannot be enabled together.

## Answers to the prototype's questions

| Question | Answer | Decision |
| --- | --- | --- |
| "Can a preservation-first OOXML representation coexist with a simpler semantic editing IR?" | Yes. The package is kept and the model points into it | D1 |
| "How much of the OOXML package must remain in memory?" | The compressed ZIP, and the decoded text of each part the engine parses | D1 |
| "Can we preserve unknown XML without fully understanding it?" | Yes, in the tested cases. An opaque node keeps its source bytes and is written back unchanged | D1 |
| "What happens when an edit intersects unsupported content?" | The command is refused with a reason, and nothing changes | D2 |
| "Can node IDs survive export and re-import?" | Yes for edits that keep the structure, and not in general | D1 |
| "Should source references be part of the public IR or an internal mapping?" | Byte ranges are internal. The part, the element's XPath and a paragraph's `w14:paraId` can be captured at import, with no upkeep. The XPath is not built | D1 |
| "Should the engine use immutable snapshots or mutable transactions?" | Prepare a batch on private semantic state, commit it atomically, and undo through its session | D2 |
| "How should text ranges be represented?" | A text node's ID and a range in Unicode scalar values | D2 |
| "How should semantic operations map back to OOXML?" | Through source ranges: export copies what is untouched and writes anew what changed | D1 |
| "How can we prevent edits from invalidating document relationships?" | Export derives new relationships from visible content. Callers can validate the resulting package with `validate_package` | D1 |

## Deferred

Each of these waits for the condition named.

- **The replica, the WebAssembly bindings and the browser editor.** Each waits for the operator to ask for its move. The WebAssembly bindings depend on the replica, and the editor depends on both.
- **The prototype's README and scripts.** The README documents every part, so it moves when the parts it describes are here. Until then this repository's README names the crate and how to run it.
- **A license field on the crate.** It waits for the crate to be published to a registry.
- **Continuous integration.** Nothing runs the checks on a pull request. It waits for the operator to ask for it.
