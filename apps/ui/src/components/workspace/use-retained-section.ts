"use client";

import { useState } from "react";

// A settings sheet clears its section as soon as it starts closing. Keep the
// last section so a wide sheet does not shrink, and its content does not
// vanish, while the close animation is still on screen.
export function useRetainedSection<T>(section: T | null): T | null {
  const [retained, setRetained] = useState(section);
  if (section !== null && section !== retained) setRetained(section);
  return section ?? retained;
}
