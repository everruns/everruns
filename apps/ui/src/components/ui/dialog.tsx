"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import * as React from "react";
import { Dialog as DialogPrimitive } from "@base-ui/react/dialog";
import { XIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { usePageChatMode } from "@/components/chat-mode/chat-mode-context";
import { ManualPageContext } from "@/components/chat-mode/manual-page-context";

function Dialog({ ...props }: DialogPrimitive.Root.Props) {
  const mode = usePageChatMode();
  return (
    <DialogPrimitive.Root
      data-slot="dialog"
      {...props}
      modal={mode?.open ? false : props.modal}
      disablePointerDismissal={mode?.open || props.disablePointerDismissal}
    />
  );
}

function DialogTrigger({
  children,
  asChild,
  ...props
}: DialogPrimitive.Trigger.Props & { children?: React.ReactNode; asChild?: boolean }) {
  if (asChild && React.isValidElement(children)) {
    return <DialogPrimitive.Trigger data-slot="dialog-trigger" render={children} {...props} />;
  }
  return (
    <DialogPrimitive.Trigger data-slot="dialog-trigger" {...props}>
      {children}
    </DialogPrimitive.Trigger>
  );
}

function DialogPortal({ ...props }: DialogPrimitive.Portal.Props) {
  const mode = usePageChatMode();
  // Keep the portal container stable when Chat opens: moving a mounted form
  // between body and pane would remount it and discard unsaved React state.
  return (
    <DialogPrimitive.Portal
      data-slot="dialog-portal"
      {...props}
      container={
        props.container ?? mode?.pageContainerElement ?? mode?.pageContainer.current ?? undefined
      }
    />
  );
}

function DialogClose({ ...props }: DialogPrimitive.Close.Props) {
  return <DialogPrimitive.Close data-slot="dialog-close" {...props} />;
}

function DialogOverlay({ className, ...props }: DialogPrimitive.Backdrop.Props) {
  return (
    <DialogPrimitive.Backdrop
      data-slot="dialog-overlay"
      className={cn(
        "data-[open]:animate-in data-[closed]:animate-out data-[closed]:fade-out-0 data-[open]:fade-in-0 data-[closed]:animation-duration-[200ms] fixed inset-0 z-50 bg-black/50",
        className,
      )}
      {...props}
    />
  );
}

function DialogContent({
  className,
  children,
  showCloseButton = true,
  chatProtected,
  ...props
}: DialogPrimitive.Popup.Props & {
  showCloseButton?: boolean;
  chatProtected?: string;
}) {
  const ui = useUiTranslations();
  const mode = usePageChatMode();
  const scoped = !!(mode?.pageContainerElement ?? mode?.pageContainer.current);
  return (
    <DialogPortal data-slot="dialog-portal">
      <DialogOverlay className={scoped ? "absolute" : undefined} />
      <DialogPrimitive.Popup
        data-slot="dialog-content"
        className={cn(
          "bg-background data-[open]:animate-in data-[open]:fade-in-0 data-[open]:zoom-in-95 data-[closed]:animate-out data-[closed]:fade-out-0 data-[closed]:zoom-out-95 fixed top-[50%] left-[50%] z-50 grid w-full max-w-[calc(100%-2rem)] translate-x-[-50%] translate-y-[-50%] gap-4 border p-6 shadow-lg duration-200 sm:max-w-lg",
          className,
          scoped && "absolute",
        )}
        {...props}
      >
        {chatProtected && <ManualPageContext label={chatProtected} />}
        {children}
        {showCloseButton && (
          <DialogPrimitive.Close
            data-slot="dialog-close"
            className="ring-offset-background focus:ring-ring data-[open]:bg-muted data-[open]:text-muted-foreground absolute top-4 right-4 opacity-70 transition-opacity hover:opacity-100 focus:ring-2 focus:ring-offset-2 focus:outline-hidden disabled:pointer-events-none [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4"
          >
            <XIcon />
            <span className="sr-only">{ui("Close")}</span>
          </DialogPrimitive.Close>
        )}
      </DialogPrimitive.Popup>
    </DialogPortal>
  );
}

function DialogHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-header"
      className={cn("flex flex-col gap-2 text-center sm:text-left", className)}
      {...props}
    />
  );
}

function DialogFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-footer"
      className={cn("flex flex-col-reverse gap-2 sm:flex-row sm:justify-end", className)}
      {...props}
    />
  );
}

function DialogTitle({ className, ...props }: DialogPrimitive.Title.Props) {
  return (
    <DialogPrimitive.Title
      data-slot="dialog-title"
      className={cn("text-lg leading-none font-semibold", className)}
      {...props}
    />
  );
}

function DialogDescription({ className, ...props }: DialogPrimitive.Description.Props) {
  return (
    <DialogPrimitive.Description
      data-slot="dialog-description"
      className={cn("text-muted-foreground text-sm", className)}
      {...props}
    />
  );
}

export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
};
