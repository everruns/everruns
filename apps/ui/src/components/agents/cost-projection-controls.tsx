"use client";
import { useState } from "react";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { formatPlanningUsd } from "@/lib/agent-cost-projection";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
export function VolumeControl({
  label,
  id,
  value,
  setValue,
  max,
  sliderMax,
}: {
  label: string;
  id: string;
  value: string;
  setValue: (v: string) => void;
  max: number;
  sliderMax: number;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  // Keep the scale stable while dragging after an exact input expands it.
  const [rangeMax, setRangeMax] = useState(() =>
    Math.max(sliderMax, Math.min(max, Number(value) || 0)),
  );
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-4">
        <div>
          <Label htmlFor={id}>{ui(label)}</Label>
          <p className="mt-1 text-xs text-muted-foreground">
            0–{max.toLocaleString(displayLocale)}
          </p>
        </div>
        <Input
          className="w-32"
          id={id}
          type="number"
          min={0}
          max={max}
          step={1}
          value={value}
          onChange={(e) => {
            const next = Number(e.target.value);
            if (Number.isFinite(next) && next > rangeMax && next <= max) setRangeMax(next);
            setValue(e.target.value);
          }}
        />
      </div>
      <input
        aria-label={ui("{label} slider", { label: ui(label) })}
        type="range"
        min={0}
        max={rangeMax}
        value={Number(value) || 0}
        onChange={(e) => setValue(e.target.value)}
        className="w-full accent-primary"
      />
    </div>
  );
}
export function ProjectionChart({
  typical,
  low,
  high,
  daily,
}: {
  typical: number;
  low: number | null;
  high: number | null;
  daily: number;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const maxDaily = Math.max(1, daily * 2);
  const ceiling = Math.max(0.0001, (high ?? typical) * maxDaily * 30);
  const y = (cost: number, volume: number) => 120 - ((cost * volume * 30) / ceiling) * 100;
  return (
    <div>
      <svg
        viewBox="0 0 600 150"
        role="img"
        aria-label={ui("Monthly cost projection with lighter and heavier task scenarios")}
        className="w-full text-primary"
      >
        <line x1="15" y1="120" x2="585" y2="120" stroke="currentColor" opacity="0.2" />
        {low != null && high != null && (
          <path
            d={`M15,120 L585,${y(high, maxDaily)} L585,${y(low, maxDaily)} Z`}
            fill="currentColor"
            opacity="0.12"
          />
        )}
        <path
          d={`M15,120 L585,${y(typical, maxDaily)}`}
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
        />
        <circle
          cx={15 + (570 * daily) / maxDaily}
          cy={y(typical, daily)}
          r="4"
          fill="currentColor"
        />
        <text x="15" y="142" fontSize="11" fill="currentColor">
          0
        </text>
        <text x="585" y="142" textAnchor="end" fontSize="11" fill="currentColor">
          {ui("{count} tasks / day", { count: maxDaily.toLocaleString(displayLocale) })}
        </text>
        <text x="585" y="12" textAnchor="end" fontSize="11" fill="currentColor">
          {formatPlanningUsd(ceiling, displayLocale)}
        </text>
      </svg>
      <div className="flex justify-between gap-4 text-xs text-muted-foreground">
        <span>{ui("At current usage")}</span>
        <span>
          {formatPlanningUsd(typical * daily * 30, displayLocale)} {ui("/ month")}
        </span>
      </div>
    </div>
  );
}
