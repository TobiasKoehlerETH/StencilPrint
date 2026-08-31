# Architecture

StencilPrint is a local Tauri application. The browser layer owns file selection and display; Rust owns parsing, geometry, and export. Gerber contents never need to leave the machine.

## Data flow

1. `index.html` provides the static interface and `src/main.ts` wires its controls.
2. The frontend reads individual Gerbers as text or a ZIP as base64, auto-assigns paste/outline filenames, and builds one nested request containing `paste`, `edge`, and `settings`; 2D pad edits are carried as `excludedOpenings` during export.
3. Tauri dispatches `preview_stencil`, `save_stencil_step`, `export_stencil_step`, or `export_stencil_stl` in `src-tauri/src/lib.rs`.
4. `gerber.rs` selects archive members and parses RS-274X primitives with `gerber_parser`, falling back to the local macro resolver for legacy macro-heavy files.
5. `geometry.rs` traces the board outline, applies `geo` offsets/unions, and derives paste-opening polygons.
6. Preview returns inline SVG; STEP export passes the same geometry to `step.rs`, which builds watertight B-Reps and delegates AP203 serialization to `brepkit`; STL export triangulates the same profiles in `stl.rs`.
7. Preview also returns the shared polygon model so the browser can render the printable plate and wall in Three.js.

## Boundaries

- `src/main.ts`: UI state, file I/O, IPC, downloads, and update notices.
- `src-tauri/src/lib.rs`: request validation and Tauri command orchestration only.
- `src-tauri/src/gerber.rs`: Gerber/ZIP input, off-the-shelf RS-274X parsing, compatibility macro resolution, and parsed layer statistics.
- `src-tauri/src/geometry.rs`: two-dimensional geometry and preview SVG.
- `src-tauri/src/step.rs`: Planar profile cleanup, B-Rep extrusion, and STEP serialization through `brepkit`.
- `src-tauri/src/stl.rs`: Fused printable-mesh triangulation and STL serialization.

Both preview and export must use `StencilGeometry::from_layers`; keeping one geometry path prevents the preview from disagreeing with the downloaded model.

## Deliberate limits

Standard RS-274X parsing and the common aperture/region path are delegated to `gerber_parser`; circular interpolation is still reported as a line-segment approximation, and unsupported macro exposure/polarity combinations remain compatibility-parser limits. The STEP path uses `brepkit` for watertight extrusion and AP203 output. Keep these dependencies behind the existing command boundary instead of expanding `lib.rs`.
