# Agent guide

This file applies to the whole repository. Read it before changing code.

## Start here

| Area | Purpose | Guide |
| --- | --- | --- |
| `index.html` + `src/` | Desktop UI and Tauri IPC client | [`src/README.md`](src/README.md) |
| `src-tauri/` | Rust backend, packaging, and native entry point | [`src-tauri/README.md`](src-tauri/README.md) |
| `src-tauri/src/` | Gerber parsing, geometry, and STEP generation | [`src-tauri/src/README.md`](src-tauri/src/README.md) |
| Whole application | Data flow and design boundaries | [`ARCHITECTURE.md`](ARCHITECTURE.md) |

## Working rules

- Keep browser-to-Rust request and response shapes synchronized. Rust uses Serde camel-case output; TypeScript interfaces use the same names.
- Keep responsibilities in their existing modules. Parsing belongs in `gerber.rs`, polygon work in `geometry.rs`, and serialization in `step.rs`.
- Prefer a small helper over repeated payload construction or repeated parsing, but do not add an abstraction used only once.
- Do not edit `dist/`, `node_modules/`, `src-tauri/target/`, generated Tauri schemas, or lockfiles by hand.
- Add focused tests beside Rust logic. Avoid snapshots for generated SVG or STEP output when a semantic assertion is clearer.
- Update the nearest Markdown guide when a file moves or a boundary changes.

## Verification

Run both checks after a functional change:

```sh
npm run build
cd src-tauri && cargo test
```

Run `cargo fmt --all -- --check` before handing off Rust changes.
