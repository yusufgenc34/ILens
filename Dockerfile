# syntax=docker/dockerfile:1
FROM rust:1.92.0-slim-trixie AS wasm
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown \
    && cargo install wasm-bindgen-cli --version 0.2.105 --locked
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo .cargo/
COPY crates crates/
RUN cargo build --locked --release --target wasm32-unknown-unknown -p decompiler-wasm \
    && wasm-bindgen --target web --out-dir /wasm --out-name decompiler \
       target/wasm32-unknown-unknown/release/decompiler_wasm.wasm \
    && sed -i "1i'use client';" /wasm/decompiler.js

FROM node:22.23.1-trixie-slim AS node-base
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3t64 libfontconfig1 libfreetype6 \
    && rm -rf /var/lib/apt/lists/*

FROM node-base AS dependencies
WORKDIR /app
COPY package.json package-lock.json .npmrc ./
RUN npm ci

FROM dependencies AS app-build
COPY . .
COPY --from=wasm /wasm/ src/wasm/
RUN npm run build:app \
    && cp node_modules/rari-linux-*/bin/rari /usr/local/bin/rari \
    && npm prune --omit=dev

FROM node-base AS runtime
ENV NODE_ENV=production RUST_LOG=warn
WORKDIR /app
COPY --from=app-build --chown=node:node /app/dist ./dist
COPY --from=app-build --chown=node:node /app/node_modules ./node_modules
COPY --from=app-build --chown=node:node /app/package.json ./package.json
COPY --from=app-build /usr/local/bin/rari /usr/local/bin/rari
USER node
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD node -e "fetch('http://127.0.0.1:3000/', {signal: AbortSignal.timeout(4000)}).then(r => process.exit(r.ok ? 0 : 1)).catch(() => process.exit(1))"
# The Rari CLI binds loopback outside its named hosting platforms. Run its
# packaged native server directly so Docker can explicitly bind all interfaces.
CMD ["rari", "--mode", "production", "--host", "0.0.0.0", "--port", "3000"]
