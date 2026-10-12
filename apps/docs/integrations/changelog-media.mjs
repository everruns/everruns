// Serves changelog videos from the entry folder they sit in.
//
// Decision: an entry's screenshots and videos live beside its index.md
// (docs/changelog/<date>-<slug>/). Markdown images go through Astro's image
// pipeline, but Astro leaves <video> sources alone, so this integration copies
// videos and their poster frames to the same path under the entry's URL
// (/changelog/<date>-<slug>/demo.mp4). An entry then writes
// `<video src="./demo.mp4" poster="./demo-poster.png">` and the same relative
// path works in the built site, the dev server and the RSS feed.
import { copyFileSync, existsSync, mkdirSync, readdirSync, createReadStream, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const MEDIA = /\.(mp4|webm)$|-poster\.(png|jpe?g|webp)$/i;
const TYPES = { ".mp4": "video/mp4", ".webm": "video/webm", ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".webp": "image/webp" };

function mediaFiles(changelogDir) {
  if (!existsSync(changelogDir)) return [];
  const out = [];
  for (const entry of readdirSync(changelogDir, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    for (const file of readdirSync(path.join(changelogDir, entry.name))) {
      if (MEDIA.test(file)) out.push(path.join(entry.name, file));
    }
  }
  return out;
}

export default function changelogMedia({ docsDir }) {
  const changelogDir = path.join(docsDir, "changelog");
  return {
    name: "changelog-media",
    hooks: {
      "astro:server:setup"({ server }) {
        server.middlewares.use((req, res, next) => {
          const match = req.url?.match(/^\/changelog\/([^/?#]+\/[^/?#]+)/);
          const file = match && path.join(changelogDir, decodeURIComponent(match[1]));
          if (!file || !MEDIA.test(file) || !file.startsWith(changelogDir) || !existsSync(file)) return next();
          res.setHeader("Content-Type", TYPES[path.extname(file).toLowerCase()] ?? "application/octet-stream");
          res.setHeader("Content-Length", statSync(file).size);
          createReadStream(file).pipe(res);
        });
      },
      "astro:build:done"({ dir }) {
        const out = path.join(fileURLToPath(dir), "changelog");
        for (const rel of mediaFiles(changelogDir)) {
          mkdirSync(path.join(out, path.dirname(rel)), { recursive: true });
          copyFileSync(path.join(changelogDir, rel), path.join(out, rel));
        }
      },
    },
  };
}
