FROM ghcr.io/foundry-rs/foundry:stable AS foundry

RUN cp "$(command -v cast)" /tmp/cast

FROM node:24-bookworm-slim AS build

WORKDIR /app

COPY package.json package-lock.json ./
COPY apps/indexer/package.json apps/indexer/package.json
COPY apps/rpc-gateway/package.json apps/rpc-gateway/package.json
COPY apps/web/package.json apps/web/package.json
COPY packages/canister-clients/package.json packages/canister-clients/package.json
COPY packages/shared/package.json packages/shared/package.json

RUN npm ci

COPY tsconfig.base.json ./
COPY packages/canister-clients packages/canister-clients
COPY packages/shared packages/shared
COPY apps/indexer apps/indexer
COPY scripts scripts
COPY scripts/seed-local-wallet.mjs scripts/seed-local-wallet.mjs

RUN npm run build --workspace @ic-automaton/shared \
  && npm run build --workspace @ic-automaton/indexer \
  && npm prune --omit=dev

FROM node:24-bookworm-slim AS runtime

WORKDIR /app

ENV NODE_ENV=production

COPY --from=build /app/package.json /app/package-lock.json ./
COPY --from=build /app/node_modules ./node_modules
COPY --from=build /app/apps/indexer/package.json ./apps/indexer/package.json
COPY --from=build /app/apps/indexer/dist ./apps/indexer/dist
COPY --from=build /app/apps/indexer/src/store/schema.sql ./apps/indexer/src/store/schema.sql
COPY --from=build /app/apps/rpc-gateway/package.json ./apps/rpc-gateway/package.json
COPY --from=build /app/apps/web/package.json ./apps/web/package.json
COPY --from=build /app/packages/canister-clients/package.json ./packages/canister-clients/package.json
COPY --from=build /app/packages/canister-clients/src ./packages/canister-clients/src
COPY --from=build /app/packages/shared/package.json ./packages/shared/package.json
COPY --from=build /app/packages/shared/dist ./packages/shared/dist
COPY --from=build /app/scripts/seed-local-wallet.mjs ./scripts/seed-local-wallet.mjs
COPY --from=foundry /tmp/cast /usr/local/bin/cast

EXPOSE 3001

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=5 \
  CMD node -e "fetch('http://127.0.0.1:3001/health').then((response) => process.exit(response.ok ? 0 : 1)).catch(() => process.exit(1))"

CMD ["node", "apps/indexer/dist/server.js"]
