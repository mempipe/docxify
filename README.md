# docxify

## Development

docxify is a monorepo of a Rust library and a TypeScript library. [proto](https://moonrepo.dev/proto) installs the tools, and [moon](https://moonrepo.dev/moon) runs the tasks.

| Path | Library |
| --- | --- |
| `crates/docxify` | The Rust crate `docxify` |
| `packages/docxify-js` | The TypeScript package `@mempipe/docxify` |

[Install proto](https://moonrepo.dev/docs/proto/install), then install the pinned tools:

```bash
proto use
```

Check everything. This checks formatting, lints, type-checks, tests and builds both libraries:

```bash
moon check --all
```

Run one task in one project:

```bash
moon run docxify:test
moon run docxify-js:build
```

The `format` tasks only check. To fix formatting, run `cargo fmt` in a crate or `pnpm exec oxfmt` in a package.

Tool versions are in `.prototools` and `rust-toolchain.toml`. Tasks are in `.moon/tasks`.
