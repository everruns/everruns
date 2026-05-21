# Multi-stage build for Next.js standalone output
#
# The builder stage is pinned to linux/amd64 because Next.js static page
# generation runs V8 JIT-compiled code that crashes under QEMU arm64
# user-mode emulation (SIGILL — V8 emits ARM64 instructions QEMU doesn't
# support). The standalone output is platform-independent JS, so building
# on amd64 and copying into the target-platform runtime image is safe.
FROM --platform=linux/amd64 node:26-alpine AS builder
WORKDIR /app

# Enable pnpm via corepack (version pinned by packageManager in package.json)
RUN corepack enable

# Install dependencies (lockfile-only — fail if pnpm-lock.yaml is stale)
COPY package.json pnpm-lock.yaml .npmrc ./
RUN pnpm install --frozen-lockfile

# Copy source and build
COPY . .
RUN pnpm run build

# Production image — uses the target platform (amd64 or arm64)
FROM node:26-alpine AS ui
WORKDIR /app

ENV NODE_ENV=production

# Create non-root user for security
RUN addgroup --system --gid 1001 nodejs && \
    adduser --system --uid 1001 nextjs

# Copy standalone build artifacts
COPY --from=builder /app/public ./public
COPY --from=builder --chown=nextjs:nodejs /app/.next/standalone ./
COPY --from=builder --chown=nextjs:nodejs /app/.next/static ./.next/static

# Note: Stage renamed from 'runner' to 'ui' for consistency with other images
USER nextjs

EXPOSE 9100

ENV PORT=9100
ENV HOSTNAME="0.0.0.0"

CMD ["node", "server.js"]
