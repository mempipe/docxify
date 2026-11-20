# docxify

## Development

docxify is a monorepo. Its first library is the Rust crate `docxify-engine`, in `crates/docxify-engine`. [proto](https://moonrepo.dev/proto) installs the tools, and [moon](https://moonrepo.dev/moon) runs the tasks.

[Install proto](https://moonrepo.dev/docs/proto/install), then install the pinned tools:

```bash
proto use
```

Check everything. This checks formatting, lints and tests every crate:

```bash
moon check --all
```

Run one task in one project:

```bash
moon run docxify-engine:test
```

The `format` task only checks. To fix formatting, run `cargo fmt`.

Tool versions are in `.prototools` and `rust-toolchain.toml`. Tasks are in `.moon/tasks`. A new crate under `crates/` gets the same tasks with no moon configuration of its own.

Shared test fixtures go in `fixtures/` at the root. moon replays a cached result unless an input changed, and every crate's tasks take all of `crates/` and `fixtures/` as inputs. Test data kept anywhere else must be added to the inputs in `.moon/tasks/rust.yml`.
