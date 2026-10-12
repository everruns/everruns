#!/usr/bin/env node
// Node >=22.18: native type stripping keeps export and React on one vector master.
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { facetIconData, facetIconSvg } from "../src/components/icons/facet-icon-data.ts";

const destination = resolve(process.argv[2] ?? fileURLToPath(new URL("../../../output/facet-icons", import.meta.url)));
await mkdir(destination, { recursive: true });
for (const name of Object.keys(facetIconData)) {
  const fileName = name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
  await writeFile(resolve(destination, `${fileName}.svg`), facetIconSvg(name));
}
console.log(`Exported ${Object.keys(facetIconData).length} SVG masters to ${destination}`);
