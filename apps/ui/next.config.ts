import type { NextConfig } from "next";

export function webMcpPermissionsPolicy(env: {
  feature: string | undefined;
  deploymentGrade: string | undefined;
  devMode: string | undefined;
}): string {
  const explicitlyEnabled = env.feature === "true" || env.feature === "1";
  const explicitlyDisabled = env.feature !== undefined && !explicitlyEnabled;
  const grade = env.deploymentGrade?.toLowerCase();
  const developmentDefault =
    !explicitlyDisabled &&
    (grade === "dev" ||
      grade === "development" ||
      (!grade && (env.devMode === "true" || env.devMode === "1")));
  return explicitlyEnabled || developmentDefault ? "tools=(self)" : "tools=()";
}

const nextConfig: NextConfig = {
  output: "standalone",
  transpilePackages: [
    "@rjsf/core",
    "@rjsf/utils",
    "@rjsf/validator-ajv8",
    "@x0k/json-schema-merge",
  ],
  async headers() {
    // THREAT[TM-WEB-014]: deny browser tool registration unless the deployment gate is open;
    // when open, keep discovery and invocation restricted to the document's own origin.
    return [
      {
        source: "/:path*",
        headers: [
          {
            key: "Permissions-Policy",
            value: webMcpPermissionsPolicy({
              feature: process.env.FEATURE_WEBMCP,
              deploymentGrade: process.env.DEPLOYMENT_GRADE,
              devMode: process.env.DEV_MODE,
            }),
          },
        ],
      },
    ];
  },
  // API routing handled by Caddy reverse proxy and backend route layout.
  // No Next.js rewrites needed
};

export default nextConfig;
