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
      }),
    }),
  }),
};
