# StencilPrint Rust source map

| File | Owns | Does not own |
| --- | --- | --- |
| `main.rs` | Native executable entry point | Application logic |
| `lib.rs` | IPC types, validation, response serialization, and command orchestration | Parsing or geometry algorithms |
| `gerber.rs` | ZIP member selection, standard RS-274X parsing via `gerber_parser`, compatibility macro parsing, layer statistics | Stencil construction |
| `geometry.rs` | Board tracing, polygon orientation helpers, nozzle-aware opening compensation, clipping, back-side mirroring, and wall profiles | File decoding or STEP syntax |
| `step.rs` | Clean planar B-Rep plate/wall construction, extrusion, and AP203 serialization via `brepkit` | Gerber interpretation or geometry generation |
| `stl.rs` | Fused printable-mesh plate/wall triangulation and STL serialization, reusing shared profile helpers | Gerber interpretation or geometry generation |

Tests live beside the private functions they exercise. Use small synthetic Gerbers and polygons so failures identify the broken layer directly.

When adding a setting, update `StencilSettings`, the TypeScript `StencilSettings` interface, the relevant geometry/export call, and this guide if ownership changes. The current settings are `clearance`, `wallThickness`, `wallHeight`, `stencilThickness`, `shrink`, `nozzleDiameter`, `enableSlotify`, and `dropUnprintableGrids`. Opening compensation follows the Stenchill defaults: shrink is applied before a configurable nozzle compensation pass, close pads are merged with a nozzle-sized morphological close, and thin grids can be filled with the same pass.

The browser default nozzle diameter is `0.2 mm`; validation accepts `0.1–0.8 mm`. `wallHeight` controls the continuous registration wall. `geometry.rs` owns shared signed-area and point-distance helpers used by both exporters so profile cleanup stays consistent.
