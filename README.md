# StencilPrint

StencilPrint is a Tauri desktop application for turning a solder-paste Gerber and a PCB `Edge.Cuts` Gerber into a board-aware stencil model.

For code navigation, start with [the agent guide](AGENTS.md) and [architecture overview](ARCHITECTURE.md).

## Current workflow

- Load a top paste Gerber (`.GTP`, `.GBR`, etc.) or a ZIP archive containing one.
- Load the board outline Gerber from `Edge.Cuts`, or a ZIP archive containing the board files.
- Switch between `F.Paste` and `B.Paste`; ZIP imports retry the alternate paste layer if the preferred member has no drawable geometry.
- Preview the paste openings and the registration wall.
- Switch to the 2D view and click individual pads to remove or restore them.
- Tune board clearance, wall thickness, wall height, and stencil thickness.
- Export an STL file locally for direct slicing, or a STEP file for CAD.

The registration wall follows the closed Edge.Cuts contour and is printed downward from the stencil underside. Its inside boundary is expanded by the configured clearance, and its outside boundary is expanded again by the wall thickness. This supports non-rectangular board outlines; curved Gerber geometry is represented by sampled polygonal geometry before it is extruded into the printable model. Paste apertures are unioned so overlapping pads do not create invalid nested holes.

ZIP archives may contain nested folders. Select a Gerber-folder ZIP once and it fills both empty layer inputs; choosing a standalone Gerber afterward overrides that layer. The app scores archive members by their names and automatically prefers top/front paste files for the paste input and `Edge.Cuts`, outline, profile, `.GKO`, or `.GM1` files for the edge input.

## Run

```sh
npm install
npm run tauri:dev
```

For a production build:

```sh
npm run tauri:build
```

For a quick verification pass:

```sh
npm run build
cd src-tauri && cargo test
```

The app provides both a 2D editor and an interactive Three.js 3D preview. In the 2D view, left-click pads to remove or restore them, right-drag to pan, and use the mouse wheel to zoom.

## Geometry roadmap

The backend uses the MIT/Apache-licensed `gerber_parser` crate for standard RS-274X files, retains a compatibility resolver for macro-heavy legacy files, and uses `geo` for profile buffering and unioning. STEP export delegates B-Rep construction and AP203 serialization to `brepkit`, producing watertight plate and registration-wall solids that can be re-imported by brepkit's STEP reader. STL export triangulates the same profiles with `earcutr` and emits one fused printable envelope. `brepkit` 3.x is AGPL-3.0-only with a separate commercial license, so a distributed proprietary build needs the appropriate upstream license.

Useful prior art includes [`gerber-stencil-3d`](https://github.com/ilkerdizbay/gerber-stencil-3d), whose registration-wall/frame behavior is a close match for this application.
