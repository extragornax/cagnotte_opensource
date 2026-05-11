FROM rust:1-slim-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY templates/ templates/
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget && rm -rf /var/lib/apt/lists/*
RUN useradd --system --no-create-home --uid 1000 app
COPY --from=builder /app/target/release/cagnotte /usr/local/bin/
RUN mkdir -p /data && chown app:app /data
USER app
WORKDIR /data
ENV PORT=9021 DB_PATH=/data/cagnotte.db RUST_LOG=info,cagnotte=info
EXPOSE 9021
CMD ["cagnotte"]
