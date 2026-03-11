/**
 * Global command palette state + keyboard shortcut.
 *
 * Opens with Cmd+K (Mac) / Ctrl+K (other). Provides a context so any
 * component in the tree can open/close the palette programmatically.
 */
"use client";

import { createContext, useContext, useEffect, useState } from "react";

export interface CommandPaletteState {
  open: boolean;
  setOpen: (open: boolean) => void;
}

export const CommandPaletteContext = createContext<CommandPaletteState>({
  open: false,
  setOpen: () => {},
});

export function useCommandPaletteState(): CommandPaletteState {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setOpen((prev) => !prev);
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  return { open, setOpen };
}

export function useCommandPalette(): CommandPaletteState {
  return useContext(CommandPaletteContext);
}
