# Editor package: plan

## Steps

1. Move the editor's 19 files from `apps/demo/web` to `packages/editor/src`, unchanged (D1, D2). Done.
2. Add the package's manifest, its compiler settings and its build (D3). Done.
3. Move the pnpm workspace file and the lockfile to the root, and split the dependencies between the two manifests (D4). Done.
4. Edit the demo's four programs to import the package, remove the import map, and rewrite the demo's build as a bundle (D5). Done.
5. Add `packages/*` to moon's projects, define the package's tasks, and reduce the demo's (D6). Done.
6. Write the package's README, and bring the demo's and the root's up to date. Done.

## Verification

Verified on macOS, by the command named:

| What | Command | Result |
| --- | --- | --- |
| The editor's files are unchanged | Each file's hash in `packages/editor/src`, against the prototype's `web/` file of the same name | All 19 equal. The prototype's were read from its repository at the revision in 00004 D3 |
| The scripts are unchanged | `git rev-parse HEAD:apps/demo/scripts` | Equals the hash in 00004 D3 |
| The crates and the fixtures are unchanged | `git rev-parse` on the three crates and `fixtures` | All four equal the hashes in 00001 D4, 00002 D4 and 00003 D2 |
| npm dependency versions | A comparison of every package in `pnpm-lock.yaml` with the prototype's lockfile | 67 kept, none at a different version. One added, `typescript` 5.9.3 |
| The sources pass a strict type check | `moon run editor:build`, which runs the TypeScript compiler | No errors, and no source was edited to get there |
| Everything `moon check` runs | `moon check --all` | All 21 tasks pass: the crates' 12, the package's five and the demo's four |
| The same tests run | `moon run editor:test demo:test` | 47 and 5 pass. Before the move, `demo:test` ran the same 52 |
| Every fixture through the package | `moon run demo:fixtures` | 31 fixtures pass |
| Shared editing in Node, through the package | `moon run demo:collab` | Passed, including an upload and a restart of the server |
| Editing in a browser, through the bundle | `moon run demo:browser`, in headless Chrome | 1,717 checks in 17 features, none failed, in 60 s. The same counts as before the move, with `scripts/` unchanged |
| The server serves what the page asks for | `curl` on the demo's server | 200 for the page, the bundle, `dist/editor.js`, the WebAssembly file, a font stylesheet and a font file through pnpm's links, and a fixture |
| A type error fails the build | A temporary line with a type error in `src/measure.ts`, then `moon run editor:build` | The task failed with the compiler's error. The line is not in the branch |
| The package's declarations work for a user | The TypeScript compiler on `apps/demo/web/browser-demo.ts`, strict, against the package | No errors. This was run once by hand and is not a task |
| A fresh clone works | `proto use`, `moon check --all`, then `moon run demo:browser`, in a new clone of the branch | All 21 tasks pass in 74 s, with a clean tree afterwards. The browser check passed there too |

Asserted, not verified here:

- No person edited in the demo. The browser check drove it in headless Chrome.
- No program outside this repository has used the package. Only the demo has.
- Safari, Firefox and mobile browsers are untested.
- Linux, Windows and a machine with none of the tools installed are untested.

## Next

Nothing is planned. The spec's deferred list names what would come with a second user of the package or with publishing it.
