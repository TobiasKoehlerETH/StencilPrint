# StencilPrint

StencilPrint is a Tauri desktop application for turning a solder-paste Gerber and a PCB `Edge.Cuts` Gerber into a board-aware stencil model.

For code navigation, start with [the agent guide](AGENTS.md) and [architecture overview](ARCHITECTURE.md).

## Getting started

Install the frontend dependencies with Node.js, then start the Tauri desktop app:

```sh
npm install
npm run dev
```

For a frontend-only development server, use `npm run dev:frontend`. A production frontend build is created with `npm run build`; `npm run tauri:build` creates the desktop bundle.

## Workflow

- Load a top paste Gerber (`.GTP`, `.GBR`, etc.) or a ZIP archive containing one.
- Load the board outline Gerber from `Edge.Cuts`, or a ZIP archive containing the board files.
- Use the single import control to select one or more Gerbers or a ZIP archive.
- Switch between `F.Paste` and `B.Paste`; ZIP imports retry the alternate paste layer if the preferred member has no drawable geometry.
- Preview the paste openings and registration wall.
- Switch to the 2D view and click individual pads to remove or restore them.
- Tune board clearance, wall thickness, wall height, stencil thickness, opening shrink, and nozzle diameter.
- Enable automatic merging of close pads and filling of thin grids so the generated openings remain printable with the selected nozzle.
- Use the export menu to choose STL for slicing or STEP for CAD; both formats open a native save dialog.

The preview rebuilds automatically after importing layers or changing settings. There is no separate rebuild action.

The registration wall follows the closed Edge.Cuts contour and is printed downward from the stencil underside. Its inside boundary is expanded by the configured clearance, and its outside boundary is expanded again by the wall thickness. This supports non-rectangular board outlines; curved Gerber geometry is represented by sampled polygonal geometry before it is extruded into the printable model. Paste apertures are unioned so overlapping or duplicate pads do not create invalid nested holes. Openings are compensated for nozzle size, clipped to the board-clearance boundary, and back-side paste is mirrored around the board centre before export; automatic decisions are shown as engine notes in the preview.

ZIP archives may contain nested folders. Select a Gerber-folder ZIP once and it fills both empty layer inputs; choosing a standalone Gerber afterward overrides that layer. The app scores archive members by their names and automatically prefers top/front paste files for the paste input and `Edge.Cuts`, outline, profile, `.GKO`, or `.GM1` files for the edge input.

The nozzle compensation default is `0.2 mm` and the accepted range is `0.1–0.8 mm`. The continuous registration wall is printed downward from the stencil underside using the configured wall height.

## Project guides

- [Architecture and data flow](ARCHITECTURE.md)
- [Frontend source and IPC contract](src/README.md)
- [Tauri backend](src-tauri/README.md)
- [Rust source map](src-tauri/src/README.md)

## Updates and releases

Production builds check the latest GitHub Release at startup and automatically install a newer signed version when one is available. Pushing a `v*` tag runs the Windows release workflow, which publishes the installer and updater metadata. Keep the updater signing key in GitHub Actions secrets; it must never be committed to the repository.
