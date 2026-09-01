# Frontend source

The frontend is a small React surface: `index.html` mounts the app, `src/App.tsx` owns import, IPC, the 2D canvas, and the Three.js render, and `styles.css` contains the single plain-CSS light theme.

The top bar has one import control for one or more Gerbers or a ZIP. Preview generation is automatic: importing layers, changing settings, or switching paste side schedules a debounced `preview_stencil` call. There is no manual rebuild button.

## IPC contract

All backend commands receive a single `request` argument. The shared request shape is:

```ts
{
  paste: { data, filename, isZip },
  edge: { data, filename, isZip },
  settings: {
    clearance, wallThickness, wallHeight, stencilThickness,
    shrink, nozzleDiameter,
  },
  pasteSide: "front" | "back"
}
```

`save_stencil_step` and `save_stencil_stl` also receive optional `excludedOpenings` indices inside that request. Both commands open the native file picker and return `saved` plus the chosen `path`. Response fields are camel-case.
`preview_stencil` also returns `model` geometry (`plate`, fused printable `openings`, un-fused `selectionOpenings`, `openingSources`, `innerWall`, `outerWall`, and `warnings`) for the previews. Point fields remain `x` and `y`. The default `nozzleDiameter` is `0.2`; the backend accepts values from `0.1` through `0.8` mm. Printable openings are automatically closed so no plate gap remains below the selected nozzle diameter.

The preview model is shared by both view modes. The 3D view extrudes the profiles in Three.js; the 2D view supports zoom, right-drag panning, and per-opening removal. Removed opening indices are sent only with export requests.

## Change guide

- Change markup in `index.html`; do not rebuild the page from a TypeScript template string.
- Add visual rules to `styles.css` without adding a utility-CSS pipeline or a second override theme.
- Keep request construction in `request` so preview and export cannot drift.
- Keep Three.js setup and disposal inside `ThreePreview`; do not move model generation into the browser.
