import { webMcpPermissionsPolicy } from "../../next.config";

describe("webMcpPermissionsPolicy", () => {
  it.each(["preview", "adoption", "prod"])("allows same-origin tools for %s", (value) => {
    expect(
      webMcpPermissionsPolicy({ feature: value, deploymentGrade: "prod", devMode: undefined }),
    ).toBe("tools=(self)");
  });

  it.each([undefined, "dev", "off", "true", "false", "", "PROD"])(
    "denies tools in production for %s",
    (value) => {
      expect(
        webMcpPermissionsPolicy({ feature: value, deploymentGrade: "prod", devMode: undefined }),
      ).toBe("tools=()");
    },
  );

  it("dev is local only and off overrides local availability", () => {
    expect(
      webMcpPermissionsPolicy({ feature: undefined, deploymentGrade: undefined, devMode: "true" }),
    ).toBe("tools=(self)");
    expect(
      webMcpPermissionsPolicy({ feature: "off", deploymentGrade: "dev", devMode: undefined }),
    ).toBe("tools=()");
    expect(
      webMcpPermissionsPolicy({ feature: "dev", deploymentGrade: "preview", devMode: "true" }),
    ).toBe("tools=()");
    expect(
      webMcpPermissionsPolicy({ feature: undefined, deploymentGrade: "", devMode: "true" }),
    ).toBe("tools=()");
  });
});
