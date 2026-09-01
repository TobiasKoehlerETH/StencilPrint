# Tauri backend

This directory contains the native Rust application, Tauri configuration, icons, and capability configuration.

## Useful commands

From this directory:

```sh
cargo test
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

From the repository root, `npm run tauri:dev` starts the desktop app and `npm run tauri:build` creates a production bundle.

The full Rust test suite expects `../gerber-sample/prod_main.zip` relative to this directory. The synthetic geometry, parser, STEP, and STL tests do not require that archive; provide it when running the two sample-ZIP integration tests.

## What to edit

- Application behavior: `src/` (see [`src/README.md`](src/README.md)).
- Window, bundle, or security configuration: `tauri.conf.json`.
- Rust dependencies: `Cargo.toml`, followed by Cargo-managed lockfile updates.
- Application icon source: `icons/icon.svg`.

Do not hand-edit `gen/schemas/`, `target/`, or `Cargo.lock`; update dependencies through Cargo.
