"use client";

import { useState } from "react";
import { X, Loader2, AlertCircle, ImageIcon, Download } from "lucide-react";
import type { PendingImage } from "@/lib/api/images";
import { getThumbnailUrl, getImageUrl } from "@/lib/api/images";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

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
    <div className="animate-chat-surface-in flex flex-wrap gap-3">
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
  const displayUrl = image.imageId ? getThumbnailUrl(image.imageId) : image.previewUrl;

  return (
    <div className="group relative w-[88px]">
      <div className="relative flex aspect-square items-center justify-center overflow-hidden border border-border/70 bg-card shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
        {image.status === "error" ? (
          <div className="flex h-full w-full flex-col items-center justify-center p-2 text-destructive">
            <AlertCircle className="mb-1 h-6 w-6" />
            <span className="line-clamp-2 text-center text-[10px]">{image.error || "Error"}</span>
          </div>
        ) : (
          <>
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img src={displayUrl} alt={image.filename} className="h-full w-full object-cover" />
            {image.status === "uploading" && (
              <div className="absolute inset-0 flex items-center justify-center bg-background/72 backdrop-blur-[1px]">
                <Loader2 className="h-6 w-6 animate-spin text-primary/80" />
              </div>
            )}
          </>
        )}
      </div>
      <button
        type="button"
        className="absolute -right-1.5 -top-1.5 flex h-5 w-5 items-center justify-center border border-border/80 bg-background opacity-0 transition-opacity group-hover:opacity-100 hover:bg-muted"
        onClick={() => onRemove(image.tempId)}
        aria-label={`Remove ${image.filename}`}
      >
        <X className="h-3 w-3 text-muted-foreground" />
      </button>
      <div className="mt-1 truncate text-[11px] text-muted-foreground" title={image.filename}>
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
  const [isOpen, setIsOpen] = useState(false);
  const thumbnailUrl = getThumbnailUrl(imageId);
  const fullImageUrl = getImageUrl(imageId);

  const handleDownload = () => {
    const link = document.createElement("a");
    link.href = fullImageUrl;
    link.download = filename || `image-${imageId}`;
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
  };

  return (
    <>
      <div className="inline-block">
        <button type="button" onClick={() => setIsOpen(true)} className="block cursor-pointer">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            src={thumbnailUrl}
            alt={filename || "Image"}
            className="max-h-[200px] max-w-[200px] border border-border/70 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)] transition-opacity hover:opacity-90"
            title={filename || "Click to view full size"}
          />
        </button>
        {filename && (
          <span className="mt-1 block max-w-[200px] truncate text-xs text-muted-foreground">
            {filename}
          </span>
        )}
      </div>

      <Dialog open={isOpen} onOpenChange={setIsOpen}>
        <DialogContent className="max-w-4xl max-h-[90vh] p-0 overflow-hidden">
          <DialogTitle className="sr-only">{filename || "Image preview"}</DialogTitle>
          <div className="relative flex flex-col">
            {/* Image container */}
            <div className="flex-1 overflow-auto p-4 flex items-center justify-center bg-muted/30">
              {/* eslint-disable-next-line @next/next/no-img-element */}
              <img
                src={fullImageUrl}
                alt={filename || "Image"}
                className="max-w-full max-h-[70vh] object-contain rounded"
              />
            </div>
            {/* Footer with filename and download */}
            <div className="flex items-center justify-between p-3 border-t bg-background">
              <span className="text-sm text-muted-foreground truncate max-w-[60%]">
                {filename || "Image"}
              </span>
              <Button
                variant="outline"
                size="sm"
                onClick={handleDownload}
                className="flex items-center gap-2"
              >
                <Download className="w-4 h-4" />
                Download
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

/**
 * Placeholder for image that failed to load
 */
export function ImagePlaceholder({ filename }: { filename?: string }) {
  return (
    <div className="inline-flex h-20 w-20 flex-col items-center justify-center border border-border/70 bg-card shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
      <ImageIcon className="h-6 w-6 text-muted-foreground" />
      {filename && (
        <span className="mt-1 max-w-[70px] truncate text-[10px] text-muted-foreground">
          {filename}
        </span>
      )}
    </div>
  );
}
