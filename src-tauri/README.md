# Tauri backend

This directory contains the native Rust application, Tauri configuration, icons, and generated capability schemas.

## Useful commands

From this directory:

```sh
cargo test
cargo fmt --all -- --check
cargo clippy --all-targets --all-features
```

From the repository root, `npm run tauri:dev` starts the desktop app and `npm run tauri:build` creates a production bundle.

## What to edit

- Application behavior: `src/` (see [`src/README.md`](src/README.md)).
- Window, bundle, or security configuration: `tauri.conf.json`.
- Rust dependencies: `Cargo.toml`, followed by Cargo-managed lockfile updates.
- Application icon source: `icons/icon.svg`.

Do not hand-edit `gen/schemas/` or `target/`.
