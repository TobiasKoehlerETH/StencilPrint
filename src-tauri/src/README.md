# StencilPrint Rust source map

| File | Owns | Does not own |
| --- | --- | --- |
| `main.rs` | Native executable entry point | Application logic |
| `lib.rs` | IPC types, validation, and command orchestration | Parsing or geometry algorithms |
| `gerber.rs` | ZIP member selection, standard RS-274X parsing via `gerber_parser`, compatibility macro parsing, layer statistics | Stencil construction |
| `geometry.rs` | Board tracing, offsets, openings, and preview profiles | File decoding or STEP syntax |
| `step.rs` | Clean planar B-Rep profile construction, extrusion, and AP203 serialization via `brepkit` | Gerber interpretation |
| `stl.rs` | Fused printable-mesh triangulation and STL serialization | Gerber interpretation |

Tests live beside the private functions they exercise. Use small synthetic Gerbers and polygons so failures identify the broken layer directly.

When adding a setting, update `StencilSettings`, the TypeScript `StencilSettings` interface, the relevant geometry/export call, and this guide if ownership changes.
