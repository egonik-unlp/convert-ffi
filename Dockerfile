# syntax=docker/dockerfile:1

# ---- Build stage: Rust + Zig + cargo-leptos ----
FROM rust:1-bookworm AS builder

ARG ZIG_VERSION=0.14.0
ARG CARGO_LEPTOS_VERSION=0.3.6

# Zig (for the convert-songs FFI .so, built by build.rs via `zig build`).
RUN apt-get update && apt-get install -y --no-install-recommends curl xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && curl -fsSL "https://ziglang.org/download/${ZIG_VERSION}/zig-linux-x86_64-${ZIG_VERSION}.tar.xz" \
        | tar -xJ -C /usr/local \
    && ln -s "/usr/local/zig-linux-x86_64-${ZIG_VERSION}/zig" /usr/local/bin/zig

# Leptos client target + the cargo-leptos build orchestrator.
RUN rustup target add wasm32-unknown-unknown \
    && cargo install cargo-leptos --version "${CARGO_LEPTOS_VERSION}" --locked

WORKDIR /app
# Whole context, including the convert-songs submodule (must be a real dir, not
# the dev symlink — see CI checkout with submodules: recursive).
COPY . .

# Produces target/release/convert-ffi (server bin), target/site (static assets),
# and convert-songs/zig-out/lib/libconvert-rs.so (build.rs runs `zig build`).
RUN cargo leptos build --release

# ---- Runtime stage ----
FROM debian:bookworm-slim AS runtime

# ca-certificates is REQUIRED: Zig's TLS client needs the system CA bundle to
# reach accounts.spotify.com / api.spotify.com.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/convert-ffi /app/convert-ffi
COPY --from=builder /app/target/site /app/site
COPY --from=builder /app/convert-songs/zig-out/lib/libconvert-rs.so /app/lib/

# The build-time rpath points at the builder's absolute deps dir, which does not
# exist here — resolve the .so via LD_LIBRARY_PATH instead.
ENV LD_LIBRARY_PATH=/app/lib
# get_configuration(None) has no Cargo.toml at runtime, so it reads these env vars.
ENV LEPTOS_OUTPUT_NAME=convert-ffi \
    LEPTOS_SITE_ROOT=site \
    LEPTOS_SITE_PKG_DIR=pkg \
    LEPTOS_SITE_ADDR=0.0.0.0:8080 \
    LEPTOS_ENV=PROD \
    PORT=8080
# client_id / client_secret / REDIRECT_URI are injected by the host (Render).

EXPOSE 8080
CMD ["/app/convert-ffi"]
