# Build stage
FROM rust:1.75 as builder

WORKDIR /app
COPY . .
RUN cargo build --release --bin sync-server

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    postgresql-client \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sync-server /usr/local/bin/
COPY --from=builder /app/sync-server/migrations /migrations

EXPOSE 8080

CMD ["sync-server"]