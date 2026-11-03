"use client";

import { createContext, useContext } from "react";
import { useQuery } from "@tanstack/react-query";

// Embedded views cannot navigate to cookie-authenticated image endpoints.
// Resolve those assets with the installed API transport instead.
export const ApiAssetContext = createContext(false);

export function useApiAsset(source: string, enabled = true) {
  const embedded = useContext(ApiAssetContext);
  const proxied = embedded && source.startsWith("/api/v1/");
  const query = useQuery({
    queryKey: ["api-asset", source],
    enabled: proxied && enabled,
    queryFn: async () => {
      const response = await fetch(source);
      if (!response.ok) throw new Error(`Image request failed (${response.status})`);
      const blob = await response.blob();
      return new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(new Error("Could not read the image"));
        reader.readAsDataURL(blob);
      });
    },
  });
  return { url: proxied ? query.data : source, error: query.error, proxied };
}
