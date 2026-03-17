// Named wrapper around useEffect with an empty dependency array.
// Makes mount-only intent explicit and enables lint rules that ban raw useEffect.
// eslint-disable-next-line no-restricted-imports
import { useEffect, type EffectCallback } from "react";

/**
 * Run an effect exactly once on mount (and cleanup on unmount).
 * Semantically identical to `useEffect(fn, [])` but communicates intent.
 */
export function useMountEffect(effect: EffectCallback): void {
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(effect, []);
}
