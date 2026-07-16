FROM node:24-bookworm-slim AS build

WORKDIR /app

COPY package.json package-lock.json ./
COPY apps/indexer/package.json apps/indexer/package.json
COPY apps/rpc-gateway/package.json apps/rpc-gateway/package.json
COPY apps/web/package.json apps/web/package.json
COPY packages/canister-clients/package.json packages/canister-clients/package.json
COPY packages/shared/package.json packages/shared/package.json
COPY packages/ui/package.json packages/ui/package.json

RUN npm ci

COPY tsconfig.base.json ./
COPY packages/canister-clients packages/canister-clients
COPY packages/shared packages/shared
COPY packages/ui packages/ui
COPY apps/web apps/web
COPY scripts scripts

ARG VITE_INDEXER_BASE_URL=
ARG VITE_SPAWN_CHAIN_NAME=
ARG VITE_SPAWN_CHAIN_RPC_URL=
ARG VITE_SPAWN_USDC_CONTRACT_ADDRESS=

ENV VITE_INDEXER_BASE_URL=${VITE_INDEXER_BASE_URL}
ENV VITE_SPAWN_CHAIN_NAME=${VITE_SPAWN_CHAIN_NAME}
ENV VITE_SPAWN_CHAIN_RPC_URL=${VITE_SPAWN_CHAIN_RPC_URL}
ENV VITE_SPAWN_USDC_CONTRACT_ADDRESS=${VITE_SPAWN_USDC_CONTRACT_ADDRESS}

RUN npm run build --workspace @ic-automaton/shared \
  && npm run build --workspace @ic-automaton/web

FROM nginx:1.29-alpine AS runtime

RUN rm /etc/nginx/conf.d/default.conf \
  && printf '%s\n' \
    'server {' \
    '  listen 3000;' \
    '  server_name _;' \
    '  root /usr/share/nginx/html;' \
    '  index index.html;' \
    '  location / {' \
    '    try_files $uri $uri/ /index.html;' \
    '  }' \
    '}' > /etc/nginx/conf.d/default.conf

COPY --from=build /app/apps/web/dist /usr/share/nginx/html

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD wget -q -O /dev/null http://127.0.0.1:3000/ || exit 1
