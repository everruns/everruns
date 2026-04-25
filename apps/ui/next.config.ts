import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: "standalone",
  transpilePackages: [
    "@rjsf/core",
    "@rjsf/utils",
    "@rjsf/validator-ajv8",
    "@x0k/json-schema-merge",
  ],
  // API routing handled by Caddy reverse proxy and backend route layout.
  // No Next.js rewrites needed
};

export default nextConfig;
