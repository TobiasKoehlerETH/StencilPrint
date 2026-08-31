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

## Updates and releases

Production builds check the latest GitHub Release at startup and automatically install a newer signed version when one is available. Pushing a `v*` tag runs the Windows release workflow, which publishes the installer and updater metadata. Keep the updater signing key in GitHub Actions secrets; it must never be committed to the repository.
