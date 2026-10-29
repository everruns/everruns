import path from "node:path";
import type { NextConfig } from "next";
const config: NextConfig = {
  output: "standalone",
  transpilePackages: ["@everruns/generative-ui"],
  // The build script checks generated route types with tsrs before compilation.
  typescript: { ignoreBuildErrors: true },
  // A wrapper workspace may hoist dependencies above the OSS checkout.
  outputFileTracingRoot:
    process.env.CHAT_OUTPUT_TRACING_ROOT ?? path.resolve(import.meta.dirname, "../.."),
  poweredByHeader: false,
  async headers() {
    return [
      {
        source: "/:path*",
        headers: [
          { key: "X-Content-Type-Options", value: "nosniff" },
          { key: "Referrer-Policy", value: "no-referrer" },
          { key: "X-Frame-Options", value: "DENY" },
          {
            key: "Content-Security-Policy",
            value: `default-src 'self'; script-src 'self' 'unsafe-inline' ${process.env.NODE_ENV === "development" ? "'unsafe-eval'" : ""}; style-src 'self' 'unsafe-inline'; img-src 'self' https: data: blob:; media-src 'self' https: blob:; worker-src 'self' blob:; connect-src 'self' https://demotiles.maplibre.org; frame-src 'self' blob: https://www.youtube-nocookie.com; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'`,
          },
        ],
      },
    ];
  },
};
export default config;
