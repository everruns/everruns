// Converts src/assets/og-image.svg → public/og-image.png (1200×630)
// Uses sharp (already a project dependency for Astro image optimization).

import sharp from "sharp";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "..");

const svgPath = path.join(root, "src/assets/og-image.svg");
const outPath = path.join(root, "public/og-image.png");

const svg = readFileSync(svgPath);

await sharp(svg).resize(1200, 630).png().toFile(outPath);

console.log(`✓ Generated ${outPath}`);
