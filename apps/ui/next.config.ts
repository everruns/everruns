import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: "standalone",
  // API routing handled by Caddy reverse proxy and backend route layout.
  // No Next.js rewrites needed
};

export default nextConfig;
