FROM node:22.23.4-slim AS build
WORKDIR /app/apps/chat-ui
RUN corepack enable && corepack prepare pnpm@10.33.0 --activate
COPY apps/chat-ui/package.json apps/chat-ui/pnpm-lock.yaml apps/chat-ui/pnpm-workspace.yaml ./
COPY apps/ui/generative-ui/ /app/apps/ui/generative-ui/
COPY apps/ui/scripts/patch-zod-v3.mjs apps/ui/scripts/zod-v3-compatibility.json /app/apps/ui/scripts/
RUN pnpm install --frozen-lockfile
COPY apps/chat-ui/ ./
RUN pnpm build
FROM node:22.23.4-slim
WORKDIR /app
ENV NODE_ENV=production HOSTNAME=0.0.0.0 PORT=3000
COPY --from=build --chown=node:node /app/apps/chat-ui/.next/standalone ./
COPY --from=build --chown=node:node /app/apps/chat-ui/.next/static ./apps/chat-ui/.next/static
RUN mkdir -p apps/chat-ui/.next/cache && chown -R node:node apps/chat-ui/.next/cache
USER node
EXPOSE 3000
CMD ["node","apps/chat-ui/server.js"]
