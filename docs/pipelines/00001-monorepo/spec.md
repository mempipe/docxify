# Monorepo for the Rust and TypeScript libraries: decisions

## D1 — Each tool version is written in one file

Proposed: awaiting operator approval.

- `.prototools` holds the moon, Node.js and pnpm versions.
- `rust-toolchain.toml` holds the Rust version and its components.

rustup reads only `rust-toolchain.toml`, so a Rust version in `.prototools` would not reach a plain `cargo` command. proto and moon both read `rust-toolchain.toml`, so `proto use` installs Rust from it.

moon takes the Node.js and pnpm versions from `.prototools` (`versionFromPrototools`). It does not copy the pnpm version into `package.json` (`syncPackageManagerField: false`), because that would be a second copy.

## D2 — Tasks are defined once per language, and a project has no moon file

Proposed: awaiting operator approval.

`.moon/tasks/rust.yml` and `.moon/tasks/typescript.yml` define every task. A project inherits the Rust tasks when it has a `Cargo.toml`, and the TypeScript tasks when it has a `tsconfig.json`. A new crate or package under `crates/` or `packages/` is checked with no moon configuration of its own.

A project that needs a different task adds a `moon.yml` then.

## D3 — A project's moon ID is its directory name

Proposed: awaiting operator approval.

The Rust library is `crates/docxify`, with the crate name `docxify`. The TypeScript library is `packages/docxify-js`, with the package name `@mempipe/docxify`.

moon IDs must be unique across the workspace, so the two libraries cannot both sit in a directory named `docxify`. moon can override an ID in a project's `moon.yml`. That was rejected: the ID would no longer be readable from the path, and D2 keeps projects free of moon files.

Both libraries are empty. Renaming either is a directory move.

## D4 — TypeScript is checked and built by `tsc`, and tested by Node.js directly

Proposed: awaiting operator approval.

- `tsc --noEmit` type-checks the sources and the tests.
- `tsc --project tsconfig.build.json` writes JavaScript and declarations to `dist`, without the tests.
- `node --test` runs `*.test.ts` files as they are. Node.js removes the types itself, so tests need no build step and no test framework.
- oxlint and oxfmt lint and format, with their default rules.

`erasableSyntaxOnly` is on because Node.js only removes types. It refuses syntax that needs code generated, such as enums. Relative imports name the `.ts` file, and `rewriteRelativeImportExtensions` writes `.js` in the build.

What would overturn this: a library that needs bundling, or tests that must run in a browser.

## Deferred

Each of these waits for the condition named.

- **Continuous integration.** Nothing runs the checks on a pull request. `moon ci` is the command a workflow would run. It waits for the operator to ask for it.
- **Publishing.** Both libraries are marked unpublishable (`publish = false`, `"private": true`). They stay so until there is a release process that fits the license.
- **A WebAssembly build of the Rust library.** It waits for TypeScript code that calls the Rust library.
- **TypeScript project references.** moon's TypeScript toolchain keeps them in step between packages. It waits for a second TypeScript package that depends on the first.
- **Git hooks.** They wait for a check that is too slow or too easy to forget to run by hand.
