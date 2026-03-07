# docup

Mark Down? Doc Up!

Early Rust demo for the `docup` CLI.

## Quick start

```bash
cargo check
cargo run -- --help
```

## Commands

```bash
docup login   # interactive
docup init    # interactive
docup sync
docup config show
docup config get auth.server_url
docup config set params.lang go
docup version
```

## Current status

- `init`, `login`, `sync`, `config`, `version` are available.
- `init` and `login` are interactive commands.
- `sync` requires login first, and currently runs in demo mode (plan output only).
