FROM rust:1.85-slim AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/roxy /usr/local/bin/roxy

EXPOSE 8080

ENV RUST_LOG=info
ENV ROXY_BIND=0.0.0.0:8080

CMD ["roxy"]
