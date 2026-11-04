"use client";
import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

// Keep native range keyboard and form semantics, with Slate's shared focus/color.
export function Slider({ className, ...props }: Omit<ComponentProps<"input">, "type">) {
  return (
    <input
      {...props}
      type="range"
      className={cn(
        "h-8 w-full cursor-pointer accent-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
    />
  );
}
