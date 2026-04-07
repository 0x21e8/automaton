import type { EvaluationDashboardCyclesPoint } from "@ic-automaton/shared";

interface CyclesSparklineProps {
  points: EvaluationDashboardCyclesPoint[];
}

const WIDTH = 120n;
const HEIGHT = 30n;

export function CyclesSparkline({ points }: CyclesSparklineProps) {
  if (points.length === 0) {
    return <div className="cycles-sparkline cycles-sparkline--empty">No samples yet</div>;
  }

  const values = points.map((point) => BigInt(point.cyclesConsumed));
  const min = values.reduce((current, value) => (value < current ? value : current), values[0]!);
  const max = values.reduce((current, value) => (value > current ? value : current), values[0]!);
  const range = max - min;
  const lastIndex = BigInt(Math.max(points.length - 1, 1));

  const polylinePoints = values
    .map((value, index) => {
      const x = Number((BigInt(index) * WIDTH) / lastIndex);
      const y =
        range === 0n
          ? Number(HEIGHT / 2n)
          : Number(HEIGHT - ((value - min) * HEIGHT) / range);

      return `${x},${y}`;
    })
    .join(" ");
  const finalPoint = polylinePoints.split(" ").at(-1)?.split(",") ?? ["0", "0"];

  return (
    <svg
      aria-label="Cycles trend"
      className="cycles-sparkline"
      viewBox={`0 0 ${WIDTH.toString()} ${HEIGHT.toString()}`}
      preserveAspectRatio="none"
      role="img"
    >
      <polyline className="cycles-sparkline__line" fill="none" points={polylinePoints} />
      <circle
        className="cycles-sparkline__dot"
        cx={finalPoint[0]}
        cy={finalPoint[1]}
        r="2.5"
      />
    </svg>
  );
}
