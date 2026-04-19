/** @jest-environment node */

import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  createUiRouteManifest,
  mergeUiRouteManifests,
  resolveUiRouteArtifactFilePath,
  type UiRouteManifestArtifact,
} from "@/lib/ui-route-manifest";

function writeArtifact(
  appDir: string,
  relativePath: string,
  contents = "export default function Test() {}",
) {
  const filePath = path.join(appDir, relativePath);
  mkdirSync(path.dirname(filePath), { recursive: true });
  writeFileSync(filePath, contents);
}

function findArtifact(artifacts: UiRouteManifestArtifact[], artifactId: string) {
  const artifact = artifacts.find((entry) => entry.artifactId === artifactId);
  expect(artifact).toBeDefined();
  return artifact!;
}

describe("createUiRouteManifest", () => {
  it("derives app-tree identity and public path from the app router tree", () => {
    const appDir = mkdtempSync(path.join(tmpdir(), "everruns-ui-route-manifest-"));

    writeArtifact(appDir, "layout.tsx");
    writeArtifact(appDir, "not-found.tsx");
    writeArtifact(appDir, "favicon.ico", "ico");
    writeArtifact(appDir, "(main)/layout.tsx");
    writeArtifact(appDir, "(main)/dashboard/page.tsx");
    writeArtifact(appDir, "(auth)/login/page.tsx");
    writeArtifact(appDir, "api/health/route.ts");

    const manifest = createUiRouteManifest({
      appDir,
      packageName: "test-ui",
    });

    expect(manifest.version).toBe(1);
    expect(manifest.sourcePackage).toBe("test-ui");

    expect(findArtifact(manifest.artifacts, "/:layout.tsx")).toMatchObject({
      sourcePackage: "test-ui",
      kind: "layout",
      appPath: "/",
      pathname: "/",
      sourcePath: "src/app/layout.tsx",
      packageRelativePath: "src/app/layout.tsx",
    });

    expect(findArtifact(manifest.artifacts, "/(main):layout.tsx")).toMatchObject({
      sourcePackage: "test-ui",
      kind: "layout",
      appPath: "/(main)",
      pathname: "/",
      sourcePath: "src/app/(main)/layout.tsx",
    });

    expect(findArtifact(manifest.artifacts, "/(main)/dashboard:page.tsx")).toMatchObject({
      sourcePackage: "test-ui",
      kind: "page",
      appPath: "/(main)/dashboard",
      pathname: "/dashboard",
      sourcePath: "src/app/(main)/dashboard/page.tsx",
    });

    expect(findArtifact(manifest.artifacts, "/(auth)/login:page.tsx")).toMatchObject({
      sourcePackage: "test-ui",
      kind: "page",
      appPath: "/(auth)/login",
      pathname: "/login",
    });

    expect(findArtifact(manifest.artifacts, "/api/health:route.ts")).toMatchObject({
      sourcePackage: "test-ui",
      kind: "route",
      appPath: "/api/health",
      pathname: "/api/health",
    });

    expect(findArtifact(manifest.artifacts, "/:favicon.ico")).toMatchObject({
      sourcePackage: "test-ui",
      kind: "favicon",
      appPath: "/",
      pathname: "/",
    });
  });
});

describe("mergeUiRouteManifests", () => {
  it("lets wrapper artifacts override OSS artifacts by artifact id while keeping extra routes", () => {
    const base = {
      version: 1 as const,
      sourcePackage: "everruns-ui",
      appDir: "src/app",
      artifacts: [
        {
          artifactId: "/(main)/dashboard:page.tsx",
          sourcePackage: "everruns-ui",
          kind: "page" as const,
          appPath: "/(main)/dashboard",
          pathname: "/dashboard",
          fileName: "page.tsx",
          sourcePath: "src/app/(main)/dashboard/page.tsx",
          packageRelativePath: "src/app/(main)/dashboard/page.tsx",
        },
      ],
    };
    const wrapper = {
      version: 1 as const,
      sourcePackage: "wrapper-ui",
      appDir: "src/app",
      artifacts: [
        {
          artifactId: "/(main)/dashboard:page.tsx",
          sourcePackage: "wrapper-ui",
          kind: "page" as const,
          appPath: "/(main)/dashboard",
          pathname: "/dashboard",
          fileName: "page.tsx",
          sourcePath: "src/app/(main)/dashboard/page.tsx",
          packageRelativePath: "src/app/(main)/dashboard/page.tsx",
        },
        {
          artifactId: "/billing:page.tsx",
          sourcePackage: "wrapper-ui",
          kind: "page" as const,
          appPath: "/billing",
          pathname: "/billing",
          fileName: "page.tsx",
          sourcePath: "src/app/billing/page.tsx",
          packageRelativePath: "src/app/billing/page.tsx",
        },
      ],
    };

    const merged = mergeUiRouteManifests(base, wrapper);

    expect(merged.artifacts).toHaveLength(2);
    expect(merged.sourcePackage).toBe("wrapper-ui");
    expect(findArtifact(merged.artifacts, "/(main)/dashboard:page.tsx").sourcePackage).toBe(
      "wrapper-ui",
    );
    expect(findArtifact(merged.artifacts, "/billing:page.tsx")).toMatchObject({
      sourcePackage: "wrapper-ui",
      pathname: "/billing",
      sourcePath: "src/app/billing/page.tsx",
    });
  });

  it("rejects packageRelativePath values outside the published src/ contract", () => {
    expect(() =>
      resolveUiRouteArtifactFilePath({
        artifactId: "/dashboard:page.tsx",
        sourcePackage: "everruns-ui",
        kind: "page",
        appPath: "/dashboard",
        pathname: "/dashboard",
        fileName: "page.tsx",
        sourcePath: "src/app/dashboard/page.tsx",
        packageRelativePath: "app/dashboard/page.tsx",
      }),
    ).toThrow('packageRelativePath must start with "src/"');
  });
});
