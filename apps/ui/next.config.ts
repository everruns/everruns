import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: 'standalone',
  // API routing handled by Caddy reverse proxy (strips /api prefix, forwards to backend)
  // No Next.js rewrites needed
};

export default nextConfig;
