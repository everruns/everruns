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
    <div className="flex flex-wrap gap-2 border border-border bg-background px-3 py-3">
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
    <div className="relative group">
      <div className="h-20 w-20 overflow-hidden border border-border bg-muted/20">
        {image.status === "error" ? (
          <div className="flex h-full w-full flex-col items-center justify-center p-1 text-destructive">
            <AlertCircle className="w-6 h-6 mb-1" />
            <span className="text-[10px] text-center line-clamp-2">{image.error || "Error"}</span>
          </div>
        ) : (
          <>
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img src={displayUrl} alt={image.filename} className="w-full h-full object-cover" />
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
        className="absolute -right-2 -top-2 flex h-5 w-5 items-center justify-center border border-border bg-background opacity-0 transition-opacity group-hover:opacity-100 hover:bg-muted"
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
            className="max-h-[200px] max-w-[200px] border border-border hover:opacity-90 transition-opacity"
            title={filename || "Click to view full size"}
          />
        </button>
        {filename && (
          <span className="text-xs text-muted-foreground mt-1 block truncate max-w-[200px]">
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
    <div className="inline-flex h-20 w-20 flex-col items-center justify-center border border-border bg-muted/20">
      <ImageIcon className="w-6 h-6 text-muted-foreground" />
      {filename && (
        <span className="text-[10px] text-muted-foreground mt-1 truncate max-w-[70px]">
          {filename}
        </span>
      )}
    </div>
  );
}
