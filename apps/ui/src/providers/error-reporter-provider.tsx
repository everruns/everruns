"use client";

// Error reporter provider
//
// Decision: Vendor-neutral embedder hook. OSS never imports Sentry/Datadog/Rollbar
//   SDKs. Wrappers implement `ErrorReporter` and install it via
//   `<ErrorReporterProvider reporter={…}>` at the application root, then feature
//   code uses `useErrorReporter()` to report errors without knowing the backend.
// Decision: Default is a no-op reporter so callers can always use the hook
//   without conditional logic. Wrappers opt in by providing a reporter.
// Decision: Scope fields mirror the backend `ErrorScope` contract so wrappers
//   can map them onto vendor scope/tag APIs one-to-one.

import { createContext, useContext, type ReactNode } from "react";

export type ErrorSeverity = "warning" | "error" | "fatal";

export interface ErrorScope {
  userId?: string;
  orgId?: string;
  sessionId?: string;
  requestId?: string;
  route?: string;
  component?: string;
  /** Extra key/value tags (provider id, feature flag, etc.). */
  extra?: Record<string, string>;
}

export interface ErrorReport {
  severity: ErrorSeverity;
  /** Short machine-stable identifier (e.g. "ui.render.error"). */
  kind: string;
  message: string;
  scope?: ErrorScope;
  cause?: unknown;
}

/**
 * Vendor-neutral error reporter contract. Wrappers implement this and install
 * it via {@link ErrorReporterProvider}. See `specs/embedding.md` for the full
 * embedding contract.
 */
export interface ErrorReporter {
  report(report: ErrorReport): void;
}

const noopReporter: ErrorReporter = {
  report: () => {
    // no-op
  },
};

const ErrorReporterContext = createContext<ErrorReporter>(noopReporter);

export interface ErrorReporterProviderProps {
  /**
   * Wrapper-provided reporter. Omit or pass `undefined` to disable error
   * reporting (defaults to a no-op reporter so `useErrorReporter()` never
   * throws).
   */
  reporter?: ErrorReporter;
  children: ReactNode;
}

export function ErrorReporterProvider({ reporter, children }: ErrorReporterProviderProps) {
  return (
    <ErrorReporterContext.Provider value={reporter ?? noopReporter}>
      {children}
    </ErrorReporterContext.Provider>
  );
}

/**
 * Access the installed error reporter. Always returns a reporter — defaults
 * to a no-op when no wrapper has installed one, so callers never need to
 * null-check.
 */
export function useErrorReporter(): ErrorReporter {
  return useContext(ErrorReporterContext);
}
