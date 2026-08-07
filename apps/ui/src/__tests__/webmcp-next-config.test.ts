import { webMcpPermissionsPolicy } from "../../next.config";

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
