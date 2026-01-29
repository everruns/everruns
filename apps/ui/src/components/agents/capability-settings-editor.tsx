"use client";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { CapabilityId } from "@/lib/api/types";

// Known capability IDs that have configurable settings
const DOCKER_CONTAINER_ID = "docker_container";

// Type definitions for capability-specific configs
export interface DockerContainerConfig {
  image?: string;
  working_dir?: string;
}

export type CapabilityConfig = DockerContainerConfig | Record<string, unknown>;

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
    default:
      // No settings editor for this capability
      return null;
  }
}

/**
 * Check if a capability has configurable settings
 */
export function hasCapabilitySettings(capabilityId: CapabilityId): boolean {
  return capabilityId === DOCKER_CONTAINER_ID;
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
