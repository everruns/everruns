"use client";

// Avatar picker for the Branding sheet: drop or choose an image, crop it to a
// square, then upload. Saves immediately rather than with the page draft: it
// is a file, not a field, and the server renders the presets on upload.
//
// Decision: cropping happens in the browser with react-easy-crop (MIT, one
// small dependency) and only the cropped square is uploaded, as PNG at up to
// 1024 px. The server still center-crops whatever it receives, so API uploads
// without this step behave the same, just without the user's framing.

import { useCallback, useEffect, useRef, useState, type DragEvent } from "react";
import Cropper, { type Area } from "react-easy-crop";
import { Grid2X2, ImageIcon, Loader2, Minus, Plus, Trash2, Upload } from "lucide-react";
import { useAgentAvatar } from "@/hooks";
import { AvatarPresetDialog } from "@/components/agents/avatar-preset-dialog";
import { AgentAvatar } from "@/components/agents/agent-avatar";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { Agent } from "@/lib/api/types";
import { cn } from "@/lib/utils";

export const AVATAR_ACCEPT = ["image/png", "image/jpeg", "image/gif", "image/webp"];
const MAX_AVATAR_BYTES = 10 * 1024 * 1024;
const MIN_AVATAR_EDGE = 64;
const OUTPUT_MAX_EDGE = 1024;

/** Why a picked file cannot be used, or `null` when it can. */
export function avatarFileError(file: File): string | null {
  if (!AVATAR_ACCEPT.includes(file.type)) return "Choose a PNG, JPEG, GIF or WebP image.";
  if (file.size > MAX_AVATAR_BYTES) return "The image must be at most 10 MB.";
  return null;
}

/** Draw the selected square of `src` to a PNG of at most 1024 px. */
async function cropToPng(src: string, area: Area): Promise<File> {
  const image = new Image();
  image.src = src;
  await image.decode();
  const edge = Math.min(OUTPUT_MAX_EDGE, Math.round(area.width));
  const canvas = document.createElement("canvas");
  canvas.width = edge;
  canvas.height = edge;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("Your browser cannot crop images.");
  context.imageSmoothingQuality = "high";
  context.drawImage(image, area.x, area.y, area.width, area.height, 0, 0, edge, edge);
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) throw new Error("Could not crop the image.");
  return new File([blob], "avatar.png", { type: "image/png" });
}

export function AgentAvatarField({ agent, readOnly }: { agent: Agent; readOnly: boolean }) {
  const inputRef = useRef<HTMLInputElement>(null);
  const { upload, remove, selectPreset } = useAgentAvatar(agent.id);
  const [presetsOpen, setPresetsOpen] = useState(false);
  const [source, setSource] = useState<string | null>(null);
  const [pickError, setPickError] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const busy = upload.isPending || remove.isPending || selectPreset.isPending;
  const disabled = readOnly || busy;
  const error = pickError ?? (upload.error ?? remove.error ?? selectPreset.error)?.message ?? null;

  // Object URLs hold the whole file in memory until revoked.
  useEffect(() => () => void (source && URL.revokeObjectURL(source)), [source]);

  const pick = (file: File | undefined) => {
    if (!file || disabled) return;
    const problem = avatarFileError(file);
    setPickError(problem);
    if (problem) return;
    upload.reset();
    selectPreset.reset();
    setSource(URL.createObjectURL(file));
  };

  const onDrop = (event: DragEvent) => {
    event.preventDefault();
    setDragging(false);
    if (!disabled) pick(event.dataTransfer.files?.[0]);
  };

  return (
    <div className="space-y-2">
      <Label>Avatar</Label>
      <div className="flex items-center gap-4">
        <button
          type="button"
          disabled={disabled}
          onClick={() => inputRef.current?.click()}
          onDragOver={(event) => {
            event.preventDefault();
            if (!disabled) setDragging(true);
          }}
          onDragLeave={() => setDragging(false)}
          onDrop={onDrop}
          aria-label={agent.avatar ? "Replace avatar" : "Upload avatar"}
          className={cn(
            "flex size-20 flex-none items-center justify-center border border-dashed bg-muted/40 text-muted-foreground transition-colors [&_svg]:size-6",
            !disabled && "cursor-pointer hover:border-foreground/40",
            dragging && "border-primary bg-primary/10",
            agent.avatar && "border-solid",
          )}
        >
          {busy ? (
            <Loader2 className="animate-spin" />
          ) : (
            <AgentAvatar avatar={agent.avatar} size={78} fallback={<ImageIcon />} />
          )}
        </button>
        <div className="flex min-w-0 flex-col gap-2">
          <p className="text-xs text-muted-foreground">
            {readOnly ? "Avatar" : "Drop an image here, or choose one."}
          </p>
          <div className="flex flex-wrap gap-2">
            <input
              ref={inputRef}
              type="file"
              accept={AVATAR_ACCEPT.join(",")}
              className="hidden"
              disabled={disabled}
              aria-label="Avatar image"
              onChange={(event) => {
                pick(event.target.files?.[0]);
                event.target.value = "";
              }}
            />
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={disabled}
              onClick={() => inputRef.current?.click()}
            >
              <Upload />
              {agent.avatar ? "Replace" : "Upload"}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={disabled}
              onClick={() => {
                setPickError(null);
                upload.reset();
                selectPreset.reset();
                setPresetsOpen(true);
              }}
            >
              <Grid2X2 />
              Choose preset
            </Button>
            {agent.avatar && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={disabled}
                onClick={() => remove.mutate()}
              >
                <Trash2 />
                Remove
              </Button>
            )}
          </div>
        </div>
      </div>
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
      <p className="text-xs text-muted-foreground">
        PNG, JPEG, GIF or WebP, at least 64×64. Shown in the UI, the A2A Agent Card, and as the icon
        of Slack apps created in one click.
      </p>
      <AvatarPresetDialog
        agent={agent}
        open={presetsOpen}
        disabled={disabled}
        saving={selectPreset.isPending}
        error={selectPreset.error?.message ?? null}
        onClose={() => setPresetsOpen(false)}
        onSave={async (id) => {
          if (disabled) return;
          await selectPreset.mutateAsync(id);
          setPresetsOpen(false);
        }}
      />
      {/* Keyed by image so each one starts centered and unzoomed. */}
      <AvatarCropDialog
        key={source ?? "none"}
        source={source}
        saving={upload.isPending}
        error={upload.error?.message ?? null}
        onCancel={() => setSource(null)}
        onSave={async (file) => {
          await upload.mutateAsync(file);
          setSource(null);
        }}
      />
    </div>
  );
}

function AvatarCropDialog({
  source,
  saving,
  error,
  onCancel,
  onSave,
}: {
  source: string | null;
  saving: boolean;
  error: string | null;
  onCancel: () => void;
  onSave: (file: File) => Promise<void>;
}) {
  const [crop, setCrop] = useState({ x: 0, y: 0 });
  const [zoom, setZoom] = useState(1);
  const [area, setArea] = useState<Area | null>(null);
  const [cropError, setCropError] = useState<string | null>(null);

  const onCropComplete = useCallback((_: Area, pixels: Area) => setArea(pixels), []);
  const tooSmall = area !== null && Math.round(area.width) < MIN_AVATAR_EDGE;

  const save = async () => {
    if (!source || !area) return;
    try {
      setCropError(null);
      await onSave(await cropToPng(source, area));
    } catch (err) {
      setCropError(err instanceof Error ? err.message : "Could not save the avatar.");
    }
  };

  return (
    <Dialog open={source !== null} onOpenChange={(open) => !open && !saving && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Crop avatar</DialogTitle>
          <DialogDescription>Drag to position, scroll or use the slider to zoom.</DialogDescription>
        </DialogHeader>
        <div className="relative aspect-square w-full overflow-hidden bg-muted">
          {source && (
            <Cropper
              image={source}
              crop={crop}
              zoom={zoom}
              minZoom={1}
              maxZoom={5}
              aspect={1}
              cropShape="rect"
              showGrid
              onCropChange={setCrop}
              onZoomChange={setZoom}
              onCropComplete={onCropComplete}
            />
          )}
        </div>
        <div className="flex items-center gap-3">
          <Minus className="size-4 text-muted-foreground" aria-hidden />
          <input
            type="range"
            min={1}
            max={5}
            step={0.01}
            value={zoom}
            onChange={(event) => setZoom(Number(event.target.value))}
            aria-label="Zoom"
            className="h-1 flex-1 cursor-pointer accent-primary"
          />
          <Plus className="size-4 text-muted-foreground" aria-hidden />
        </div>
        {tooSmall && (
          <p className="text-xs text-destructive">
            The selection is smaller than 64×64 pixels. Zoom out.
          </p>
        )}
        {(cropError ?? error) && <p className="text-xs text-destructive">{cropError ?? error}</p>}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={onCancel} disabled={saving}>
            Cancel
          </Button>
          <Button type="button" onClick={save} disabled={saving || !area || tooSmall}>
            {saving && <Loader2 className="animate-spin" />}
            Save avatar
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
