import { useEffect, useRef, useState } from "react";

import type {
  AutomatonSummary,
  AutomatonTier
} from "@ic-automaton/shared";
import { themeTokens } from "../../theme/tokens";

const CELL_SIZE = 10;
const CELL_GAP = 1;
const CELL_FULL = CELL_SIZE + CELL_GAP;
const BASE_LAYOUT_WIDTH = 880;
const BASE_LAYOUT_HEIGHT = 520;
const MAX_LAYOUT_SCALE = 1.15;

interface AutomatonCanvasProps {
  automatons: readonly AutomatonSummary[];
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
}

export function getCanvasLayoutScale(width: number, height: number): number {
  return Math.min(
    Math.min(width / BASE_LAYOUT_WIDTH, height / BASE_LAYOUT_HEIGHT),
    MAX_LAYOUT_SCALE
  );
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

function buildRenderNodes(
  automatons: readonly AutomatonSummary[],
  width: number,
  height: number
): RenderNode[] {
  if (automatons.length === 0) {
    return [];
  }

  const xs = automatons.map((entry) => entry.gridPosition.x);
  const ys = automatons.map((entry) => entry.gridPosition.y);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  const padding = 8 * CELL_FULL;
  const gridWidth = Math.max(maxX - minX + 14, 24);
  const gridHeight = Math.max(maxY - minY + 14, 18);
  const scale = Math.min(
    Math.min(
      (width - padding * 2) / (gridWidth * CELL_FULL),
      (height - padding * 2) / (gridHeight * CELL_FULL)
    ),
    MAX_LAYOUT_SCALE
  );
  const offsetX = (width - gridWidth * CELL_FULL * scale) / 2;
  const offsetY = (height - gridHeight * CELL_FULL * scale) / 2;

  return automatons.map((automaton) => ({
    automaton,
    cx: offsetX + (automaton.gridPosition.x - minX + 7) * CELL_FULL * scale,
    cy: offsetY + (automaton.gridPosition.y - minY + 7) * CELL_FULL * scale,
    radiusCells: computeRadiusCells(automaton)
  }));
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
  onSpawn,
  selectedCanisterId,
  statusNotice,
  viewerAddress,
  onSelect
}: AutomatonCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const hitAreasRef = useRef<HitArea[]>([]);
  const [hoveredCanisterId, setHoveredCanisterId] = useState<string | null>(null);
  const [tooltip, setTooltip] = useState<TooltipState>({
    left: 0,
    top: 0,
    label: "",
    visible: false
  });

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
      const layoutScale = getCanvasLayoutScale(width, height);

      drawingContext.fillStyle = themeTokens.colors.gridDot;
      for (let y = 0; y < height; y += CELL_FULL) {
        for (let x = 0; x < width; x += CELL_FULL) {
          drawingContext.fillRect(x + 3, y + 3, 3, 3);
        }
      }

      const nodes = buildRenderNodes(automatons, width, height);
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
        const radiusPixels = Math.max(
          node.radiusCells * CELL_FULL * layoutScale,
          26
        );

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
          const size = Math.max(2, (CELL_SIZE + jitter) * layoutScale);
          const x = node.cx + cell.dx * CELL_FULL * layoutScale - size / 2;
          const y = node.cy + cell.dy * CELL_FULL * layoutScale - size / 2;

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
        className="canvas-wrap"
        onClick={(event) => {
          const hit = findHit(event.clientX, event.clientY);

          if (hit !== undefined) {
            onSelect(hit.canisterId);
          }
        }}
        onMouseLeave={() => {
          setHoveredCanisterId(null);
          setTooltip((previous) => ({ ...previous, visible: false }));
        }}
        onMouseMove={(event) => {
          updateHover(event.clientX, event.clientY);
        }}
        ref={containerRef}
      >
        <canvas className="automaton-canvas" ref={canvasRef} />
        <button
          className="canvas-spawn-button"
          onClick={() => {
            onSpawn();
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
