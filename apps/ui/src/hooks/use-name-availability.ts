// Hook for checking harness name availability with debounce
"use client";

import { useState, useEffect, useRef } from "react";
import { checkHarnessName } from "@/lib/api/harnesses";

interface NameAvailabilityResult {
  /** Whether the name is available. null = not yet checked or checking */
  available: boolean | null;
  /** Whether a check is currently in-flight */
  isChecking: boolean;
}

/**
 * Debounced harness name availability check.
 * Uses a request counter to discard stale responses from superseded requests.
 *
 * @param name - current name value from the input
 * @param excludeId - harness ID to exclude (for edit forms)
 * @param debounceMs - debounce delay (default 300ms)
 */
export function useHarnessNameAvailability(
  name: string,
  excludeId?: string,
  debounceMs = 300,
): NameAvailabilityResult {
  const [available, setAvailable] = useState<boolean | null>(null);
  const [isChecking, setIsChecking] = useState(false);
  const requestIdRef = useRef(0);

  useEffect(() => {
    // Reset if name is empty or too short
    if (!name || name.length < 2) {
      setAvailable(null);
      setIsChecking(false);
      return;
    }

    setIsChecking(true);
    setAvailable(null);

    // Increment to invalidate any in-flight request
    const currentRequestId = ++requestIdRef.current;

    const timer = setTimeout(async () => {
      try {
        const result = await checkHarnessName(name, excludeId);
        // Only apply result if this is still the latest request
        if (currentRequestId === requestIdRef.current) {
          setAvailable(result.available);
          setIsChecking(false);
        }
      } catch {
        if (currentRequestId === requestIdRef.current) {
          setAvailable(null);
          setIsChecking(false);
        }
      }
    }, debounceMs);

    return () => {
      clearTimeout(timer);
    };
  }, [name, excludeId, debounceMs]);

  return { available, isChecking };
}
