# DOCX engine demo: plan

## Steps

1. Copy the prototype's `web/` and `scripts/` to `apps/demo`, and its `.gitignore`, at the revision in D3, unchanged. Done.
2. Add the two links, and except `apps/demo/target` in the root `.gitignore` (D4). Done.
3. Pin Node.js and pnpm in `.prototools` (D5). Done.
4. Add `apps/*` to moon's projects and define the demo's tasks (D6). Done.
5. Write `apps/demo/README.md`, and describe the demo in the root README. Done.
6. Bring the earlier pipelines' specs and plans up to date where they said the editor had not moved. Done.

## Verification

Verified on macOS, by the command named:

| What | Command | Result |
| --- | --- | --- |
| The moved files are the source's | `git rev-parse HEAD:apps/demo/web HEAD:apps/demo/scripts HEAD:apps/demo/.gitignore` | Each hash equals the prototype's at the D3 revision. The prototype's were read from its repository at that revision |
| The crates and the fixtures are at the same revision | `git rev-parse` on the three crates and `fixtures` | All four hashes equal the ones in 00001 D4, 00002 D4 and 00003 D2 |
| Everything `moon check` runs | `moon check --all` | All 19 tasks pass: the crates' 12 and the demo's seven |
| The demo builds through the links | `moon run demo:build` | Wrote `web/pkg/` and `web/dist/`. The module is 3,739 KiB raw and 891 KiB with gzip |
| The demo's tests | `moon run demo:test` | 52 tests pass |
| Every fixture through the module | `moon run demo:fixtures` | 31 fixtures open, edit, export, validate, re-open and lay out |
| Shared editing in Node | `moon run demo:collab` | Passed, including an upload and a restart of the server |
| Editing in a browser | `moon run demo:browser`, in headless Chrome | 1,717 checks in 17 features, none failed, in 61 s. Each feature ran alone and in two tabs sharing a room |
| The `test` and `types` tasks can fail | A temporary failing test, and a temporary line in `web/wire.generated.ts` | Each task failed. Neither change is in the branch, and the `web` hash was checked again after they were removed |
| A script reads the links from the shell | `scripts/check-with-python-docx.sh`, with uv | Every fixture passed |
| The tool install works from nothing | `cargo install --locked wasm-bindgen-cli --version 0.2.130 --root` a scratch directory | Built and installed in 29 s. The installed `wasm-bindgen` reported 0.2.130 |
| A fresh clone works | `proto use`, then `moon check --all`, in a new clone of the branch | All 19 tasks pass in 71 s, with a clean tree afterwards. `apps/demo/target` pointed at nothing until Cargo built. `demo:tools` found `wasm-bindgen-cli` already installed on this machine |

Asserted, not verified here:

- D1 and D2 are the prototype's own statements about the editor. This pipeline ran the demo's checks. It did not check each statement against the code.
- No person edited in the demo. The browser check drove it in headless Chrome.
- Safari, Firefox and mobile browsers are untested.
- The comparison with Word and the editor's timing script were not run.
- Linux, Windows and a machine with none of the tools installed are untested.

## Next

Every part of the prototype's code has moved. The operator plans to pull the editor out of the demo as a library, in a later pull request.

While the prototype keeps changing, the copies here fall behind it. Bringing them up to date is one copy of the crates, the fixtures, `web/` and `scripts/` at a newer revision, with new hashes in 00001 D4, 00002 D4, 00003 D2 and D3 here.
