# DOCX engine replica: plan

## Steps

1. Copy `crates/docx-engine-yrs` from the prototype at the revision in D4, unchanged. Done.
2. Derive `Cargo.lock` from the prototype's lockfile (D5). Done.
3. Describe the crate in the README. Done.
4. Bring 00001's spec and plan up to date where they said the replica had not moved. Done.

## Verification

Verified on macOS, by the command named:

| What | Command | Result |
| --- | --- | --- |
| The moved directory is the source's | `git rev-parse HEAD:crates/docx-engine-yrs` | The hash equals the prototype's at the D4 revision. The prototype's was read from its repository at that revision |
| No file differs | `diff -r` between the prototype's directory and this repository's | No output |
| The core and the fixtures are at the same revision | `git rev-parse HEAD:crates/docx-engine HEAD:fixtures` | Both hashes equal the ones in 00001 D4 |
| Dependency versions | A comparison of every package in `Cargo.lock` with the prototype's lockfile | 77 here, none at a different version. The prototype has one more, its WebAssembly bindings crate |
| Dependency licenses | `cargo metadata`, read for the 27 new third-party packages | Each declares MIT, Apache-2.0, or a choice of the two |
| Format, lint and tests | `moon check --all` | All six tasks pass. In release, 258 tests pass in the core and 175 in the replica |
| The replica's `test` task can fail | A temporary failing test file in the replica, then `moon run docx-engine-yrs:test` | The task failed. The file is not in the branch, and the crate's hash was checked again after it was removed |
| Two replicas converge | `cargo run --release -p docx-engine-yrs --example collab` on `fixtures/generated/simple-paragraphs.docx` | Printed that they converged, and wrote a DOCX. The core's `inspect` example read both people's text from it |
| A fresh clone works | `proto use`, then `moon check --all`, in a new clone of the branch | Pass, with a clean tree afterwards |

The replica's 175 tests ran in 13 s in release with the binaries already built. That was timed once, on a machine doing other work.

Asserted, not verified here:

- D1 to D3 are the prototype's own statements about shared editing. This pipeline ran the crate's tests. It did not check each statement against the code.
- The run counts in D1, such as 16,245 and 68,921, are the prototype's. They were not counted here.
- No merged document was opened in Word or LibreOffice.
- Linux, Windows and a machine with none of the tools installed are untested.
- The cost of editing was not measured. The crate's `edit_cost` example measures it.

## Next

The WebAssembly bindings moved in pipeline 00003. The browser editor moves next, in its own pull request.

While the prototype keeps changing, the crates here fall behind it. Bringing them up to date is one copy of every crate and the fixtures at a newer revision, with new hashes in D4 here, in 00001 D4 and in 00003 D2.
