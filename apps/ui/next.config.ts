import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: 'standalone',
  // Proxy /api/* requests to backend in development
  // Strips the /api prefix and forwards to backend (e.g., /api/v1/agents -> http://localhost:9000/v1/agents)
  // In production, configure your reverse proxy to do the same
  async rewrites() {
    const apiUrl = process.env.NEXT_PUBLIC_API_URL || 'http://localhost:9000';
    return [
      {
        source: '/api/:path*',
        destination: `${apiUrl}/:path*`,
      },
    ];
  },
};

export default nextConfig;
