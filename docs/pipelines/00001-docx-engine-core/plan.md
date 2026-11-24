# DOCX engine core: plan

## Steps

1. Copy `crates/docx-engine` and `fixtures/` from the prototype at the revision in D4, unchanged. Done.
2. Remove the placeholder crate `docxify-engine`. Done.
3. Add the crate's workspace dependencies to the root `Cargo.toml`, and derive `Cargo.lock` from the prototype's (D5). Done.
4. Set the shared `test` and `lint` tasks for the engine (D6). Done.
5. Describe the crate in the README. Done.

## Verification

Verified on macOS, by the command named:

| What | Command | Result |
| --- | --- | --- |
| The moved directories are the source's | `git rev-parse HEAD:crates/docx-engine HEAD:fixtures` | Both hashes equal the prototype's at the D4 revision. The prototype's were read from its repository at that revision |
| No file differs | `diff -r` between the prototype's two directories and this repository's | No output |
| Dependency versions | A comparison of every package in `Cargo.lock` with the prototype's lockfile | 49 kept, none at a different version |
| Format, lint and tests | `moon check --all` | All three tasks pass. 258 tests pass in release |
| The tasks can fail | A temporary test file with a failing test, an unused variable and bad formatting | `test`, `lint` and `format` each failed. The file is not in the branch, and the crate's hash was checked again after it was removed |
| The examples run | `cargo run -p docx-engine --example inspect` and `--example edit` on two fixtures | Printed the inspection, and wrote an edited DOCX |
| A fresh clone works | `proto use`, then `moon check --all`, in a new clone of the branch | Pass, with a clean tree afterwards |

Asserted, not verified here:

- D1 to D3 are the prototype's own statements about the engine. This pipeline ran the crate's tests. It did not check each statement against the code.
- The prototype's run over a corpus of 638 real documents was not repeated.
- No export was opened in Word or LibreOffice.
- Linux, Windows and a machine with none of the tools installed are untested.

## Next

The replica for shared editing, the WebAssembly bindings and the browser editor have not moved. Each is a later piece of work, in that order, because each depends on the one before.

While the prototype keeps changing, the core here falls behind it. Bringing it up to date is the same copy at a newer revision, with new hashes in D4.
