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
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    // Reset if name is empty or too short
    if (!name || name.length < 2) {
      setAvailable(null);
      setIsChecking(false);
      return;
    }

    setIsChecking(true);
    setAvailable(null);

    const timer = setTimeout(async () => {
      // Cancel any in-flight request
      abortRef.current?.abort();
      const controller = new AbortController();
      abortRef.current = controller;

      try {
        const result = await checkHarnessName(name, excludeId);
        if (!controller.signal.aborted) {
          setAvailable(result.available);
          setIsChecking(false);
        }
      } catch {
        if (!controller.signal.aborted) {
          setAvailable(null);
          setIsChecking(false);
        }
      }
    }, debounceMs);

    return () => {
      clearTimeout(timer);
      abortRef.current?.abort();
    };
  }, [name, excludeId, debounceMs]);

  return { available, isChecking };
}
