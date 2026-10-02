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
      }),
    }),
  }),
};
