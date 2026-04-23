"use client";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { CapabilityId } from "@/lib/api/types";

// Known capability IDs that have configurable settings
const DOCKER_CONTAINER_ID = "docker_container";
const GPT_IMAGE_GEN_ID = "gpt_image_gen";

// Type definitions for capability-specific configs
export interface DockerContainerConfig {
  image?: string;
  working_dir?: string;
}

type GptImageGenModel = "gpt-image-2" | "gpt-image-1";
type GptImageGenQuality = "low" | "medium" | "high" | "auto";

export interface GptImageGenConfig {
  model?: GptImageGenModel;
  default_quality?: GptImageGenQuality;
}

export type CapabilityConfig = DockerContainerConfig | GptImageGenConfig | Record<string, unknown>;

interface CapabilitySettingsEditorProps {
  /** The capability ID */
  capabilityId: CapabilityId;
  /** Current config value */
  config: Record<string, unknown>;
  /** Callback when config changes */
  onChange: (config: Record<string, unknown>) => void;
  /** Whether editing is disabled */
  disabled?: boolean;
}

/**
 * Editor for capability-specific configuration.
 * Renders appropriate form fields based on the capability type.
 */
export function CapabilitySettingsEditor({
  capabilityId,
  config,
  onChange,
  disabled,
}: CapabilitySettingsEditorProps) {
  // Render appropriate editor based on capability type
  switch (capabilityId) {
    case DOCKER_CONTAINER_ID:
      return (
        <DockerContainerEditor
          config={config as DockerContainerConfig}
          onChange={onChange}
          disabled={disabled}
        />
      );
    case GPT_IMAGE_GEN_ID:
      return (
        <GptImageGenEditor
          config={config as GptImageGenConfig}
          onChange={onChange}
          disabled={disabled}
        />
      );
    default:
      // No settings editor for this capability
      return null;
  }
}

/**
 * Check if a capability has configurable settings
 */
export function hasCapabilitySettings(capabilityId: CapabilityId): boolean {
  return capabilityId === DOCKER_CONTAINER_ID || capabilityId === GPT_IMAGE_GEN_ID;
}

// ============================================================================
// Docker Container Config Editor
// ============================================================================

const DEFAULT_DOCKER_IMAGE = "mcr.microsoft.com/devcontainers/python:3.11";
const DEFAULT_WORKING_DIR = "/workspace";

interface DockerContainerEditorProps {
  config: DockerContainerConfig;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}

function DockerContainerEditor({ config, onChange, disabled }: DockerContainerEditorProps) {
  const handleImageChange = (value: string) => {
    // Only set the value if it's different from default, to keep config clean
    const newConfig = { ...config };
    if (value && value !== DEFAULT_DOCKER_IMAGE) {
      newConfig.image = value;
    } else {
      delete newConfig.image;
    }
    onChange(newConfig);
  };

  const handleWorkingDirChange = (value: string) => {
    const newConfig = { ...config };
    if (value && value !== DEFAULT_WORKING_DIR) {
      newConfig.working_dir = value;
    } else {
      delete newConfig.working_dir;
    }
    onChange(newConfig);
  };

  return (
    <div className="space-y-3 pt-2">
      <div className="space-y-1.5">
        <Label htmlFor="docker-image" className="text-xs font-normal text-muted-foreground">
          Docker Image
        </Label>
        <Input
          id="docker-image"
          placeholder={DEFAULT_DOCKER_IMAGE}
          value={config.image || ""}
          onChange={(e) => handleImageChange(e.target.value)}
          disabled={disabled}
          className="h-8 text-sm"
        />
        <p className="text-xs text-muted-foreground">Custom base image for the container</p>
      </div>

      <div className="space-y-1.5">
        <Label htmlFor="docker-working-dir" className="text-xs font-normal text-muted-foreground">
          Working Directory
        </Label>
        <Input
          id="docker-working-dir"
          placeholder={DEFAULT_WORKING_DIR}
          value={config.working_dir || ""}
          onChange={(e) => handleWorkingDirChange(e.target.value)}
          disabled={disabled}
          className="h-8 text-sm"
        />
        <p className="text-xs text-muted-foreground">
          Default working directory inside the container
        </p>
      </div>
    </div>
  );
}

// ============================================================================
// OpenAI Image Generation Config Editor
// ============================================================================

const DEFAULT_GPT_IMAGE_MODEL: GptImageGenModel = "gpt-image-2";
const DEFAULT_GPT_IMAGE_QUALITY: GptImageGenQuality = "medium";

const GPT_IMAGE_MODEL_OPTIONS: Array<{
  value: GptImageGenModel;
  label: string;
  description: string;
}> = [
  {
    value: "gpt-image-2",
    label: "ChatGPT Images 2.0",
    description: "Default. Uses OpenAI's current ChatGPT Images 2.0 API model (`gpt-image-2`).",
  },
  {
    value: "gpt-image-1",
    label: "GPT Image 1",
    description: "Legacy fallback if you need the previous image model.",
  },
];

const GPT_IMAGE_QUALITY_OPTIONS: Array<{
  value: GptImageGenQuality;
  label: string;
  description: string;
}> = [
  {
    value: "medium",
    label: "Medium",
    description: "Default. Balanced quality and latency for ChatGPT Images 2.0.",
  },
  {
    value: "low",
    label: "Low",
    description: "Fastest and cheapest. Best when throughput matters more than fidelity.",
  },
  {
    value: "high",
    label: "High",
    description: "Highest fidelity, but materially slower on `gpt-image-2`.",
  },
  {
    value: "auto",
    label: "Auto",
    description: "Let OpenAI choose the quality dynamically.",
  },
];

interface GptImageGenEditorProps {
  config: GptImageGenConfig;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}

function GptImageGenEditor({ config, onChange, disabled }: GptImageGenEditorProps) {
  const selectedModel = config.model || DEFAULT_GPT_IMAGE_MODEL;
  const selectedQuality = config.default_quality || DEFAULT_GPT_IMAGE_QUALITY;
  const selectedOption =
    GPT_IMAGE_MODEL_OPTIONS.find((option) => option.value === selectedModel) ??
    GPT_IMAGE_MODEL_OPTIONS[0];
  const selectedQualityOption =
    GPT_IMAGE_QUALITY_OPTIONS.find((option) => option.value === selectedQuality) ??
    GPT_IMAGE_QUALITY_OPTIONS[0];

  const handleModelChange = (value: string) => {
    const newConfig = { ...config };
    if (value !== DEFAULT_GPT_IMAGE_MODEL) {
      newConfig.model = value as GptImageGenModel;
    } else {
      delete newConfig.model;
    }
    onChange(newConfig);
  };

  const handleQualityChange = (value: string) => {
    const newConfig = { ...config };
    if (value !== DEFAULT_GPT_IMAGE_QUALITY) {
      newConfig.default_quality = value as GptImageGenQuality;
    } else {
      delete newConfig.default_quality;
    }
    onChange(newConfig);
  };

  return (
    <div className="space-y-3 pt-2">
      <div className="space-y-1.5">
        <Label htmlFor="gpt-image-model" className="text-xs font-normal text-muted-foreground">
          Image Model
        </Label>
        <Select value={selectedModel} onValueChange={handleModelChange} disabled={disabled}>
          <SelectTrigger id="gpt-image-model" className="w-full">
            <SelectValue>{selectedOption.label}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            {GPT_IMAGE_MODEL_OPTIONS.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground">{selectedOption.description}</p>
      </div>

      <div className="space-y-1.5">
        <Label htmlFor="gpt-image-quality" className="text-xs font-normal text-muted-foreground">
          Default Quality
        </Label>
        <Select value={selectedQuality} onValueChange={handleQualityChange} disabled={disabled}>
          <SelectTrigger id="gpt-image-quality" className="w-full">
            <SelectValue>{selectedQualityOption.label}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            {GPT_IMAGE_QUALITY_OPTIONS.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground">{selectedQualityOption.description}</p>
      </div>
    </div>
  );
}
