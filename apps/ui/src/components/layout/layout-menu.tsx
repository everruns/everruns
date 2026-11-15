/**
 * Decisions:
 * - Layout is a per-person preference next to Theme, so it is one click from
 *   either layout (knowledge/ui/chat-mode.md).
 */
"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { Columns2, Maximize2, PanelLeft, PanelRight, PictureInPicture2 } from "lucide-react";
import {
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from "@/components/ui/dropdown-menu";
import {
  parseAppLayout,
  parseClassicChat,
  useClassicChatPreference,
  useLayoutPreference,
} from "@/hooks/use-layout-preference";

export function LayoutMenuSub() {
  const ui = useUiTranslations();
  const { layout, setLayout } = useLayoutPreference();
  const { classicChat, setClassicChat } = useClassicChatPreference();
  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>
        <Columns2 className="icon-sharp mr-2 h-4 w-4" />
        {ui("Layout")}
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent className="w-44">
        <DropdownMenuRadioGroup
          value={layout}
          onValueChange={(next) => setLayout(parseAppLayout(next))}
        >
          <DropdownMenuRadioItem value="chat_first">
            <Columns2 className="icon-sharp h-4 w-4" />
            {ui("Chat first")}
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="classic">
            <PanelLeft className="icon-sharp h-4 w-4" />
            {ui("Classic")}
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
        {layout === "classic" && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuRadioGroup
              value={classicChat}
              onValueChange={(next) => setClassicChat(parseClassicChat(next))}
            >
              <DropdownMenuLabel>{ui("Chat in Classic")}</DropdownMenuLabel>
              <DropdownMenuRadioItem value="full_screen">
                <Maximize2 className="icon-sharp h-4 w-4" />
                {ui("Full screen")}
              </DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="floating">
                <PictureInPicture2 className="icon-sharp h-4 w-4" />
                {ui("Floating")}
              </DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="docked">
                <PanelRight className="icon-sharp h-4 w-4" />
                {ui("Docked")}
              </DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
          </>
        )}
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}
