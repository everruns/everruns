import { defineCollection } from "astro:content";
import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";
import { z } from "zod";

export const collections = {
  docs: defineCollection({
    loader: docsLoader(),
    schema: docsSchema({
      extend: z.object({
        notebook: z.string().optional(),
        published: z.string().optional(),
        topics: z.array(z.string()).optional(),
        github: z.url().optional(),
        // Which ways of running Everruns a page applies to, rendered as a
        // badge row under the title by components/PageTitle.astro. Omit it
        // when availability has not been checked against the code: an absent
        // row is better than a wrong one.
        appliesTo: z.array(z.enum(["framework", "platform", "cloud"])).optional(),
        // Changelog entry fields (docs/changelog/<date>-<slug>/index.md). Only
        // changelog entries set them; scripts/check-changelog.mjs makes the
        // required ones required there. Taxonomy: src/lib/changelog.ts.
        date: z.coerce.date().optional(),
        kind: z
          .enum(["feature", "improvement", "fix", "breaking", "deprecation", "incident", "release"])
          .optional(),
        significance: z.enum(["major", "notable", "minor"]).optional(),
        products: z
          .array(z.enum(["cloud", "platform", "framework", "sdk", "cli", "yolop", "mira", "bashkit"]))
          .optional(),
        surfaces: z.array(z.enum(["ui", "api", "code", "cli", "agent", "operations"])).optional(),
        // First version of the product that shipped the change. Stamped by the
        // release skills, not by the entry's author.
        release: z.string().optional(),
        // Date the change went live on Everruns Cloud. Stamped by /release-to-saas.
        cloud: z.coerce.date().optional(),
        links: z
          .object({
            docs: z.array(z.string()).optional(),
            prs: z.array(z.string()).optional(),
          })
          .optional(),
        // An entry is a dated record: when the feature is replaced or removed it
        // keeps its text and points to the newer entry instead.
        supersededBy: z.string().optional(),
        removedIn: z.string().optional(),
        // Backfill note for a screenshot or video still to capture.
        visual: z.string().optional(),
      }),
    }),
  }),
};
