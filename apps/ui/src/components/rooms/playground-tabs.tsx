"use client";

// The Playground holds two kinds of test bench: one-agent chats and rooms
// where several agents and test users talk together.

import { SectionTabs } from "@/components/layout/page-layout";
import { useUiTranslations } from "@/providers/locale-provider";

export function PlaygroundTabs({ value }: { value: "chats" | "rooms" }) {
  const ui = useUiTranslations();
  return (
    <SectionTabs
      aria-label={ui("Playground")}
      value={value}
      items={[
        { value: "chats", label: ui("Chats"), href: "/playground" },
        { value: "rooms", label: ui("Rooms"), href: "/playground/rooms" },
      ]}
    />
  );
}
