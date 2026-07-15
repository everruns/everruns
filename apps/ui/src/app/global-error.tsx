"use client";

// Last-resort error boundary: catches errors thrown by the root layout
// itself, where app CSS/providers may be unavailable. Must render its own
// <html>/<body> (Next.js contract) and stick to inline-safe styling.

export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  console.error("Root layout error:", error);
  return (
    <html lang="en">
      <body
        style={{
          margin: 0,
          minHeight: "100vh",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          fontFamily: "system-ui, sans-serif",
          background: "#fafafa",
          color: "#0a0a0a",
        }}
      >
        <div style={{ textAlign: "center", padding: 24, maxWidth: 420 }}>
          <p style={{ fontSize: 48, fontWeight: 600, color: "#a0a0a0", margin: 0 }}>500</p>
          <h1 style={{ fontSize: 22, margin: "16px 0 8px" }}>Something went wrong</h1>
          <p style={{ color: "#525252", margin: "0 0 24px" }}>
            An unexpected error occurred. Please try again.
          </p>
          <button
            type="button"
            onClick={() => reset()}
            style={{
              background: "#0A1636",
              color: "#fff",
              border: "none",
              padding: "10px 22px",
              fontSize: 14,
              cursor: "pointer",
            }}
          >
            Try again
          </button>
        </div>
      </body>
    </html>
  );
}
