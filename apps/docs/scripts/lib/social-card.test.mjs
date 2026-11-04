import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  ART_STAGE,
  artImageRefs,
  iconIllustration,
  parseFacetIcons,
  pickIcons,
  placeArt,
  socialCardSvg,
  validateArt,
} from "./social-card.mjs";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");
const icons = parseFacetIcons(
  readFileSync(path.join(repoRoot, "apps/ui/src/components/icons/facet-icon-data.ts"), "utf8"),
);
const art = (body, viewBox = "0 0 520 560") =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="520" height="560" viewBox="${viewBox}">${body}</svg>`;

test("parses the app's Facet masters", () => {
  assert.ok(icons.agent.solid);
  assert.ok(icons.observer.paths.length > 0);
});

test("picks icons from explicit frontmatter first", () => {
  assert.deepEqual(pickIcons({ icons: ["sandbox", "schedule"], title: "Observers" }, icons), {
    hero: "sandbox",
    satellites: ["schedule"],
  });
});

test("ignores explicit icons that do not exist and falls back to keywords", () => {
  assert.equal(pickIcons({ icons: ["nope"], title: "Observers score conversations" }, icons).hero, "observer");
});

test("matches docs links and falls back by kind", () => {
  assert.equal(pickIcons({ title: "Faster pages", links: ["/features/costs/"] }, icons).hero, "payments");
  assert.equal(pickIcons({ title: "Everruns 0.50.1", kind: "release" }, icons).hero, "durable");
  assert.equal(pickIcons({ title: "Something unrelated", kind: "fix" }, icons).hero, "health");
});

test("generated art is deterministic per seed", () => {
  const a = iconIllustration({ seed: "x", hero: "agent", satellites: ["session"], icons });
  assert.equal(a, iconIllustration({ seed: "x", hero: "agent", satellites: ["session"], icons }));
  assert.notEqual(a, iconIllustration({ seed: "y", hero: "agent", satellites: ["session"], icons }));
});

test("accepts a self-contained drawing with repository images", () => {
  assert.deepEqual(validateArt(art('<image href="../shot@2x.png" x="0" y="0" width="10" height="10"/>')), []);
});

test("rejects scripts, handlers, remote images and the wrong stage", () => {
  assert.ok(validateArt(art("<script>alert(1)</script>")).length > 0);
  assert.ok(validateArt(art('<rect onload="x()"/>')).length > 0);
  assert.ok(validateArt(art('<image href="https://example.com/a.png"/>')).length > 0);
  assert.ok(validateArt(art('<image href="data:image/png;base64,AA"/>')).length > 0);
  assert.ok(validateArt(art("<foreignObject/>")).length > 0);
  assert.ok(validateArt(art("", "0 0 100 100")).length > 0);
  assert.ok(validateArt("<g/>").length > 0);
});

test("places art on the stage with inlined images and scoped ids", () => {
  const source = art('<defs><linearGradient id="g"/></defs><rect fill="url(#g)"/><image href="a.png"/>');
  assert.deepEqual(artImageRefs(source), ["a.png"]);
  const placed = placeArt(source, { "a.png": "data:image/png;base64,AA" });
  assert.match(placed, new RegExp(`x="${ART_STAGE.x}" y="${ART_STAGE.y}" width="520" height="560"`));
  assert.match(placed, /href="data:image\/png;base64,AA"/);
  assert.match(placed, /id="art-g"/);
  assert.match(placed, /url\(#art-g\)/);
});

test("plain cards show the description, illustrated cards leave room for the art", () => {
  const plain = socialCardSvg({ eyebrow: "Changelog · Fix", title: "A fix", description: "What changed", meta: ["Nov 2"], url: "u" });
  assert.match(plain, /What changed/);
  const illustrated = socialCardSvg({
    eyebrow: "Changelog · Feature",
    title: "A feature",
    description: "What changed",
    meta: ["Nov 2"],
    url: "u",
    illustration: "<g/>",
  });
  assert.doesNotMatch(illustrated, /What changed/);
});

test("escapes text", () => {
  assert.match(socialCardSvg({ eyebrow: "e", title: "A <b> & c", url: "u" }), /A &lt;b&gt; &amp; c/);
});
