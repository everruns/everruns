import assert from "node:assert/strict";
import test from "node:test";

import {
  releaseCardSvg,
  validateChangelog,
  validateReleaseCard,
} from "./generate-release-card.mjs";

const validCard = {
  version: "1.2.3",
  date: "2026-09-04",
  headline: ["Reasoning,", "in order."],
  summary: ["A concise release summary."],
  highlights: [
    {
      title: "Ordered artifacts",
      description: "Replay work in the order it happened.",
      source: "#123",
    },
  ],
};

test("validates and normalizes release card content", () => {
  assert.deepEqual(
    validateReleaseCard(
      {
        ...validCard,
        version: " 1.2.3 ",
        date: " 2026-09-04 ",
        headline: [" Reasoning, ", " in order. "],
        summary: [" A concise release summary. "],
        highlights: [
          {
            title: " Ordered artifacts ",
            description: " Replay work in the order it happened. ",
            source: " #123 ",
          },
        ],
        ignored: "not part of the card",
      },
      "1.2.3",
    ),
    validCard,
  );
});

test("rejects a card for a different release", () => {
  assert.throws(() => validateReleaseCard(validCard, "1.2.4"), /does not match release 1\.2\.4/);
});

test("rejects content that cannot fit the template", () => {
  assert.deepEqual(validateReleaseCard({ ...validCard, headline: ["x".repeat(24)] }).headline, [
    "x".repeat(24),
  ]);
  assert.throws(
    () => validateReleaseCard({ ...validCard, headline: ["x".repeat(25)] }),
    /headline\[0\] must be at most 24 characters/,
  );
  assert.throws(
    () =>
      validateReleaseCard({
        ...validCard,
        highlights: [
          {
            ...validCard.highlights[0],
            description: "x ".repeat(49).trim(),
          },
        ],
      }),
    /description must fit on two lines/,
  );
});

test("rejects impossible calendar dates", () => {
  assert.throws(
    () => validateReleaseCard({ ...validCard, date: "2026-02-31" }),
    /valid YYYY-MM-DD date/,
  );
});

test("requires highlight sources in the matching changelog section", () => {
  const card = validateReleaseCard(validCard);
  validateChangelog(card, "## [1.2.3] - 2026-09-04\n\n- Ordered artifacts ([#123](example))\n");
  assert.throws(
    () => validateChangelog(card, "## [1.2.3] - 2026-09-04\n\n- Something else\n"),
    /source #123 is not present/,
  );
  for (const changelog of [
    "## [1.2.4] - 2026-09-05\n\n- #123\n\n## [1.2.3] - 2026-09-04\n\n- Something else\n",
    "## [1.2.3] - 2026-09-04\n\n- Something else\n\n## [1.2.2] - 2026-09-03\n\n- #123\n",
  ]) {
    assert.throws(() => validateChangelog(card, changelog), /source #123 is not present/);
  }
});

test("escapes user-controlled SVG text", () => {
  const svg = releaseCardSvg({
    ...validCard,
    highlights: [{ title: "Files & tools", description: "Use <guarded> input.", source: "#123" }],
  });
  assert.match(svg, /Files &amp; tools/);
  assert.match(svg, /Use &lt;guarded&gt; input\./);
  assert.doesNotMatch(svg, /Use <guarded>/);
});
