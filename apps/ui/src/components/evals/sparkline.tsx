// A small pass-rate trend line for tables. Values are 0..1, oldest first.

export function Sparkline({
  values,
  label,
  width = 96,
  height = 24,
}: {
  values: number[];
  label: string;
  width?: number;
  height?: number;
}) {
  if (values.length < 2) return <span className="text-xs text-muted-foreground">—</span>;
  const step = width / (values.length - 1);
  const points = values
    .map((value, index) => {
      const y = height - 2 - Math.max(0, Math.min(1, value)) * (height - 4);
      return `${(index * step).toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg
      role="img"
      aria-label={label}
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      className="text-foreground"
    >
      <polyline points={points} fill="none" stroke="currentColor" strokeWidth={1.5} />
    </svg>
  );
}
