import nextConfig, { webMcpPermissionsPolicy } from "../../next.config";

describe("webMcpPermissionsPolicy", () => {
  it.each(["true", "1"])("allows same-origin tools for %s", (value) => {
    expect(
      webMcpPermissionsPolicy({ feature: value, deploymentGrade: "prod", devMode: undefined }),
    ).toBe("tools=(self)");
  });

  it.each([undefined, "false", "0", "TRUE"])("denies tools for %s", (value) => {
    expect(
      webMcpPermissionsPolicy({ feature: value, deploymentGrade: "prod", devMode: undefined }),
    ).toBe("tools=()");
  });

  it("matches the backend development default", () => {
    expect(
      webMcpPermissionsPolicy({
        feature: undefined,
        deploymentGrade: undefined,
        devMode: "true",
      }),
    ).toBe("tools=(self)");
    expect(
      webMcpPermissionsPolicy({
        feature: "false",
        deploymentGrade: "dev",
        devMode: undefined,
      }),
    ).toBe("tools=()");
  });
});

describe("legacy agent endpoint routes", () => {
  it("permanently redirects /agents/:id/endpoints/* to /agents/:id/channels/*", async () => {
    const redirects = await nextConfig.redirects?.();
    expect(redirects).toContainEqual({
      source: "/agents/:agentId/endpoints/:path*",
      destination: "/agents/:agentId/channels/:path*",
      permanent: true,
    });
  });
});
