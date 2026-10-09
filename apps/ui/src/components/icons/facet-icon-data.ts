/**
 * Facet's canonical vector masters. Paths are authored on a 24-unit artboard;
 * React, the gallery, and standalone SVG exports all read these same masters.
 * Intent is the approved original solid Agent mark. See knowledge/ui/iconography.md.
 */
export const facetIconData = {
  agent: { solid: true, paths: ["M4 7 11 3V10L4 14Z M4 16 19 8V15L4 23Z", "M14 2 19 5V6L14 9Z"] },
  chat: { paths: ["M7 3H17L21 7V13L17 17H9L4 21V15L3 13V7Z", "M7 8H17M7 12H13"] },
  playground: { paths: ["M8 3H16M10 3V9L4 19V21H20V19L14 9V3", "M7 15H17"] },
  harness: { paths: ["M12 3 20 7V13L16 19 12 22 8 19 4 13V7Z", "M8 9H16V15H8Z"] },
  virtualUser: { paths: ["M9 3H15L17 7 15 11H9L7 7Z", "M4 21V17L9 14H15L20 17V21", "M9 18H15"] },
  session: { paths: ["M3 4H21V18H9L3 22Z", "M7 8H17M7 12H13"] },
  exposure: {
    paths: [
      "M10 10 12 8 14 10 12 12Z",
      "M8 6 4 10 8 14M16 6 20 10 16 14M7 3 1 10 7 17M17 3 23 10 17 17",
      "M12 12V21M8 21H16",
    ],
  },
  models: {
    paths: [
      "M7 6H17V18H7Z",
      "M3 9H7M3 15H7M17 9H21M17 15H21M10 2V6M14 2V6M10 18V22M14 18V22",
      "M10 10H14V14H10Z",
    ],
  },
  skills: {
    paths: ["M3 4H9L12 6 15 4H21V19H15L12 21 9 19H3Z", "M12 6V21M6 9H9M15 9H18M6 13H9M15 13H18"],
  },
  capabilities: {
    paths: ["M3 3H10V10H3Z M14 3H21V10H14Z M3 14H10V21H3Z", "M17.5 14V21M14 17.5H21"],
  },
  plugins: { paths: ["M8 2V7M16 2V7M5 7H19V12L15 16H9L5 12Z", "M12 16V22"] },
  knowledge: { paths: ["M3 4H7V20H3Z M10 4H14V20H10Z M17 4 21 3 23 19 19 20Z"] },
  memory: { paths: ["M3 5 6 3H18L21 5V19L18 21H6L3 19Z", "M3 8H21M8 8V21M12 12H17M12 16H17"] },
  evals: { paths: ["M8 4H4V21H20V4H16M8 2H16V6H8Z", "M8 13 11 16 17 10"] },
  observer: { paths: ["M2 12 8 6H16L22 12 16 18H8Z", "M12 8 16 12 12 16 8 12Z"] },
  report: { paths: ["M4 3H16L20 7V21H4Z", "M16 3V7H20M8 17V13M12 17V9M16 17V11"] },
  sandbox: { paths: ["M3 8 12 3 21 8V18L12 23 3 18Z", "M3 8 12 13 21 8M12 13V23M8 5 17 10"] },
  sandboxTemplate: {
    paths: [
      "M7 2H21V16M3 6H17V22H3Z",
      "M6 12 10 10 14 12V17L10 19 6 17Z M6 12 10 14 14 12M10 14V19",
    ],
  },
  providerAccount: { paths: ["M3 4H21V20H3Z", "M7 8 10 6 13 8V12L10 14 7 12Z M13 10H18M17 10V13"] },
  settings: {
    paths: ["M9 3H15L16 6 19 7 22 12 19 17 16 18 15 21H9L8 18 5 17 2 12 5 7 8 6Z", "M9 9H15V15H9Z"],
  },
  durable: { paths: ["M7 3H17L22 12 17 21H7L2 12Z", "M9 6V18L17 12Z"] },
  worker: { paths: ["M3 3H21V10H3Z M3 14H21V21H3Z", "M7 6.5H8M7 17.5H8M12 6.5H17M12 17.5H17"] },
  workflow: { paths: ["M9 2H15V8H9Z M2 16H8V22H2Z M16 16H22V22H16Z", "M12 8V12M5 16V12H19V16"] },
  queue: { paths: ["M3 4H7V8H3Z M3 10H7V14H3Z M3 16H7V20H3Z", "M11 6H21M11 12H18M11 18H15"] },
  schedule: { paths: ["M4 4H20V21H4Z M8 2V6M16 2V6M4 9H20", "M12 12V16H16"] },
  circuitBreaker: { paths: ["M2 12H7M17 12H22M7 8V16M17 8V16M7 12 15 6", "M4 4H7M17 20H20"] },
  organization: {
    paths: ["M4 21V7L12 3 20 7V21Z", "M1 21H23M9 21V16H15V21M8 9H10M14 9H16M8 12H10M14 12H16"],
  },
  provider: { paths: ["M3 4H21V20H3Z", "M7 8H17M7 12H13M7 16H10M17 14V18M15 16H19"] },
  team: {
    paths: ["M5 3H10V9H5Z M14 3H19V9H14Z", "M2 21V15L5 12H10L12 14 14 12H19L22 15V21M12 14V21"],
  },
  health: { paths: ["M2 12H6L9 5 14 19 17 12H22"] },
  features: { paths: ["M4 3H20V9H4Z M4 15H20V21H4Z", "M8 3V9M16 15V21"] },
  payments: { paths: ["M3 5H21V20H3Z", "M3 10H21M7 15H11M15 15H17"] },
  account: { paths: ["M8 3H16V11H8Z", "M4 21V17L8 14H16L20 17V21Z"] },
  agentExperience: {
    paths: [
      "M3 4 8 2V7L3 10Z M3 12 14 6V11L3 18Z M10 1 14 3V4L10 6Z",
      "M18 11H22V15H18Z M16 22V19L18 17H22L23 18V22Z",
    ],
  },
  token: { paths: ["M3 6H11L14 9V15L11 18H3V6Z", "M14 12H22M18 12V16M21 12V15M7 10V14"] },
} satisfies Record<string, { solid?: boolean; paths: readonly string[] }>;

export type FacetIconName = keyof typeof facetIconData;
export const facetStrokeWidth = 1.75;

/** Export only trusted, compile-time masters; never serialize user-supplied SVG. */
export function facetIconSvg(name: FacetIconName): string {
  const data: { solid?: boolean; paths: readonly string[] } = facetIconData[name];
  const paint = data.solid
    ? 'fill="currentColor" stroke="none"'
    : `fill="none" stroke="currentColor" stroke-width="${facetStrokeWidth}"`;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" ${paint} stroke-linecap="square" stroke-linejoin="miter">${data.paths.map((d) => `<path d="${d}"/>`).join("")}</svg>\n`;
}
