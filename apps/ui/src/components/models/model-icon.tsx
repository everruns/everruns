import type { LlmModelWithProvider, ModelVendor } from "@/lib/api/types";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { cn } from "@/lib/utils";

// Per-model vendor icons.
//
// Provider icons are keyed by provider *type*, but many notable models are
// served over OpenAI-compatible gateways (NVIDIA Nemotron, Qwen, MiniMax, Kimi,
// Grok, Microsoft MAI, ...) where the provider type alone would render an
// OpenAI mark. The backend model registry tags each model with a vendor
// (`model_vendor`); we map that tag to a brand icon and fall back to the
// provider-type icon for first-party vendors (OpenAI, Anthropic, Google) and
// unknown models.

function NvidiaIcon({ size }: { size: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M8.948 8.798v-1.43a6.7 6.7 0 0 1 .424-.018c3.922-.124 6.493 3.374 6.493 3.374s-2.774 3.851-5.75 3.851c-.398 0-.787-.062-1.158-.185v-4.346c1.528.185 1.837.857 2.747 2.385l2.04-1.714s-1.492-1.952-4-1.952a6.016 6.016 0 0 0-.796.035m0-4.735v2.138l.424-.027c5.45-.185 9.01 4.47 9.01 4.47s-4.08 4.964-8.33 4.964c-.37 0-.733-.035-1.095-.097v1.325c.3.035.61.062.91.062 3.957 0 6.82-2.023 9.593-4.408.459.371 2.34 1.263 2.73 1.652-2.633 2.208-8.772 3.984-12.253 3.984-.335 0-.653-.018-.971-.053v1.864H24V4.063zm0 10.326v1.131c-3.657-.654-4.673-4.46-4.673-4.46s1.758-1.944 4.673-2.262v1.237H8.94c-1.528-.186-2.73 1.245-2.73 1.245s.68 2.412 2.739 3.11M2.456 10.9s2.164-3.197 6.5-3.533V6.201C4.153 6.59 0 10.653 0 10.653s2.35 6.802 8.948 7.42v-1.237c-4.84-.6-6.492-5.936-6.492-5.936z" />
    </svg>
  );
}

function QwenIcon({ size }: { size: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M23.919 14.545 20.817 9.17l1.47-2.544a.56.56 0 0 0 0-.566l-1.633-2.83a.57.57 0 0 0-.49-.283h-6.207L12.487.402a.57.57 0 0 0-.49-.284H8.732a.56.56 0 0 0-.49.284L5.139 5.775h-2.94a.56.56 0 0 0-.49.284L.077 8.887a.56.56 0 0 0 0 .567L3.18 14.83l-1.47 2.545a.56.56 0 0 0 0 .566l1.634 2.83a.57.57 0 0 0 .49.283h6.205l1.47 2.545a.57.57 0 0 0 .49.284h3.266a.57.57 0 0 0 .49-.284l3.104-5.375h2.94a.57.57 0 0 0 .49-.283l1.634-2.828a.55.55 0 0 0-.004-.568M8.733.686l1.634 2.828-1.634 2.828H21.8L20.164 9.17H7.425L5.63 6.06Zm1.306 19.801-6.205-.002 1.634-2.83h3.265L2.201 6.344h3.267q3.182 5.517 6.367 11.032zm10.124-5.66L18.53 12l-6.532 11.315-1.634-2.83c2.129-3.673 4.25-7.351 6.373-11.028h3.592l3.102 5.374z" />
    </svg>
  );
}

function XaiIcon({ size }: { size: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M14.234 10.162 22.977 0h-2.072l-7.591 8.824L7.251 0H.258l9.168 13.343L.258 24H2.33l8.016-9.318L16.749 24h6.993zm-2.837 3.299-.929-1.329L3.076 1.56h3.182l5.965 8.532.929 1.329 7.754 11.09h-3.182z" />
    </svg>
  );
}

function MicrosoftIcon({ size }: { size: number }) {
  // Microsoft's four-square mark, rendered monochrome with a gap.
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M2 2h9.2v9.2H2zM12.8 2H22v9.2h-9.2zM2 12.8h9.2V22H2zM12.8 12.8H22V22h-9.2z" />
    </svg>
  );
}

function MiniMaxIcon({ size }: { size: number }) {
  // Lettermark "M" — MiniMax has no published monochrome glyph icon.
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M3 20V4h3.2l5.8 8 5.8-8H21v16h-3.3V9.4L12 17.2 6.3 9.4V20z" />
    </svg>
  );
}

function MoonshotIcon({ size }: { size: number }) {
  // Crescent moon for Moonshot AI (Kimi).
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
    </svg>
  );
}

interface VendorIcon {
  label: string;
  Component: React.ComponentType<{ size: number }>;
}

// Brand icons for vendors that lack a dedicated provider-type icon. First-party
// vendors (openai, anthropic, google, llmsim) intentionally fall through to the
// provider icon, which already renders their brand.
const VENDOR_ICONS: Partial<Record<ModelVendor, VendorIcon>> = {
  nvidia: { label: "NVIDIA", Component: NvidiaIcon },
  qwen: { label: "Qwen", Component: QwenIcon },
  microsoft: { label: "Microsoft", Component: MicrosoftIcon },
  minimax: { label: "MiniMax", Component: MiniMaxIcon },
  moonshot: { label: "Moonshot", Component: MoonshotIcon },
  xai: { label: "xAI", Component: XaiIcon },
};

const sizeMap = {
  sm: { icon: 16, container: "p-1.5" },
  md: { icon: 20, container: "p-2" },
  lg: { icon: 24, container: "p-2.5" },
};

type ModelLike = Pick<LlmModelWithProvider, "provider_type"> & {
  model_vendor?: ModelVendor | null;
};

interface ModelIconProps {
  model: ModelLike;
  size?: "sm" | "md" | "lg";
  className?: string;
  showBackground?: boolean;
}

/**
 * Icon for a specific model. Prefers a per-vendor brand icon from the model's
 * registry `model_vendor` tag, falling back to the provider-type icon (OpenAI,
 * Anthropic, Gemini, ...).
 */
export function ModelIcon({
  model,
  size = "md",
  className,
  showBackground = true,
}: ModelIconProps) {
  const vendor = model.model_vendor ? VENDOR_ICONS[model.model_vendor] : undefined;
  if (!vendor) {
    return (
      <ProviderIcon
        providerType={model.provider_type}
        size={size}
        className={className}
        showBackground={showBackground}
      />
    );
  }

  const { icon: iconSize, container } = sizeMap[size];
  const { label, Component } = vendor;
  return (
    <div className={cn(showBackground && "bg-primary/10", container, className)} title={label}>
      <Component size={iconSize} />
    </div>
  );
}
