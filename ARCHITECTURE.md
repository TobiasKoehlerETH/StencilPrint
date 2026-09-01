# Architecture

StencilPrint is a local Tauri application. The browser layer owns file selection and display; Rust owns parsing, geometry, and export. Gerber contents never need to leave the machine.

## Data flow

1. `index.html` provides the static interface and `src/main.tsx` mounts the React application; `src/styles.css` supplies the plain CSS theme.
2. The frontend reads selected Gerbers as text or a ZIP as base64, auto-assigns paste/outline filenames, and builds one nested request containing `paste`, `edge`, `settings`, and `pasteSide`; 2D pad edits are carried as `excludedOpenings` during export.
3. Tauri dispatches `preview_stencil`, `save_stencil_step`, or `save_stencil_stl` in `src-tauri/src/lib.rs`.
4. `gerber.rs` selects archive members and parses RS-274X primitives with `gerber_parser`, falling back to the local macro resolver for legacy macro-heavy files.
5. `geometry.rs` traces the board outline, applies `geo` offsets/unions, mirrors back paste around the board centre, compensates openings for nozzle size, clips paste to the clearance boundary, and derives source-pad selection profiles plus fused printable polygons and registration wall profiles.
6. Preview returns the shared polygon model; STEP export passes the same geometry to `step.rs`, which builds watertight B-RePs for the plate and registration wall and delegates AP203 serialization to `brepkit`; STL export triangulates the same profiles in `stl.rs`.
7. Preview also returns the shared polygon model so the browser can render the printable plate and wall in Three.js. The browser schedules preview generation automatically when the request changes.

## Boundaries

- `src/main.tsx`: React bootstrap.
- `src/App.tsx`: UI state, file I/O, IPC, preview interactions, and downloads.
- `src-tauri/src/lib.rs`: request validation and Tauri command orchestration only.
- `src-tauri/src/gerber.rs`: Gerber/ZIP input, off-the-shelf RS-274X parsing, compatibility macro resolution, and parsed layer statistics.
- `src-tauri/src/geometry.rs`: two-dimensional geometry and preview profiles.
- `src-tauri/src/step.rs`: Planar profile cleanup, B-Rep extrusion, and STEP serialization through `brepkit`.
- `src-tauri/src/stl.rs`: Fused printable-mesh triangulation and STL serialization.
- `vite.config.ts`: Vite and React development/build configuration; frontend styling is handled directly by `src/styles.css`.

Both preview and export must use `StencilGeometry::from_layers`; keeping one geometry path prevents the preview from disagreeing with the downloaded model. The geometry result also carries human-readable notes for automatic compensation, clipping, mirroring, and fused duplicate/overlapping shapes.

## IPC contract

All three Tauri commands accept a request with `paste`, `edge`, `settings`, and `pasteSide`. Export requests additionally carry optional `excludedOpenings` indices. Settings use camel-case names in the browser and include clearance, wall, height, stencil thickness, shrink, nozzle compensation, and pad/grid handling.

`preview_stencil` returns layer statistics plus `model` profiles: `plate`, fused `openings`, un-fused `selectionOpenings`, `openingSources`, `innerWall`, `outerWall`, and `warnings`. Both export commands open a native file picker and return `saved` plus the chosen `path`; the generated file content never passes through the browser.

The default nozzle diameter is `0.2 mm`; the registration wall always uses the configured wall height.

## Deliberate limits

Standard RS-274X parsing and the common aperture/region path are delegated to `gerber_parser`; circular interpolation is still reported as a line-segment approximation, and unsupported macro exposure/polarity combinations remain compatibility-parser limits. The STEP path uses `brepkit` for watertight extrusion and AP203 output. Keep these dependencies behind the existing command boundary instead of expanding `lib.rs`.
