"use client";

import type { ReactNode } from "react";
import { useTheme } from "@/providers/theme-provider";
import type { McpServerIcon } from "@/lib/api/mcp-server-types";

const RASTER_DATA = /^data:image\/(?:png|jpeg|jpg|webp|gif);base64,[A-Za-z0-9+/=]+$/;
const MAX_DATA_URI = 32 * 1024 + 64;

/** Same-origin https and small raster data URIs are the only icons we paint. */
export function safeMcpIconSrc(src: string | null | undefined): string | null {
  if (!src) return null;
  if (src.startsWith("https://") && !src.includes("@")) return src;
  if (src.startsWith("data:") && src.length <= MAX_DATA_URI && RASTER_DATA.test(src)) return src;
  return null;
}

export function pickMcpIconSrc(
  icons: McpServerIcon[] | null | undefined,
  theme: "light" | "dark",
  iconUrl?: string | null,
): string | null {
  const themed = icons?.find((icon) => icon.theme === theme);
  const neutral = icons?.find((icon) => !icon.theme);
  return (
    safeMcpIconSrc(themed?.src) ??
    safeMcpIconSrc(neutral?.src) ??
    safeMcpIconSrc(icons?.[0]?.src) ??
    safeMcpIconSrc(iconUrl)
  );
}

export function McpServerMark({
  icons,
  iconUrl,
  fallback,
  className,
}: {
  icons?: McpServerIcon[] | null;
  iconUrl?: string | null;
  fallback: ReactNode;
  className?: string;
}) {
  const { theme } = useTheme();
  const src = pickMcpIconSrc(icons, theme, iconUrl);
  if (!src) return fallback;
  return <img src={src} alt="" className={className ?? "size-5 object-contain"} />;
}
