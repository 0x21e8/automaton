import { useEffect, useRef, useState } from "react";

import type {
  AutomatonSummary,
  AutomatonTier
} from "@ic-automaton/shared";
import { themeTokens } from "../../theme/tokens";

const CELL_SIZE = 10;
const CELL_GAP = 1;
const CELL_FULL = CELL_SIZE + CELL_GAP;
const DEFAULT_CAMERA_ZOOM = 0.95;
const FOCUS_CAMERA_ZOOM = 1.2;
const MIN_CAMERA_ZOOM = 0.35;
const MAX_CAMERA_ZOOM = 1.8;
const MIN_NODE_RADIUS_PX = 26;
const NODE_COLLISION_PADDING_PX = 12;
const NODE_COLLISION_ITERATIONS = 18;
const CAMERA_SPRING_FACTOR = 0.08;
const CLICK_DRAG_THRESHOLD_PX = 6;

interface AutomatonCanvasProps {
  automatons: readonly AutomatonSummary[];
  focusCanisterId?: string | null;
  onSpawn: () => void;
  selectedCanisterId: string | null;
  statusNotice: string | null;
  viewerAddress: string | null;
  onSelect: (canisterId: string) => void;
}

interface TooltipState {
  left: number;
  top: number;
  label: string;
  visible: boolean;
}

interface HitArea {
  canisterId: string;
  cx: number;
  cy: number;
  radius: number;
}

interface RenderNode {
  automaton: AutomatonSummary;
  cx: number;
  cy: number;
  radiusCells: number;
  radiusPixels: number;
}

interface CameraState {
  centerX: number;
  centerY: number;
  zoom: number;
}

interface ViewportSnapshot {
  width: number;
  height: number;
}

interface ProjectedNode {
  automaton: AutomatonSummary;
  baseCx: number;
  baseCy: number;
  cx: number;
  cy: number;
  radiusCells: number;
  radiusPixels: number;
}

function hexToRgba(hex: string, alpha: number): string {
  const normalized = hex.replace("#", "");
  const red = Number.parseInt(normalized.slice(0, 2), 16);
  const green = Number.parseInt(normalized.slice(2, 4), 16);
  const blue = Number.parseInt(normalized.slice(4, 6), 16);
  return `rgba(${red}, ${green}, ${blue}, ${alpha})`;
}

function getTierColor(tier: AutomatonTier): string {
  switch (tier) {
    case "low":
      return themeTokens.colors.gridLow;
    case "critical":
    case "out_of_cycles":
      return themeTokens.colors.gridCritical;
    default:
      return themeTokens.colors.gridNormal;
  }
}

function formatUsd(value: string | null): string {
  if (value === null) {
    return "$0";
  }

  const amount = Number(value);

  if (amount >= 1_000_000) {
    return `$${(amount / 1_000_000).toFixed(1)}M`;
  }

  if (amount >= 1_000) {
    return `$${(amount / 1_000).toFixed(1)}K`;
  }

  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 0
  }).format(amount);
}

function computeRadiusCells(automaton: AutomatonSummary): number {
  const worth = Number(automaton.netWorthUsd ?? "0");

  if (worth >= 10_000) {
    return 8;
  }

  if (worth >= 8_000) {
    return 7;
  }

  if (worth >= 5_000) {
    return 6;
  }

  return 5;
}

function buildCoreCells(
  automaton: AutomatonSummary,
  timeSeconds: number,
  radiusCells: number
): Array<{
  dx: number;
  dy: number;
  isCore: boolean;
}> {
  const corePattern =
    automaton.corePattern ??
    [
      [0, 0],
      [1, 0],
      [0, 1],
      [1, 1]
    ];

  const cells = new Map<string, { dx: number; dy: number; isCore: boolean }>();
  const seed = automaton.canisterId.length + automaton.corePatternIndex * 17;
  const beat = automaton.heartbeatIntervalSeconds ?? 45;
  const phase = timeSeconds / Math.max(beat / 16, 1);

  for (const [x, y] of corePattern) {
    const dx = x - 1;
    const dy = y - 1;
    cells.set(`${dx}:${dy}`, { dx, dy, isCore: true });
  }

  for (let dy = -radiusCells; dy <= radiusCells; dy += 1) {
    for (let dx = -radiusCells; dx <= radiusCells; dx += 1) {
      const distance = Math.hypot(dx, dy);

      if (distance > radiusCells + 0.2) {
        continue;
      }

      const noise =
        Math.sin((dx + seed) * 0.82 + phase * 1.3) +
        Math.cos((dy - seed) * 0.74 - phase * 1.1) +
        Math.sin((dx - dy) * 0.52 + phase * 0.8);

      const threshold = 1.5 - radiusCells * 0.08;

      if (noise > threshold) {
        const key = `${dx}:${dy}`;

        if (!cells.has(key)) {
          cells.set(key, { dx, dy, isCore: false });
        }
      }
    }
  }

  return [...cells.values()];
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function getAutomatonWorldPosition(
  automaton: Pick<AutomatonSummary, "gridPosition">
) {
  return {
    x: automaton.gridPosition.x * CELL_FULL,
    y: automaton.gridPosition.y * CELL_FULL
  };
}

export function createFocusCameraState(
  automaton: Pick<AutomatonSummary, "gridPosition">,
  currentZoom: number
): CameraState {
  const worldPosition = getAutomatonWorldPosition(automaton);

  return {
    centerX: worldPosition.x,
    centerY: worldPosition.y,
    zoom: clamp(Math.max(currentZoom, FOCUS_CAMERA_ZOOM), MIN_CAMERA_ZOOM, MAX_CAMERA_ZOOM)
  };
}

function hashString(value: string): number {
  let hash = 0;

  for (let index = 0; index < value.length; index += 1) {
    hash = (hash << 5) - hash + value.charCodeAt(index);
    hash |= 0;
  }

  return Math.abs(hash);
}

function createProjectedNodes(
  automatons: readonly AutomatonSummary[],
  camera: CameraState,
  viewport: ViewportSnapshot
): ProjectedNode[] {
  if (automatons.length === 0) {
    return [];
  }

  return [...automatons]
    .sort((left, right) => left.canisterId.localeCompare(right.canisterId))
    .map((automaton) => {
      const radiusCells = computeRadiusCells(automaton);
      const radiusPixels = Math.max(
        radiusCells * CELL_FULL * camera.zoom,
        MIN_NODE_RADIUS_PX
      );
      const worldPosition = getAutomatonWorldPosition(automaton);
      const cx =
        viewport.width / 2 + (worldPosition.x - camera.centerX) * camera.zoom;
      const cy =
        viewport.height / 2 + (worldPosition.y - camera.centerY) * camera.zoom;

      return {
        automaton,
        baseCx: cx,
        baseCy: cy,
        cx,
        cy,
        radiusCells,
        radiusPixels
      };
    });
}

function resolveProjectedNodeOverlaps(
  projectedNodes: readonly ProjectedNode[]
): RenderNode[] {
  const resolvedNodes = projectedNodes.map((node) => ({ ...node }));

  for (let iteration = 0; iteration < NODE_COLLISION_ITERATIONS; iteration += 1) {
    for (const node of resolvedNodes) {
      node.cx += (node.baseCx - node.cx) * CAMERA_SPRING_FACTOR;
      node.cy += (node.baseCy - node.cy) * CAMERA_SPRING_FACTOR;
    }

    for (let index = 0; index < resolvedNodes.length; index += 1) {
      const current = resolvedNodes[index];

      for (
        let otherIndex = index + 1;
        otherIndex < resolvedNodes.length;
        otherIndex += 1
      ) {
        const other = resolvedNodes[otherIndex];
        const dx = other.cx - current.cx;
        const dy = other.cy - current.cy;
        const distance = Math.hypot(dx, dy);
        const minimumDistance =
          current.radiusPixels + other.radiusPixels + NODE_COLLISION_PADDING_PX;

        if (distance >= minimumDistance) {
          continue;
        }

        let normalX = 0;
        let normalY = 0;

        if (distance < 0.001) {
          const angle =
            ((hashString(`${current.automaton.canisterId}:${other.automaton.canisterId}`) %
              360) *
              Math.PI) /
            180;
          normalX = Math.cos(angle);
          normalY = Math.sin(angle);
        } else {
          normalX = dx / distance;
          normalY = dy / distance;
        }

        const push = (minimumDistance - Math.max(distance, 1)) / 2;

        current.cx -= normalX * push;
        current.cy -= normalY * push;
        other.cx += normalX * push;
        other.cy += normalY * push;
      }
    }
  }

  return resolvedNodes.map((node) => ({
    automaton: node.automaton,
    cx: node.cx,
    cy: node.cy,
    radiusCells: node.radiusCells,
    radiusPixels: node.radiusPixels
  }));
}

function getViewportCenter(
  automatons: readonly AutomatonSummary[]
): Pick<CameraState, "centerX" | "centerY"> {
  if (automatons.length === 0) {
    return {
      centerX: 0,
      centerY: 0
    };
  }

  const positions = automatons.map(getAutomatonWorldPosition);
  const xs = positions.map((position) => position.x);
  const ys = positions.map((position) => position.y);

  return {
    centerX: (Math.min(...xs) + Math.max(...xs)) / 2,
    centerY: (Math.min(...ys) + Math.max(...ys)) / 2
  };
}

export function buildRenderNodes(
  automatons: readonly AutomatonSummary[],
  camera: CameraState,
  viewport: ViewportSnapshot
): RenderNode[] {
  return resolveProjectedNodeOverlaps(
    createProjectedNodes(automatons, camera, viewport)
  );
}

function drawManhattanPath(
  context: CanvasRenderingContext2D,
  from: RenderNode,
  to: RenderNode
): void {
  context.beginPath();
  context.moveTo(from.cx, from.cy);
  context.lineTo(to.cx, from.cy);
  context.lineTo(to.cx, to.cy);
  context.stroke();
}

export function AutomatonCanvas({
  automatons,
  focusCanisterId = null,
  onSpawn,
  selectedCanisterId,
  statusNotice,
  viewerAddress,
  onSelect
}: AutomatonCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const hitAreasRef = useRef<HitArea[]>([]);
  const cameraRef = useRef<CameraState>({
    centerX: 0,
    centerY: 0,
    zoom: DEFAULT_CAMERA_ZOOM
  });
  const cameraTargetRef = useRef<CameraState>({
    centerX: 0,
    centerY: 0,
    zoom: DEFAULT_CAMERA_ZOOM
  });
  const cameraInitializedRef = useRef(false);
  const interactionRef = useRef<{
    pointerId: number | null;
    lastClientX: number;
    lastClientY: number;
    totalMovement: number;
    suppressClick: boolean;
  }>({
    pointerId: null,
    lastClientX: 0,
    lastClientY: 0,
    totalMovement: 0,
    suppressClick: false
  });
  const [hoveredCanisterId, setHoveredCanisterId] = useState<string | null>(null);
  const [isPanning, setIsPanning] = useState(false);
  const [tooltip, setTooltip] = useState<TooltipState>({
    left: 0,
    top: 0,
    label: "",
    visible: false
  });

  useEffect(() => {
    if (
      cameraInitializedRef.current ||
      automatons.length === 0 ||
      focusCanisterId !== null
    ) {
      return;
    }

    const center = getViewportCenter(automatons);
    const nextCamera = {
      centerX: center.centerX,
      centerY: center.centerY,
      zoom: DEFAULT_CAMERA_ZOOM
    };
    cameraRef.current = nextCamera;
    cameraTargetRef.current = nextCamera;
    cameraInitializedRef.current = true;
  }, [automatons, focusCanisterId]);

  useEffect(() => {
    if (focusCanisterId === null) {
      return;
    }

    const focusedAutomaton = automatons.find(
      (entry) => entry.canisterId === focusCanisterId
    );

    if (focusedAutomaton === undefined) {
      return;
    }

    cameraTargetRef.current = createFocusCameraState(
      focusedAutomaton,
      cameraTargetRef.current.zoom
    );
    cameraInitializedRef.current = true;
    setTooltip((previous) => ({ ...previous, visible: false }));
  }, [automatons, focusCanisterId]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;

    if (canvas === null || container === null) {
      return;
    }

    const context = canvas.getContext("2d");

    if (context === null) {
      return;
    }

    const canvasElement = canvas;
    const containerElement = container;
    const drawingContext = context;

    let animationFrame = 0;
    let width = 0;
    let height = 0;

    function resizeCanvas() {
      const rect = containerElement.getBoundingClientRect();
      const nextWidth = Math.max(rect.width, 320);
      const nextHeight = Math.max(rect.height, 280);
      const dpr = window.devicePixelRatio || 1;

      width = nextWidth;
      height = nextHeight;
      canvasElement.width = Math.floor(nextWidth * dpr);
      canvasElement.height = Math.floor(nextHeight * dpr);
      canvasElement.style.width = `${nextWidth}px`;
      canvasElement.style.height = `${nextHeight}px`;
      drawingContext.setTransform(dpr, 0, 0, dpr, 0, 0);
    }

    resizeCanvas();

    const observer = new ResizeObserver(() => {
      resizeCanvas();
    });

    observer.observe(containerElement);

    const render = (time: number) => {
      const timeSeconds = time / 1000;
      drawingContext.clearRect(0, 0, width, height);
      const camera = cameraRef.current;
      const cameraTarget = cameraTargetRef.current;

      camera.centerX += (cameraTarget.centerX - camera.centerX) * CAMERA_SPRING_FACTOR;
      camera.centerY += (cameraTarget.centerY - camera.centerY) * CAMERA_SPRING_FACTOR;
      camera.zoom += (cameraTarget.zoom - camera.zoom) * CAMERA_SPRING_FACTOR;

      if (Math.abs(cameraTarget.centerX - camera.centerX) < 0.1) {
        camera.centerX = cameraTarget.centerX;
      }

      if (Math.abs(cameraTarget.centerY - camera.centerY) < 0.1) {
        camera.centerY = cameraTarget.centerY;
      }

      if (Math.abs(cameraTarget.zoom - camera.zoom) < 0.001) {
        camera.zoom = cameraTarget.zoom;
      }

      const scaledCell = CELL_FULL * camera.zoom;
      const gridStepMultiplier = Math.max(
        1,
        Math.ceil(14 / Math.max(scaledCell, 1))
      );
      const gridSpacing = scaledCell * gridStepMultiplier;
      const gridOffsetX =
        ((width / 2 - camera.centerX * camera.zoom) % gridSpacing) + gridSpacing;
      const gridOffsetY =
        ((height / 2 - camera.centerY * camera.zoom) % gridSpacing) + gridSpacing;

      drawingContext.fillStyle = themeTokens.colors.gridDot;
      for (let y = gridOffsetY % gridSpacing; y < height; y += gridSpacing) {
        for (let x = gridOffsetX % gridSpacing; x < width; x += gridSpacing) {
          drawingContext.fillRect(x + 3, y + 3, 3, 3);
        }
      }

      const nodes = buildRenderNodes(automatons, camera, { width, height });
      const nodeById = new Map(
        nodes.map((node) => [node.automaton.canisterId, node] as const)
      );

      drawingContext.strokeStyle = "rgba(0, 0, 0, 0.2)";
      drawingContext.lineWidth = 1;
      drawingContext.setLineDash([5, 6]);
      for (const node of nodes) {
        if (node.automaton.parentId === null) {
          continue;
        }

        const parent = nodeById.get(node.automaton.parentId);

        if (parent !== undefined) {
          drawManhattanPath(drawingContext, node, parent);
        }
      }
      drawingContext.setLineDash([]);

      const nextHitAreas: HitArea[] = [];

      for (const node of nodes) {
        const owned =
          viewerAddress !== null &&
          node.automaton.steward.address.toLowerCase() === viewerAddress.toLowerCase();
        const selected = selectedCanisterId === node.automaton.canisterId;
        const color = getTierColor(node.automaton.tier);
        const pulse = 0.55 + Math.sin(timeSeconds * 2 + node.cx * 0.01) * 0.18;
        const radiusPixels = node.radiusPixels;

        nextHitAreas.push({
          canisterId: node.automaton.canisterId,
          cx: node.cx,
          cy: node.cy,
          radius: radiusPixels
        });

        if (selected) {
          drawingContext.strokeStyle = "rgba(230, 51, 18, 0.85)";
          drawingContext.lineWidth = 1.2;
          drawingContext.strokeRect(
            node.cx - radiusPixels - 12,
            node.cy - radiusPixels - 12,
            radiusPixels * 2 + 24,
            radiusPixels * 2 + 24
          );
        }

        const liveCells = buildCoreCells(node.automaton, timeSeconds, node.radiusCells);

        for (const cell of liveCells) {
          const alpha = cell.isCore ? 0.82 : pulse * 0.72;
          const jitter = cell.isCore ? 0 : Math.sin(timeSeconds * 4 + cell.dx * 2 + cell.dy) * 0.4;
          const size = Math.max(2, (CELL_SIZE + jitter) * camera.zoom);
          const x = node.cx + cell.dx * CELL_FULL * camera.zoom - size / 2;
          const y = node.cy + cell.dy * CELL_FULL * camera.zoom - size / 2;

          drawingContext.fillStyle = hexToRgba(color, alpha);
          drawingContext.fillRect(x, y, size, size);
        }

        if (owned || selected) {
          drawingContext.fillStyle = "rgba(26, 26, 26, 0.92)";
          drawingContext.font = "700 11px Azeret Mono";
          drawingContext.textAlign = "center";
          drawingContext.fillText(
            node.automaton.name,
            node.cx,
            node.cy - radiusPixels - 16
          );
        }
      }

      hitAreasRef.current = nextHitAreas;
      animationFrame = window.requestAnimationFrame(render);
    };

    animationFrame = window.requestAnimationFrame(render);

    return () => {
      observer.disconnect();
      window.cancelAnimationFrame(animationFrame);
    };
  }, [automatons, hoveredCanisterId, selectedCanisterId, viewerAddress]);

  function findHit(clientX: number, clientY: number): HitArea | undefined {
    const container = containerRef.current;

    if (container === null) {
      return undefined;
    }

    const rect = container.getBoundingClientRect();
    const x = clientX - rect.left;
    const y = clientY - rect.top;

    return hitAreasRef.current.find(
      (entry) => Math.hypot(entry.cx - x, entry.cy - y) <= entry.radius
    );
  }

  function updateHover(clientX: number, clientY: number) {
    const hit = findHit(clientX, clientY);

    if (hit === undefined) {
      setHoveredCanisterId(null);
      setTooltip((previous) => ({ ...previous, visible: false }));
      return;
    }

    const automaton = automatons.find(
      (entry) => entry.canisterId === hit.canisterId
    );

    if (automaton === undefined) {
      return;
    }

    setHoveredCanisterId(hit.canisterId);
    setTooltip({
      left: clientX + 14,
      top: clientY - 14,
      label: `${automaton.name} — ${automaton.tier} — ${formatUsd(automaton.netWorthUsd)}`,
      visible: true
    });
  }

  return (
    <div className="canvas-shell">
      <div
        aria-label="Automaton grid"
        className={`canvas-wrap${isPanning ? " is-panning" : ""}`}
        onClick={(event) => {
          if (interactionRef.current.suppressClick) {
            interactionRef.current.suppressClick = false;
            return;
          }

          const hit = findHit(event.clientX, event.clientY);

          if (hit !== undefined) {
            onSelect(hit.canisterId);
          }
        }}
        onMouseLeave={() => {
          if (interactionRef.current.pointerId !== null) {
            return;
          }

          setHoveredCanisterId(null);
          setTooltip((previous) => ({ ...previous, visible: false }));
        }}
        onMouseMove={(event) => {
          if (interactionRef.current.pointerId !== null) {
            return;
          }

          updateHover(event.clientX, event.clientY);
        }}
        onPointerCancel={() => {
          interactionRef.current.pointerId = null;
          interactionRef.current.suppressClick =
            interactionRef.current.totalMovement > CLICK_DRAG_THRESHOLD_PX;
          setIsPanning(false);
        }}
        onPointerDown={(event) => {
          if (event.button !== 0) {
            return;
          }

          interactionRef.current.pointerId = event.pointerId;
          interactionRef.current.lastClientX = event.clientX;
          interactionRef.current.lastClientY = event.clientY;
          interactionRef.current.totalMovement = 0;
          interactionRef.current.suppressClick = false;
          setIsPanning(true);
          setHoveredCanisterId(null);
          setTooltip((previous) => ({ ...previous, visible: false }));
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          if (interactionRef.current.pointerId !== event.pointerId) {
            return;
          }

          const deltaX = event.clientX - interactionRef.current.lastClientX;
          const deltaY = event.clientY - interactionRef.current.lastClientY;

          interactionRef.current.lastClientX = event.clientX;
          interactionRef.current.lastClientY = event.clientY;
          interactionRef.current.totalMovement += Math.hypot(deltaX, deltaY);

          cameraRef.current = {
            ...cameraRef.current,
            centerX: cameraRef.current.centerX - deltaX / cameraRef.current.zoom,
            centerY: cameraRef.current.centerY - deltaY / cameraRef.current.zoom
          };
          cameraTargetRef.current = cameraRef.current;
        }}
        onPointerUp={(event) => {
          if (interactionRef.current.pointerId !== event.pointerId) {
            return;
          }

          interactionRef.current.pointerId = null;
          interactionRef.current.suppressClick =
            interactionRef.current.totalMovement > CLICK_DRAG_THRESHOLD_PX;
          setIsPanning(false);
          event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onWheel={(event) => {
          event.preventDefault();

          const rect = event.currentTarget.getBoundingClientRect();
          const pointerX = event.clientX - rect.left;
          const pointerY = event.clientY - rect.top;
          const { centerX, centerY, zoom } = cameraRef.current;
          const nextZoom = clamp(
            zoom * Math.exp(-event.deltaY * 0.0014),
            MIN_CAMERA_ZOOM,
            MAX_CAMERA_ZOOM
          );

          const worldX = centerX + (pointerX - rect.width / 2) / zoom;
          const worldY = centerY + (pointerY - rect.height / 2) / zoom;

          const nextCamera = {
            centerX: worldX - (pointerX - rect.width / 2) / nextZoom,
            centerY: worldY - (pointerY - rect.height / 2) / nextZoom,
            zoom: nextZoom
          };
          cameraRef.current = nextCamera;
          cameraTargetRef.current = nextCamera;
          setTooltip((previous) => ({ ...previous, visible: false }));
        }}
        ref={containerRef}
      >
        <canvas className="automaton-canvas" ref={canvasRef} />
        <button
          className="canvas-spawn-button"
          onClick={(event) => {
            event.stopPropagation();
            onSpawn();
          }}
          onPointerDown={(event) => {
            event.stopPropagation();
          }}
          type="button"
        >
          Spawn
        </button>
        <div
          className={`canvas-tooltip${tooltip.visible ? " is-visible" : ""}`}
          style={{
            left: `${tooltip.left}px`,
            top: `${tooltip.top}px`
          }}
        >
          {tooltip.label}
        </div>
        {statusNotice !== null ? (
          <p aria-live="polite" className="canvas-notice" role="status">
            {statusNotice}
          </p>
        ) : null}
      </div>
    </div>
  );
}
