/**
 * Decisions:
 * - Three explicit choices instead of a light/dark switch: a user who never
 *   picked keeps following their OS, and "System" stays reachable afterwards.
 * - Lives in the user menu next to the other per-user preferences.
 */
"use client";

import { Monitor, Moon, Sun } from "lucide-react";
import {
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from "@/components/ui/dropdown-menu";
import { useTheme } from "@/providers/theme-provider";
import { parseThemeMode } from "@/lib/theme";

export function ThemeMenuSub() {
  const { mode, theme, setMode } = useTheme();
  const TriggerIcon = theme === "dark" ? Moon : Sun;

  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>
        <TriggerIcon className="icon-sharp mr-2 h-4 w-4" />
        Theme
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent className="w-40">
        <DropdownMenuRadioGroup
          value={mode}
          onValueChange={(next) => setMode(parseThemeMode(next))}
        >
          <DropdownMenuRadioItem value="light">
            <Sun className="icon-sharp h-4 w-4" />
            Light
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="dark">
            <Moon className="icon-sharp h-4 w-4" />
            Dark
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="system">
            <Monitor className="icon-sharp h-4 w-4" />
            System
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}
