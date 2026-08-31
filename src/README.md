# Frontend source

The frontend is a small React surface: `index.html` mounts the app, `src/App.tsx` owns import, IPC, the 2D canvas, and the Three.js render, and `styles.css` contains the single light theme.

## IPC contract

All backend commands receive a single `request` argument. The shared request shape is:

```ts
{
  paste: { data, filename, isZip },
  edge: { data, filename, isZip },
  settings: { clearance, wallThickness, wallHeight, stencilThickness }
}
```

`export_stencil_step` and `export_stencil_stl` also receive `mirror` and optional `excludedOpenings` indices inside that request. STEP responses contain `filename`, `step`, and `summary`; STL responses contain `filename`, `stl`, and `summary`. Response fields are camel-case.
`preview_stencil` also returns `model` geometry (`plate`, `openings`, `innerWall`, and `outerWall`) for the Three.js preview. Point fields remain `x` and `y`.

## Change guide

- Change markup in `index.html`; do not rebuild the page from a TypeScript template string.
- Add visual rules to `styles.css` without creating a second override theme.
- Keep request construction in `request` so preview and export cannot drift.
- Keep Three.js setup and disposal inside `ThreePreview`; do not move model generation into the browser.
