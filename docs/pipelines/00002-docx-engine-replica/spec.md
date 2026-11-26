# DOCX engine replica: decisions

D1 to D3 describe shared editing as it stands. They are carried from the prototype's design record. D4 and D5 are about the move into this repository.

The core's own rules are in [00001](../00001-docx-engine-core/spec.md). Where this spec says "00001 D2", it means that decision there.

## Set by the operator

These are not the builder's to change.

- 2026-11-17, collaboration: "Shared state may contain conflicting choices. Every client must interpret them consistently into usable output, without automatic repair writes."
- 2026-11-17, backend: "i don't really wanna test on automerge and loro at the moment as i want yrs and something more general".
- 2026-11-24, publication: "ok then let’s just expose, since not super important". The replica is published with the rest of the engine.
- 2026-11-26, scope: "lets expose everything lets do the next pr step by step,". One part moves in each pull request.
- Fidelity and name follow 00001: the moved files are the prototype's, and the crate keeps its name, `docx-engine-yrs`.

## Shared editing

### D1 — The contract

Proposed: awaiting operator approval. The requirement in it is the operator's.

**The requirement.** "Shared state may contain conflicting choices. Every client must interpret them consistently into usable output, without automatic repair writes."

1. **Shared facts, one reading.** The shared state is facts that replicas add: content, removals, ranges over content, and properties of paragraph marks. Every client reads the same facts into the same usable document. Reading writes nothing. Conflicting choices may stay stored.
2. **Unrelated meaning is kept.** A reading may leave out one of two competing choices, by a stated rule. It must not lose content, formatting, authorship or unknown properties that no edit was over.
3. **The rules cover ordinary edits, and examples fix the boundaries.** The reading rules below settle any combination of the edits they name. The table shows the cases where two edits meet or a boundary is in doubt, and those rows are checked. A case gets a row when the rules leave it open, not before.

How the facts are read:

- **Content states what it has when it is put in:** a formatting basis, and the tracked insertion it is part of or none. A basis is the style references, direct properties and unknown properties text came with. Neither changes afterwards.
- **A key with one value at a time,** such as bold or a paragraph's style: the latest command over the content holds. Of two made at the same time, one holds, the same for everyone.
- **A key that holds several,** such as comments: all of them hold.
- **Runs are not stored.** Export makes a run of each stretch of characters that read alike.
- **Tracked deletion cancels the same author's pending insertion. Otherwise it retains the content and records the deleter without replacing the inserter.** The operator selected this behavior on 2026-11-18.
  - A deletion range applies to earlier insertion generations, not concurrent or later typing.
  - Undo restores a generation, so restored earlier content inside a still-active deletion remains marked deleted.
  - Deleting already deleted content again is a no-op.
  - Overlapping concurrent deletions retain both actions. The latest applicable action supplies the displayed deletion author, and undo can expose the other.
  - Author means the author identifier supplied in `Track`.
- **An inline object or paragraph boundary keeps one identity.** If concurrent undo restores multiple references to it, the first in the converged sequence is displayed. Created paragraph boundaries carry an identity distinct from the source template, so two independent Enter presses remain two boundaries. Other references remain stored and become visible if that occurrence is removed. This is a reading rule, not a repair write.

Unless a row gives a sequence, the two edits happen concurrently and then sync. `R1` and `R2` in the table are formatting bases.

| # | Before | Alice | Bob | All replicas display | Stays stored |
| --- | --- | --- | --- | --- | --- |
| 1 | `pa` and `agraph` bold, `r` between them plain | Deletes `pa` | Deletes `r` | `agraph`, bold | Three removals. Every bold range |
| 2 | `abcde` with R1 | Types `Z` after `d`, stating R1 | Deletes `abcde` | `Z` with R1 | `Z` and what it stated. Five removals |
| 3 | `AB` with R1, `CD` with R2, `Q` with R1 between `C` and `D` | Deletes `BC` | Deletes `Q` | `A` with R1, `D` with R2 | Three removals |
| 4 | `abcde` plain | Bolds `bc` | Bolds `cde` | `bcde` bold | Both ranges |
| 5 | `abcde` plain | Sets bold on over `bcd` | Sets bold off over `cd` | `b` bold. `c` and `d` alike: both on or both off, the same for Alice and Bob | Both ranges |
| 6 | `abcde` plain | Bolds `bcd` | Types `Q` between `c` and `d` | `Q` bold, with the basis Bob stated | The range, and `Q` |
| 7 | `abc` plain | Bolds `bc` | Types `Q` right after `c` | `Q` bold for both, or plain for both | The range, and `Q` |
| 8 | `wx` is Alice's tracked insertion | Nothing | Types `Q` between `w` and `x` with tracking on | `w` and `x` Alice's insertion, `Q` Bob's | `Q` and the revision it stated |
| 9 | `abcde` plain | Comments on `bc` | Comments on `cde` | `b`: Alice's. `c`: both. `d`, `e`: Bob's | Both ranges |
| 10 | One paragraph | Presses Enter at a place | Presses Enter at the same place | Three paragraphs, the middle one empty | Both marks |
| 11 | `abcde` with R1 | Deletes `bcd`, and undoes after the sync | Types `Z` between `c` and `d` | `abcZde`, each character with its basis | The removals, new `bcd`, and `Z` |
| 12 | `AB` with R1. Alice closes and opens the document | Types `Q` with R2 between `A` and `B` | Nothing | `A` R1, `Q` R2, `B` R1 | `Q`, with a name nothing had |
| 13 | A table of two rows and two columns | Adds a column | Adds a row | A table of three rows and three columns, with the four cells it had and what was in them | Both changes |
| 14 | A table of two rows | Removes the upper row | Removes the lower row | A table of one row, the same row for both. A table keeps at least one row, so the later removal in the log does not count | Both removals |
| 15 | Alice bolded `A` in `AB`, then Bob bolded `AB`. Cara sees both and types `X` between the letters. | Nothing | After syncing, undoes his bold | `AX` bold, `B` plain | Alice’s bold range and Cara’s insertion |

Other content follows the same rules:

- **Links** use the same paired-range boundary rule as formatting. Concurrent interior typing is covered, and typing at an edge follows the converged sequence order, as row 7 permits. A typed command explicitly takes the link of the text or run it names. An explicit unlink clears it.
- **Comment payloads are immutable, and their paired ranges activate them.** A comment states the selected part of each paragraph, in the order the paragraphs are shown, and runs from its first shown anchor to its last. Storage order does not decide what it covers: the cells a row insertion creates are stored after the source paragraphs. An active reply retains its root, and uses its own anchor if the root's author undoes the original range.
- **Assignments such as control fills and field results remain separate actions.** The latest active causal turn wins, ties resolve by action identity, and undo removes one's own action.
- **A fill of a content control that a removal takes out of the document stays stored.** It is not shown while the control is gone, and shows again when the removal is undone.

No reading writes a repair.

The rules do not yet say what accepting or rejecting a tracked change does. That needs a rule before it needs a row.

What the tests cover:

- **Rows 1 to 12** run as lower-store tests in `tests/contract.rs`, using a test reader over paired Yrs items. They do not themselves exercise replica command preparation or DOCX materialization.
- **Rows 3 to 7, 11 and 15** also run through the replica in `tests/replica_contract.rs`. It checks displayed text and formatting and a freshly exported DOCX, and exercises both delivery orders and duplicate updates.
- **Rows 13 and 14** are tests of the replica.
- **Where a row permits two outcomes,** as row 7 does, either passes.

`tests/contract.rs` also runs every pair of edits from a small set, made at the same time. It checks each result against rules 1 and 2 and the reading rules: what must hold, and what may. Its bounds are in the tests:

- One five-character document and 57 single edits.
- Every pair runs with either replica receiving first, with and without one undo, and once after a replica is closed and opened: 16,245 runs.
- Every three of 41 of the edits run once, one edit each by three people: 68,921 runs.
- Separate Yrs tests deliver three independent updates to one receiver in all six orders for 343 edit combinations, with duplicate delivery. They also cover a dependent update arriving before its prerequisite, including text outside the Basic Multilingual Plane.

This says nothing of longer documents or longer histories.

Taken for now, and the operator can move it: content that two people both deleted comes back when one of them undoes. Yrs's undo does this, and the other choice means undo cannot be Yrs's own.

### D2 — Meaning is stated, never guessed back

Proposed: awaiting operator approval.

**Do not make up for missing meaning with automatic repair writes, guesses from neighbouring nodes, or treating arbitrary user-command refusals as merge resolution.**

It names three things, and each was once in this code:

- **A repair write:** a client adding to the shared state to put right what a merge left.
- **A guess from neighbours:** giving text the text node, and so the formatting, of whatever is beside it.
- **Unspecified command refusal:** making a view by running user commands and silently omitting whatever they refuse. The structural interpreter in D3 has explicit resolution rules. Codec failures outside those rules are errors, not additional merge policy. Text and range materialization build directly.

### D3 — One semantic state, with a Yrs adapter

Proposed: awaiting operator approval. The operator authorized this design in the prototype on 2026-11-18.

This describes the implementation. Passing bounded checks is not a proof over arbitrary histories.

- **One editing model.** `docx_engine::editing` prepares commands against semantic state and derives the document. `editing::Editor` runs without Yrs. `docx_engine_yrs::Replica` commits the same prepared writes to Yrs. The base DOCX is immutable. The projected tree is never edited to discover shared changes.
- **A flow is divided into strands.** Each source paragraph has its own text, and paragraphs created by splitting it stay in that strand. A split inserts a boundary and a join removes one, without moving characters. Story and cell boundaries constrain editing. New table cells have stable identities built from the row identity and cell origin, with named roots, so concurrent first typing in the same cell retains both authors' text.
- **Ranges are paired ordinary sequence items, not Yrs range formats.** They carry bases, formatting, links, revisions and comment anchors. Content deletion skips the boundaries. Undo and redo remove and restore the pair together. The reader rejects an unpaired boundary as damaged state.
- **Content carries its formatting basis and insertion generation.** Source text also names its XML leaf, so export can retain attributes and markup between leaves. The reader makes runs from contiguous content with the same properties. Tracked deletion follows D1's author and generation rules.
- **At a tied insertion point, a newly minted generation can pass the closing boundary of an earlier generation.** Formatting and other shared boundaries retain their existing precedence. This keeps successive keys at a paragraph end side by side, and preserves row 15 under undo.
- **Assignments are independent immutable actions.** Settings, field results, control fills and structural payloads are facts keyed by identity. The latest active causal turn wins, with identity as a tie-break. Undo removes the author's action. Payloads needed by other surviving actions stay available.

#### Table coverage and author-time checks

**Table facts decide cell coverage.** The importer supplies cell origins, column identities and vertical-merge dependencies.

- A new column names its donor and both neighbours. A cell or wrapped cell covering both neighbours widens. Otherwise the cell covering the donor is copied. A missing donor cell extends a partial row’s gap. A wrapped donor supplies a bare cell.
- Removing a wrapper vacates its slots and preserves the columns of surviving cells. A vacancy has no typing target. A copied row inherits its template’s origins, and gets an ordinary editable empty cell for a wrapper or vacancy. The slot identity carries the merge intent through copying.
- Wrappers stay whole. Templates remain available independently of undoable activations.
- A removed column transfers width to its previous live neighbour, or to its next neighbour at the left edge. A copy of a hidden column uses its covering cell and the width of the live recipient.
- A row or copy extending beyond the declared grid retains all its cells. Its visibility holds the table on the source grid.

#### Vertical merges and concurrent visibility

**One merge-start rule for rows and wrappers.** A deletion is refused when the author can see a vertical merge continuing from that start. Whether a merge continues is the table-level decision below.

- If a dependency appears concurrently or returns through undo, the start row and its whole owning wrapper are retained as long as a visible continuation needs them.
- The retention is reported, with which was kept: a deleted row, or a removed wrapper.
- The deletion stays stored, and takes effect when the last dependency disappears. Retaining a wrapper can expose further dependencies.

**Vertical merges are decided once for the whole table.** The operator accepted this design on 2026-11-19.

- A continuation states the start it was made to continue: the start row and the start cell. The importer reads that from the file, and a row insertion records it for each new cell.
- The reader walks the shown rows from the top. A continuation is part of its merge only when the cell directly above is that merge's start, or a continuation already found to be part of that merge, and both hold the same shown columns.
- A break anywhere leaves every cell below it ordinary, however the break arose. The merge returns when the break closes.
- A cell never joins a merge it was not made to continue, even under a start of the same width.
- Nothing is reported, because no accepted action becomes ineffective.

**The order is fixed.** Rows are chosen first, from the bottom up: a deleted start, or a removed wrapper holding one, stays only while the next shown row below is part of its merge. Each answer depends only on rows below it. Then the merges are written from the top down. A cell that is cut off no longer holds its start in place.

**Commands use the same reading.** A row insertion is refused when a merge runs across its place and the copied row has no cell with that merge's columns. Removing a wrapper is refused while a cell below continues through it. Both checks see the author's own view, and count a continuation only when the merged cells above it reach a start. A mark that a wrapper keeps after a cut therefore refuses nothing and continues nothing. A break that arises concurrently, or returns through undo, is read by the rule above.

**The writer follows the decision.** An editable source cell that is cut off is written without its merge mark, on a row the reader otherwise keeps byte for byte. This is a reading, not a stored repair. A file that arrives with a continuation under an unrelated cell states no start for it, and that cell is left as it was found.

**One case is left as written.** A cut-off continuation inside a preserved wrapper cannot be rewritten, so its bytes still say it continues.

- Two authors can reach that state with three actions and an undo, which is past the enumeration's bound. The test `a_cut_off_continuation_inside_a_wrapper_is_left_as_written` shows one such history.
- Nothing is reported for it. The mark stays in the file, but no command treats it as a merge.
- How Word shows it is unverified, and holding the edit that cut it is not built. The operator keeps this open as a question of Word compatibility (2026-11-19): what Word does with such a cell decides whether the cut needs holding.

**A closed break can bring a start back.** The choice of rows depends only on the stored facts. So when a start row is deleted while a break keeps anything from depending on it, and the break then closes, the start shows again with a report. This also happens when one author did both. The operator accepted this on 2026-11-19. The prototype's editor says why in its summary line: a row is visible because a merged cell below still needs it. For a kept wrapper it names the content control instead.

**Choose a grid that can display the required rows.** A table keeps at least one row and column.

- For a last-row fallback, the latest deletion is tried first, then eligible rows in reverse document order, including their transitive merge starts.
- When required rows cannot fit the edited grid, the source grid is shown and the hold is reported. This includes visible opaque wrappers, and a single cell that would cover disconnected slots.
- The four reportable outcomes are last row, last column, required merge start and source grid.
- One data-only selection supplies row visibility, widths and cell placement to the writer. Nested tables are written before parents, and each table gets one choice of grid.
- The reader writes no repair.

#### History, projection and wire format

- **Undo uses Yrs's undo manager over this replica's writes.** Headless undo swaps the changed slot interval in each affected strand, and restores the mutable facts written by the command. The identity clock keeps advancing, and permanent facts survive undo. Relative positions carry carets through sync. Projected node IDs are local view addresses, not identities shared between replicas.
- **Table visibility is derived and never stored.** A state holds strands, facts and pending writes. The table shape read from the facts says which strands are shown, and every position is counted on a shape.
- **Projection shares unchanged node pages and strand readings.** Scanning strand metadata and updating absolute positions still cost time proportional to document size.
- **The browser editor and the headless editor share the table reader.** It is exercised by the bounded creation tests and by `tests/table_merges.rs`.
- **Shared states use the format `docx-engine.yrs/9`.** Version 9 seeds text leaves containing embedded XML markup as preserved items. Version 8 records source text leaves. Version 7 added the start cell of a vertical merge. A state in an older format is refused.

Remaining gaps include acceptance by Word, testing with a real input method and on mobile, memory measurement in the browser, and the features 00001 D3 lists as not built.

## The move

### D4 — The crate is the prototype's, unchanged

Proposed: awaiting operator approval.

The source is the prototype at revision `2210c69073059d82b4bba72b1349fbbed49169ca`. That is the revision the core and the fixtures are at, in 00001 D4.

| Here | Git tree hash |
| --- | --- |
| `crates/docx-engine-yrs` | `07ee745f5fa96a38d09cf988dad564aaaa508361` |

A git tree hash covers the bytes, name and mode of every file below a directory. The prototype's directory has this hash at that revision. To check this repository's side:

```bash
git rev-parse HEAD:crates/docx-engine-yrs
```

The replica and the core move together from now on, for two reasons:

- The replica calls the core's `editing`, `flow` and `state` modules, which change with it.
- Ten of its test files and one example include the core's test helpers by path, from `crates/docx-engine/tests/common`. Its tests also read the fixtures in `fixtures/`.

So a later sync copies both crates and the fixtures at one revision, and records it in both specs.

### D5 — Dependencies are locked at the prototype's versions

Proposed: awaiting operator approval.

- **The crate names its own two dependencies,** `yrs` and `sha2`. The root `Cargo.toml` does not change.
- **`yrs` is pinned to exactly `0.28.1`.** The crate's manifest gives the reason: the update encoding and the offset behaviour are what the prototype measured, so a newer version is a reason to measure again.
- **`Cargo.lock` is again derived from the prototype's lockfile.** It holds 77 packages, each at the prototype's version. The prototype's lockfile has one more, the WebAssembly bindings crate itself.
- **27 third-party packages are new here.** Each declares the MIT license, the Apache-2.0 license, or a choice of the two, as `cargo metadata` reports them.

The shared tasks need no change. A crate under `crates/` inherits them, and 00001 D6 already sets how they run.

## Answers to the prototype's questions

| Question | Answer | Decision |
| --- | --- | --- |
| "Should Yrs hold the live canonical editing state or synchronize a separate semantic model?" | Yrs holds it, for what is collaborative. The document is derived from it and the base | D3 |
| "How should structural edits be represented?" | The same semantic writes for headless and shared editing, including paragraph boundaries and stable table facts | 00001 D2, D3 |
| "How should concurrent table edits be resolved?" | Cell text merges as text. Each reader resolves stable row, column and cell facts and writes the selected grid | D1, D3 |
| "How do we maintain stable anchors for comments and revisions?" | A comment's ends are items in the sequence, and a revision is content. A caret is a Yrs sticky index | 00001 D3, D3 |
| "How do we reconstruct a consistent document from Yrs state?" | The base is opened once, and each paragraph is built from its flow | D3 |
| "How should agent edits participate in the same collaboration model?" | An agent is one more replica with its own client ID, using the same commands | D3 |

## Deferred

Each of these waits for the condition named.

- **Rules for storing a shared session.** The prototype has rules for what is stored and how updates are compacted. Nothing in this repository stores a session, so they wait for the first part that does.

The WebAssembly bindings moved in [00003](../00003-docx-engine-wasm/spec.md), and the browser editor in [00004](../00004-docx-engine-demo/spec.md). The rules for how an edit's cost is bounded while redrawing are 00004 D2.
- **Continuous integration.** Nothing runs the checks on a pull request. It waits for the operator to ask for it.
