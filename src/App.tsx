import { invoke } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import { Box, ChevronDown, Download, LoaderCircle, Settings2, Upload, X } from "lucide-react";
import { Component, useEffect, useMemo, useRef, useState } from "react";
import type { BufferGeometry, ExtrudeGeometry, Object3D } from "three";

type LayerKind = "paste" | "edge";
type PasteSide = "front" | "back";
type BusyAction = "preview" | "step" | "stl";
type ExportAction = Exclude<BusyAction, "preview">;
type ExportCommand = "save_stencil_step" | "save_stencil_stl";

interface LayerSource {
  data: string;
  filename: string;
  isZip: boolean;
}

interface StencilSettings {
  clearance: number;
  wallThickness: number;
  wallHeight: number;
  stencilThickness: number;
  shrink: number;
  nozzleDiameter: number;
  enableSlotify: boolean;
  dropUnprintableGrids: boolean;
}

interface StencilRequest {
  paste: LayerSource;
  edge: LayerSource;
  settings: StencilSettings;
  pasteSide: PasteSide;
}

interface LayerStats {
  filename: string;
  units: string;
  primitives: number;
  flashes: number;
  strokes: number;
  regions: number;
  widthMm: number;
  heightMm: number;
  warning?: string;
}

interface Point {
  x: number;
  y: number;
}

interface ViewBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface ModelBounds {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
  size: number;
}

interface ModelGeometry {
  plate: Point[];
  openings: Point[][];
  selectionOpenings: Point[][];
  openingSources: number[][];
  innerWall: Point[];
  outerWall: Point[];
  warnings: string[];
}

interface PreviewResult {
  paste: LayerStats;
  edge: LayerStats;
  model: ModelGeometry;
}

interface SaveResult {
  saved: boolean;
  path?: string;
}

const DEFAULT_SETTINGS: StencilSettings = {
  clearance: 0.3,
  wallThickness: 1,
  wallHeight: 1,
  stencilThickness: 0.4,
  shrink: 0,
  nozzleDiameter: 0.2,
  enableSlotify: true,
  dropUnprintableGrids: true,
};

const GERBER_EXTENSIONS = [".gbr", ".ger", ".gtp", ".gbp", ".gko", ".gm1", ".pho", ".gbrjob", ".edge_cuts"];

function classNames(...names: Array<string | false | undefined>) {
  return names.filter(Boolean).join(" ");
}

function boundsForModel(model: ModelGeometry): ModelBounds {
  const points = [model.outerWall, ...model.openings].flat();
  const x = points.map((point) => point.x);
  const y = points.map((point) => point.y);
  const minX = Math.min(...x);
  const maxX = Math.max(...x);
  const minY = Math.min(...y);
  const maxY = Math.max(...y);
  return { minX, maxX, minY, maxY, size: Math.max(maxX - minX, maxY - minY, 1) };
}

function NumberField({
  label,
  value,
  min,
  step,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  step: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="compact-field">
      <span>{label}</span>
      <input
        type="number"
        inputMode="decimal"
        value={value}
        min={min}
        step={step}
        onChange={(event) => {
          const next = event.currentTarget.valueAsNumber;
          if (Number.isFinite(next) && next >= min) onChange(next);
        }}
      />
      <em>mm</em>
    </label>
  );
}

function ToggleField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div className="switch-row">
      <span>{label}</span>
      <button
        className={classNames("switch-control", value && "is-active")}
        type="button"
        role="switch"
        aria-checked={value}
        aria-label={label}
        onClick={() => onChange(!value)}
      >
        <span />
      </button>
    </div>
  );
}

function UniversalImport({ onFiles }: { onFiles: (files: File[]) => void }) {
  const takeFiles = (files: FileList | null) => {
    onFiles(Array.from(files ?? []));
  };

  return (
    <label className="command-icon" title="Import Gerbers or ZIP" aria-label="Import Gerbers or ZIP">
      <Upload />
      <input type="file" multiple accept={`${GERBER_EXTENSIONS.join(",")},.zip`} onChange={(event) => { takeFiles(event.currentTarget.files); event.currentTarget.value = ""; }} />
    </label>
  );
}

function ThreePreview({ model, settings }: { model: ModelGeometry; settings: StencilSettings }) {
  const mountRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;
    const previewMount = mount;

    let cancelled = false;
    let disposePreview: (() => void) | undefined;

    async function loadPreview() {
      try {
        const [THREE, { OrbitControls }] = await Promise.all([
          import("three"),
          import("three/examples/jsm/controls/OrbitControls.js"),
        ]);
        if (cancelled) return;

    const scene = new THREE.Scene();
    scene.background = new THREE.Color("#ffffff");
    const camera = new THREE.PerspectiveCamera(32, 1, 0.01, 10000);
    camera.up.set(0, 0, 1);
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    previewMount.replaceChildren(renderer.domElement);

    const modelGroup = new THREE.Group();
    scene.add(modelGroup);

    const { minX, maxX, minY, maxY, size } = boundsForModel(model);
    const centerX = (minX + maxX) / 2;
    const centerY = (minY + maxY) / 2;

    const pathFor = (points: Point[], reverse = false) => {
      const path = new THREE.Shape();
      const ordered = reverse ? [...points].reverse() : points;
      ordered.forEach((point, index) => {
        if (index === 0) path.moveTo(point.x, point.y);
        else path.lineTo(point.x, point.y);
      });
      path.closePath();
      return path;
    };

    const plateShape = pathFor(model.plate);
    model.openings.forEach((opening) => plateShape.holes.push(pathFor(opening, true)));
    const plateGeometry = new THREE.ExtrudeGeometry(plateShape, {
      depth: settings.stencilThickness,
      bevelEnabled: false,
      curveSegments: 1,
    });
    const plateMaterial = new THREE.MeshStandardMaterial({ color: 0x7bc8ee, roughness: 0.3, metalness: 0, side: THREE.DoubleSide });
    const plate = new THREE.Mesh(plateGeometry, plateMaterial);
    const edgeMaterial = new THREE.LineBasicMaterial({ color: 0x164e78 });
    const outlineGeometries: BufferGeometry[] = [];
    const addProfileLines = (parent: Object3D, profiles: Point[][], z: number) => {
      profiles.forEach((points) => {
        const lineGeometry = new THREE.BufferGeometry().setFromPoints(points.map((point) => new THREE.Vector3(point.x, point.y, z)));
        parent.add(new THREE.LineLoop(lineGeometry, edgeMaterial));
        outlineGeometries.push(lineGeometry);
      });
    };
    addProfileLines(plate, [model.plate, ...model.openings], settings.stencilThickness);
    modelGroup.add(plate);

    const wallMaterial = new THREE.MeshStandardMaterial({ color: 0x4fa6d8, roughness: 0.3, metalness: 0, side: THREE.DoubleSide });
    const supportGeometries: ExtrudeGeometry[] = [];
    const wallShape = pathFor(model.outerWall);
    wallShape.holes.push(pathFor(model.innerWall, true));
    const wallGeometry = new THREE.ExtrudeGeometry(wallShape, {
      depth: settings.wallHeight,
      bevelEnabled: false,
      curveSegments: 1,
    });
    const wall = new THREE.Mesh(wallGeometry, wallMaterial);
    wall.position.z = -settings.wallHeight;
    addProfileLines(wall, [model.outerWall, model.innerWall], settings.wallHeight);
    modelGroup.add(wall);
    supportGeometries.push(wallGeometry);
    modelGroup.position.set(-centerX, -centerY, 0);

    scene.add(new THREE.HemisphereLight(0xffffff, 0x52606d, 2.1));
    const keyLight = new THREE.DirectionalLight(0xffffff, 3.4);
    keyLight.position.set(70, -90, 120);
    const fillLight = new THREE.DirectionalLight(0xbfd8ff, 1.4);
    fillLight.position.set(-80, 40, 50);
    scene.add(keyLight, fillLight);

    camera.position.set(size * 0.9, -size * 1.25, size * 0.95);
    const modelCenterZ = (settings.stencilThickness - settings.wallHeight) / 4;
    camera.lookAt(0, 0, modelCenterZ);
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.target.set(0, 0, modelCenterZ);
    controls.enableDamping = true;
    controls.dampingFactor = 0.08;
    controls.minDistance = size * 0.65;
    controls.maxDistance = size * 4;
    controls.update();

    const resize = () => {
      const width = previewMount.clientWidth;
      const height = previewMount.clientHeight;
      if (!width || !height) return;
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      renderer.setSize(width, height, false);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(previewMount);
    resize();

    let animationFrame = 0;
    const animate = () => {
      controls.update();
      renderer.render(scene, camera);
      animationFrame = requestAnimationFrame(animate);
    };
    animate();

    disposePreview = () => {
      cancelAnimationFrame(animationFrame);
      observer.disconnect();
      controls.dispose();
      plateGeometry.dispose();
      supportGeometries.forEach((geometry) => geometry.dispose());
      outlineGeometries.forEach((geometry) => geometry.dispose());
      edgeMaterial.dispose();
      plateMaterial.dispose();
      wallMaterial.dispose();
      renderer.dispose();
      previewMount.replaceChildren();
    };
      } catch (error) {
        if (cancelled) return;
      console.error("3D preview failed", error);
      previewMount.replaceChildren();
      const fallback = document.createElement("div");
      fallback.className = "empty-state";
      const title = document.createElement("strong");
      title.textContent = "3D preview unavailable";
      const detail = document.createElement("span");
      detail.textContent = error instanceof Error ? error.message : "WebGL could not render this model";
      fallback.append(title, detail);
      previewMount.append(fallback);
      }
    }

    void loadPreview();

    return () => {
      cancelled = true;
      disposePreview?.();
    };
  }, [model, settings.stencilThickness, settings.wallHeight]);

  return <div ref={mountRef} className="three-stage" aria-label="Interactive 3D stencil preview" />;
}

function TwoDPreview({
  model,
  removedOpenings,
  onToggleOpening,
  onRestoreOpenings,
}: {
  model: ModelGeometry;
  removedOpenings: Set<number>;
  onToggleOpening: (index: number) => void;
  onRestoreOpenings: () => void;
}) {
  const { minX, maxX, minY, maxY } = boundsForModel(model);
  const width = Math.max(maxX - minX, 1);
  const height = Math.max(maxY - minY, 1);
  const padding = Math.max(Math.min(width, height) * 0.06, 1);
  const baseViewBox: ViewBox = {
    x: minX - padding,
    y: -(maxY + padding),
    width: width + padding * 2,
    height: height + padding * 2,
  };
  const [viewBox, setViewBox] = useState(baseViewBox);
  const svgRef = useRef<SVGSVGElement>(null);
  const panRef = useRef<{
    pointerId: number;
    clientX: number;
    clientY: number;
    viewBox: ViewBox;
    moved: boolean;
  } | null>(null);
  const suppressClickRef = useRef(false);
  useEffect(() => {
    setViewBox(baseViewBox);
  }, [minX, maxX, minY, maxY]);

  const svgPoint = (clientX: number, clientY: number, current: ViewBox) => {
    const rect = svgRef.current?.getBoundingClientRect();
    if (!rect || !rect.width || !rect.height) return null;
    return {
      x: current.x + ((clientX - rect.left) / rect.width) * current.width,
      y: current.y + ((clientY - rect.top) / rect.height) * current.height,
    };
  };
  const pointsFor = (points: Point[], reverse = false) =>
    (reverse ? [...points].reverse() : points)
      .map((point) => `${point.x},${-point.y}`)
      .join(" ");

  return (
    <div className="svg-stage viewport-svg two-d-preview">
      <svg
        ref={svgRef}
        xmlns="http://www.w3.org/2000/svg"
        viewBox={`${viewBox.x} ${viewBox.y} ${viewBox.width} ${viewBox.height}`}
        preserveAspectRatio="xMidYMid meet"
        role="img"
        aria-label="Interactive 2D stencil preview"
        onWheel={(event) => {
          event.preventDefault();
          const cursor = svgPoint(event.clientX, event.clientY, viewBox);
          if (!cursor) return;
          const wheelFactor = event.deltaY > 0 ? 1.12 : 1 / 1.12;
          setViewBox((current) => {
            const nextWidth = Math.min(baseViewBox.width * 3, Math.max(baseViewBox.width * 0.2, current.width * wheelFactor));
            const scale = nextWidth / current.width;
            return {
              x: cursor.x - (cursor.x - current.x) * scale,
              y: cursor.y - (cursor.y - current.y) * scale,
              width: nextWidth,
              height: current.height * scale,
            };
          });
        }}
        onPointerDown={(event) => {
          if (event.button !== 2) return;
          suppressClickRef.current = false;
          panRef.current = {
            pointerId: event.pointerId,
            clientX: event.clientX,
            clientY: event.clientY,
            viewBox,
            moved: false,
          };
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          const pan = panRef.current;
          if (!pan || pan.pointerId !== event.pointerId) return;
          const distance = Math.hypot(event.clientX - pan.clientX, event.clientY - pan.clientY);
          if (distance > 3) pan.moved = true;
          const rect = event.currentTarget.getBoundingClientRect();
          if (!rect.width || !rect.height) return;
          setViewBox({
            x: pan.viewBox.x - ((event.clientX - pan.clientX) / rect.width) * pan.viewBox.width,
            y: pan.viewBox.y - ((event.clientY - pan.clientY) / rect.height) * pan.viewBox.height,
            width: pan.viewBox.width,
            height: pan.viewBox.height,
          });
        }}
        onPointerUp={(event) => {
          const pan = panRef.current;
          if (!pan || pan.pointerId !== event.pointerId) return;
          suppressClickRef.current = pan.moved;
          panRef.current = null;
          if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={() => {
          panRef.current = null;
        }}
        onContextMenu={(event) => event.preventDefault()}
        onClickCapture={(event) => {
          if (!suppressClickRef.current) return;
          event.preventDefault();
          event.stopPropagation();
          suppressClickRef.current = false;
        }}
      >
        <rect x={baseViewBox.x} y={baseViewBox.y} width={baseViewBox.width} height={baseViewBox.height} fill="#fbfcfd" />
        <polygon className="plate" points={pointsFor(model.plate)} />
        <path className="wall" fillRule="evenodd" d={`M ${pointsFor(model.outerWall)} Z M ${pointsFor(model.innerWall, true)} Z`} />
        <g className="opening-layer">
          {model.selectionOpenings.map((opening, index) => {
            const removed = removedOpenings.has(index);
            return (
              <polygon
                key={index}
                className={classNames("opening", removed && "is-removed")}
                points={pointsFor(opening)}
                role="button"
                tabIndex={0}
                aria-label={`Pad ${index + 1}${removed ? " (removed)" : ""}`}
                aria-pressed={removed}
                onClick={() => onToggleOpening(index)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    onToggleOpening(index);
                  }
                }}
              >
                <title>{removed ? `Restore pad ${index + 1}` : `Remove pad ${index + 1}`}</title>
              </polygon>
            );
          })}
        </g>
      </svg>
      <div className="two-d-toolbar" role="status" aria-live="polite">
        <span>
          {removedOpenings.size
            ? `${removedOpenings.size} pad${removedOpenings.size === 1 ? "" : "s"} removed`
            : "Click a pad to remove it · Right-drag to pan · Scroll to zoom"}
        </span>
        {removedOpenings.size > 0 && (
          <button type="button" onClick={onRestoreOpenings}>
            Restore all
          </button>
        )}
        <button type="button" onClick={() => setViewBox(baseViewBox)}>
          Reset view
        </button>
      </div>
    </div>
  );
}

interface PreviewErrorBoundaryProps {
  children: React.ReactNode;
}

interface PreviewErrorBoundaryState {
  error: Error | null;
}

class PreviewErrorBoundary extends Component<PreviewErrorBoundaryProps, PreviewErrorBoundaryState> {
  state: PreviewErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: unknown): PreviewErrorBoundaryState {
    return { error: error instanceof Error ? error : new Error("The preview could not be rendered") };
  }

  componentDidCatch(error: Error) {
    console.error("Preview render failed", error);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="empty-state" role="alert">
          <strong>3D preview unavailable</strong>
          <span>{this.state.error.message}</span>
        </div>
      );
    }
    return this.props.children;
  }
}

export function App() {
  const [layers, setLayers] = useState<Partial<Record<LayerKind, LayerSource>>>({});
  const [settings, setSettings] = useState(DEFAULT_SETTINGS);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [busy, setBusy] = useState<BusyAction | null>(null);
  const [notice, setNotice] = useState("Drop files to begin");
  const [noticeError, setNoticeError] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [viewMode, setViewMode] = useState<"3d" | "2d">("3d");
  const [pasteSide, setPasteSide] = useState<PasteSide>("front");
  const [removedOpenings, setRemovedOpenings] = useState<Set<number>>(() => new Set());
  const [exportMenuOpen, setExportMenuOpen] = useState(false);

  useEffect(() => {
    if (import.meta.env.DEV) return;

    let cancelled = false;
    async function installAvailableUpdate() {
      try {
        const update = await check();
        if (!update || cancelled) return;
        setNotice(`Updating to ${update.version}…`);
        await update.downloadAndInstall();
        if (!cancelled) await relaunch();
      } catch {
        // Update failures must not prevent the application from starting.
      }
    }

    void installAvailableUpdate();
    return () => {
      cancelled = true;
    };
  }, []);

  const request = useMemo<StencilRequest | null>(
    () => (layers.paste && layers.edge ? { paste: layers.paste, edge: layers.edge, settings, pasteSide } : null),
    [layers, settings, pasteSide],
  );

  useEffect(() => {
    if (!request) return;
    const timer = window.setTimeout(() => void renderPreview(request), 180);
    return () => window.clearTimeout(timer);
  }, [request]);

  function patchSettings(patch: Partial<StencilSettings>) {
    setSettings((current) => ({ ...current, ...patch }));
  }

  async function importFiles(files: File[]) {
    const supported = files.filter((file) => {
      const name = file.name.toLowerCase();
      return name.endsWith(".zip") || GERBER_EXTENSIONS.some((extension) => name.endsWith(extension));
    });
    if (!supported.length) {
      setNoticeError(true);
      setNotice("No Gerber files found");
      return;
    }

    setNoticeError(false);
    setNotice(`Reading ${supported.length} file${supported.length === 1 ? "" : "s"}…`);
    try {
      const sources = await Promise.all(
        supported.map(async (file) => {
          const isZip = file.name.toLowerCase().endsWith(".zip");
          return { source: { data: await readFile(file, isZip), filename: file.name, isZip }, file };
        }),
      );
      const next: Partial<Record<LayerKind, LayerSource>> = {};
      const zip = sources.find(({ source }) => source.isZip)?.source;
      if (zip) {
        next.paste = zip;
        next.edge = zip;
      }

      const gerbers = sources.filter(({ source }) => !source.isZip);
      const paste = gerbers.find(({ file }) => looksLikePaste(file.name))?.source;
      const edge = gerbers.find(({ file }) => looksLikeEdge(file.name))?.source;
      if (paste) next.paste = paste;
      if (edge) next.edge = edge;

      const unassigned = gerbers
        .filter(({ source }) => source !== paste && source !== edge)
        .map(({ source }) => source);
      if (!next.paste) next.paste = unassigned.shift();
      if (!next.edge) next.edge = unassigned.shift();
      setLayers(next);
      setPreview(null);
      setPasteSide("front");
      setRemovedOpenings(new Set());
      setSidebarOpen(true);
      setNotice(next.paste && next.edge ? "Ready" : "Add an outline layer");
    } catch (error) {
      setNoticeError(true);
      setNotice(`Import failed: ${errorMessage(error)}`);
    }
  }

  async function renderPreview(input: StencilRequest | null = request) {
    if (!input || busy) return;
    setBusy("preview");
    setNoticeError(false);
    setNotice("Building…");
    try {
      const result = await invoke<PreviewResult>("preview_stencil", { request: input });
      setPreview(result);
      setRemovedOpenings((current) => new Set([...current].filter((index) => index < result.model.selectionOpenings.length)));
      setNotice("Ready");
    } catch (error) {
      setNoticeError(true);
      setNotice(errorMessage(error));
    } finally {
      setBusy(null);
    }
  }

  async function exportStep() {
    await runExport<SaveResult>("step", "save_stencil_step", (result) => {
      setNotice(result.saved ? "STEP saved" : "Export cancelled");
    });
  }

  async function exportStl() {
    await runExport<SaveResult>("stl", "save_stencil_stl", (result) => {
      setNotice(result.saved ? "STL saved" : "Export cancelled");
    });
  }

  async function runExport<T>(action: ExportAction, command: ExportCommand, onSuccess: (result: T) => void) {
    if (!request || busy) return;
    setBusy(action);
    setNoticeError(false);
    setNotice("Exporting…");
    try {
      const result = await invoke<T>(command, {
        request: { ...request, excludedOpenings: [...removedOpenings] },
      });
      onSuccess(result);
    } catch (error) {
      setNoticeError(true);
      setNotice(errorMessage(error));
    } finally {
      setBusy(null);
    }
  }

  function handleDrop(event: React.DragEvent<HTMLDivElement>) {
    event.preventDefault();
    setDragging(false);
    const items = Array.from(event.dataTransfer.items);
    const files = Array.from(event.dataTransfer.files);
    if (items.length) {
      void readDroppedFiles(items).then((dropped) => importFiles(dropped.length ? dropped : files));
    } else {
      void importFiles(files);
    }
  }

  function closeProject() {
    setLayers({});
    setPreview(null);
    setSettings(DEFAULT_SETTINGS);
    setPasteSide("front");
    setRemovedOpenings(new Set());
    setBusy(null);
    setNoticeError(false);
    setNotice("Drop files to begin");
    setViewMode("3d");
    setExportMenuOpen(false);
  }

  const fileTitle = layers.paste?.filename ?? layers.edge?.filename ?? "Untitled stencil";
  const hasLayers = Boolean(layers.paste || layers.edge);
  const dashboard = hasLayers || Boolean(preview);
  const visibleModel = preview
    ? {
        ...preview.model,
        openings: preview.model.openings.filter((_, index) =>
          !(preview.model.openingSources[index] ?? []).some((sourceIndex) => removedOpenings.has(sourceIndex)),
        ),
      }
    : null;

  return (
    <main
      className={classNames(dashboard ? "app-shell" : "welcome-screen", dragging && "is-dragging")}
      style={dashboard ? ({ "--sidebar-width": sidebarOpen ? "34rem" : "0px" } as React.CSSProperties) : undefined}
      onDragEnter={(event) => {
        event.preventDefault();
        setDragging(true);
      }}
      onDragOver={(event) => event.preventDefault()}
      onDragLeave={(event) => {
        if (event.currentTarget === event.target) setDragging(false);
      }}
      onDrop={handleDrop}
    >
      {!dashboard ? (
        <section className="welcome-content" aria-labelledby="welcome-title">
          <h1 id="welcome-title">StencilPrint</h1>
          <div className="welcome-actions">
            <label className="welcome-button">
              <Upload />
              <span>import</span>
              <input
                type="file"
                multiple
                accept={`${GERBER_EXTENSIONS.join(",")},.zip`}
                onChange={(event) => {
                  void importFiles(Array.from(event.currentTarget.files ?? []));
                  event.currentTarget.value = "";
                }}
              />
            </label>
          </div>
        </section>
      ) : (
        <>
          <aside className="app-rail" aria-label="Stencil navigation">
            <div className="app-rail-group">
              <button
                className={classNames("app-rail-tool", sidebarOpen && "is-active")}
                type="button"
                aria-label="Toggle settings"
                aria-pressed={sidebarOpen}
                title="Settings"
                onClick={() => setSidebarOpen((open) => !open)}
              >
                <Settings2 />
              </button>
            </div>
            <div className="app-rail-spacer" />
            <div className="app-rail-version">v0.1.0</div>
          </aside>

          {sidebarOpen && (
            <aside className="settings-sidebar" aria-label="Settings">
              <header className="workspace-sidebar-header">
                <strong className="workspace-sidebar-title">Settings</strong>
              </header>
              <div className="workspace-sidebar-content">
                <section className="workspace-sidebar-column workspace-sidebar-stencil-column">
                  <div className="workspace-sidebar-column-heading"><span>Stencil</span></div>
                  <div className="inspector-section">
                    <NumberField label="Clearance" value={settings.clearance} min={0} step={0.05} onChange={(clearance) => patchSettings({ clearance })} />
                    <NumberField label="Wall" value={settings.wallThickness} min={0.5} step={0.1} onChange={(wallThickness) => patchSettings({ wallThickness })} />
                    <NumberField label="Height" value={settings.wallHeight} min={0.5} step={0.1} onChange={(wallHeight) => patchSettings({ wallHeight })} />
                    <NumberField label="Thickness" value={settings.stencilThickness} min={0.1} step={0.05} onChange={(stencilThickness) => patchSettings({ stencilThickness })} />
                  </div>

                  <div className="sidebar-divider" />
                  <div className="workspace-sidebar-column-heading"><span>Printability</span><small>COMPENSATION</small></div>
                  <div className="inspector-section">
                    <NumberField label="Shrink" value={settings.shrink} min={-0.2} step={0.05} onChange={(shrink) => patchSettings({ shrink })} />
                    <NumberField label="Nozzle" value={settings.nozzleDiameter} min={0.1} step={0.05} onChange={(nozzleDiameter) => patchSettings({ nozzleDiameter })} />
                    <ToggleField label="Merge close pads" value={settings.enableSlotify} onChange={(enableSlotify) => patchSettings({ enableSlotify })} />
                    <ToggleField label="Fill thin grids" value={settings.dropUnprintableGrids} onChange={(dropUnprintableGrids) => patchSettings({ dropUnprintableGrids })} />
                  </div>

                  <div className="sidebar-divider" />
                  <div className="workspace-sidebar-column-heading"><span>Inputs</span></div>
                  <div className="inspector-section source-list">
                    <div className="paste-side-row">
                      <span>Paste side</span>
                      <div className="paste-side-toggle" role="group" aria-label="Paste side">
                        <button className={classNames(pasteSide === "front" && "is-active")} type="button" aria-pressed={pasteSide === "front"} onClick={() => setPasteSide("front")}>F.Paste</button>
                        <button className={classNames(pasteSide === "back" && "is-active")} type="button" aria-pressed={pasteSide === "back"} onClick={() => setPasteSide("back")}>B.Paste</button>
                      </div>
                    </div>
                  </div>
                </section>

                <section className="workspace-sidebar-column workspace-sidebar-view-column">
                  <div className="workspace-sidebar-column-heading"><span>Viewport</span></div>
                  <div className="inspector-section">
                    <span className="inspector-label">Preview</span>
                    <div className="view-mode-group">
                      <button className={classNames("view-mode-button", viewMode === "3d" && "is-active")} type="button" aria-label="3D preview" title="3D preview" aria-pressed={viewMode === "3d"} onClick={() => setViewMode("3d")}>
                        3d
                      </button>
                      <button className={classNames("view-mode-button", viewMode === "2d" && "is-active")} type="button" aria-label="2D preview" title="2D preview" aria-pressed={viewMode === "2d"} onClick={() => setViewMode("2d")}>
                        2d
                      </button>
                    </div>
                  </div>
                  <div className="sidebar-divider" />
                  <div className="workspace-sidebar-column-heading"><span>Build</span></div>
                  <div className="inspector-section">
                    {preview?.model.warnings.length ? (
                      <div className="geometry-notes" role="note">
                        <strong>Engine notes</strong>
                        {preview.model.warnings.map((warning) => <span key={warning}>{warning}</span>)}
                      </div>
                    ) : null}
                  </div>
                </section>
              </div>
            </aside>
          )}

          <section className="sidebar-inset">
            <header className="command-bar">
              <div className="file-name" title={fileTitle}>{fileTitle}</div>
              <div className="command-actions">
                {notice && notice !== "Ready" && <span className={classNames("command-status", noticeError && "is-error")} role="status" aria-live="polite">{notice}</span>}
                <UniversalImport onFiles={importFiles} />
                <div className="export-menu">
                  <button
                    className="command-icon"
                    type="button"
                    aria-label="Export stencil"
                    aria-haspopup="menu"
                    aria-expanded={exportMenuOpen}
                    title="Export stencil"
                    disabled={!preview || busy !== null}
                    onClick={() => setExportMenuOpen((open) => !open)}
                  >
                    {busy === "step" || busy === "stl" ? <LoaderCircle className="spin" /> : <Download />}
                    {exportMenuOpen && <ChevronDown className="export-menu-chevron" />}
                  </button>
                  {exportMenuOpen && (
                    <div className="export-menu-options" role="menu" aria-label="Export format">
                      <button type="button" role="menuitem" onClick={() => { setExportMenuOpen(false); void exportStl(); }}>STL</button>
                      <button type="button" role="menuitem" onClick={() => { setExportMenuOpen(false); void exportStep(); }}>STEP</button>
                    </div>
                  )}
                </div>
              </div>
            </header>

            <section className="viewport" aria-label="Stencil workspace">
              <div className="viewport-grid" />
              <button className="viewer-close-button" type="button" aria-label="Close project" title="Close project" onClick={closeProject}><X /></button>
              {preview ? (
                viewMode === "3d" && visibleModel ? <PreviewErrorBoundary><ThreePreview model={visibleModel} settings={settings} /></PreviewErrorBoundary> : visibleModel ? <TwoDPreview model={preview.model} removedOpenings={removedOpenings} onToggleOpening={(index) => setRemovedOpenings((current) => {
                  const next = new Set(current);
                  if (next.has(index)) next.delete(index);
                  else next.add(index);
                  return next;
                })} onRestoreOpenings={() => setRemovedOpenings(new Set())} /> : null
              ) : (
                <div className="empty-state" role="status">
                  <Box />
                  <strong>{busy === "preview" ? "Building stencil…" : "Drop Gerbers or a ZIP"}</strong>
                  <span>{notice}</span>
                </div>
              )}
              {preview && <div className="app-meta">{preview.edge.widthMm.toFixed(1)} × {preview.edge.heightMm.toFixed(1)} mm · {viewMode.toUpperCase()}</div>}
            </section>
          </section>
        </>
      )}
    </main>
  );
}

function looksLikePaste(filename: string) {
  const name = filename.toLowerCase();
  return /paste|cream|solder.?paste|\.gtp$|\.gbp$/.test(name);
}

function looksLikeEdge(filename: string) {
  const name = filename.toLowerCase();
  return /edge[._ -]?cuts|edgecuts|outline|profile|\.gko$|\.gm1$/.test(name);
}

function readFile(file: File, isZip: boolean): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.addEventListener("error", () => reject(reader.error));
    reader.addEventListener("load", () => {
      const result = String(reader.result ?? "");
      resolve(isZip ? result.slice(result.indexOf(",") + 1) : result);
    });
    if (isZip) reader.readAsDataURL(file);
    else reader.readAsText(file);
  });
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

interface DroppedEntry {
  isFile: boolean;
  isDirectory: boolean;
  file: (success: (file: File) => void, failure?: (error: DOMException) => void) => void;
  createReader: () => { readEntries: (success: (entries: DroppedEntry[]) => void, failure?: (error: DOMException) => void) => void };
}

function readDroppedFiles(items: DataTransferItem[]): Promise<File[]> {
  const entries = items
    .map((item) => (item as unknown as { webkitGetAsEntry?: () => DroppedEntry | null }).webkitGetAsEntry?.())
    .filter((entry): entry is DroppedEntry => Boolean(entry));
  return Promise.all(entries.map(readDroppedEntry)).then((groups) => groups.flat());
}

function readDroppedEntry(entry: DroppedEntry): Promise<File[]> {
  if (entry.isFile) {
    return new Promise((resolve, reject) => entry.file((file) => resolve([file]), reject));
  }

  const reader = entry.createReader();
  const readBatch = (): Promise<DroppedEntry[]> =>
    new Promise((resolve, reject) => reader.readEntries(resolve, reject));
  const readAll = async (entries: DroppedEntry[] = []): Promise<DroppedEntry[]> => {
    const batch = await readBatch();
    return batch.length ? readAll(entries.concat(batch)) : entries;
  };

  return readAll().then((entries) => Promise.all(entries.map(readDroppedEntry)).then((groups) => groups.flat()));
}
