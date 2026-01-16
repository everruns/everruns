"use client";

import { X, Loader2, AlertCircle, ImageIcon } from "lucide-react";
import type { PendingImage } from "@/lib/api/images";
import { getThumbnailUrl } from "@/lib/api/images";

interface ImageAttachmentsProps {
  images: PendingImage[];
  onRemove: (tempId: string) => void;
}

/**
 * Component to display pending image attachments in the chat input area
 */
export function ImageAttachments({ images, onRemove }: ImageAttachmentsProps) {
  if (images.length === 0) return null;

  return (
    <div className="flex flex-wrap gap-2 p-2 bg-muted/30 rounded-lg">
      {images.map((img) => (
        <ImageAttachmentItem key={img.tempId} image={img} onRemove={onRemove} />
      ))}
    </div>
  );
}

interface ImageAttachmentItemProps {
  image: PendingImage;
  onRemove: (tempId: string) => void;
}

function ImageAttachmentItem({ image, onRemove }: ImageAttachmentItemProps) {
  // Use thumbnail URL if uploaded, otherwise use object URL preview
  const displayUrl = image.imageId
    ? getThumbnailUrl(image.imageId)
    : image.previewUrl;

  return (
    <div className="relative group">
      <div className="w-20 h-20 rounded-md overflow-hidden bg-muted border">
        {image.status === "error" ? (
          <div className="w-full h-full flex flex-col items-center justify-center text-destructive p-1">
            <AlertCircle className="w-6 h-6 mb-1" />
            <span className="text-[10px] text-center line-clamp-2">{image.error || "Error"}</span>
          </div>
        ) : (
          <>
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img
              src={displayUrl}
              alt={image.filename}
              className="w-full h-full object-cover"
            />
            {image.status === "uploading" && (
              <div className="absolute inset-0 bg-black/50 flex items-center justify-center">
                <Loader2 className="w-6 h-6 text-white animate-spin" />
              </div>
            )}
          </>
        )}
      </div>
      {/* Remove button - monochrome style */}
      <button
        type="button"
        className="absolute -top-2 -right-2 w-5 h-5 rounded-full bg-background border border-border flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity hover:bg-muted"
        onClick={() => onRemove(image.tempId)}
      >
        <X className="w-3 h-3 text-muted-foreground" />
      </button>
      {/* Filename tooltip */}
      <div className="absolute bottom-0 left-0 right-0 bg-black/70 text-white text-[10px] px-1 py-0.5 truncate opacity-0 group-hover:opacity-100 transition-opacity">
        {image.filename}
      </div>
    </div>
  );
}

/**
 * Component to display image content in a message (for viewing uploaded images)
 */
interface MessageImageProps {
  imageId: string;
  filename?: string;
}

export function MessageImage({ imageId, filename }: MessageImageProps) {
  const thumbnailUrl = getThumbnailUrl(imageId);

  return (
    <div className="inline-block">
      {/* API endpoint, not a Next.js page - using <a> is intentional */}
      {/* eslint-disable-next-line @next/next/no-html-link-for-pages */}
      <a
        href={`/v1/images/${imageId}/data`}
        target="_blank"
        rel="noopener noreferrer"
        className="block"
      >
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src={thumbnailUrl}
          alt={filename || "Image"}
          className="max-w-[200px] max-h-[200px] rounded-md border hover:opacity-90 transition-opacity"
          title={filename || "Click to view full size"}
        />
      </a>
      {filename && (
        <span className="text-xs text-muted-foreground mt-1 block truncate max-w-[200px]">
          {filename}
        </span>
      )}
    </div>
  );
}

/**
 * Placeholder for image that failed to load
 */
export function ImagePlaceholder({ filename }: { filename?: string }) {
  return (
    <div className="inline-flex flex-col items-center justify-center w-20 h-20 bg-muted rounded-md border">
      <ImageIcon className="w-6 h-6 text-muted-foreground" />
      {filename && (
        <span className="text-[10px] text-muted-foreground mt-1 truncate max-w-[70px]">
          {filename}
        </span>
      )}
    </div>
  );
}
