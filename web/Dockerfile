# jellybridge.oddbench.co: one binary with every page, font and image compiled in.
# Stateless: no volume, no outbound calls. Build context is this directory (web/).

FROM rust:1.98-slim-bookworm AS build
WORKDIR /app

# Dependencies first, so a copy or style change does not recompile axum and friends.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
 && echo 'fn main() {}' > src/main.rs \
 && cargo build --release --locked \
 && rm -rf src target/release/jellybridge-site* target/release/deps/jellybridge_site* target/release/.fingerprint/jellybridge-site-*

COPY build.rs ./
COPY src src
COPY templates templates
COPY static static
RUN cargo build --release --locked

# distroless/cc: glibc and nothing else (no shell, no package manager); runs as uid 65532.
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /app/target/release/jellybridge-site /usr/local/bin/jellybridge-site
ENV PORT=8080 \
    SITE_URL=https://jellybridge.oddbench.co \
    RUST_LOG=info
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/jellybridge-site"]
