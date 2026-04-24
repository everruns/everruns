// Decision: artifact identity uses app-tree path plus filename, not public pathname.
// Route groups can share the same public pathname while applying different layouts.
// Wrappers need the app-tree identity to override intentionally without flattening Next's structure.

import { readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const UI_ROUTE_MANIFEST_VERSION = 1 as const;
export const UI_ROUTE_MANIFEST_APP_DIR = "src/app" as const;
export const UI_ROUTE_MANIFEST_PACKAGE = "everruns-ui" as const;

export type UiRouteArtifactKind =
  | "page"
  | "layout"
  | "loading"
  | "error"
  | "template"
  | "route"
  | "not-found"
  | "icon"
  | "apple-icon"
  | "opengraph-image"
  | "twitter-image"
  | "favicon";

export interface UiRouteManifestArtifact {
  artifactId: string;
  sourcePackage: string;
  kind: UiRouteArtifactKind;
  appPath: string;
  pathname: string;
  fileName: string;
  sourcePath: string;
  packageRelativePath: string;
}

export interface UiRouteManifest {
  version: typeof UI_ROUTE_MANIFEST_VERSION;
  sourcePackage: string;
  appDir: typeof UI_ROUTE_MANIFEST_APP_DIR;
  artifacts: UiRouteManifestArtifact[];
}

interface CreateUiRouteManifestOptions {
  appDir?: string;
  packageName?: string;
}

const ROUTE_ARTIFACT_KINDS = new Set<UiRouteArtifactKind>([
  "page",
  "layout",
  "loading",
  "error",
  "template",
  "route",
  "not-found",
  "icon",
  "apple-icon",
  "opengraph-image",
  "twitter-image",
  "favicon",
]);
const PACKAGE_RELATIVE_PATH_PREFIX = "src/";

let ossUiRouteManifestCache: UiRouteManifest | undefined;

function defaultAppDir(): string {
  return fileURLToPath(new URL("../app", import.meta.url));
}

function isRouteGroupSegment(segment: string): boolean {
  return /^\([^/]+\)$/.test(segment);
}

function isInvisibleUrlSegment(segment: string): boolean {
  return isRouteGroupSegment(segment) || segment.startsWith("@");
}

function normalizePosixPath(value: string): string {
  return value.split(path.sep).join("/");
}

function toAppPath(segments: string[]): string {
  return segments.length === 0 ? "/" : `/${segments.join("/")}`;
}

function toPublicPath(segments: string[]): string {
  const pathnameSegments = segments.filter((segment) => !isInvisibleUrlSegment(segment));
  return pathnameSegments.length === 0 ? "/" : `/${pathnameSegments.join("/")}`;
}

function classifyArtifact(fileName: string): UiRouteArtifactKind | null {
  if (fileName === "favicon.ico") {
    return "favicon";
  }

  const basename = fileName.replace(/\.[^.]+$/, "") as UiRouteArtifactKind;
  return ROUTE_ARTIFACT_KINDS.has(basename) ? basename : null;
}

function compareArtifacts(left: UiRouteManifestArtifact, right: UiRouteManifestArtifact): number {
  return left.sourcePath.localeCompare(right.sourcePath);
}

function walkArtifacts(
  appDir: string,
  currentDir: string,
  packageName: string,
  artifacts: UiRouteManifestArtifact[],
) {
  for (const entry of readdirSync(currentDir, { withFileTypes: true })) {
    const absolutePath = path.join(currentDir, entry.name);

    if (entry.isDirectory()) {
      walkArtifacts(appDir, absolutePath, packageName, artifacts);
      continue;
    }

    if (!entry.isFile()) {
      continue;
    }

    const kind = classifyArtifact(entry.name);
    if (!kind) {
      continue;
    }

    const routeDir = path.dirname(absolutePath);
    const routeSegments = path.relative(appDir, routeDir).split(path.sep).filter(Boolean);
    const sourcePath = normalizePosixPath(
      path.join(UI_ROUTE_MANIFEST_APP_DIR, ...routeSegments, entry.name),
    );

    artifacts.push({
      artifactId: `${toAppPath(routeSegments)}:${entry.name}`,
      sourcePackage: packageName,
      kind,
      appPath: toAppPath(routeSegments),
      pathname: toPublicPath(routeSegments),
      fileName: entry.name,
      sourcePath,
      packageRelativePath: sourcePath,
    });
  }
}

export function createUiRouteManifest(options: CreateUiRouteManifestOptions = {}): UiRouteManifest {
  const appDir = options.appDir ?? defaultAppDir();
  const packageName = options.packageName ?? UI_ROUTE_MANIFEST_PACKAGE;
  const artifacts: UiRouteManifestArtifact[] = [];

  walkArtifacts(appDir, appDir, packageName, artifacts);

  return {
    version: UI_ROUTE_MANIFEST_VERSION,
    sourcePackage: packageName,
    appDir: UI_ROUTE_MANIFEST_APP_DIR,
    artifacts: artifacts.sort(compareArtifacts),
  };
}

export function mergeUiRouteManifests(
  base: UiRouteManifest,
  overrides: UiRouteManifest,
): UiRouteManifest {
  if (base.version !== overrides.version) {
    throw new Error(
      `Cannot merge UI route manifests with different versions (${base.version} vs ${overrides.version})`,
    );
  }

  const merged = new Map(base.artifacts.map((artifact) => [artifact.artifactId, artifact]));
  for (const artifact of overrides.artifacts) {
    merged.set(artifact.artifactId, artifact);
  }

  return {
    version: base.version,
    // Merged manifests reflect the effective override layer, not the original OSS base.
    sourcePackage: overrides.sourcePackage,
    appDir: base.appDir,
    artifacts: [...merged.values()].sort(compareArtifacts),
  };
}

export function resolveUiRouteArtifactFilePath(artifact: UiRouteManifestArtifact): string {
  if (artifact.sourcePackage !== UI_ROUTE_MANIFEST_PACKAGE) {
    throw new Error(
      `Cannot resolve ${artifact.artifactId} from ${artifact.sourcePackage} with the ${UI_ROUTE_MANIFEST_PACKAGE} loader`,
    );
  }

  const normalizedPackageRelativePath = path.posix.normalize(
    artifact.packageRelativePath.replaceAll("\\", "/"),
  );

  if (!normalizedPackageRelativePath.startsWith(PACKAGE_RELATIVE_PATH_PREFIX)) {
    throw new Error(
      `Cannot resolve ${artifact.artifactId} because packageRelativePath must start with "${PACKAGE_RELATIVE_PATH_PREFIX}", received "${artifact.packageRelativePath}"`,
    );
  }

  const resolvedPath = fileURLToPath(
    new URL(
      `../${normalizedPackageRelativePath.slice(PACKAGE_RELATIVE_PATH_PREFIX.length)}`,
      import.meta.url,
    ),
  );
  const packageRoot = fileURLToPath(new URL("..", import.meta.url));
  const relativeToPackageRoot = path.relative(packageRoot, resolvedPath);

  if (relativeToPackageRoot.startsWith("..") || path.isAbsolute(relativeToPackageRoot)) {
    throw new Error(
      `Cannot resolve ${artifact.artifactId} because packageRelativePath escapes the package root, received "${artifact.packageRelativePath}"`,
    );
  }

  return resolvedPath;
}

export async function loadUiRouteArtifactModule(
  artifact: UiRouteManifestArtifact,
): Promise<unknown> {
  return import(pathToFileURL(resolveUiRouteArtifactFilePath(artifact)).href);
}

export function getOssUiRouteManifest(): UiRouteManifest {
  if (!ossUiRouteManifestCache) {
    ossUiRouteManifestCache = createUiRouteManifest();
  }

  return ossUiRouteManifestCache;
}
